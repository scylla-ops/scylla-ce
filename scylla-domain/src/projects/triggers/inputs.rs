//! The inputs of a trigger: the variables that each node of a job of the trigger gets.

use crate::projects::pipelines::env_name;
use crate::DomainError;

/// The value of an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputValue {
    /// A text.
    Literal(String),
    /// A JSON pointer (RFC 6901) into the body of a webhook, for example `/after`.
    Pointer(String),
}

/// An input of a trigger: a variable that each node of the job gets. A variable of the node with
/// the same name wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// The name of the variable.
    pub name: String,
    /// The value.
    pub value: InputValue,
}

/// The checked inputs of a trigger.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inputs(Vec<Input>);

impl Inputs {
    /// The most inputs.
    pub const MAX: usize = 128;

    /// Checks the inputs: at most 128, unique names of variables, a pointer only for a webhook,
    /// and a pointer that is empty or starts with `/`. Returns the path and the rule of a refusal.
    pub fn parse(inputs: Vec<Input>, webhook: bool) -> Result<Self, (String, DomainError)> {
        if inputs.len() > Self::MAX {
            return Err(("inputs".to_owned(), DomainError::TooMany { max: Self::MAX }));
        }
        let mut names = std::collections::HashSet::new();
        for (index, input) in inputs.iter().enumerate() {
            let at = format!("inputs[{index}]");
            env_name(&input.name).map_err(|error| (format!("{at}.name"), error))?;
            if !names.insert(input.name.as_str()) {
                return Err((format!("{at}.name"), DomainError::Duplicate));
            }
            match &input.value {
                InputValue::Literal(text) if text.len() > 64 << 10 => {
                    let error = DomainError::TooLarge { max: 64 << 10 };
                    return Err((format!("{at}.literal"), error));
                }
                InputValue::Pointer(_) if !webhook => {
                    return Err((format!("{at}.json_pointer"), DomainError::NotAllowed));
                }
                InputValue::Pointer(pointer)
                    if !pointer.is_empty() && !pointer.starts_with('/') =>
                {
                    return Err((format!("{at}.json_pointer"), DomainError::NotAPointer));
                }
                _ => {}
            }
        }
        Ok(Self(inputs))
    }

    /// Returns the inputs.
    pub fn as_slice(&self) -> &[Input] {
        &self.0
    }
}
