//! Statement executor: turns a parsed [`Statement`] into storage operations.

use core::cmp::Ordering;

use crate::catalog::{Catalog, Column, TableSchema};
use crate::error::{Error, Result};
use crate::heap;
use crate::sql::ast::*;
use crate::storage::Storage;
use crate::value::{DataType, Value};

/// The outcome of executing a statement.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecResult {
    /// A DDL/DML statement; carries the number of affected rows.
    Affected(usize),
    /// A `SELECT` result set.
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}

impl ExecResult {
    /// True when the statement may have changed catalog metadata and the
    /// catalog page should be flushed.
    pub fn is_mutation(&self) -> bool {
        matches!(self, ExecResult::Affected(_))
    }
}

/// Executes one statement against the given catalog and storage.
pub fn execute(
    catalog: &mut Catalog,
    storage: &mut dyn Storage,
    mut stmt: Statement,
    params: &[Value],
) -> Result<ExecResult> {
    bind_params(&mut stmt, params)?;
    match stmt {
        Statement::CreateTable(c) => exec_create_table(catalog, c),
        Statement::Insert(i) => exec_insert(catalog, storage, i),
        Statement::Select(s) => exec_select(catalog, storage, s),
        Statement::Update(u) => exec_update(catalog, storage, u),
        Statement::Delete(d) => exec_delete(catalog, storage, d),
    }
}

// --- Parameter binding -----------------------------------------------------

/// Replaces `?` placeholders with concrete values in textual order.
fn bind_params(stmt: &mut Statement, params: &[Value]) -> Result<()> {
    let mut idx = 0usize;
    match stmt {
        Statement::CreateTable(_) => {}
        Statement::Insert(i) => {
            for row in &mut i.rows {
                for e in row {
                    bind_expr(e, params, &mut idx)?;
                }
            }
        }
        Statement::Select(s) => {
            if let Some(f) = &mut s.filter {
                bind_expr(f, params, &mut idx)?;
            }
        }
        Statement::Update(u) => {
            for (_, e) in &mut u.assignments {
                bind_expr(e, params, &mut idx)?;
            }
            if let Some(f) = &mut u.filter {
                bind_expr(f, params, &mut idx)?;
            }
        }
        Statement::Delete(d) => {
            if let Some(f) = &mut d.filter {
                bind_expr(f, params, &mut idx)?;
            }
        }
    }
    if idx != params.len() {
        return Err(Error::Parameter(format!(
            "statement uses {idx} parameter(s) but {} were provided",
            params.len()
        )));
    }
    Ok(())
}

fn bind_expr(expr: &mut Expr, params: &[Value], idx: &mut usize) -> Result<()> {
    match expr {
        Expr::Param => {
            let v = params.get(*idx).cloned().ok_or_else(|| {
                Error::Parameter(format!("missing value for parameter #{}", *idx + 1))
            })?;
            *idx += 1;
            *expr = Expr::Literal(v);
            Ok(())
        }
        Expr::Binary { left, right, .. } => {
            bind_expr(left, params, idx)?;
            bind_expr(right, params, idx)
        }
        Expr::Literal(_) | Expr::Column(_) => Ok(()),
    }
}

// --- CREATE TABLE ----------------------------------------------------------

fn exec_create_table(catalog: &mut Catalog, c: CreateTable) -> Result<ExecResult> {
    if catalog.contains(&c.name) {
        if c.if_not_exists {
            return Ok(ExecResult::Affected(0));
        }
        return Err(Error::Catalog(format!("table '{}' already exists", c.name)));
    }

    let mut seen: Vec<String> = Vec::new();
    let mut pk_count = 0;
    let mut columns = Vec::with_capacity(c.columns.len());
    for def in c.columns {
        let lower = def.name.to_ascii_lowercase();
        if seen.contains(&lower) {
            return Err(Error::Catalog(format!(
                "duplicate column name '{}'",
                def.name
            )));
        }
        seen.push(lower);
        if def.primary_key {
            pk_count += 1;
        }
        columns.push(Column {
            name: def.name,
            data_type: def.data_type,
            primary_key: def.primary_key,
        });
    }
    if pk_count > 1 {
        return Err(Error::Catalog(
            "only a single-column PRIMARY KEY is supported".into(),
        ));
    }

    catalog.add_table(TableSchema {
        name: c.name,
        columns,
        first_page: 0,
        last_page: 0,
    })?;
    Ok(ExecResult::Affected(0))
}

// --- INSERT ----------------------------------------------------------------

