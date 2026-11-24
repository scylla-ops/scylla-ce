//! The switch and the merge: the nodes with no process. A switch checks its cases and opens
//! ports, and a merge joins branches again.

use std::collections::HashSet;

use super::needs::{NodeId, PortName};
use super::nodes::{text, var_name, Node, NodeError, Runtime, Step};
use crate::DomainError;

#[cfg(test)]
mod tests;

/// A switch: it checks its cases in order and opens the port of each case that matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    /// The cases, with unique ports.
    pub cases: Vec<Case>,
    /// The port to open when no case matches. `None` opens no port.
    pub otherwise: Option<PortName>,
    /// `true` opens the port of each case that matches, `false` the port of the first case only.
    pub all_matches: bool,
}

/// A case of a switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    /// The port that the case opens.
    pub port: PortName,
    /// How the rules combine.
    pub combinator: Combinator,
    /// The rules.
    pub rules: Vec<Rule>,
}

/// How the rules of a case combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combinator {
    /// Each rule must match.
    All,
    /// One rule or more must match.
    Any,
}

/// A rule of a case: a test of a left value against a right value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The left value.
    pub left: Operand,
    /// The test.
    pub test: Test,
    /// The right value. `None` for `Empty` and `NotEmpty` only.
    pub right: Option<Operand>,
}

/// A value of a rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    /// A text.
    Literal(String),
    /// A variable that the switch node gets.
    Var(String),
    /// A key of the output file of a run node before the switch.
    Output {
        /// The run node.
        node: NodeId,
        /// The key.
        key: String,
    },
}

/// The test of a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test {
    /// The texts are equal.
    Equals,
    /// The texts are not equal.
    NotEquals,
    /// The left text contains the right text.
    Contains,
    /// The left text does not contain the right text.
    NotContains,
    /// The left text starts with the right text.
    StartsWith,
    /// The left text ends with the right text.
    EndsWith,
    /// The left text is empty.
    Empty,
    /// The left text is not empty.
    NotEmpty,
    /// The numbers are equal.
    NumberEq,
    /// The numbers are not equal.
    NumberNe,
    /// The left number is greater.
    NumberGt,
    /// The left number is greater or equal.
    NumberGe,
    /// The left number is less.
    NumberLt,
    /// The left number is less or equal.
    NumberLe,
}

impl Test {
    /// Each test.
    pub const ALL: [Self; 14] = [
        Self::Equals,
        Self::NotEquals,
        Self::Contains,
        Self::NotContains,
        Self::StartsWith,
        Self::EndsWith,
        Self::Empty,
        Self::NotEmpty,
        Self::NumberEq,
        Self::NumberNe,
        Self::NumberGt,
        Self::NumberGe,
        Self::NumberLt,
        Self::NumberLe,
    ];

    /// Returns the name of the test, in snake case.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Equals => "equals",
            Self::NotEquals => "not_equals",
            Self::Contains => "contains",
            Self::NotContains => "not_contains",
            Self::StartsWith => "starts_with",
            Self::EndsWith => "ends_with",
            Self::Empty => "empty",
            Self::NotEmpty => "not_empty",
            Self::NumberEq => "number_eq",
            Self::NumberNe => "number_ne",
            Self::NumberGt => "number_gt",
            Self::NumberGe => "number_ge",
            Self::NumberLt => "number_lt",
            Self::NumberLe => "number_le",
        }
    }

    /// Reads the name of a test.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|test| test.as_str() == name)
    }

    /// Returns `true` if the test has a right value.
    pub fn has_right(self) -> bool {
        !matches!(self, Self::Empty | Self::NotEmpty)
    }

    /// Returns `true` for a test of numbers.
    pub fn is_number(self) -> bool {
        self.as_str().starts_with("number_")
    }
}

impl Switch {
    /// The most cases.
    pub const MAX_CASES: usize = 16;
    /// The most rules in a case.
    pub const MAX_RULES: usize = 32;

    /// Returns `true` if `port` is a port of a case or the otherwise port.
    pub fn has_port(&self, port: &PortName) -> bool {
        self.otherwise.as_ref() == Some(port) || self.cases.iter().any(|case| &case.port == port)
    }

    /// Returns `true` if a switch can open these ports: its own ports, each one time, and one
    /// port at most when `all_matches` is `false`.
    pub fn allows(&self, ports: &[PortName]) -> bool {
        let unique = ports.iter().collect::<HashSet<_>>().len() == ports.len();
        unique
            && (self.all_matches || ports.len() <= 1)
            && ports.iter().all(|port| self.has_port(port))
    }

