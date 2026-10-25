//! Port of `helpers/StringHelper.{h,cpp}`.

use windows::core::PCSTR;
use windows::Win32::Globalization::{
    MultiByteToWideChar, WideCharToMultiByte, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
};
use windows::Win32::Globalization::{CP_ACP, CP_UTF8};

/// Replaces any occurrence of a character from `chars` in `s` with `replacement`.
pub fn replace_characters(s: &str, chars: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        if chars.contains(c) {
            result.push_str(replacement);
        } else {
            result.push(c);
        }
    }
    result
}

/// Replaces the characters Windows forbids in file names with `_`.
pub fn replace_illegal_characters(filename: &str) -> String {
    replace_characters(filename, "<>:\"/\\|?*", "_")
}

/// `MultiByteToWideChar` equivalent. Returns replacement characters for
/// undecodable bytes exactly like the Win32 API does.  Like the C++ helper the
/// result stops at the first NUL.
pub fn to_wstring(s: &[u8], codepage: u32) -> String {
    if s.is_empty() {
        return String::new();
    }
    unsafe {
        let len = MultiByteToWideChar(codepage, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), s, None);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize];
        MultiByteToWideChar(
            codepage,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            s,
            Some(&mut buf),
        );
        if let Some(pos) = buf.iter().position(|&c| c == 0) {
            buf.truncate(pos);
        }
        String::from_utf16_lossy(&buf)
    }
}

/// `WideCharToMultiByte` equivalent.  Like the C++ helper the input is
/// truncated at the first NUL.
pub fn to_string(s: &str, codepage: u32) -> Vec<u8> {
    let mut utf16: Vec<u16> = s.encode_utf16().collect();
    if let Some(pos) = utf16.iter().position(|&c| c == 0) {
        utf16.truncate(pos);
    }
    unsafe {
        let len = WideCharToMultiByte(codepage, 0, &utf16, None, PCSTR::null(), None);
        if len <= 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; len as usize];
        WideCharToMultiByte(codepage, 0, &utf16, Some(&mut buf), PCSTR::null(), None);
        buf
    }
}

/// Converts UTF-8 bytes to a Rust string (`CP_UTF8`).
pub fn utf8_to_wstring(s: &[u8]) -> String {
    to_wstring(s, CP_UTF8)
}

/// Converts system-ANSI bytes to a Rust string (`CP_ACP`).
pub fn ansi_to_wstring(s: &[u8]) -> String {
    to_wstring(s, CP_ACP)
}

/// Converts a Rust string to UTF-8 bytes.
pub fn string_to_utf8(s: &str) -> Vec<u8> {
    to_string(s, CP_UTF8)
}

/// Converts a Rust string to system-ANSI bytes (`CP_ACP`).
pub fn string_to_ansi(s: &str) -> Vec<u8> {
    to_string(s, CP_ACP)
}

/// Lower-cases a string (the C++ code uses `_wcslwr_s`).
pub fn to_lower_case(s: &str) -> String {
    s.to_lowercase()
}

/// Upper-cases a string (the C++ code uses `_wcsupr_s`).
pub fn to_upper_case(s: &str) -> String {
    s.to_uppercase()
}

/// Removes leading and trailing whitespace.
pub fn trim(s: &str) -> String {
    s.trim_matches(|c: char| c.is_whitespace()).to_string()
}

/// Splits `s` at `split_char`, optionally dropping empty parts.
pub fn split(s: &str, split_char: char, skip_empty: bool) -> Vec<String> {
    let mut result = Vec::new();
    let mut prev = 0usize;
    for (pos, c) in s.char_indices() {
        if c == split_char {
            let part = &s[prev..pos];
            if !part.is_empty() || !skip_empty {
                result.push(part.to_string());
            }
            prev = pos + c.len_utf8();
        }
    }
    let part = &s[prev..];
    if !part.is_empty() || !skip_empty {
        result.push(part.to_string());
    }
    result
}

/// Joins strings with `separator`.
pub fn join(strings: &[String], separator: &str) -> String {
    strings.join(separator)
}

/// Port of `StringHelper::getSystemErrorString` using `FormatMessageW`.
pub fn get_system_error_string(status: i32) -> String {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::System::Diagnostics::Debug::{
        FormatMessageW, FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
        FORMAT_MESSAGE_IGNORE_INSERTS,
    };
    unsafe {
        let mut buf = windows::core::PWSTR::null();
        let result = FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            status as u32,
            0,
            windows::core::PWSTR(&mut buf as *mut _ as *mut u16),
            0,
            None,
        );
        if result != 0 && !buf.is_null() {
            let mut out = buf.to_string().unwrap_or_default();
            let _ = LocalFree(Some(HLOCAL(buf.0 as *mut _)));
            // The C++ code removes one trailing newline and one CR.
            if out.ends_with('\n') {
                out.pop();
            }
            if out.ends_with('\r') {
                out.pop();
            }
            out
        } else {
            String::new()
        }
    }
}

/// Port of `StringHelper::splitQuoted`.
pub fn split_quoted(s: &str, split_char: char, quote_char: char) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut result = Vec::new();
    let mut in_quotes = false;
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == split_char && !in_quotes {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
        } else if c == quote_char {
            in_quotes = !in_quotes;
            if in_quotes && i > 0 && chars[i - 1] == quote_char {
                current.push(quote_char);
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_illegal_filename_characters() {
        assert_eq!(
            replace_illegal_characters(r#"a<b>c:d"e/f\g|h?i*j"#),
            "a_b_c_d_e_f_g_h_i_j"
        );
        assert_eq!(replace_characters("abc", "b", "--"), "a--c");
        assert_eq!(replace_characters("abc", "", "_"), "abc");
    }

    #[test]
    fn utf8_round_trip() {
        let text = "Hällo Wörld \u{1F600}";
        let utf8 = string_to_utf8(text);
        assert_eq!(utf8_to_wstring(&utf8), text);
        assert_eq!(to_wstring(b"abc\0def", CP_UTF8), "abc");
    }

    #[test]
    fn case_and_trim_match_cpp_semantics() {
        assert_eq!(to_lower_case("AbC"), "abc");
        assert_eq!(to_upper_case("AbC"), "ABC");
        assert_eq!(trim("  \t x y \r\n "), "x y");
        assert_eq!(trim("   "), "");
    }

    #[test]
    fn split_and_join() {
        assert_eq!(split("a,b,,c", ',', true), vec!["a", "b", "c"]);
        assert_eq!(split("a,b,,c", ',', false), vec!["a", "b", "", "c"]);
        assert_eq!(split("", ',', false), vec![""]);
        assert_eq!(join(&["a".to_string(), "b".to_string()], ", "), "a, b");
    }

    #[test]
    fn split_quoted_skips_empty_parts() {
        assert_eq!(
            split_quoted(r#"a "b c" d"#, ' ', '"'),
            vec!["a".to_string(), "b c".to_string(), "d".to_string()]
        );
        // Two quote characters inside a quoted string produce one quote.
        assert_eq!(
            split_quoted(r#""a""b""#, ' ', '"'),
            vec!["a\"b".to_string()]
        );
    }

    #[test]
    fn system_error_string_has_message() {
        let message = get_system_error_string(2); // ERROR_FILE_NOT_FOUND
        assert!(!message.is_empty());
        assert!(!message.ends_with('\n'));
        assert!(!message.ends_with('\r'));
    }
}