fn exec_insert(catalog: &mut Catalog, storage: &mut dyn Storage, ins: Insert) -> Result<ExecResult> {
    let schema = catalog
        .get(&ins.table)
        .ok_or_else(|| Error::Catalog(format!("no such table '{}'", ins.table)))?
        .clone();

    // Map explicit column list (if any) to positions in the schema.
    let positions: Vec<usize> = match &ins.columns {
        Some(cols) => cols
            .iter()
            .map(|name| {
                schema
                    .column_index(name)
                    .ok_or_else(|| Error::Catalog(format!("no such column '{name}'")))
            })
            .collect::<Result<_>>()?,
        None => (0..schema.columns.len()).collect(),
    };

    let mut count = 0;
    for row_exprs in ins.rows {
        if row_exprs.len() != positions.len() {
            return Err(Error::Type(format!(
                "row has {} value(s) but {} column(s) were targeted",
                row_exprs.len(),
                positions.len()
            )));
        }

        let mut values = vec![Value::Null; schema.columns.len()];
        for (expr, &pos) in row_exprs.iter().zip(positions.iter()) {
            let v = eval_const(expr)?;
            values[pos] = v.coerce_to(schema.columns[pos].data_type)?;
        }

        // Primary-key constraints.
        if let Some(pk) = schema.primary_key_index() {
            if values[pk].is_null() {
                return Err(Error::Constraint(format!(
                    "primary key '{}' cannot be NULL",
                    schema.columns[pk].name
                )));
            }
            if pk_exists(storage, &schema, pk, &values[pk], None)? {
                return Err(Error::Constraint(format!(
                    "duplicate primary key value in column '{}'",
                    schema.columns[pk].name
                )));
            }
        }

        let schema_mut = catalog.get_mut(&ins.table).expect("table exists");
        heap::insert_row(storage, schema_mut, &values)?;
        count += 1;
    }
    Ok(ExecResult::Affected(count))
}

// --- SELECT ----------------------------------------------------------------

fn exec_select(catalog: &Catalog, storage: &dyn Storage, sel: Select) -> Result<ExecResult> {
    let schema = catalog
        .get(&sel.table)
        .ok_or_else(|| Error::Catalog(format!("no such table '{}'", sel.table)))?;

    let out_indices: Vec<usize> = match &sel.projection {
        Projection::All => (0..schema.columns.len()).collect(),
        Projection::Columns(cols) => cols
            .iter()
            .map(|name| {
                schema
                    .column_index(name)
                    .ok_or_else(|| Error::Catalog(format!("no such column '{name}'")))
            })
            .collect::<Result<_>>()?,
    };
    let columns: Vec<String> = out_indices
        .iter()
        .map(|&i| schema.columns[i].name.clone())
        .collect();

    let mut rows = Vec::new();
    for located in heap::scan(storage, schema)? {
        if let Some(filter) = &sel.filter {
            if !filter_matches(filter, &located.values, schema)? {
                continue;
            }
        }
        let projected = out_indices
            .iter()
            .map(|&i| located.values[i].clone())
            .collect();
        rows.push(projected);
    }
    Ok(ExecResult::Rows { columns, rows })
}

// --- UPDATE ----------------------------------------------------------------

fn exec_update(catalog: &mut Catalog, storage: &mut dyn Storage, upd: Update) -> Result<ExecResult> {
    let schema = catalog
        .get(&upd.table)
        .ok_or_else(|| Error::Catalog(format!("no such table '{}'", upd.table)))?
        .clone();

    // Resolve assignment targets up front.
    let mut targets = Vec::with_capacity(upd.assignments.len());
    for (name, expr) in &upd.assignments {
        let idx = schema
            .column_index(name)
            .ok_or_else(|| Error::Catalog(format!("no such column '{name}'")))?;
        let value = eval_const(expr)?.coerce_to(schema.columns[idx].data_type)?;
        targets.push((idx, value));
    }

    let mut count = 0;
    for located in heap::scan(storage, &schema)? {
        let matched = match &upd.filter {
            Some(f) => filter_matches(f, &located.values, &schema)?,
            None => true,
        };
        if !matched {
            continue;
        }
        let mut new_values = located.values.clone();
        for (idx, value) in &targets {
            new_values[*idx] = value.clone();
        }
        if let Some(pk) = schema.primary_key_index() {
            if new_values[pk].is_null() {
                return Err(Error::Constraint(format!(
                    "primary key '{}' cannot be NULL",
                    schema.columns[pk].name
                )));
            }
            if pk_exists(storage, &schema, pk, &new_values[pk], Some(located.location))? {
                return Err(Error::Constraint(format!(
                    "duplicate primary key value in column '{}'",
                    schema.columns[pk].name
                )));
            }
        }
        heap::update_row(storage, located.location, &new_values)?;
        count += 1;
    }
    Ok(ExecResult::Affected(count))
}