    /// Returns each output operand with its path, for the check of the graph.
    pub(super) fn outputs<'a>(&'a self, at: &'a str) -> impl Iterator<Item = (String, &'a NodeId)> {
        self.cases.iter().enumerate().flat_map(move |(c, case)| {
            case.rules.iter().enumerate().flat_map(move |(r, rule)| {
                let sides = [("left", Some(&rule.left)), ("right", rule.right.as_ref())];
                sides
                    .into_iter()
                    .filter_map(move |(side, operand)| match operand {
                        Some(Operand::Output { node, .. }) => Some((
                            format!("{at}.cases[{c}].rules[{r}].{side}.output.node"),
                            node,
                        )),
                        _ => None,
                    })
            })
        })
    }

    /// Checks the cases and their rules, at the path `at` of the switch.
    pub(super) fn check(&self, at: &str) -> Result<(), NodeError> {
        // Check the count of the cases, and their ports.
        count(self.cases.len(), Self::MAX_CASES, format!("{at}.cases"))?;
        let mut ports = HashSet::new();
        let cases = self.cases.iter().enumerate();
        let cases = cases.map(|(c, case)| (format!("{at}.cases[{c}].port"), &case.port));
        let otherwise = self
            .otherwise
            .iter()
            .map(|port| (format!("{at}.otherwise"), port));
        for (path, port) in cases.chain(otherwise) {
            let error = match port.as_str() {
                PortName::ERROR => DomainError::ReservedPort,
                _ if !ports.insert(port) => DomainError::Duplicate,
                _ => continue,
            };
            return Err(NodeError::at(path, error));
        }

        // Check each rule.
        for (c, case) in self.cases.iter().enumerate() {
            let path = format!("{at}.cases[{c}].rules");
            count(case.rules.len(), Self::MAX_RULES, path.clone())?;
            for (r, rule) in case.rules.iter().enumerate() {
                let path = format!("{path}[{r}]");
                if rule.right.is_some() != rule.test.has_right() {
                    let error = match rule.right {
                        None => DomainError::Required,
                        Some(_) => DomainError::NotExpected,
                    };
                    return Err(NodeError::at(format!("{path}.right"), error));
                }
                operand(&rule.left, rule.test, &format!("{path}.left"))?;
                if let Some(right) = &rule.right {
                    operand(right, rule.test, &format!("{path}.right"))?;
                }
            }
        }
        Ok(())
    }
}

/// Checks the size of a list of 1 to `max` items.
fn count(len: usize, max: usize, path: String) -> Result<(), NodeError> {
    match len {
        0 => Err(NodeError::at(path, DomainError::Empty)),
        len if len > max => Err(NodeError::at(path, DomainError::TooMany { max })),
        _ => Ok(()),
    }
}

/// Checks one value of a rule: a text, a number for a number test, or the form of a name.
fn operand(operand: &Operand, test: Test, at: &str) -> Result<(), NodeError> {
    match operand {
        Operand::Literal(value) => {
            text(value, format!("{at}.literal"))?;
            let number = value.trim().parse::<f64>().is_ok_and(f64::is_finite);
            if test.is_number() && !number {
                return Err(NodeError::at(
                    format!("{at}.literal"),
                    DomainError::NotANumber,
                ));
            }
        }
        Operand::Var(name) => {
            var_name(name).map_err(|error| NodeError::at(format!("{at}.var"), error))?;
        }
        Operand::Output { key, .. } => {
            var_name(key).map_err(|error| NodeError::at(format!("{at}.output.key"), error))?;
        }
    }
    Ok(())
}

/// Checks the fields of a switch or of a merge: no retries, no work folder, and no container. A
/// merge also needs one need or more, and has no environment.
pub(super) fn check_flow(node: &Node, at: &str) -> Result<(), NodeError> {
    let merge = node.step == Step::Merge;
    let work = node
        .working_dir
        .as_deref()
        .is_some_and(|dir| !dir.is_empty());
    let container = matches!(node.runtime, Runtime::Container { .. });
    let fields = [
        ("retries", node.retries > 0),
        ("working_dir", work),
        ("container", container),
        ("env", merge && !node.env.is_empty()),
    ];
    if let Some((field, _)) = fields.into_iter().find(|(_, set)| *set) {
        return Err(NodeError::at(
            format!("{at}.{field}"),
            DomainError::NotExpected,
        ));
    }
    if merge && node.needs.is_empty() {
        return Err(NodeError::at(format!("{at}.needs"), DomainError::Empty));
    }
    Ok(())
}
