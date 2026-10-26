use serde_json::{Map, Value};
use std::path::Path;

pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

pub fn lines_from(path: &Path, offset: u64) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(start, |i| (i + 1).max(start));
    let text = strip_ansi(&String::from_utf8_lossy(&bytes[start..end]));
    text.lines().map(str::to_string).collect()
}

pub fn matches(line: &str, target: &str, message: &str) -> bool {
    line.split_whitespace().any(|word| {
        word.trim_end_matches(':') == target || word.starts_with(&format!("{target}::"))
    }) && line.contains(message)
}

pub fn fields(line: &str, message: &str) -> Map<String, Value> {
    let rest = line.split_once(message).map_or("", |(_, rest)| rest);
    rest.split_whitespace()
        .filter_map(|word| word.split_once('='))
        .map(|(k, v)| {
            (
                k.to_string(),
                Value::String(v.trim_matches('"').to_string()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_gives_its_fields_after_the_message() {
        let line = "\u{1b}[2m2026-10-26T10:00:00.000001Z\u{1b}[0m  INFO scylla_core: password reset link user_id=u1 link=http://h/reset-password#token=abc";
        let line = strip_ansi(line);
        assert!(matches(&line, "scylla_core", "password reset link"));
        assert!(!matches(&line, "scylla_strata", "password reset link"));
        let fields = fields(&line, "password reset link");
        assert_eq!(fields["user_id"], "u1");
        assert_eq!(fields["link"], "http://h/reset-password#token=abc");
    }
}
