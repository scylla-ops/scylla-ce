//! The checks of the values of a fragment: its description, its tags, the lines of a version and
//! its parameters. Each error has the path of the value in the request.

use std::collections::HashSet;

use super::{NewOrigin, Parameter, ParameterValue};
use crate::projects::pipelines::{env_name, Node, NodeError};
use crate::DomainError;

/// The most characters in a description.
pub const MAX_DESCRIPTION: usize = 2000;

/// The most tags of a fragment.
pub const MAX_TAGS: usize = 16;

/// The most characters in a tag.
pub const MAX_TAG: usize = 32;

/// The most lines of what changed in a version.
pub const MAX_CHANGES: usize = 20;

/// The most characters in a line of what changed.
pub const MAX_CHANGE: usize = 200;

/// The most parameters of a version: one for each environment variable of a node at most.
pub const MAX_PARAMETERS: usize = Node::MAX_ENV;

/// The most characters in the description of a parameter.
pub const MAX_PARAMETER_DESCRIPTION: usize = 500;

/// Trims a description and checks its length.
pub fn description(raw: String) -> Result<String, NodeError> {
    let text = raw.trim().to_owned();
    if text.chars().count() > MAX_DESCRIPTION {
        let error = DomainError::TooLong {
            max: MAX_DESCRIPTION,
        };
        return Err(NodeError::at("description", error));
    }
    Ok(text)
}

/// Trims each tag, and refuses an empty, long, multi-line or second equal tag.
pub fn tags(raw: Vec<String>) -> Result<Vec<String>, NodeError> {
    if raw.len() > MAX_TAGS {
        return Err(NodeError::at(
            "tags",
            DomainError::TooMany { max: MAX_TAGS },
        ));
    }
    let mut seen = HashSet::new();
    raw.into_iter()
        .enumerate()
        .map(|(index, tag)| {
            let path = format!("tags[{index}]");
            let tag = line(tag, MAX_TAG).map_err(|error| NodeError::at(&path, error))?;
            if !seen.insert(tag.clone()) {
                return Err(NodeError::at(path, DomainError::Duplicate));
            }
            Ok(tag)
        })
        .collect()
}

/// Trims each line of what changed, and refuses an empty, long or multi-line one.
pub fn changes(raw: Vec<String>) -> Result<Vec<String>, NodeError> {
    if raw.len() > MAX_CHANGES {
        let error = DomainError::TooMany { max: MAX_CHANGES };
        return Err(NodeError::at("changes", error));
    }
    raw.into_iter()
        .enumerate()
        .map(|(index, change)| {
            line(change, MAX_CHANGE)
                .map_err(|error| NodeError::at(format!("changes[{index}]"), error))
        })
        .collect()
}

/// Checks the parameters against the checked nodes of their version: each key is an environment
/// variable of a node, once, a literal default fits in a node value, and a description is short.
pub fn parameters(raw: Vec<Parameter>, nodes: &[Node]) -> Result<Vec<Parameter>, NodeError> {
    if raw.len() > MAX_PARAMETERS {
        let error = DomainError::TooMany {
            max: MAX_PARAMETERS,
        };
        return Err(NodeError::at("parameters", error));
    }
    let variables: HashSet<&str> = nodes
        .iter()
        .flat_map(|node| node.env.iter().map(|var| var.name.as_str()))
        .collect();
    let mut keys = HashSet::new();
    raw.into_iter()
        .enumerate()
        .map(|(index, mut parameter)| {
            let at =
                |field: &str, error| NodeError::at(format!("parameters[{index}].{field}"), error);

            // Check the key: a variable of a node, once.
            env_name(&parameter.key).map_err(|error| at("key", error))?;
            if !variables.contains(parameter.key.as_str()) {
                return Err(at("key", DomainError::UnknownVariable));
            }
            if !keys.insert(parameter.key.clone()) {
                return Err(at("key", DomainError::Duplicate));
            }

            // Check the default of a literal and the description.
            if let ParameterValue::Literal(default) = &parameter.value {
                if default.len() > Node::MAX_TEXT {
                    let error = DomainError::TooLarge {
                        max: Node::MAX_TEXT,
                    };
                    return Err(at("default_value", error));
                }
            }
            parameter.description = parameter.description.trim().to_owned();
            if parameter.description.chars().count() > MAX_PARAMETER_DESCRIPTION {
                let error = DomainError::TooLong {
                    max: MAX_PARAMETER_DESCRIPTION,
                };
                return Err(at("description", error));
            }
            Ok(parameter)
        })
        .collect()
}

