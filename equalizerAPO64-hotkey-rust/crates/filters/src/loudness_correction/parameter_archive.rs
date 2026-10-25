//! Port of `filters/loudnessCorrection/ParameterArchive.h`.

use regex::Regex;

pub trait ArchiveValue: Sized {
    fn to_wide_string(&self) -> String;
    fn from_wide_string(value: &str) -> Self;
}

impl ArchiveValue for bool {
    fn to_wide_string(&self) -> String {
        if *self { "1" } else { "0" }.to_string()
    }

    fn from_wide_string(value: &str) -> Self {
        parse_leading_number(value) != 0.0
    }
}

impl ArchiveValue for i32 {
    fn to_wide_string(&self) -> String {
        self.to_string()
    }

    fn from_wide_string(value: &str) -> Self {
        parse_leading_number(value) as i32
    }
}

impl ArchiveValue for f32 {
    fn to_wide_string(&self) -> String {
        format!("{:.6}", self)
    }

    fn from_wide_string(value: &str) -> Self {
        parse_leading_number(value) as f32
    }
}

impl ArchiveValue for f64 {
    fn to_wide_string(&self) -> String {
        format!("{:.6}", self)
    }

    fn from_wide_string(value: &str) -> Self {
        parse_leading_number(value)
    }
}

impl ArchiveValue for String {
    fn to_wide_string(&self) -> String {
        self.clone()
    }

    fn from_wide_string(value: &str) -> Self {
        value.to_string()
    }
}

pub(crate) fn parse_leading_number(value: &str) -> f64 {
    let trimmed = value.trim_start();
    let bytes = trimmed.as_bytes();
    let mut end = 0;
    if end < bytes.len() && (bytes[end] == b'+' || bytes[end] == b'-') {
        end += 1;
    }
    let mut digits = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        digits += 1;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0.0;
    }
    let mut exponent = end;
    if exponent < bytes.len() && (bytes[exponent] == b'e' || bytes[exponent] == b'E') {
        exponent += 1;
        if exponent < bytes.len() && (bytes[exponent] == b'+' || bytes[exponent] == b'-') {
            exponent += 1;
        }
        let mut exponent_digits = 0;
        while exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
            exponent += 1;
            exponent_digits += 1;
        }
        if exponent_digits > 0 {
            end = exponent;
        }
    }
    trimmed[..end].parse().unwrap_or(0.0)
}

#[derive(Default)]
pub struct ParameterArchive {
    serialized_parameters: String,
}

impl ParameterArchive {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_string(parameters: &str) -> Self {
        Self {
            serialized_parameters: parameters.to_string(),
        }
    }

    pub fn add<T: ArchiveValue, U: AsRef<str>>(&mut self, parameter: T, name: U) {
        let name = name.as_ref();
        if !name.is_empty() {
            self.serialized_parameters.push_str(name);
            self.serialized_parameters.push(' ');
        }
        self.serialized_parameters
            .push_str(&parameter.to_wide_string());
        self.serialized_parameters.push(' ');
    }

    pub fn get<T: ArchiveValue>(&self, search_argument: &Regex) -> Option<T> {
        let captures = search_argument.captures(&self.serialized_parameters)?;
        let matched = captures.get(1)?.as_str();
        let last = matched.split(' ').filter(|part| !part.is_empty()).last()?;
        Some(T::from_wide_string(last))
    }

    pub fn find(&self, search_argument: &Regex) -> bool {
        search_argument.is_match(&self.serialized_parameters)
    }

    pub fn serialized_parameters(&self) -> &str {
        &self.serialized_parameters
    }

    pub fn get_serialized_parameters(&self) -> Vec<u8> {
        self.serialized_parameters
            .encode_utf16()
            .flat_map(|code_unit| code_unit.to_le_bytes())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    #[test]
    fn leading_number_matches_wcstod_behavior() {
        assert_eq!(parse_leading_number("-20.5"), -20.5);
        assert_eq!(parse_leading_number("1,5"), 1.0);
        assert_eq!(parse_leading_number("1e3"), 1000.0);
        assert_eq!(parse_leading_number("abc"), 0.0);
        assert_eq!(parse_leading_number(".5"), 0.5);
    }

    #[test]
    fn archive_round_trips_values() {
        let mut archive = ParameterArchive::new();
        archive.add(true, "State");
        archive.add(-12.5f32, "ReferenceLevel");
        assert_eq!(archive.serialized_parameters(), "State 1 ReferenceLevel -12.500000 ");

        let state: bool = archive.get(&Regex::new(r"State\s+(\d+)").unwrap()).unwrap();
        let level: f32 = archive
            .get(&Regex::new(r"ReferenceLevel\s+([-+0-9.]+)").unwrap())
            .unwrap();
        assert!(state);
        assert_eq!(level, -12.5);
    }

    #[test]
    fn serialized_parameters_are_utf16_little_endian() {
        let archive = ParameterArchive::from_string("State 1 ");
        assert_eq!(
            archive.get_serialized_parameters(),
            "State 1 "
                .encode_utf16()
                .flat_map(|unit| unit.to_le_bytes())
                .collect::<Vec<_>>()
        );
    }
}
