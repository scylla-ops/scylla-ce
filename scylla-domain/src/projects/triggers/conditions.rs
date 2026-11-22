//! The conditions of a webhook trigger: what a delivery must hold to start a job, their checks,
//! and their match on the headers and the body of a delivery.

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::projects::git_rules::globs;
use crate::DomainError;

/// The most conditions of one webhook.
pub const MAX_CONDITIONS: usize = 32;

/// The most values of one condition.
pub const MAX_CONDITION_VALUES: usize = 32;

/// The longest value of a condition, in characters.
pub const MAX_CONDITION_VALUE_LEN: usize = 1024;

/// The longest header name or JSON pointer of a condition, in characters.
pub const MAX_CONDITION_FIELD_LEN: usize = 256;

/// The longest value of a delivery in the reason of a skip, in characters.
const MAX_REASON_VALUE_LEN: usize = 200;

/// The rule of a webhook trigger: a delivery starts a job only when all its conditions match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookRule {
    /// The conditions. No condition: each delivery starts a job.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

/// A condition on a delivery of a webhook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// The value of the delivery that the condition reads.
    pub source: ConditionSource,
    /// How the value compares with the values of the condition.
    pub operator: ConditionOperator,
    /// The values of the condition.
    pub values: Vec<String>,
}

/// The value of a delivery that a condition reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionSource {
    /// A header, by its name. Upper and lower case are the same.
    Header(String),
    /// A JSON pointer (RFC 6901) into the body, for example `/ref`.
    Pointer(String),
}

/// How a condition compares the value of a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    /// The value is equal to the one value of the condition.
    Is,
    /// The value matches the one glob of the condition, as a glob of the branches.
    Matches,
    /// The value is equal to one of the values of the condition.
    OneOf,
}

/// What a delivery gives to the conditions: its headers and its body.
pub struct Delivery<'a> {
    /// Returns the value of a header by its name in lower case, or `None` with no such header.
    pub header: &'a dyn Fn(&str) -> Option<String>,
    /// The body, or `None` when it is not JSON.
    pub body: Option<&'a Value>,
}

impl WebhookRule {
    /// Checks the conditions at the path `at`: at most 32, each one valid. Returns the path and
    /// the rule of the first refusal.
    pub fn check(&self, at: &str) -> Result<(), (String, DomainError)> {
        let at = format!("{at}.conditions");
        if self.conditions.len() > MAX_CONDITIONS {
            let error = DomainError::TooMany {
                max: MAX_CONDITIONS,
            };
            return Err((at, error));
        }
        self.conditions
            .iter()
            .enumerate()
            .try_for_each(|(index, condition)| condition.check(&format!("{at}[{index}]")))
    }

    /// Returns the reason of the first condition that the delivery does not match, or `None`
    /// when each condition matches.
    pub fn skip_reason(&self, delivery: &Delivery<'_>) -> Option<String> {
        self.conditions
            .iter()
            .find_map(|condition| condition.miss(delivery))
    }
}

impl Condition {
    /// Checks the source and the values of a condition at the path `at`.
    fn check(&self, at: &str) -> Result<(), (String, DomainError)> {
        // Check the header name or the JSON pointer.
        match &self.source {
            ConditionSource::Header(name) => {
                let at = format!("{at}.header");
                if name.trim().is_empty() {
                    return Err((at, DomainError::Empty));
                }
                if name.chars().count() > MAX_CONDITION_FIELD_LEN || !is_token(name) {
                    return Err((at, DomainError::NotAHeaderName));
                }
            }
            ConditionSource::Pointer(pointer) => {
                let at = format!("{at}.json_pointer");
                if !pointer.starts_with('/') || pointer.chars().count() > MAX_CONDITION_FIELD_LEN {
                    return Err((at, DomainError::NotABodyPointer));
                }
            }
        }

        // Check the count of the values, then each value.
        let at = format!("{at}.values");
        let most = match self.operator {
            ConditionOperator::Is | ConditionOperator::Matches => 1,
            ConditionOperator::OneOf => MAX_CONDITION_VALUES,
        };
        if self.values.is_empty() {
            return Err((at, DomainError::Empty));
        }
        if self.values.len() > most {
            return Err((at, DomainError::TooMany { max: most }));
        }
        for (index, value) in self.values.iter().enumerate() {
            let at = format!("{at}[{index}]");
            if value.trim().is_empty() {
                return Err((at, DomainError::Empty));
            }
            if value.chars().count() > MAX_CONDITION_VALUE_LEN {
                let error = DomainError::TooLong {
                    max: MAX_CONDITION_VALUE_LEN,
                };
                return Err((at, error));
            }
            if self.values[..index].contains(value) {
                return Err((at, DomainError::Duplicate));
            }
        }

        // Check the glob of a match.
        match self.operator {
            ConditionOperator::Matches => match globs::check_globs(&at, &self.values).pop() {
                Some(refused) => Err(refused),
                None => Ok(()),
            },
            ConditionOperator::Is | ConditionOperator::OneOf => Ok(()),
        }
    }

    /// Returns the reason when the delivery does not match the condition, or `None`.
    fn miss(&self, delivery: &Delivery<'_>) -> Option<String> {
        // Read the value of the delivery.
        let (name, value) = match &self.source {
            ConditionSource::Header(name) => {
                (name.as_str(), (delivery.header)(&name.to_ascii_lowercase()))
            }
            ConditionSource::Pointer(pointer) => {
                let Some(body) = delivery.body else {
                    return Some("No match: the body is not JSON".to_owned());
                };
                (pointer.as_str(), body.pointer(pointer).map(text))
            }
        };
        let Some(value) = value else {
            return Some(format!("No match: {name} is missing"));
        };

        // Compare it with the values of the condition.
        let matches = match self.operator {
            ConditionOperator::Is | ConditionOperator::OneOf => self.values.contains(&value),
            ConditionOperator::Matches => globs::any(&self.values, &value),
        };
        (!matches).then(|| format!("No match: {name} is {}", short(&value)))
    }
}

/// Returns the text of a value of the body: a text gives the text, null an empty text, and
/// another value its JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Returns a value cut to the longest length of a reason.
fn short(value: &str) -> String {
    match value.char_indices().nth(MAX_REASON_VALUE_LEN) {
        Some((end, _)) => format!("{}...", &value[..end]),
        None => value.to_owned(),
    }
}

/// Returns `true` if the name is an HTTP token: letters, digits and ``!#$%&'*+-.^_`|~``.
fn is_token(name: &str) -> bool {
    name.chars()
        .all(|char| char.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(char))
}
