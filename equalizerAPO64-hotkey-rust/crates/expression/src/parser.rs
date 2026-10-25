//! Recursive-descent parser and evaluator compatible with the muparserx
//! subset documented by EqualizerAPO.

use std::collections::{HashMap, HashSet};

use crate::functions::call_function;
use crate::lexer::{Lexer, Token};
use crate::value::{Matrix, ParserError, Value};

/// Host callbacks for registry functions.
pub trait ExpressionHost {
    fn read_reg_string(&mut self, key: &str, value_name: &str) -> Result<String, String>;
    fn read_reg_dword(&mut self, key: &str, value_name: &str) -> Result<u32, String>;
    fn watch_registry_key(&mut self, key: &str);
}

pub struct ParserX {
    variables: HashMap<String, Value>,
    constants: HashMap<String, Value>,
    builtin_constants: HashSet<String>,
    auto_create_var: bool,
    expr: String,
}

impl ParserX {
    pub fn new() -> Self {
        let mut constants = HashMap::new();
        constants.insert("pi".to_string(), Value::Float(std::f64::consts::PI));
        constants.insert("e".to_string(), Value::Float(std::f64::consts::E));
        let mut builtin_constants = HashSet::new();
        builtin_constants.insert("pi".to_string());
        builtin_constants.insert("e".to_string());
        Self {
            variables: HashMap::new(),
            constants,
            builtin_constants,
            auto_create_var: false,
            expr: String::new(),
        }
    }

    pub fn enable_auto_create_var(&mut self, enable: bool) {
        self.auto_create_var = enable;
    }

    pub fn clear_var(&mut self) {
        self.variables.clear();
    }

    pub fn clear_const(&mut self) {
        self.constants
            .retain(|name, _| self.builtin_constants.contains(name));
    }

    pub fn define_var(&mut self, name: &str, value: Value) {
        self.variables.insert(name.to_string(), value);
    }

    pub fn define_const(&mut self, name: &str, value: Value) {
        self.constants.insert(name.to_string(), value);
    }

    pub fn remove_var(&mut self, name: &str) {
        self.variables.remove(name);
    }

    pub fn get_var(&self, name: &str) -> Option<&Value> {
        self.variables
            .get(name)
            .or_else(|| self.constants.get(name))
    }

    pub fn set_expr(&mut self, expr: &str) {
        self.expr = expr.to_string();
    }

    pub fn eval(&mut self) -> Result<Value, ParserError> {
        let expr = self.expr.clone();
        self.eval_str(&expr)
    }

    pub fn eval_str(&mut self, expr: &str) -> Result<Value, ParserError> {
        self.eval_with_host(expr, None)
    }

    pub fn eval_with_host(
        &mut self,
        expr: &str,
        host: Option<&mut dyn ExpressionHost>,
    ) -> Result<Value, ParserError> {
        self.expr = expr.to_string();
        let ast = parse(expr)?;
        match host {
            Some(host) => {
                eval_program(&ast, &mut self.variables, &self.constants, self.auto_create_var, host)
            }
            None => {
                let mut no_host = NoExpressionHost;
                eval_program(
                    &ast,
                    &mut self.variables,
                    &self.constants,
                    self.auto_create_var,
                    &mut no_host,
                )
            }
        }
    }
}

impl Default for ParserX {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug)]
enum UnOp {
    Minus,
    Plus,
    Not,
}

#[derive(Clone, Copy, Debug)]
enum CastType {
    Int,
    Float,
}

#[derive(Clone, Debug)]
enum Expr {
    Value(Value),
    Var(String),
    Assign(String, AssignOp, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Call(String, Vec<Expr>),
    Array(Vec<Vec<Expr>>),
    Index(Box<Expr>, Vec<Expr>),
    Cast(CastType, Box<Expr>),
}

fn parse(expr: &str) -> Result<Expr, ParserError> {
    let mut lexer = Lexer::new(expr);
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token().map_err(ParserError::new)?;
        let end = matches!(t, Token::Eof);
        tokens.push(t);
        if end {
            break;
        }
    }
    let mut p = ExprParser { tokens, pos: 0 };
    let program = p.parse_program()?;
    Ok(program)
}

struct ExprParser {
    tokens: Vec<Token>,
    pos: usize,
}