/// Checks the origin of a new fragment: an http or https registry, a package id and name of one
/// line, and a version of 1 or more.
pub fn origin(mut raw: NewOrigin) -> Result<NewOrigin, NodeError> {
    let registry = raw.registry.trim();
    let web = registry.starts_with("https://") || registry.starts_with("http://");
    if !web || registry.len() > 512 || registry.contains(char::is_whitespace) {
        return Err(NodeError::at("origin.registry", DomainError::NotAnUrl));
    }
    raw.registry = registry.to_owned();
    raw.package_id =
        line(raw.package_id, 128).map_err(|e| NodeError::at("origin.package_id", e))?;
    raw.package_name =
        line(raw.package_name, 200).map_err(|e| NodeError::at("origin.package_name", e))?;
    if raw.version == 0 {
        let error = DomainError::OutOfRange {
            min: 1,
            max: u32::MAX.into(),
        };
        return Err(NodeError::at("origin.version", error));
    }
    Ok(raw)
}

/// Trims a text of one line and checks its length.
fn line(raw: String, max: usize) -> Result<String, DomainError> {
    let text = raw.trim().to_owned();
    if text.is_empty() {
        return Err(DomainError::Empty);
    }
    if text.contains(['\n', '\r']) {
        return Err(DomainError::NotOneLine);
    }
    if text.chars().count() > max {
        return Err(DomainError::TooLong { max });
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::pipelines::{EnvValue, EnvVar, NodeId, Runtime, Step};

    /// Makes a node with one literal variable.
    fn node(variable: &str) -> Node {
        Node {
            id: NodeId::parse("build".to_owned()).unwrap(),
            needs: Vec::new(),
            step: Step::Exec(vec!["make".to_owned()]),
            env: vec![EnvVar {
                name: variable.to_owned(),
                value: EnvValue::Literal("1".to_owned()),
            }],
            timeout_seconds: Node::DEFAULT_TIMEOUT,
            retries: 0,
            working_dir: None,
            runtime: Runtime::Process,
        }
    }

    /// Makes a literal parameter.
    fn literal(key: &str) -> Parameter {
        Parameter {
            key: key.to_owned(),
            value: ParameterValue::Literal(String::new()),
            description: "  the mode  ".to_owned(),
        }
    }

    /// A tag is trimmed, and an empty tag, a second equal tag and a 17th tag are refused.
    #[test]
    fn tags_are_trimmed_and_unique() {
        assert_eq!(tags(vec![" ci ".to_owned()]).unwrap(), ["ci"]);
        assert_eq!(tags(vec![" ".to_owned()]).unwrap_err().path, "tags[0]");
        let twice = tags(vec!["ci".to_owned(), "ci ".to_owned()]).unwrap_err();
        assert_eq!(
            (twice.path.as_str(), twice.error),
            ("tags[1]", DomainError::Duplicate)
        );
        let many = vec!["t".to_owned(); MAX_TAGS + 1];
        assert_eq!(tags(many).unwrap_err().path, "tags");
    }

    /// A line of what changed is one line of at most 200 characters.
    #[test]
    fn changes_are_short_lines() {
        assert_eq!(
            changes(vec!["a\nb".to_owned()]).unwrap_err().error,
            DomainError::NotOneLine
        );
        let long = "x".repeat(MAX_CHANGE + 1);
        assert_eq!(changes(vec![long]).unwrap_err().path, "changes[0]");
        assert_eq!(changes(vec![" Faster ".to_owned()]).unwrap(), ["Faster"]);
    }

    /// A parameter names a variable of a node, once, and its description is trimmed.
    #[test]
    fn parameters_name_variables_of_the_nodes() {
        let nodes = [node("MODE")];
        let checked = parameters(vec![literal("MODE")], &nodes).unwrap();
        assert_eq!(checked[0].description, "the mode");
        let unknown = parameters(vec![literal("OTHER")], &nodes).unwrap_err();
        assert_eq!(unknown.path, "parameters[0].key");
        let twice = parameters(vec![literal("MODE"), literal("MODE")], &nodes).unwrap_err();
        assert_eq!(
            (twice.path.as_str(), twice.error),
            ("parameters[1].key", DomainError::Duplicate)
        );
    }

    /// A description of more than 2000 characters is refused.
    #[test]
    fn description_has_a_limit() {
        assert_eq!(description("  ok ".to_owned()).unwrap(), "ok");
        assert!(description("x".repeat(MAX_DESCRIPTION + 1)).is_err());
    }
}
