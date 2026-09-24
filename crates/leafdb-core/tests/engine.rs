//! End-to-end tests for the leafdb engine, including the ADR-0001 success
//! criteria (CREATE/INSERT/SELECT and image persistence).

use leafdb_core::{Database, ExecResult, Value};

fn rows(result: ExecResult) -> Vec<Vec<Value>> {
    match result {
        ExecResult::Rows { rows, .. } => rows,
        other => panic!("expected rows, got {other:?}"),
    }
}

fn affected(result: ExecResult) -> usize {
    match result {
        ExecResult::Affected(n) => n,
        other => panic!("expected affected count, got {other:?}"),
    }
}

fn seed_todos() -> Database {
    let mut db = Database::create().unwrap();
    db.execute_str(
        "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT, done BOOLEAN)",
    )
    .unwrap();
    db
}

#[test]
fn success_criteria_from_adr() {
    let mut db = seed_todos();

    // INSERT ... VALUES (1, 'Build a database', false)
    let n = affected(
        db.execute_str("INSERT INTO todos VALUES (1, 'Build a database', false)")
            .unwrap(),
    );
    assert_eq!(n, 1);

    // SELECT * FROM todos WHERE done = false
    let result = db
        .execute_str("SELECT * FROM todos WHERE done = false")
        .unwrap();
    let out = rows(result);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0],
        vec![
            Value::Integer(1),
            Value::Text("Build a database".into()),
            Value::Boolean(false),
        ]
    );
}

#[test]
fn columns_are_reported_for_select_star() {
    let mut db = seed_todos();
    db.execute_str("INSERT INTO todos VALUES (1, 'x', true)")
        .unwrap();
    match db.execute_str("SELECT * FROM todos").unwrap() {
        ExecResult::Rows { columns, .. } => {
            assert_eq!(columns, vec!["id", "title", "done"]);
        }
        other => panic!("expected rows, got {other:?}"),
    }
}

