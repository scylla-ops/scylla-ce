use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct Normalizer {
    known: Vec<(String, String)>,
    ids: HashMap<String, String>,
    stamps: HashMap<String, String>,
    fixed: HashSet<String>,
}

impl Normalizer {
    pub fn new(known: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut seen = HashSet::new();
        let mut known: Vec<(String, String)> = known
            .into_iter()
            .filter(|(value, _)| value.len() >= 4 && seen.insert(value.clone()))
            .collect();
        known.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Self {
            known,
            ids: HashMap::new(),
            stamps: HashMap::new(),
            fixed: HashSet::new(),
        }
    }

    #[must_use]
    pub fn keep(mut self, values: &[&Value]) -> Self {
        for value in values {
            collect_ids(value, &mut self.fixed);
        }
        self
    }

    pub fn value(&mut self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.value(v)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.value(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    pub fn text(&mut self, text: &str) -> String {
        let mut out = text.to_string();
        for (value, name) in &self.known {
            if out.contains(value.as_str()) {
                out = replace_at_borders(&out, value, &format!("<{name}>"));
            }
        }
        let out = self.timestamps_in(&out);
        self.ids_in(&out)
    }

    fn stamp(&mut self, value: &str) -> String {
        let next = self.stamps.len() + 1;
        self.stamps
            .entry(value.to_string())
            .or_insert_with(|| format!("<ts:{next}>"))
            .clone()
    }

    fn timestamps_in(&mut self, text: &str) -> String {
        let b = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;
        while i < b.len() {
            if let Some(len) = timestamp_at(&b[i..]) {
                let label = self.stamp(&text[i..i + len]);
                out.push_str(&label);
                i += len;
            } else {
                let c = text[i..].chars().next().unwrap_or_default();
                out.push(c);
                i += c.len_utf8();
            }
        }
        out
    }

    fn ids_in(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut word = String::new();
        for c in text.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_alphanumeric() || c == '-' {
                word.push(c);
                continue;
            }
            if (is_ulid(&word) || is_uuid(&word)) && !self.fixed.contains(&word) {
                let next = self.ids.len() + 1;
                let id = self
                    .ids
                    .entry(word.clone())
                    .or_insert_with(|| format!("<id:{next}>"));
                out.push_str(id);
            } else {
                out.push_str(&word);
            }
            word.clear();
            out.push(c);
        }
        out.pop();
        out
    }
}

fn replace_at_borders(text: &str, value: &str, label: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut before: Option<char> = None;
    while let Some(at) = rest.find(value) {
        let prev = rest[..at].chars().next_back().or(before);
        let next = rest[at + value.len()..].chars().next();
        let border = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
        out.push_str(&rest[..at]);
        if border(prev) && border(next) {
            out.push_str(label);
        } else {
            out.push_str(value);
        }
        before = value.chars().next_back();
        rest = &rest[at + value.len()..];
    }
    out.push_str(rest);
    out
}

fn collect_ids(value: &Value, into: &mut HashSet<String>) {
    match value {
        Value::String(s) => {
            for word in s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
                if is_ulid(word) || is_uuid(word) {
                    into.insert(word.to_owned());
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_ids(v, into)),
        Value::Object(map) => map.values().for_each(|v| collect_ids(v, into)),
        _ => {}
    }
}

fn timestamp_at(b: &[u8]) -> Option<usize> {
    let digits = |range: std::ops::Range<usize>| {
        b.get(range)
            .is_some_and(|d| d.iter().all(u8::is_ascii_digit))
    };
    let shaped = digits(0..4)
        && b.get(4) == Some(&b'-')
        && digits(5..7)
        && b.get(7) == Some(&b'-')
        && digits(8..10)
        && b.get(10) == Some(&b'T')
        && digits(11..13)
        && b.get(13) == Some(&b':')
        && digits(14..16)
        && b.get(16) == Some(&b':')
        && digits(17..19);
    if !shaped {
        return None;
    }
    let mut end = 19;
    if b.get(end) == Some(&b'.') {
        end += 1;
        while b.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    (b.get(end) == Some(&b'Z')).then_some(end + 1)
}

pub fn is_ulid(word: &str) -> bool {
    word.len() == 26
        && word.starts_with(|c: char| ('0'..='7').contains(&c))
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || (c.is_ascii_alphabetic() && !"ILOUilou".contains(c)))
}

pub fn is_strict_ulid(word: &str) -> bool {
    word.len() == 26
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || (c.is_ascii_lowercase() && !"ilou".contains(c)))
}

fn is_uuid(word: &str) -> bool {
    let parts: Vec<&str> = word.split('-').collect();
    parts.len() == 5
        && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_id_written_in_the_scenario_stays_as_it_is() {
        let fixed = json!({"organization_id": {"value": "01m5tev79p6e446kzd1shmxz02"}});
        let mut n = Normalizer::new([]).keep(&[&fixed]);
        let out = n.text(
            "Organization '01m5tev79p6e446kzd1shmxz02' not found, 01m5tfzng7ze84zg64a74v0r5k",
        );
        assert_eq!(
            out,
            "Organization '01m5tev79p6e446kzd1shmxz02' not found, <id:1>"
        );
    }

    #[test]
    fn known_values_ids_and_timestamps_become_placeholders() {
        let mut n = Normalizer::new([("tok-secret-1".to_string(), "admin_token".to_string())]);
        let out = n.value(&json!({
            "token": "tok-secret-1",
            "id": "01JAQ3G3ZVZ9X1W0T2R9C4M5NB",
            "again": "Project 01JAQ3G3ZVZ9X1W0T2R9C4M5NB not found",
            "other": "6f1c2a9e-3b4d-4c5e-8f9a-0b1c2d3e4f5a",
            "at": "2026-10-25T20:53:54.123Z",
            "later": "Try again after 2026-10-25T21:48:06Z.",
        }));
        assert_eq!(
            out,
            json!({
                "token": "<admin_token>",
                "id": "<id:1>",
                "again": "Project <id:1> not found",
                "other": "<id:2>",
                "at": "<ts:1>",
                "later": "Try again after <ts:2>.",
            })
        );
    }

    #[test]
    fn equal_timestamps_share_a_label_and_a_new_value_gets_a_new_one() {
        let mut n = Normalizer::new([]);
        let out = n.value(&json!([
            {"created_at": "2026-10-25T20:53:54.123Z", "updated_at": "2026-10-25T20:53:54.123Z"},
            {"created_at": "2026-10-25T20:53:54.123Z", "updated_at": "2026-10-25T20:53:55Z"},
            "until 2026-10-25T20:53:55Z",
        ]));
        assert_eq!(
            out,
            json!([
                {"created_at": "<ts:1>", "updated_at": "<ts:1>"},
                {"created_at": "<ts:1>", "updated_at": "<ts:2>"},
                "until <ts:2>",
            ])
        );
    }

    #[test]
    fn a_known_value_inside_an_id_does_not_break_the_id() {
        let mut n = Normalizer::new([
            ("acme".to_string(), "org_name".to_string()),
            ("01m5tev79p6e446kzd1shmxz09".to_string(), "org".to_string()),
        ]);
        let out = n.text("acme 01jacmev79p6e446kzd1shmx 01m5tev79p6e446kzd1shmxz09 x-acme");
        assert_eq!(
            out,
            "<org_name> 01jacmev79p6e446kzd1shmx <org> x-<org_name>"
        );
        let id = "01jzacme79p6e446kzd1shmxz0";
        assert_eq!(n.text(id), "<id:1>");
    }

    #[test]
    fn a_strict_ulid_is_lowercase_crockford() {
        assert!(is_strict_ulid("01m5tev79p6e446kzd1shmxz02"));
        assert!(!is_strict_ulid("01M5TEV79P6E446KZD1SHMXZ02"));
        assert!(!is_strict_ulid("organization-admin"));
        assert!(!is_strict_ulid("6f1c2a9e-3b4d-4c5e-8f9a-0b1c2d3e4f5a"));
    }
}
