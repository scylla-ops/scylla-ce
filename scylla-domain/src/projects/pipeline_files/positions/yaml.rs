//! The places of the keys and of the items of a YAML file, from the spans of its parser.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_saphyr::Spanned;

use super::Positions;
use crate::projects::pipeline_files::read;

/// The shape of a YAML value, with the place of each key and item.
enum Shape {
    /// A mapping: each key and its value.
    Map(Vec<(Spanned<Key>, Spanned<Shape>)>),
    /// A sequence of items.
    Seq(Vec<Spanned<Shape>>),
    /// A scalar.
    Leaf,
}

/// The text of a key of a mapping.
struct Key(String);

/// Keeps the place of each key and item of a YAML text. A text that does not read gives no
/// place.
pub(super) fn places(text: &str, positions: &mut Positions<'_>) {
    if let Ok(shape) = serde_saphyr::from_str_with_options::<Shape>(text, read::yaml_options()) {
        walk(&shape, "", positions);
    }
}

/// Keeps the places in a value at `at`.
fn walk(shape: &Shape, at: &str, positions: &mut Positions<'_>) {
    match shape {
        Shape::Map(entries) => {
            for (index, (key, value)) in entries.iter().enumerate() {
                let offset = offset(&key.referenced, positions);
                let path = positions.record_key(at, &key.value.0, index, offset);
                walk(&value.value, &path, positions);
            }
        }
        Shape::Seq(items) => {
            for (index, item) in items.iter().enumerate() {
                let path = format!("{at}[{index}]");
                positions.record(path.clone(), offset(&item.referenced, positions));
                walk(&item.value, &path, positions);
            }
        }
        Shape::Leaf => {}
    }
}

/// Returns the byte offset of a place of the parser.
fn offset(location: &serde_saphyr::Location, positions: &Positions<'_>) -> usize {
    let span = location.span();
    span.byte_offset()
        .and_then(|offset| usize::try_from(offset).ok())
        .unwrap_or_else(|| {
            let line = usize::try_from(location.line()).unwrap_or(0);
            positions.offset(line, usize::try_from(location.column()).unwrap_or(0))
        })
}

impl<'de> Deserialize<'de> for Shape {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ShapeVisitor)
    }
}

/// Reads the shape of any YAML value.
struct ShapeVisitor;

impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a YAML value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Shape, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Shape::Map(entries))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Shape, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Shape::Seq(items))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Shape, D::Error> {
        Shape::deserialize(deserializer)
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_i128<E: de::Error>(self, _: i128) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_u128<E: de::Error>(self, _: u128) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_none<E: de::Error>(self) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Shape, E> {
        Ok(Shape::Leaf)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(KeyVisitor)
    }
}

/// Reads a scalar key as its text.
struct KeyVisitor;

impl Visitor<'_> for KeyVisitor {
    type Value = Key;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a scalar key")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Key, E> {
        Ok(Key(value.to_string()))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Key, E> {
        Ok(Key(value.to_owned()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Key, E> {
        Ok(Key(String::new()))
    }
}
