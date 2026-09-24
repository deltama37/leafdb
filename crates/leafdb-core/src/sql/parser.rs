//! Recursive-descent parser for the supported SQL subset.

use crate::error::{Error, Result};
use crate::sql::ast::*;
use crate::sql::token::{tokenize, Token};
use crate::value::{DataType, Value};

/// Parses a single SQL statement.
pub fn parse(sql: &str) -> Result<Statement> {
    let tokens = tokenize(sql)?;
    let mut p = Parser { tokens, pos: 0 };
    let stmt = p.parse_statement()?;
    p.skip_optional(&Token::Semicolon);
    if p.pos != p.tokens.len() {
        return Err(Error::Parse(
            "unexpected trailing tokens after statement".into(),
        ));
    }
    Ok(stmt)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Result<Token> {
        let t = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or_else(|| Error::Parse("unexpected end of input".into()))?;
        self.pos += 1;
        Ok(t)
    }

    fn skip_optional(&mut self, tok: &Token) {
        if self.peek() == Some(tok) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, tok: &Token) -> Result<()> {
        let got = self.next()?;
        if &got == tok {
            Ok(())
        } else {
            Err(Error::Parse(format!("expected {tok:?}, found {got:?}")))
        }
    }

    /// Consumes an identifier/word (any word, including keywords used as names).
    fn expect_word(&mut self) -> Result<String> {
        match self.next()? {
            Token::Word(w) => Ok(w),
            other => Err(Error::Parse(format!("expected identifier, found {other:?}"))),
        }
    }

    /// Returns true and advances if the next token is the given keyword.
    fn eat_keyword(&mut self, kw: &str) -> bool {
        if let Some(Token::Word(w)) = self.peek() {
            if w.eq_ignore_ascii_case(kw) {
                self.pos += 1;
                return true;
            }
        }
        false
    }

    fn expect_keyword(&mut self, kw: &str) -> Result<()> {
        if self.eat_keyword(kw) {
            Ok(())
        } else {
            Err(Error::Parse(format!(
                "expected keyword '{kw}', found {:?}",
                self.peek()
            )))
        }
    }

    fn parse_statement(&mut self) -> Result<Statement> {
        match self.peek() {
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("CREATE") => {
                Ok(Statement::CreateTable(self.parse_create_table()?))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("INSERT") => {
                Ok(Statement::Insert(self.parse_insert()?))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("SELECT") => {
                Ok(Statement::Select(self.parse_select()?))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("UPDATE") => {
                Ok(Statement::Update(self.parse_update()?))
            }
            Some(Token::Word(w)) if w.eq_ignore_ascii_case("DELETE") => {
                Ok(Statement::Delete(self.parse_delete()?))
            }
            other => Err(Error::Parse(format!(
                "unsupported or unknown statement start: {other:?}"
            ))),
        }
    }

    fn parse_create_table(&mut self) -> Result<CreateTable> {
        self.expect_keyword("CREATE")?;
        self.expect_keyword("TABLE")?;
        let if_not_exists = if self.eat_keyword("IF") {
            self.expect_keyword("NOT")?;
            self.expect_keyword("EXISTS")?;
            true
        } else {
            false
        };
        let name = self.expect_word()?;
        self.expect(&Token::LParen)?;
        let mut columns = Vec::new();
        loop {
            let col_name = self.expect_word()?;
            let type_kw = self.expect_word()?;
            let data_type = DataType::from_keyword(&type_kw)
                .ok_or_else(|| Error::Parse(format!("unknown column type '{type_kw}'")))?;
            let mut primary_key = false;
            // Optional column constraints.
            loop {
                if self.eat_keyword("PRIMARY") {
                    self.expect_keyword("KEY")?;
                    primary_key = true;
                } else if self.eat_keyword("NOT") {
                    self.expect_keyword("NULL")?;
                } else {
                    break;
                }
            }
            columns.push(ColumnDef {
                name: col_name,
                data_type,
                primary_key,
            });
            match self.next()? {
                Token::Comma => continue,
                Token::RParen => break,
                other => {
                    return Err(Error::Parse(format!(
                        "expected ',' or ')' in column list, found {other:?}"
                    )))
                }
            }
        }
        if columns.is_empty() {
            return Err(Error::Parse("table must have at least one column".into()));
        }
        Ok(CreateTable {
            name,
            if_not_exists,
            columns,
        })
    }

    fn parse_insert(&mut self) -> Result<Insert> {
        self.expect_keyword("INSERT")?;
        self.expect_keyword("INTO")?;
        let table = self.expect_word()?;
        let columns = if self.peek() == Some(&Token::LParen) {
            self.expect(&Token::LParen)?;
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_word()?);
                match self.next()? {
                    Token::Comma => continue,
                    Token::RParen => break,
                    other => {
                        return Err(Error::Parse(format!(
                            "expected ',' or ')' in column list, found {other:?}"
                        )))
                    }
                }
            }
            Some(cols)
        } else {
            None
        };
        self.expect_keyword("VALUES")?;
        let mut rows = Vec::new();
        loop {
            self.expect(&Token::LParen)?;
            let mut row = Vec::new();
            loop {
                row.push(self.parse_expr()?);
                match self.next()? {
                    Token::Comma => continue,
                    Token::RParen => break,
                    other => {
                        return Err(Error::Parse(format!(
                            "expected ',' or ')' in value list, found {other:?}"
                        )))
                    }
                }
            }
            rows.push(row);
            if self.peek() == Some(&Token::Comma) {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(Insert {
            table,
            columns,
            rows,
        })
    }

    fn parse_select(&mut self) -> Result<Select> {
        self.expect_keyword("SELECT")?;
        let projection = if self.peek() == Some(&Token::Star) {
            self.pos += 1;
            Projection::All
        } else {
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_word()?);
                if self.peek() == Some(&Token::Comma) {
                    self.pos += 1;
                    continue;
                }
                break;
            }
            Projection::Columns(cols)
        };
        self.expect_keyword("FROM")?;
        let table = self.expect_word()?;
        let filter = if self.eat_keyword("WHERE") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Select {
            projection,
            table,
            filter,
        })
    }

    fn parse_update(&mut self) -> Result<Update> {
        self.expect_keyword("UPDATE")?;
        let table = self.expect_word()?;
        self.expect_keyword("SET")?;
        let mut assignments = Vec::new();
        loop {
            let col = self.expect_word()?;
            self.expect(&Token::Eq)?;
            let value = self.parse_expr()?;
            assignments.push((col, value));
            if self.peek() == Some(&Token::Comma) {
                self.pos += 1;
                continue;
            }
            break;
        }
        let filter = if self.eat_keyword("WHERE") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Update {
            table,
            assignments,
            filter,
        })
    }

    fn parse_delete(&mut self) -> Result<Delete> {
        self.expect_keyword("DELETE")?;
        self.expect_keyword("FROM")?;
        let table = self.expect_word()?;
        let filter = if self.eat_keyword("WHERE") {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Delete { table, filter })
    }

    // --- Expression parsing with precedence: OR < AND < comparison < primary.

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("OR") {
            let right = self.parse_and()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_comparison()?;
        while self.eat_keyword("AND") {
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let left = self.parse_primary()?;
        let op = match self.peek() {
            Some(Token::Eq) => BinOp::Eq,
            Some(Token::NotEq) => BinOp::NotEq,
            Some(Token::Lt) => BinOp::Lt,
            Some(Token::LtEq) => BinOp::LtEq,
            Some(Token::Gt) => BinOp::Gt,
            Some(Token::GtEq) => BinOp::GtEq,
            _ => return Ok(left),
        };
        self.pos += 1;
        let right = self.parse_primary()?;
        Ok(Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        })
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.next()? {
            Token::LParen => {
                let e = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            Token::Question => Ok(Expr::Param),
            Token::Integer(i) => Ok(Expr::Literal(Value::Integer(i))),
            Token::String(s) => Ok(Expr::Literal(Value::Text(s))),
            Token::Word(w) => {
                if w.eq_ignore_ascii_case("TRUE") {
                    Ok(Expr::Literal(Value::Boolean(true)))
                } else if w.eq_ignore_ascii_case("FALSE") {
                    Ok(Expr::Literal(Value::Boolean(false)))
                } else if w.eq_ignore_ascii_case("NULL") {
                    Ok(Expr::Literal(Value::Null))
                } else {
                    Ok(Expr::Column(w))
                }
            }
            other => Err(Error::Parse(format!("unexpected token in expression: {other:?}"))),
        }
    }
}
