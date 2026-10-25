use std::fmt;

/// Mirrors `mup::Value` as used by EqualizerAPO.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Matrix),
}

/// Row-major matrix, mirroring `mup::Matrix<Value>`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<Value>,
}

impl Matrix {
    pub fn from_row(values: Vec<Value>) -> Self {
        let cols = values.len();
        Self { rows: 1, cols, data: values }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn get(&self, row: usize, col: usize) -> Option<&Value> {
        if row >= self.rows || col >= self.cols {
            None
        } else {
            self.data.get(row * self.cols + col)
        }
    }
}

impl Value {
    pub fn type_char(&self) -> char {
        match self {
            Value::Int(_) => 'i',
            Value::Float(_) => 'f',
            Value::Bool(_) => 'b',
            Value::String(_) => 's',
            Value::Array(_) => 'm',
        }
    }

    pub fn as_float(&self) -> f64 {
        match self {
            Value::Int(i) => *i as f64,
            Value::Float(f) => *f,
            Value::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Value::String(s) => s.parse().unwrap_or(f64::NAN),
            Value::Array(m) => {
                if let Some(Value::Float(f)) = m.get(0, 0) {
                    *f
                } else if let Some(Value::Int(i)) = m.get(0, 0) {
                    *i as f64
                } else {
                    f64::NAN
                }
            }
        }
    }

    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Float(f) => *f as i64,
            Value::Bool(b) => *b as i64,
            Value::String(s) => s.parse().unwrap_or(0),
            Value::Array(_) => self.as_float() as i64,
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0 && !f.is_nan(),
            Value::Bool(b) => *b,
            Value::String(s) => !s.is_empty() && s != "false" && s != "0",
            Value::Array(m) => m.rows > 0 && m.cols > 0,
        }
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    pub fn to_string_repr(&self) -> String {
        match self {
            Value::Int(i) => i.to_string(),
            Value::Float(f) => format_float(*f),
            Value::Bool(b) => {
                if *b {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            Value::String(s) => s.clone(),
            Value::Array(m) => {
                let mut out = String::new();
                for r in 0..m.rows {
                    if r > 0 {
                        out.push(',');
                    }
                    out.push('{');
                    for c in 0..m.cols {
                        if c > 0 {
                            out.push(',');
                        }
                        if let Some(v) = m.get(r, c) {
                            out.push_str(&v.to_string_repr());
                        }
                    }
                    out.push('}');
                }
                out
            }
        }
    }
}

/// muparserx formats floats with up to 15 significant digits and no trailing
/// zeroes, e.g. `0.5`, `44100`, `1e-05`.
fn format_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let s = format!("{:.15}", f);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() || s == "-" {
        "0".into()
    } else {
        s
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_repr())
    }
}

#[derive(Clone, Debug)]
pub struct ParserError {
    pub message: String,
}

impl ParserError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParserError {}