impl ExprParser {
    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.pos + offset)
            .unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn advance(&mut self) {
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
    }

    fn at(&self, token: &Token) -> bool {
        std::mem::discriminant(self.current()) == std::mem::discriminant(token)
    }

    fn accept(&mut self, token: &Token) -> bool {
        if self.at(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token) -> Result<(), ParserError> {
        if self.accept(token) {
            Ok(())
        } else {
            Err(ParserError::new(format!(
                "expected {}, got {}",
                describe_expected(token),
                self.current().describe()
            )))
        }
    }

    fn parse_program(&mut self) -> Result<Expr, ParserError> {
        let mut expressions = Vec::new();
        if self.at(&Token::Eof) {
            return Err(ParserError::new("empty expression"));
        }
        loop {
            expressions.push(self.parse_assignment()?);
            if self.accept(&Token::Semicolon) {
                if self.at(&Token::Eof) {
                    break;
                }
                continue;
            }
            break;
        }
        if !self.at(&Token::Eof) {
            return Err(ParserError::new(format!(
                "unexpected {}",
                self.current().describe()
            )));
        }
        Ok(Expr::Array(vec![expressions]))
    }

    fn parse_assignment(&mut self) -> Result<Expr, ParserError> {
        if let Token::Ident(name) = self.current().clone() {
            let op = match self.peek(1) {
                Token::Assign => Some(AssignOp::Assign),
                Token::PlusAssign => Some(AssignOp::Add),
                Token::MinusAssign => Some(AssignOp::Sub),
                Token::StarAssign => Some(AssignOp::Mul),
                Token::SlashAssign => Some(AssignOp::Div),
                _ => None,
            };
            if let Some(op) = op {
                self.advance();
                self.advance();
                let value = self.parse_assignment()?;
                return Ok(Expr::Assign(name, op, Box::new(value)));
            }
        }
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, ParserError> {
        let cond = self.parse_or()?;
        if self.accept(&Token::Question) {
            let then_expr = self.parse_assignment()?;
            self.expect(&Token::Colon)?;
            let else_expr = self.parse_assignment()?;
            Ok(Expr::Ternary(
                Box::new(cond),
                Box::new(then_expr),
                Box::new(else_expr),
            ))
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_and()?;
        while self.accept(&Token::Or) {
            let right = self.parse_and()?;
            left = Expr::Binary(BinOp::Or, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_bit_or()?;
        while self.accept(&Token::And) {
            let right = self.parse_bit_or()?;
            left = Expr::Binary(BinOp::And, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_bit_or(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_bit_xor()?;
        while self.accept(&Token::BitOr) {
            let right = self.parse_bit_xor()?;
            left = Expr::Binary(BinOp::BitOr, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_bit_xor(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_bit_and()?;
        while self.accept(&Token::Xor) {
            let right = self.parse_bit_and()?;
            left = Expr::Binary(BinOp::Xor, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_bit_and(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_equality()?;
        while self.accept(&Token::BitAnd) {
            let right = self.parse_equality()?;
            left = Expr::Binary(BinOp::BitAnd, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_relational()?;
        loop {
            let op = if self.accept(&Token::Eq) {
                Some(BinOp::Eq)
            } else if self.accept(&Token::Ne) {
                Some(BinOp::Ne)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let right = self.parse_relational()?;
                    left = Expr::Binary(op, Box::new(left), Box::new(right));
                }
                None => break,
            }
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_shift()?;
        loop {
            let op = if self.accept(&Token::Lt) {
                Some(BinOp::Lt)
            } else if self.accept(&Token::Le) {
                Some(BinOp::Le)
            } else if self.accept(&Token::Gt) {
                Some(BinOp::Gt)
            } else if self.accept(&Token::Ge) {
                Some(BinOp::Ge)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let right = self.parse_shift()?;
                    left = Expr::Binary(op, Box::new(left), Box::new(right));
                }
                None => break,
            }
        }
        Ok(left)
    }

    fn parse_shift(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = if self.accept(&Token::Shl) {
                Some(BinOp::Shl)
            } else if self.accept(&Token::Shr) {
                Some(BinOp::Shr)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let right = self.parse_additive()?;
                    left = Expr::Binary(op, Box::new(left), Box::new(right));
                }
                None => break,
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = if self.accept(&Token::Plus) {
                Some(BinOp::Add)
            } else if self.accept(&Token::Minus) {
                Some(BinOp::Sub)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let right = self.parse_multiplicative()?;
                    left = Expr::Binary(op, Box::new(left), Box::new(right));
                }
                None => break,
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParserError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = if self.accept(&Token::Star) {
                Some(BinOp::Mul)
            } else if self.accept(&Token::Slash) {
                Some(BinOp::Div)
            } else if self.accept(&Token::Percent) {
                Some(BinOp::Mod)
            } else {
                None
            };
            match op {
                Some(op) => {
                    let right = self.parse_unary()?;
                    left = Expr::Binary(op, Box::new(left), Box::new(right));
                }
                None => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParserError> {
        if self.accept(&Token::Minus) {
            Ok(Expr::Unary(UnOp::Minus, Box::new(self.parse_unary()?)))
        } else if self.accept(&Token::Plus) {
            Ok(Expr::Unary(UnOp::Plus, Box::new(self.parse_unary()?)))
        } else if self.accept(&Token::Not) {
            Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)))
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Result<Expr, ParserError> {
        let left = self.parse_postfix()?;
        if self.accept(&Token::Caret) {
            let right = self.parse_unary()?;
            Ok(Expr::Binary(BinOp::Pow, Box::new(left), Box::new(right)))
        } else {
            Ok(left)
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParserError> {
        let mut expr = self.parse_primary()?;
        while self.accept(&Token::LBracket) {
            let mut indices = Vec::new();
            indices.push(self.parse_assignment()?);
            while self.accept(&Token::Comma) {
                indices.push(self.parse_assignment()?);
            }
            self.expect(&Token::RBracket)?;
            expr = Expr::Index(Box::new(expr), indices);
        }
        Ok(expr)
    }

    fn is_cast(&self) -> Option<CastType> {
        if !self.at(&Token::LParen) {
            return None;
        }
        let name = match self.peek(1) {
            Token::Ident(name) => name.to_ascii_lowercase(),
            _ => return None,
        };
        let cast = match name.as_str() {
            "int" => CastType::Int,
            "float" | "double" => CastType::Float,
            _ => return None,
        };
        if matches!(self.peek(2), Token::RParen) {
            Some(cast)
        } else {
            None
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParserError> {
        if let Some(cast) = self.is_cast() {
            self.advance();
            self.advance();
            self.advance();
            let value = self.parse_unary()?;
            return Ok(Expr::Cast(cast, Box::new(value)));
        }
        match self.current().clone() {
            Token::Int(i) => {
                self.advance();
                Ok(Expr::Value(Value::Int(i)))
            }
            Token::Float(f) => {
                self.advance();
                Ok(Expr::Value(Value::Float(f)))
            }
            Token::Str(s) => {
                self.advance();
                Ok(Expr::Value(Value::String(s)))
            }
            Token::Ident(name) => {
                self.advance();
                if self.accept(&Token::LParen) {
                    let mut args = Vec::new();
                    if !self.at(&Token::RParen) {
                        loop {
                            args.push(self.parse_assignment()?);
                            if self.accept(&Token::Comma) {
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    match name.as_str() {
                        "true" => Ok(Expr::Value(Value::Bool(true))),
                        "false" => Ok(Expr::Value(Value::Bool(false))),
                        _ => Ok(Expr::Var(name)),
                    }
                }
            }
            Token::LParen => {
                self.advance();
                let inner = self.parse_assignment()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Token::LBrace => self.parse_array_literal(),
            other => Err(ParserError::new(format!(
                "unexpected {}",
                other.describe()
            ))),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, ParserError> {
        self.expect(&Token::LBrace)?;
        let mut rows: Vec<Vec<Expr>> = vec![Vec::new()];
        loop {
            if self.accept(&Token::RBrace) {
                break;
            }
            let value = self.parse_assignment()?;
            rows.last_mut().unwrap().push(value);
            if self.accept(&Token::Comma) {
                continue;
            }
            if self.accept(&Token::Semicolon) {
                rows.push(Vec::new());
                continue;
            }
            self.expect(&Token::RBrace)?;
            break;
        }
        Ok(Expr::Array(rows))
    }
}

fn describe_expected(token: &Token) -> String {
    format!("'{}'", token.describe().trim_matches('\''))
}

struct NoExpressionHost;

impl ExpressionHost for NoExpressionHost {
    fn read_reg_string(&mut self, _key: &str, _value_name: &str) -> Result<String, String> {
        Err("readRegString is not available in this context".into())
    }

    fn read_reg_dword(&mut self, _key: &str, _value_name: &str) -> Result<u32, String> {
        Err("readRegDWORD is not available in this context".into())
    }

    fn watch_registry_key(&mut self, _key: &str) {}
}

fn eval_program(
    ast: &Expr,
    vars: &mut HashMap<String, Value>,
    consts: &HashMap<String, Value>,
    auto_create: bool,
    host: &mut dyn ExpressionHost,
) -> Result<Value, ParserError> {
    let Expr::Array(rows) = ast else {
        return eval_expr(ast, vars, consts, auto_create, host);
    };
    // Top-level programs are represented as a single row of expressions.
    let Some(expressions) = rows.first() else {
        return Err(ParserError::new("empty expression"));
    };
    let mut last = Value::Int(0);
    for expression in expressions {
        last = eval_expr(expression, vars, consts, auto_create, host)?;
    }
    Ok(last)
}

fn eval_expr(
    expr: &Expr,
    vars: &mut HashMap<String, Value>,
    consts: &HashMap<String, Value>,
    auto_create: bool,
    host: &mut dyn ExpressionHost,
) -> Result<Value, ParserError> {
    match expr {
        Expr::Value(v) => Ok(v.clone()),
        Expr::Var(name) => {
            if let Some(v) = consts.get(name) {
                return Ok(v.clone());
            }
            if let Some(v) = vars.get(name) {
                return Ok(v.clone());
            }
            if auto_create {
                vars.insert(name.clone(), Value::Int(0));
                return Ok(Value::Int(0));
            }
            Err(ParserError::new(format!("unknown variable '{}'", name)))
        }
        Expr::Assign(name, op, value) => {
            if consts.contains_key(name) {
                return Err(ParserError::new(format!(
                    "cannot assign to constant '{}'",
                    name
                )));
            }
            let rhs = eval_expr(value, vars, consts, auto_create, host)?;
            let result = match op {
                AssignOp::Assign => rhs,
                AssignOp::Add => {
                    let current = vars.get(name).cloned().unwrap_or(Value::Int(0));
                    binary(BinOp::Add, current, rhs)?
                }
                AssignOp::Sub => {
                    let current = vars.get(name).cloned().unwrap_or(Value::Int(0));
                    binary(BinOp::Sub, current, rhs)?
                }
                AssignOp::Mul => {
                    let current = vars.get(name).cloned().unwrap_or(Value::Int(0));
                    binary(BinOp::Mul, current, rhs)?
                }
                AssignOp::Div => {
                    let current = vars.get(name).cloned().unwrap_or(Value::Int(0));
                    binary(BinOp::Div, current, rhs)?
                }
            };
            vars.insert(name.clone(), result.clone());
            Ok(result)
        }
        Expr::Ternary(cond, then_expr, else_expr) => {
            if eval_expr(cond, vars, consts, auto_create, host)?.as_bool() {
                eval_expr(then_expr, vars, consts, auto_create, host)
            } else {
                eval_expr(else_expr, vars, consts, auto_create, host)
            }
        }
        Expr::Binary(op, left, right) => {
            let l = eval_expr(left, vars, consts, auto_create, host)?;
            let r = eval_expr(right, vars, consts, auto_create, host)?;
            binary(*op, l, r)
        }
        Expr::Unary(op, value) => {
            let v = eval_expr(value, vars, consts, auto_create, host)?;
            Ok(match op {
                UnOp::Minus => match v {
                    Value::Int(i) => Value::Int(i.wrapping_neg()),
                    other => Value::Float(-other.as_float()),
                },
                UnOp::Plus => match v {
                    Value::Int(i) => Value::Int(i),
                    other => Value::Float(other.as_float()),
                },
                UnOp::Not => Value::Bool(!v.as_bool()),
            })
        }
        Expr::Call(name, args) => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval_expr(arg, vars, consts, auto_create, host)?);
            }
            let lower = name.to_ascii_lowercase();
            call_function(name, &lower, values, host)
        }
        Expr::Array(rows) => {
            let mut evaluated_rows = Vec::with_capacity(rows.len());
            for row in rows {
                let mut evaluated = Vec::with_capacity(row.len());
                for value in row {
                    evaluated.push(eval_expr(value, vars, consts, auto_create, host)?);
                }
                evaluated_rows.push(evaluated);
            }
            if evaluated_rows.len() == 1 && evaluated_rows[0].is_empty() {
                return Ok(Value::Array(Matrix::empty()));
            }
            // muparserx matrix literals use nested braces for rows:
            // {{1,2},{3,4}} or {{1,2};{3,4}}.
            let nested: Option<Vec<Value>> = if evaluated_rows.len() == 1
                && !evaluated_rows[0].is_empty()
                && evaluated_rows[0]
                    .iter()
                    .all(|v| matches!(v, Value::Array(_)))
            {
                Some(evaluated_rows[0].clone())
            } else if evaluated_rows
                .iter()
                .all(|r| r.len() == 1 && matches!(r[0], Value::Array(_)))
            {
                Some(evaluated_rows.iter().map(|r| r[0].clone()).collect())
            } else {
                None
            };
            if let Some(rows_of_arrays) = nested {
                let mut cols = 0usize;
                let mut data = Vec::new();
                for value in rows_of_arrays {
                    if let Value::Array(m) = value {
                        if cols == 0 {
                            cols = m.cols;
                        } else if cols != m.cols {
                            return Err(ParserError::new("array rows have different lengths"));
                        }
                        data.extend(m.data);
                    }
                }
                let row_count = if cols == 0 { 0 } else { data.len() / cols };
                return Ok(Value::Array(Matrix {
                    rows: row_count,
                    cols,
                    data,
                }));
            }
            let cols = evaluated_rows.first().map(|r| r.len()).unwrap_or(0);
            if evaluated_rows.iter().any(|r| r.len() != cols) {
                return Err(ParserError::new("array rows have different lengths"));
            }
            let mut data = Vec::new();
            for row in evaluated_rows {
                data.extend(row);
            }
            Ok(Value::Array(Matrix {
                rows: if cols == 0 { 0 } else { data.len() / cols.max(1) },
                cols,
                data,
            }))
        }
        Expr::Index(target, indices) => {
            let target = eval_expr(target, vars, consts, auto_create, host)?;
            let mut index_values = Vec::with_capacity(indices.len());
            for index in indices {
                index_values.push(
                    eval_expr(index, vars, consts, auto_create, host)?
                        .as_int(),
                );
            }
            index_value(target, &index_values)
        }
        Expr::Cast(cast, value) => {
            let v = eval_expr(value, vars, consts, auto_create, host)?;
            Ok(match cast {
                CastType::Int => Value::Int(v.as_int()),
                CastType::Float => Value::Float(v.as_float()),
            })
        }
    }
}

fn index_value(target: Value, indices: &[i64]) -> Result<Value, ParserError> {
    match target {
        Value::Array(m) => match indices {
            [i] => {
                if *i < 0 {
                    return Err(ParserError::new("index out of range"));
                }
                let flat = *i as usize;
                if flat >= m.data.len() {
                    return Err(ParserError::new("index out of range"));
                }
                Ok(m.data[flat].clone())
            }
            [r, c] => {
                if *r < 0 || *c < 0 {
                    return Err(ParserError::new("index out of range"));
                }
                match m.get(*r as usize, *c as usize) {
                    Some(v) => Ok(v.clone()),
                    None => Err(ParserError::new("index out of range")),
                }
            }
            _ => Err(ParserError::new("too many array indices")),
        },
        Value::String(s) => {
            let i = indices.first().copied().unwrap_or(-1);
            if i < 0 {
                return Err(ParserError::new("index out of range"));
            }
            match s.chars().nth(i as usize) {
                Some(c) => Ok(Value::String(c.to_string())),
                None => Err(ParserError::new("index out of range")),
            }
        }
        _ => Err(ParserError::new("value is not indexable")),
    }
}

fn numeric_int(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Bool(_))
}

fn binary(op: BinOp, left: Value, right: Value) -> Result<Value, ParserError> {
    use BinOp::*;
    match op {
        Add => {
            if left.is_string() || right.is_string() {
                Ok(Value::String(format!(
                    "{}{}",
                    left.to_string_repr(),
                    right.to_string_repr()
                )))
            } else if numeric_int(&left) && numeric_int(&right) {
                Ok(Value::Int(left.as_int().wrapping_add(right.as_int())))
            } else {
                Ok(Value::Float(left.as_float() + right.as_float()))
            }
        }
        Sub => {
            if numeric_int(&left) && numeric_int(&right) {
                Ok(Value::Int(left.as_int().wrapping_sub(right.as_int())))
            } else {
                Ok(Value::Float(left.as_float() - right.as_float()))
            }
        }
        Mul => {
            if numeric_int(&left) && numeric_int(&right) {
                Ok(Value::Int(left.as_int().wrapping_mul(right.as_int())))
            } else {
                Ok(Value::Float(left.as_float() * right.as_float()))
            }
        }
        Div => {
            let divisor = right.as_float();
            if divisor == 0.0 {
                return Err(ParserError::new("division by zero"));
            }
            Ok(Value::Float(left.as_float() / divisor))
        }
        Mod => {
            if divisor_is_zero(&right) {
                return Err(ParserError::new("division by zero"));
            }
            if numeric_int(&left) && numeric_int(&right) {
                Ok(Value::Int(left.as_int().wrapping_rem(right.as_int())))
            } else {
                Ok(Value::Float(left.as_float() % right.as_float()))
            }
        }
        Pow => Ok(Value::Float(left.as_float().powf(right.as_float()))),
        Eq => Ok(Value::Bool(compare(&left, &right) == std::cmp::Ordering::Equal)),
        Ne => Ok(Value::Bool(compare(&left, &right) != std::cmp::Ordering::Equal)),
        Lt => Ok(Value::Bool(compare(&left, &right) == std::cmp::Ordering::Less)),
        Le => Ok(Value::Bool(
            compare(&left, &right) != std::cmp::Ordering::Greater,
        )),
        Gt => Ok(Value::Bool(
            compare(&left, &right) == std::cmp::Ordering::Greater,
        )),
        Ge => Ok(Value::Bool(
            compare(&left, &right) != std::cmp::Ordering::Less,
        )),
        And => Ok(Value::Bool(left.as_bool() && right.as_bool())),
        Or => Ok(Value::Bool(left.as_bool() || right.as_bool())),
        Xor => Ok(Value::Bool(left.as_bool() ^ right.as_bool())),
        BitAnd => Ok(Value::Int(left.as_int() & right.as_int())),
        BitOr => Ok(Value::Int(left.as_int() | right.as_int())),
        Shl => Ok(Value::Int(
            left.as_int().wrapping_shl(right.as_int() as u32),
        )),
        Shr => Ok(Value::Int(
            left.as_int().wrapping_shr(right.as_int() as u32),
        )),
    }
}

fn divisor_is_zero(v: &Value) -> bool {
    match v {
        Value::Int(i) => *i == 0,
        Value::Float(f) => *f == 0.0,
        Value::Bool(b) => !*b,
        _ => v.as_float() == 0.0,
    }
}

fn compare(left: &Value, right: &Value) -> std::cmp::Ordering {
    match (left, right) {
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Array(a), Value::Array(b)) => a.data.len().cmp(&b.data.len()),
        _ => left
            .as_float()
            .partial_cmp(&right.as_float())
            .unwrap_or(std::cmp::Ordering::Equal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_examples() {
        let mut p = ParserX::new();
        assert_eq!(p.eval_str("a=0; b=pi").unwrap(), Value::Float(std::f64::consts::PI));
        assert_eq!(
            p.eval_str("sin(a)^2 + cos(a)^2 == 1").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            p.eval_str("1 + 2 * 3").unwrap(),
            Value::Int(7)
        );
        assert_eq!(
            p.eval_str("2^3^2").unwrap(),
            Value::Float(512.0)
        );
        assert_eq!(
            p.eval_str("\"a\" + 1 + 2").unwrap(),
            Value::String("a12".into())
        );
    }

    #[test]
    fn ternary_only_evaluates_chosen_branch() {
        let mut p = ParserX::new();
        p.eval_str("x = true ? 1 : (y = 2)").unwrap();
        assert!(p.get_var("y").is_none());
        p.eval_str("z = false ? (w = 3) : 4").unwrap();
        assert!(p.get_var("w").is_none());
        assert_eq!(p.get_var("z"), Some(&Value::Int(4)));
    }

    #[test]
    fn arrays_and_indexing() {
        let mut p = ParserX::new();
        p.eval_str("a = {1, 2, 3}").unwrap();
        assert_eq!(p.eval_str("a[1]").unwrap(), Value::Int(2));
        assert_eq!(p.eval_str("sizeof(a)").unwrap(), Value::Int(3));
        assert_eq!(p.eval_str("sum(a)").unwrap(), Value::Float(6.0));
        assert_eq!(p.eval_str("b = {{1,2};{3,4}}; b[1,0]").unwrap(), Value::Int(3));
    }

    #[test]
    fn casts_and_bitwise() {
        let mut p = ParserX::new();
        assert_eq!(p.eval_str("(int) 3.9").unwrap(), Value::Int(3));
        assert_eq!(p.eval_str("(float) 3").unwrap(), Value::Float(3.0));
        assert_eq!(p.eval_str("5 & 3").unwrap(), Value::Int(1));
        assert_eq!(p.eval_str("1 << 4").unwrap(), Value::Int(16));
        assert_eq!(p.eval_str("!false").unwrap(), Value::Bool(true));
        assert_eq!(p.eval_str("not false").unwrap(), Value::Bool(true));
    }
}
