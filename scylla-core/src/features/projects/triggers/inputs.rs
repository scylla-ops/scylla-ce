//! The JSON form of the inputs of a trigger, as the `jsonb` column keeps them.

use scylla_domain::projects::triggers::{Input, InputValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An input in JSON.
#[derive(Serialize, Deserialize)]
struct InputJson {
    /// The name.
    name: String,
    /// A literal.
    literal: Option<String>,
    /// A JSON pointer.
    pointer: Option<String>,
}

/// Returns the JSON of inputs.
pub(crate) fn json(inputs: &[Input]) -> Value {
    let items: Vec<InputJson> = inputs
        .iter()
        .map(|input| match &input.value {
            InputValue::Literal(text) => InputJson {
                name: input.name.clone(),
                literal: Some(text.clone()),
                pointer: None,
            },
            InputValue::Pointer(pointer) => InputJson {
                name: input.name.clone(),
                literal: None,
                pointer: Some(pointer.clone()),
            },
        })
        .collect();
    serde_json::to_value(items).unwrap_or(Value::Null)
}

/// Reads inputs from their JSON, or `None` for bad data.
pub(crate) fn read(value: Value) -> Option<Vec<Input>> {
    let items: Vec<InputJson> = serde_json::from_value(value).ok()?;
    items
        .into_iter()
        .map(|input| {
            let value = match (input.literal, input.pointer) {
                (Some(text), None) => InputValue::Literal(text),
                (None, Some(pointer)) => InputValue::Pointer(pointer),
                _ => return None,
            };
            Some(Input {
                name: input.name,
                value,
            })
        })
        .collect()
}