#[test]
fn parameters_bind_in_order() {
    let mut db = seed_todos();
    db.execute(
        "INSERT INTO todos VALUES (?, ?, ?)",
        &[
            Value::Integer(1),
            Value::Text("Build a database".into()),
            Value::Boolean(false),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO todos VALUES (?, ?, ?)",
        &[
            Value::Integer(2),
            Value::Text("Ship it".into()),
            Value::Boolean(true),
        ],
    )
    .unwrap();

    let out = rows(
        db.execute("SELECT title FROM todos WHERE done = ?", &[Value::Boolean(false)])
            .unwrap(),
    );
    assert_eq!(out, vec![vec![Value::Text("Build a database".into())]]);
}

#[test]
fn wrong_parameter_count_is_rejected() {
    let mut db = seed_todos();
    let err = db
        .execute("INSERT INTO todos VALUES (?, ?, ?)", &[Value::Integer(1)])
        .unwrap_err();
    assert!(matches!(err, leafdb_core::Error::Parameter(_)));
}

#[test]
fn persistence_round_trip_via_image() {
    let mut db = seed_todos();
    for i in 1..=3 {
        db.execute(
            "INSERT INTO todos VALUES (?, ?, ?)",
            &[
                Value::Integer(i),
                Value::Text(format!("task {i}")),
                Value::Boolean(i % 2 == 0),
            ],
        )
        .unwrap();
    }
    let image = db.export_image();

    // Reopen from the exported bytes, as the browser would after reload.
    let mut reopened = Database::open_image(image).unwrap();
    let out = rows(reopened.execute_str("SELECT id, title, done FROM todos").unwrap());
    assert_eq!(out.len(), 3);
    assert_eq!(out[0][1], Value::Text("task 1".into()));

    // Data is writable after reopening.
    reopened
        .execute_str("INSERT INTO todos VALUES (4, 'task 4', true)")
        .unwrap();
    let out = rows(reopened.execute_str("SELECT * FROM todos").unwrap());
    assert_eq!(out.len(), 4);
}

#[test]
fn primary_key_uniqueness_is_enforced() {
    let mut db = seed_todos();
    db.execute_str("INSERT INTO todos VALUES (1, 'a', false)")
        .unwrap();
    let err = db
        .execute_str("INSERT INTO todos VALUES (1, 'b', false)")
        .unwrap_err();
    assert!(matches!(err, leafdb_core::Error::Constraint(_)));
}

#[test]
fn null_primary_key_is_rejected() {
    let mut db = seed_todos();
    let err = db
        .execute_str("INSERT INTO todos VALUES (NULL, 'a', false)")
        .unwrap_err();
    assert!(matches!(err, leafdb_core::Error::Constraint(_)));
}

#[test]
fn insert_with_named_columns_defaults_rest_to_null() {
    let mut db = seed_todos();
    db.execute_str("INSERT INTO todos (id, title) VALUES (7, 'partial')")
        .unwrap();
    let out = rows(db.execute_str("SELECT id, title, done FROM todos").unwrap());
    assert_eq!(
        out[0],
        vec![
            Value::Integer(7),
            Value::Text("partial".into()),
            Value::Null,
        ]
    );
}

#[test]
fn update_and_delete() {
    let mut db = seed_todos();
    for i in 1..=3 {
        db.execute(
            "INSERT INTO todos VALUES (?, ?, ?)",
            &[Value::Integer(i), Value::Text(format!("t{i}")), Value::Boolean(false)],
        )
        .unwrap();
    }

    let updated = affected(
        db.execute("UPDATE todos SET done = ? WHERE id = ?", &[Value::Boolean(true), Value::Integer(2)])
            .unwrap(),
    );
    assert_eq!(updated, 1);

    let done_rows = rows(
        db.execute("SELECT id FROM todos WHERE done = ?", &[Value::Boolean(true)])
            .unwrap(),
    );
    assert_eq!(done_rows, vec![vec![Value::Integer(2)]]);

    let deleted = affected(db.execute_str("DELETE FROM todos WHERE id = 1").unwrap());
    assert_eq!(deleted, 1);
    let remaining = rows(db.execute_str("SELECT id FROM todos").unwrap());
    assert_eq!(remaining.len(), 2);
}

#[test]
fn where_supports_and_or_and_comparisons() {
    let mut db = Database::create().unwrap();
    db.execute_str("CREATE TABLE nums (id INTEGER PRIMARY KEY, n INTEGER)")
        .unwrap();
    for i in 1..=10 {
        db.execute(
            "INSERT INTO nums VALUES (?, ?)",
            &[Value::Integer(i), Value::Integer(i * 10)],
        )
        .unwrap();
    }
    let out = rows(
        db.execute_str("SELECT id FROM nums WHERE n > 30 AND n <= 60")
            .unwrap(),
    );
    let ids: Vec<i64> = out
        .into_iter()
        .map(|r| match r[0] {
            Value::Integer(i) => i,
            _ => panic!("expected int"),
        })
        .collect();
    assert_eq!(ids, vec![4, 5, 6]);

    let out = rows(
        db.execute_str("SELECT id FROM nums WHERE id = 1 OR id = 9")
            .unwrap(),
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn rows_spill_across_multiple_pages() {
    // With a 4 KiB page, a few hundred rows must span more than one page,
    // exercising heap page allocation and chaining.
    let mut db = Database::create().unwrap();
    db.execute_str("CREATE TABLE big (id INTEGER PRIMARY KEY, payload TEXT)")
        .unwrap();
    let count = 500;
    for i in 0..count {
        db.execute(
            "INSERT INTO big VALUES (?, ?)",
            &[Value::Integer(i), Value::Text(format!("row-{i}-{}", "x".repeat(20)))],
        )
        .unwrap();
    }
    let out = rows(db.execute_str("SELECT id FROM big").unwrap());
    assert_eq!(out.len(), count as usize);

    // Survives a persistence round trip too.
    let image = db.export_image();
    let mut reopened = Database::open_image(image).unwrap();
    let out = rows(
        reopened
            .execute("SELECT payload FROM big WHERE id = ?", &[Value::Integer(499)])
            .unwrap(),
    );
    assert_eq!(out.len(), 1);
}

#[test]
fn type_mismatch_is_rejected() {
    let mut db = seed_todos();
    let err = db
        .execute_str("INSERT INTO todos VALUES ('not-an-int', 'x', false)")
        .unwrap_err();
    assert!(matches!(err, leafdb_core::Error::Type(_)));
}

#[test]
fn create_if_not_exists_is_idempotent() {
    let mut db = seed_todos();
    // Second create without IF NOT EXISTS fails.
    assert!(db
        .execute_str("CREATE TABLE todos (id INTEGER)")
        .is_err());
    // With IF NOT EXISTS it is a no-op.
    db.execute_str("CREATE TABLE IF NOT EXISTS todos (id INTEGER)")
        .unwrap();
}
