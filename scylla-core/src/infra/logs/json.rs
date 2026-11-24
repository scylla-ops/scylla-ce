//! The JSON lines, for a machine: one object for each line, with the fields of the call and of
//! the event.

use serde_json::{Map, Value as Json};

use super::record::{Fields, Line, Value};

/// The fields of the call, in the order of a line.
const CALL_FIELDS: [&str; 4] = ["actor", "actor_kind", "ip", "request_id"];

/// Writes one JSON object and a line break. `ts` is the time in RFC 3339, in UTC.
pub(super) fn render(line: &Line, ts: &str) -> String {
    // Start with the time, the level and the name of the event.
    let mut object = Object::default();
    object.text("ts", ts);
    object.text("level", &line.level.as_str().to_lowercase());
    object.text("event", line.event.as_deref().unwrap_or("log"));

    // Add the route, the result and the context of the call.
    let rpc = line.event.as_deref() == Some("rpc");
    if let Some(call) = &line.call {
        object.field(call, "rpc");
        if rpc {
            object.field(&line.fields, "code");
            object.field(&line.fields, "status");
            object.field(&line.fields, "ms");
        }
        for name in CALL_FIELDS {
            match call.get(name) {
                Some(value) => object.value(name, value),
                None if rpc && name != "actor_kind" => object.put(name, Json::Null),
                None => {}
            }
        }
    }

    // Add the outcome of a call.
    if let Some(outcome) = &line.outcome {
        object.option("permission", &outcome.permission);
        object.option("object", &outcome.object);
        object.option("reason", &outcome.reason);
        object.pairs("metadata", &outcome.metadata);
        object.pairs("violations", &outcome.violations);
        object.option("message", &outcome.message);
        object.option("error", &outcome.error);
    }

    // Add the message, the module of a plain line, and the other fields of the event.
    if line.event.is_none() {
        object.text("target", &line.target);
    }
    object.option("message", &line.message);
    for (name, value) in line.fields.iter() {
        if !(rpc && matches!(*name, "code" | "status" | "ms")) {
            object.value(name, value);
        }
    }
    object.finish()
}

/// A JSON object that keeps the order of its keys.
#[derive(Default)]
struct Object {
    /// The text of the object so far, with no braces.
    text: String,
}

impl Object {
    /// Adds a key and a JSON value.
    fn put(&mut self, key: &str, value: Json) {
        if !self.text.is_empty() {
            self.text.push(',');
        }
        self.text
            .push_str(&Json::String(key.to_owned()).to_string());
        self.text.push(':');
        self.text.push_str(&value.to_string());
    }

    /// Adds a text.
    fn text(&mut self, key: &str, value: &str) {
        self.put(key, Json::String(value.to_owned()));
    }

    /// Adds a text if it is there.
    fn option(&mut self, key: &str, value: &Option<String>) {
        if let Some(value) = value {
            self.text(key, value);
        }
    }

    /// Adds a value of a field.
    fn value(&mut self, key: &str, value: &Value) {
        let json = match value {
            Value::Text(text) => Json::String(text.clone()),
            Value::Int(number) => Json::from(*number),
            Value::Uint(number) => Json::from(*number),
            Value::Float(number) => Json::from(*number),
            Value::Bool(flag) => Json::Bool(*flag),
        };
        self.put(key, json);
    }

    /// Adds a field of a set of fields, if it is there.
    fn field(&mut self, fields: &Fields, key: &str) {
        if let Some(value) = fields.get(key) {
            self.value(key, value);
        }
    }

    /// Adds pairs of texts as an object, if there are any.
    fn pairs(&mut self, key: &str, pairs: &[(String, String)]) {
        if pairs.is_empty() {
            return;
        }
        let map: Map<String, Json> = pairs
            .iter()
            .map(|(name, value)| (name.clone(), Json::String(value.clone())))
            .collect();
        self.put(key, Json::Object(map));
    }

    /// Returns the object and a line break.
    fn finish(self) -> String {
        format!("{{{}}}\n", self.text)
    }
}
