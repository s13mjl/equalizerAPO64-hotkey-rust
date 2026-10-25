//! A pure-Rust implementation of the muparserx expression subset used by
//! EqualizerAPO configurations.
//!
//! Supports numeric (integer, float, bool), string and array values, the
//! operator set of muparserx's common/non-complex/string/matrix packages and
//! the `regexSearch`, `regexReplace`, `readRegString`, `readRegDWORD`
//! functions. See [`ParserX`].

mod value;
mod lexer;
mod parser;
mod functions;

pub use value::{Matrix, ParserError, Value};
pub use parser::{ExpressionHost, ParserX};
