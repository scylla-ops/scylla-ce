//! The parts of a log line: the fields of an event and of its call, and the outcome of a call.

use std::fmt;

use tracing::field::{Field, Visit};
use tracing::Level;

/// A value of a field.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A text, or a value that the code formats.
    Text(String),
    /// A signed number.
    Int(i64),
    /// An unsigned number.
    Uint(u64),
    /// A number with a fraction.
    Float(f64),
    /// A flag.
    Bool(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Int(number) => write!(f, "{number}"),
            Self::Uint(number) => write!(f, "{number}"),
            Self::Float(number) => write!(f, "{number}"),
            Self::Bool(flag) => write!(f, "{flag}"),
        }
    }
}

/// The fields of an event or of a span, in their order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields(Vec<(&'static str, Value)>);

impl Fields {
    /// Returns the value of a field.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value)
    }

    /// Returns the value of a field as text.
    pub fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(Value::to_string)
    }

    /// Sets a field, in place of its old value.
    pub fn set(&mut self, name: &'static str, value: Value) {
        match self.0.iter_mut().find(|(key, _)| *key == name) {
            Some((_, old)) => *old = value,
            None => self.0.push((name, value)),
        }
    }

    /// Removes a field and returns its value as text.
    pub fn take(&mut self, name: &str) -> Option<String> {
        let index = self.0.iter().position(|(key, _)| *key == name)?;
        Some(self.0.remove(index).1.to_string())
    }

    /// Returns the fields in their order.
    pub fn iter(&self) -> impl Iterator<Item = &(&'static str, Value)> {
        self.0.iter()
    }
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.set(field.name(), Value::Text(value.to_owned()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.set(field.name(), Value::Int(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.set(field.name(), Value::Uint(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.set(field.name(), Value::Float(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.set(field.name(), Value::Bool(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.set(field.name(), Value::Text(format!("{value:?}")));
    }
}

/// What a call told about its result, beyond its code: the refusal of the gate, the details of
/// the answer, and the error of a failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The permission that the gate refused.
    pub permission: Option<String>,
    /// The object of the refused permission.
    pub object: Option<String>,
    /// The reason of a refusal: the `ErrorInfo` reason, or the reason of the gate.
    pub reason: Option<String>,
    /// The `ErrorInfo` metadata, in key order.
    pub metadata: Vec<(String, String)>,
    /// The fields of the request that are not valid, with their problem.
    pub violations: Vec<(String, String)>,
    /// The message of the answer.
    pub message: Option<String>,
    /// The error of a failure of the server, which the client does not see.
    pub error: Option<String>,
}

/// One line to write: an event, with the fields of its call.
#[derive(Clone, Debug)]
pub struct Line {
    /// The level of the event.
    pub level: Level,
    /// The module of the code that logs it.
    pub target: String,
    /// The name of the event: `rpc` or the name of a business event.
    pub event: Option<String>,
    /// The message of the event.
    pub message: Option<String>,
    /// The other fields of the event.
    pub fields: Fields,
    /// The fields of the call of the event: request id, route, IP address, actor.
    pub call: Option<Fields>,
    /// The outcome of the call, on the line of the call.
    pub outcome: Option<Outcome>,
}
