use crate::scenario::Record;
use anyhow::{Context, Result};
use std::path::Path;

pub fn read(path: &Path) -> Result<Vec<Record>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .with_context(|| format!("parse a line of {}", path.display()))
        })
        .collect()
}

pub fn write(path: &Path, records: &[Record]) -> Result<()> {
    let mut text = String::new();
    for record in records {
        text.push_str(&serde_json::to_string(record)?);
        text.push('\n');
    }
    std::fs::write(path, text).with_context(|| format!("write {}", path.display()))
}

fn without(record: &Record, ignore_meta: &[String]) -> Record {
    let mut record = record.clone();
    record.meta.retain(|key, _| !ignore_meta.contains(key));
    record
}

pub fn differences(left: &[Record], right: &[Record], ignore_meta: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for index in 0..left.len().max(right.len()) {
        let l = left.get(index).map(|r| without(r, ignore_meta));
        let r = right.get(index).map(|r| without(r, ignore_meta));
        match (l, r) {
            (Some(l), Some(r)) if l == r => {}
            (Some(l), Some(r)) => out.push(format!(
                "step {index} {} {}/{}:\n  - {}\n  + {}",
                l.label,
                l.service,
                l.rpc,
                serde_json::to_string(&l).unwrap_or_default(),
                serde_json::to_string(&r).unwrap_or_default()
            )),
            (Some(l), None) => out.push(format!(
                "step {index} {}/{}: only in the first output",
                l.service, l.rpc
            )),
            (None, Some(r)) => out.push(format!(
                "step {index} {}/{}: only in the second output",
                r.service, r.rpc
            )),
            (None, None) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Meta;
    use serde_json::json;

    fn record(code: &str) -> Record {
        Record::local(String::new(), "r", code, String::new(), json!({}))
    }

    #[test]
    fn equal_outputs_have_no_difference() {
        assert!(differences(&[record("OK")], &[record("OK")], &[]).is_empty());
    }

    #[test]
    fn a_different_code_or_a_missing_line_is_a_difference() {
        assert_eq!(
            differences(&[record("OK")], &[record("NOT_FOUND")], &[]).len(),
            1
        );
        assert_eq!(
            differences(&[record("OK"), record("OK")], &[record("OK")], &[]).len(),
            1
        );
    }

    #[test]
    fn a_meta_difference_shows_unless_it_is_ignored() {
        let mut echo = record("OK");
        echo.meta = Meta::from([("x-request-id".to_string(), "parity-1".to_string())]);
        assert_eq!(differences(&[record("OK")], &[echo.clone()], &[]).len(), 1);
        assert!(differences(&[record("OK")], &[echo], &["x-request-id".to_string()]).is_empty());
    }
}
