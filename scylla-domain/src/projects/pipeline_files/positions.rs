//! The places of the values of a pipeline file: a map from the path of a value, as the checks
//! write it (`nodes[2].needs[0]`), to its line and column. A key of a table is also at the index
//! of its order, so `nodes[0].env[1]` is the place of the second variable.

mod yaml;

use std::collections::HashMap;

use toml::de::{DeTable, DeValue};

use super::{FileFormat, Location};

/// The places of the keys and of the items of one file.
pub(super) struct Positions<'a> {
    /// The text of the file.
    text: &'a str,
    /// The place of each path, in bytes.
    places: HashMap<String, usize>,
}

impl<'a> Positions<'a> {
    /// Reads the places of a text that its parser accepted. A text that does not read gives no
    /// place.
    pub(super) fn of(format: FileFormat, text: &'a str) -> Self {
        let mut positions = Self {
            text,
            places: HashMap::new(),
        };
        match format {
            FileFormat::Toml => {
                if let Ok(root) = DeTable::parse(text) {
                    positions.toml_table(root.get_ref(), "");
                }
            }
            FileFormat::Yaml => yaml::places(text, &mut positions),
            FileFormat::Json => {
                let mut scan = JsonScan { text, at: 0 };
                scan.value("", &mut positions);
            }
        }
        positions
    }

    /// Returns the place of a path, or of its nearest parent with a place.
    pub(super) fn find(&self, path: &str) -> Option<Location> {
        let mut path = path;
        loop {
            if let Some(offset) = self.places.get(path) {
                return Some(locate(self.text, *offset));
            }
            path = &path[..path.rfind(['.', '['])?];
        }
    }

    /// Returns the byte offset of a line and a column, both from 1.
    fn offset(&self, line: usize, column: usize) -> usize {
        let Some(start) = line_starts(self.text).nth(line.saturating_sub(1)) else {
            return self.text.len();
        };
        self.text[start..]
            .char_indices()
            .nth(column.saturating_sub(1))
            .map_or(self.text.len(), |(at, _)| start + at)
    }

    /// Keeps the place of a path. The first place of a path stays.
    fn record(&mut self, path: String, offset: usize) {
        self.places.entry(path).or_insert(offset);
    }

    /// Keeps the place of the key `key` of the table at `at`, the entry `index` in its order.
    fn record_key(&mut self, at: &str, key: &str, index: usize, offset: usize) -> String {
        let path = child(at, key);
        self.record(path.clone(), offset);
        self.record(format!("{at}[{index}]"), offset);
        path
    }

    /// Keeps the places of the keys of a TOML table at `at`, and of their values.
    fn toml_table(&mut self, table: &DeTable<'_>, at: &str) {
        for (index, (key, value)) in table.iter().enumerate() {
            let path = self.record_key(at, key.get_ref(), index, key.span().start);
            self.toml_value(value.get_ref(), &path);
        }
    }

    /// Keeps the places in a TOML value at `at`.
    fn toml_value(&mut self, value: &DeValue<'_>, at: &str) {
        match value {
            DeValue::Table(table) => self.toml_table(table, at),
            DeValue::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{at}[{index}]");
                    self.record(path.clone(), item.span().start);
                    self.toml_value(item.get_ref(), &path);
                }
            }
            _ => {}
        }
    }
}

/// Returns the line and the column of a byte offset in a text.
pub(super) fn locate(text: &str, offset: usize) -> Location {
    let offset = offset.min(text.len());
    let start = line_starts(text)
        .take_while(|start| *start <= offset)
        .last()
        .unwrap_or(0);
    let line = text[..start].matches('\n').count() + 1;
    let column = text
        .get(start..offset)
        .map_or(0, |part| part.chars().count())
        + 1;
    Location {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

/// Returns the byte offset of the start of each line of a text.
fn line_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1))
}

/// Returns the path of the key `key` of the table at `at`.
fn child(at: &str, key: &str) -> String {
    if at.is_empty() {
        key.to_owned()
    } else {
        format!("{at}.{key}")
    }
}

/// A scan of a JSON text that its parser accepted: it keeps the place of each key and item.
struct JsonScan<'a> {
    /// The text.
    text: &'a str,
    /// The next byte.
    at: usize,
}

impl JsonScan<'_> {
    /// Returns the next byte that is not a space.
    fn next(&mut self) -> Option<u8> {
        let bytes = self.text.as_bytes();
        while bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
        bytes.get(self.at).copied()
    }

    /// Reads one value at the path `at`, and keeps the places in it. `None` at the end of the
    /// text.
    fn value(&mut self, at: &str, positions: &mut Positions<'_>) -> Option<()> {
        match self.next()? {
            b'{' => self.object(at, positions),
            b'[' => self.array(at, positions),
            b'"' => self.string().map(|_| ()),
            _ => {
                let bytes = self.text.as_bytes();
                while bytes.get(self.at).is_some_and(|byte| {
                    !matches!(byte, b',' | b']' | b'}') && !byte.is_ascii_whitespace()
                }) {
                    self.at += 1;
                }
                Some(())
            }
        }
    }

    /// Reads an object at the path `at`: the place of each key, and its value.
    fn object(&mut self, at: &str, positions: &mut Positions<'_>) -> Option<()> {
        self.at += 1;
        let mut index = 0;
        loop {
            match self.next()? {
                b'}' => break,
                b',' => self.at += 1,
                _ => {
                    let start = self.at;
                    let key = self.string()?;
                    let path = positions.record_key(at, &key, index, start);
                    index += 1;
                    self.next()?;
                    self.at += 1;
                    self.value(&path, positions)?;
                }
            }
        }
        self.at += 1;
        Some(())
    }

    /// Reads an array at the path `at`: the place of each item, and the item.
    fn array(&mut self, at: &str, positions: &mut Positions<'_>) -> Option<()> {
        self.at += 1;
        let mut index = 0;
        loop {
            match self.next()? {
                b']' => break,
                b',' => self.at += 1,
                _ => {
                    let path = format!("{at}[{index}]");
                    positions.record(path.clone(), self.at);
                    index += 1;
                    self.value(&path, positions)?;
                }
            }
        }
        self.at += 1;
        Some(())
    }

    /// Reads a JSON string at the next byte and returns its text.
    fn string(&mut self) -> Option<String> {
        let bytes = self.text.as_bytes();
        let start = self.at;
        self.at += 1;
        loop {
            match bytes.get(self.at)? {
                b'\\' => self.at += 2,
                b'"' => break,
                _ => self.at += 1,
            }
        }
        self.at += 1;
        serde_json::from_str(self.text.get(start..self.at)?).ok()
    }
}
