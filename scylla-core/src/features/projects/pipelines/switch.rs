//! The conversions of the needs and of a switch: the proto messages, and the JSON of the `jsonb`
//! columns. A bad value of a request gives `INVALID_ARGUMENT` with its path.

use scylla_domain::projects::pipelines::{
    Case, Combinator, Need, NodeId, Operand, PortName, Rule, Switch, Test,
};
use scylla_proto::pipelines::v1 as pb;
use serde::{Deserialize, Serialize};

use super::json::bad;
use crate::Error;

/// Reads the needs of a node of a request, at the path `at` of the node.
pub(super) fn needs_of(needs: Vec<pb::Need>, at: &str) -> Result<Vec<Need>, Error> {
    needs
        .into_iter()
        .enumerate()
        .map(|(index, need)| {
            let at = format!("{at}.needs[{index}]");
            let node = NodeId::parse(need.node)
                .map_err(|error| Error::invalid(format!("{at}.node"), error.to_string()))?;
            let port = port_of(need.port, &format!("{at}.port"))?;
            Ok(Need { node, port })
        })
        .collect()
}

/// Returns the proto of needs.
pub(crate) fn needs(needs: Vec<Need>) -> Vec<pb::Need> {
    needs
        .into_iter()
        .map(|need| pb::Need {
            node: need.node.as_str().to_owned(),
            port: need
                .port
                .map(|port| port.as_str().to_owned())
                .unwrap_or_default(),
        })
        .collect()
}

/// Reads an optional port of a request. Empty gives `None`.
fn port_of(port: String, at: &str) -> Result<Option<PortName>, Error> {
    if port.is_empty() {
        return Ok(None);
    }
    let port = PortName::parse(port).map_err(|error| Error::invalid(at, error.to_string()))?;
    Ok(Some(port))
}

/// Reads a switch of a request, at the path `at` of the switch.
pub(super) fn switch_of(switch: pb::Switch, at: &str) -> Result<Switch, Error> {
    let mut cases = Vec::with_capacity(switch.cases.len());
    for (c, case) in switch.cases.into_iter().enumerate() {
        let at = format!("{at}.cases[{c}]");
        let combinator = match case.combinator() {
            pb::Combinator::Any => Combinator::Any,
            pb::Combinator::All | pb::Combinator::Unspecified => Combinator::All,
        };
        let port = PortName::parse(case.port)
            .map_err(|error| Error::invalid(format!("{at}.port"), error.to_string()))?;
        let rules = case
            .rules
            .into_iter()
            .enumerate()
            .map(|(r, rule)| rule_of(rule, &format!("{at}.rules[{r}]")))
            .collect::<Result<_, _>>()?;
        cases.push(Case {
            port,
            combinator,
            rules,
        });
    }
    Ok(Switch {
        cases,
        otherwise: port_of(switch.otherwise, &format!("{at}.otherwise"))?,
        all_matches: switch.all_matches,
    })
}

/// Reads a rule of a request. A right value with no operand is no right value.
fn rule_of(rule: pb::Rule, at: &str) -> Result<Rule, Error> {
    let test = rule
        .test()
        .as_str_name()
        .strip_prefix("TEST_")
        .and_then(|name| Test::parse(&name.to_ascii_lowercase()))
        .ok_or_else(|| Error::invalid(format!("{at}.test"), "is required"))?;
    let left = rule.left.and_then(|operand| operand.value);
    let left = left.ok_or_else(|| Error::invalid(format!("{at}.left"), "is required"))?;
    let right = rule.right.and_then(|operand| operand.value);
    Ok(Rule {
        left: operand_of(left, &format!("{at}.left"))?,
        test,
        right: right
            .map(|right| operand_of(right, &format!("{at}.right")))
            .transpose()?,
    })
}

/// Reads an operand of a request.
fn operand_of(value: pb::operand::Value, at: &str) -> Result<Operand, Error> {
    Ok(match value {
        pb::operand::Value::Literal(text) => Operand::Literal(text),
        pb::operand::Value::Var(name) => Operand::Var(name),
        pb::operand::Value::Output(output) => Operand::Output {
            node: NodeId::parse(output.node)
                .map_err(|error| Error::invalid(format!("{at}.output.node"), error.to_string()))?,
            key: output.key,
        },
    })
}