// --- DELETE ----------------------------------------------------------------

fn exec_delete(catalog: &mut Catalog, storage: &mut dyn Storage, del: Delete) -> Result<ExecResult> {
    let schema = catalog
        .get(&del.table)
        .ok_or_else(|| Error::Catalog(format!("no such table '{}'", del.table)))?
        .clone();

    let mut count = 0;
    for located in heap::scan(storage, &schema)? {
        let matched = match &del.filter {
            Some(f) => filter_matches(f, &located.values, &schema)?,
            None => true,
        };
        if matched {
            heap::delete_row(storage, located.location)?;
            count += 1;
        }
    }
    Ok(ExecResult::Affected(count))
}

// --- Expression evaluation -------------------------------------------------

/// Evaluates an expression that must not reference any columns (INSERT values,
/// UPDATE right-hand sides).
fn eval_const(expr: &Expr) -> Result<Value> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param => Err(Error::Parameter("unbound parameter".into())),
        Expr::Column(name) => Err(Error::Type(format!(
            "column reference '{name}' is not allowed here"
        ))),
        Expr::Binary { .. } => Err(Error::Type(
            "expressions are not supported in this position".into(),
        )),
    }
}

fn filter_matches(expr: &Expr, row: &[Value], schema: &TableSchema) -> Result<bool> {
    Ok(matches!(eval_predicate(expr, row, schema)?, Value::Boolean(true)))
}

fn eval_predicate(expr: &Expr, row: &[Value], schema: &TableSchema) -> Result<Value> {
    match expr {
        Expr::Literal(v) => Ok(v.clone()),
        Expr::Param => Err(Error::Parameter("unbound parameter".into())),
        Expr::Column(name) => {
            let idx = schema
                .column_index(name)
                .ok_or_else(|| Error::Catalog(format!("no such column '{name}'")))?;
            Ok(row[idx].clone())
        }
        Expr::Binary { left, op, right } => {
            let l = eval_predicate(left, row, schema)?;
            let r = eval_predicate(right, row, schema)?;
            Ok(eval_binop(*op, l, r))
        }
    }
}

fn truthy(v: &Value) -> bool {
    matches!(v, Value::Boolean(true))
}

fn eval_binop(op: BinOp, l: Value, r: Value) -> Value {
    match op {
        BinOp::And => Value::Boolean(truthy(&l) && truthy(&r)),
        BinOp::Or => Value::Boolean(truthy(&l) || truthy(&r)),
        _ => match compare_values(&l, &r) {
            None => Value::Null,
            Some(ord) => Value::Boolean(match op {
                BinOp::Eq => ord == Ordering::Equal,
                BinOp::NotEq => ord != Ordering::Equal,
                BinOp::Lt => ord == Ordering::Less,
                BinOp::LtEq => ord != Ordering::Greater,
                BinOp::Gt => ord == Ordering::Greater,
                BinOp::GtEq => ord != Ordering::Less,
                BinOp::And | BinOp::Or => unreachable!(),
            }),
        },
    }
}

/// Total-ish ordering across comparable values. Returns `None` when either
/// side is NULL or the types are not comparable (SQL NULL semantics).
fn compare_values(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Null, _) | (_, Value::Null) => None,
        (Value::Integer(x), Value::Integer(y)) => Some(x.cmp(y)),
        (Value::Text(x), Value::Text(y)) => Some(x.cmp(y)),
        (Value::Boolean(x), Value::Boolean(y)) => Some(x.cmp(y)),
        (Value::Boolean(x), Value::Integer(y)) => Some((*x as i64).cmp(y)),
        (Value::Integer(x), Value::Boolean(y)) => Some(x.cmp(&(*y as i64))),
        _ => None,
    }
}

/// Checks whether a primary-key value already exists, optionally ignoring the
/// row at `skip` (used by UPDATE so a row can keep its own key).
fn pk_exists(
    storage: &dyn Storage,
    schema: &TableSchema,
    pk: usize,
    value: &Value,
    skip: Option<heap::RowLocation>,
) -> Result<bool> {
    for located in heap::scan(storage, schema)? {
        if Some(located.location) == skip {
            continue;
        }
        if compare_values(&located.values[pk], value) == Some(Ordering::Equal) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Convenience for the CLI/tests: a stringified column type list.
pub fn describe_columns(cols: &[Column]) -> Vec<(String, DataType)> {
    cols.iter().map(|c| (c.name.clone(), c.data_type)).collect()
}
