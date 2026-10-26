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

/// A log line: its target (or a module under it), its text (a `*` matches any characters) and its fields.
#[derive(Clone, Debug)]
pub struct LogLine {
    target: String,
    text: String,
    fields: Vec<(String, String)>,
    pub(crate) label: String,
}

impl LogLine {
    #[must_use]
    pub fn new(target: &str, text: &str) -> Self {
        Self {
            target: target.to_owned(),
            text: text.to_owned(),
            fields: Vec::new(),
            label: format!("{target}: {text}"),
        }
    }

    #[must_use]
    pub fn with_field(mut self, key: &str, value: &str) -> Self {
        self.fields.push((key.to_owned(), value.to_owned()));
        self.label = format!("{} {key}={value}", self.label);
        self
    }

    pub(crate) fn labelled(mut self, label: String) -> Self {
        self.label = label;
        self
    }

    pub(crate) fn is(&self, line: &str) -> bool {
        let Some(rest) = after_target(line, &self.target) else {
            return false;
        };
        let mut tail = rest;
        for part in self.text.split('*') {
            match tail.find(part) {
                Some(i) => tail = &tail[i + part.len()..],
                None => return false,
            }
        }
        let head = self.text.split('*').next().unwrap_or_default();
        let found = fields(rest, head);
        self.fields
            .iter()
            .all(|(k, v)| found.get(k).and_then(Value::as_str) == Some(v.as_str()))
    }

    pub(crate) fn count(&self, text: &str) -> usize {
        text.lines().filter(|line| self.is(line)).count()
    }
}

const LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

fn after_target<'a>(line: &'a str, target: &str) -> Option<&'a str> {
    let mut words = line.split_whitespace();
    words.find(|word| LEVELS.contains(word))?;
    let word = words.next()?;
    let name = word.trim_end_matches(':');
    if name != target && !name.starts_with(&format!("{target}::")) {
        return None;
    }
    let start = line.find(word)? + word.len();
    Some(&line[start..])
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