/// Returns the proto of a switch.
pub(crate) fn switch(switch: Switch) -> pb::Switch {
    let operand = |operand: Operand| pb::Operand {
        value: Some(match operand {
            Operand::Literal(text) => pb::operand::Value::Literal(text),
            Operand::Var(name) => pb::operand::Value::Var(name),
            Operand::Output { node, key } => pb::operand::Value::Output(pb::NodeOutput {
                node: node.as_str().to_owned(),
                key,
            }),
        }),
    };
    let rule = |rule: Rule| pb::Rule {
        left: Some(operand(rule.left)),
        test: pb::Test::from_str_name(&format!("TEST_{}", rule.test.as_str().to_uppercase()))
            .unwrap_or_default()
            .into(),
        right: rule.right.map(operand),
    };
    let cases = switch.cases.into_iter().map(|case| pb::Case {
        port: case.port.as_str().to_owned(),
        combinator: match case.combinator {
            Combinator::All => pb::Combinator::All,
            Combinator::Any => pb::Combinator::Any,
        }
        .into(),
        rules: case.rules.into_iter().map(rule).collect(),
    });
    pb::Switch {
        cases: cases.collect(),
        otherwise: switch
            .otherwise
            .map(|port| port.as_str().to_owned())
            .unwrap_or_default(),
        all_matches: switch.all_matches,
    }
}

/// A need in JSON.
#[derive(Serialize, Deserialize)]
pub(super) struct NeedJson {
    /// The node.
    node: String,
    /// The port.
    port: Option<String>,
}

impl NeedJson {
    /// Returns the JSON of a need.
    pub(super) fn of(need: &Need) -> Self {
        Self {
            node: need.node.as_str().to_owned(),
            port: need.port.as_ref().map(|port| port.as_str().to_owned()),
        }
    }

    /// Reads a need from its JSON.
    pub(super) fn read(self) -> Result<Need, Error> {
        Ok(Need {
            node: NodeId::parse(self.node).map_err(bad)?,
            port: self.port.map(PortName::parse).transpose().map_err(bad)?,
        })
    }
}

/// A switch in JSON.
#[derive(Serialize, Deserialize)]
pub(super) struct SwitchJson {
    /// The cases.
    cases: Vec<CaseJson>,
    /// The otherwise port.
    otherwise: Option<String>,
    /// `true` opens the port of each case that matches.
    all_matches: bool,
}

/// A case in JSON.
#[derive(Serialize, Deserialize)]
struct CaseJson {
    /// The port.
    port: String,
    /// `true` for `Any`, `false` for `All`.
    any: bool,
    /// The rules.
    rules: Vec<RuleJson>,
}

/// A rule in JSON.
#[derive(Serialize, Deserialize)]
struct RuleJson {
    /// The left value.
    left: OperandJson,
    /// The name of the test.
    test: String,
    /// The right value.
    right: Option<OperandJson>,
}

/// An operand in JSON.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperandJson {
    /// A text.
    Literal(String),
    /// The name of a variable.
    Var(String),
    /// A key of the output of a node.
    Output {
        /// The node.
        node: String,
        /// The key.
        key: String,
    },
}

impl SwitchJson {
    /// Returns the JSON of a switch.
    pub(super) fn of(switch: &Switch) -> Self {
        let operand = |operand: &Operand| match operand {
            Operand::Literal(text) => OperandJson::Literal(text.clone()),
            Operand::Var(name) => OperandJson::Var(name.clone()),
            Operand::Output { node, key } => OperandJson::Output {
                node: node.as_str().to_owned(),
                key: key.clone(),
            },
        };
        let rule = |rule: &Rule| RuleJson {
            left: operand(&rule.left),
            test: rule.test.as_str().to_owned(),
            right: rule.right.as_ref().map(operand),
        };
        let cases = switch.cases.iter().map(|case| CaseJson {
            port: case.port.as_str().to_owned(),
            any: case.combinator == Combinator::Any,
            rules: case.rules.iter().map(rule).collect(),
        });
        Self {
            cases: cases.collect(),
            otherwise: switch
                .otherwise
                .as_ref()
                .map(|port| port.as_str().to_owned()),
            all_matches: switch.all_matches,
        }
    }

    /// Reads a switch from its JSON.
    pub(super) fn read(self) -> Result<Switch, Error> {
        let operand = |operand: OperandJson| -> Result<Operand, Error> {
            Ok(match operand {
                OperandJson::Literal(text) => Operand::Literal(text),
                OperandJson::Var(name) => Operand::Var(name),
                OperandJson::Output { node, key } => Operand::Output {
                    node: NodeId::parse(node).map_err(bad)?,
                    key,
                },
            })
        };
        let rule = |rule: RuleJson| -> Result<Rule, Error> {
            Ok(Rule {
                left: operand(rule.left)?,
                test: Test::parse(&rule.test).ok_or_else(|| bad("test"))?,
                right: rule.right.map(operand).transpose()?,
            })
        };
        let case = |case: CaseJson| -> Result<Case, Error> {
            Ok(Case {
                port: PortName::parse(case.port).map_err(bad)?,
                combinator: if case.any {
                    Combinator::Any
                } else {
                    Combinator::All
                },
                rules: case.rules.into_iter().map(rule).collect::<Result<_, _>>()?,
            })
        };
        Ok(Switch {
            cases: self.cases.into_iter().map(case).collect::<Result<_, _>>()?,
            otherwise: self
                .otherwise
                .map(PortName::parse)
                .transpose()
                .map_err(bad)?,
            all_matches: self.all_matches,
        })
    }
}
