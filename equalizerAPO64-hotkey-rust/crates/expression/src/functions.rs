//! Built-in functions of the expression language.

use regex::Regex;

use crate::parser::ExpressionHost;
use crate::value::{Matrix, ParserError, Value};

fn num(args: &[Value], index: usize, name: &str) -> Result<f64, ParserError> {
    args.get(index)
        .map(|v| v.as_float())
        .ok_or_else(|| ParserError::new(format!("function {} expects more arguments", name)))
}

fn str_arg(args: &[Value], index: usize, name: &str) -> Result<String, ParserError> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(ParserError::new(format!(
            "type conflict in function {}: argument {} is '{}', expected 's'",
            name,
            index + 1,
            other.type_char()
        ))),
        None => Err(ParserError::new(format!(
            "function {} expects more arguments",
            name
        ))),
    }
}

fn flatten_args(args: &[Value]) -> Vec<Value> {
    if args.len() == 1 {
        if let Value::Array(m) = &args[0] {
            return m.data.clone();
        }
    }
    args.to_vec()
}

pub fn call_function(
    name: &str,
    lower_name: &str,
    args: Vec<Value>,
    host: &mut dyn ExpressionHost,
) -> Result<Value, ParserError> {
    match lower_name {
        "abs" => match args.first() {
            Some(Value::Int(i)) => Ok(Value::Int(i.abs())),
            Some(v) => Ok(Value::Float(v.as_float().abs())),
            None => Err(ParserError::new("abs expects 1 argument")),
        },
        "sin" => Ok(Value::Float(num(&args, 0, name)?.sin())),
        "cos" => Ok(Value::Float(num(&args, 0, name)?.cos())),
        "tan" => Ok(Value::Float(num(&args, 0, name)?.tan())),
        "asin" => Ok(Value::Float(num(&args, 0, name)?.asin())),
        "acos" => Ok(Value::Float(num(&args, 0, name)?.acos())),
        "atan" => Ok(Value::Float(num(&args, 0, name)?.atan())),
        "atan2" => Ok(Value::Float(num(&args, 0, name)?.atan2(num(&args, 1, name)?))),
        "sinh" => Ok(Value::Float(num(&args, 0, name)?.sinh())),
        "cosh" => Ok(Value::Float(num(&args, 0, name)?.cosh())),
        "tanh" => Ok(Value::Float(num(&args, 0, name)?.tanh())),
        "asinh" => Ok(Value::Float(num(&args, 0, name)?.asinh())),
        "acosh" => Ok(Value::Float(num(&args, 0, name)?.acosh())),
        "atanh" => Ok(Value::Float(num(&args, 0, name)?.atanh())),
        "ln" | "log" => Ok(Value::Float(num(&args, 0, name)?.ln())),
        "log2" => Ok(Value::Float(num(&args, 0, name)?.log2())),
        "log10" => Ok(Value::Float(num(&args, 0, name)?.log10())),
        "exp" => Ok(Value::Float(num(&args, 0, name)?.exp())),
        "sqrt" => Ok(Value::Float(num(&args, 0, name)?.sqrt())),
        "cbrt" => Ok(Value::Float(num(&args, 0, name)?.cbrt())),
        "pow" => Ok(Value::Float(
            num(&args, 0, name)?.powf(num(&args, 1, name)?),
        )),
        "fmod" => Ok(Value::Float(
            num(&args, 0, name)?.rem_euclid(num(&args, 1, name)?.abs()),
        )),
        "sign" => {
            let x = num(&args, 0, name)?;
            Ok(Value::Float(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            }))
        }
        "rint" => Ok(Value::Float(num(&args, 0, name)?.round_ties_even())),
        "floor" => Ok(Value::Float(num(&args, 0, name)?.floor())),
        "ceil" => Ok(Value::Float(num(&args, 0, name)?.ceil())),
        "round" => Ok(Value::Float(num(&args, 0, name)?.round())),
        "min" => {
            let values = flatten_args(&args);
            if values.is_empty() {
                return Err(ParserError::new("min expects at least 1 argument"));
            }
            let mut best = values[0].clone();
            for v in &values[1..] {
                if v.as_float() < best.as_float() {
                    best = v.clone();
                }
            }
            Ok(best)
        }
        "max" => {
            let values = flatten_args(&args);
            if values.is_empty() {
                return Err(ParserError::new("max expects at least 1 argument"));
            }
            let mut best = values[0].clone();
            for v in &values[1..] {
                if v.as_float() > best.as_float() {
                    best = v.clone();
                }
            }
            Ok(best)
        }
        "sum" => {
            let values = flatten_args(&args);
            let mut sum = 0.0;
            for v in values {
                sum += v.as_float();
            }
            Ok(Value::Float(sum))
        }
        "avg" | "mean" => {
            let values = flatten_args(&args);
            if values.is_empty() {
                return Err(ParserError::new("avg expects at least 1 argument"));
            }
            let mut sum = 0.0;
            for v in &values {
                sum += v.as_float();
            }
            Ok(Value::Float(sum / values.len() as f64))
        }
        "str2dbl" => {
            let s = str_arg(&args, 0, name)?;
            s.trim()
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|_| ParserError::new(format!("str2dbl cannot convert '{}'", s)))
        }
        "strlen" => {
            let s = str_arg(&args, 0, name)?;
            Ok(Value::Int(s.chars().count() as i64))
        }
        "tolower" => Ok(Value::String(str_arg(&args, 0, name)?.to_lowercase())),
        "toupper" => Ok(Value::String(str_arg(&args, 0, name)?.to_uppercase())),
        "sizeof" => match args.first() {
            Some(Value::Array(m)) => Ok(Value::Int((m.rows * m.cols) as i64)),
            Some(Value::String(s)) => Ok(Value::Int(s.chars().count() as i64)),
            Some(_) => Ok(Value::Int(1)),
            None => Err(ParserError::new("sizeof expects 1 argument")),
        },
        "int" => Ok(Value::Int(num(&args, 0, name)? as i64)),
        "float" | "double" => Ok(Value::Float(num(&args, 0, name)?)),
        "regexsearch" => {
            let pattern = str_arg(&args, 0, name)?;
            let text = str_arg(&args, 1, name)?;
            let regex = Regex::new(&pattern)
                .map_err(|e| ParserError::new(format!("invalid regular expression: {}", e)))?;
            match regex.captures(&text) {
                None => Ok(Value::Array(Matrix::empty())),
                Some(caps) => {
                    let mut values = Vec::with_capacity(caps.len());
                    for c in caps.iter() {
                        values.push(Value::String(c.map(|m| m.as_str()).unwrap_or("").to_string()));
                    }
                    Ok(Value::Array(Matrix::from_row(values)))
                }
            }
        }
        "regexreplace" => {
            let pattern = str_arg(&args, 0, name)?;
            let text = str_arg(&args, 1, name)?;
            let replacement = str_arg(&args, 2, name)?;
            let regex = Regex::new(&pattern)
                .map_err(|e| ParserError::new(format!("invalid regular expression: {}", e)))?;
            Ok(Value::String(
                regex.replace_all(&text, replacement.as_str()).into_owned(),
            ))
        }
        "readregstring" => {
            let key = str_arg(&args, 0, name)?;
            let value_name = str_arg(&args, 1, name)?;
            match host.read_reg_string(&key, &value_name) {
                Ok(value) => {
                    host.watch_registry_key(&key);
                    Ok(Value::String(value))
                }
                Err(e) => Err(ParserError::new(e)),
            }
        }
        "readregdword" => {
            let key = str_arg(&args, 0, name)?;
            let value_name = str_arg(&args, 1, name)?;
            match host.read_reg_dword(&key, &value_name) {
                Ok(value) => {
                    host.watch_registry_key(&key);
                    Ok(Value::Int(value as i64))
                }
                Err(e) => Err(ParserError::new(e)),
            }
        }
        _ => Err(ParserError::new(format!("unknown function '{}'", name))),
    }
}
