//! Tests of the read of a pipeline file: the examples of the spec, the defaults and the traps of
//! YAML.

use serde::de::IgnoredAny;

use super::{error, read, JSON, TOML, YAML};
use crate::projects::git_rules::TriggerEvent;
use crate::projects::pipeline_files::read::yaml_options;
use crate::projects::pipeline_files::{CheckoutSettings, FileFormat};
use crate::projects::pipelines::{EnvValue, Node, Runtime, Shell, Step};

/// The TOML, YAML and JSON forms of the example of the spec give the same pipeline.
#[test]
fn the_examples_give_the_same_pipeline() {
    // given
    let texts = [
        (FileFormat::Toml, TOML),
        (FileFormat::Yaml, YAML),
        (FileFormat::Json, JSON),
    ];

    // when
    let pipelines: Vec<_> = texts
        .into_iter()
        .map(|(format, text)| read(format, text).unwrap())
        .collect();

    // then
    assert_eq!(pipelines[0], pipelines[1]);
    assert_eq!(pipelines[0], pipelines[2]);
    let pipeline = &pipelines[0];
    assert_eq!(pipeline.name.as_str(), "Build and deploy");
    let ids: Vec<_> = pipeline
        .nodes
        .as_slice()
        .iter()
        .map(|node| node.id.as_str())
        .collect();
    assert_eq!(ids, ["build", "test", "lint", "deploy"]);
    let deploy = &pipeline.nodes.as_slice()[3];
    assert_eq!(deploy.timeout_seconds, 900);
    assert!(
        matches!(&deploy.step, Step::Script { shell: Shell::Bash, source } if source.ends_with("SHA\"\n"))
    );
    assert!(
        matches!(&deploy.env[0].value, EnvValue::Secret(name) if name.as_str() == "deploy-token")
    );
    let lint = &pipeline.nodes.as_slice()[2];
    assert_eq!(
        lint.step,
        Step::Exec(
            ["cargo", "clippy", "--", "-D", "warnings"]
                .map(String::from)
                .to_vec()
        )
    );
    let push = TriggerEvent::Push {
        branch: "release/1.0",
        message: "Fix the build",
    };
    assert!(pipeline.rules.matches(&push));
}

/// A file with only the required keys gets the defaults: its key as its name, no rule, the
/// default checkout, sh, one hour and no retry. A byte order mark is ignored.
#[test]
fn a_short_file_gets_the_defaults() {
    // given
    let text = "\u{feff}version = 1\n[[nodes]]\nid = \"build\"\nscript = \"make\"\n";

    // when
    let pipeline = read(FileFormat::Toml, text).unwrap();

    // then
    assert_eq!(pipeline.name.as_str(), "build");
    assert!(pipeline.rules.is_empty());
    assert_eq!(pipeline.checkout, CheckoutSettings::default());
    let node = &pipeline.nodes.as_slice()[0];
    assert_eq!(node.timeout_seconds, Node::DEFAULT_TIMEOUT);
    assert_eq!(node.retries, 0);
    assert_eq!(node.runtime, Runtime::Process);
    assert!(matches!(
        node.step,
        Step::Script {
            shell: Shell::Sh,
            ..
        }
    ));
}

/// In YAML, `on:` is the key of the rules and `no` is a text, not a boolean.
#[test]
fn on_is_a_key_and_no_is_a_text() {
    // given
    let text = "version: 1\nname: no\non:\n  push:\n    branches: [main]\nnodes:\n  - id: a\n    exec: [make]\n    env:\n      ANSWER: no\n      OFF: off\n";

    // when
    let pipeline = read(FileFormat::Yaml, text).unwrap();

    // then
    assert_eq!(pipeline.name.as_str(), "no");
    assert!(pipeline.rules.push.is_some());
    let env = &pipeline.nodes.as_slice()[0].env;
    assert_eq!(env[0].value, EnvValue::Literal("no".to_owned()));
    assert_eq!(env[1].value, EnvValue::Literal("off".to_owned()));
}

/// In YAML, a boolean is only `true` or `false`: `yes` is refused.
#[test]
fn a_yaml_boolean_is_true_or_false() {
    // given
    let text =
        "version: 1\non:\n  pull_request:\n    drafts: yes\nnodes:\n  - id: a\n    exec: [make]\n";

    // when
    let error = error(FileFormat::Yaml, text);

    // then
    assert_eq!(error.location.map(|place| place.line), Some(4));
}

/// An alias bomb is refused before it grows: the read ends with an error.
#[test]
fn an_alias_bomb_is_refused() {
    // given
    let mut bomb = String::from(
        "version: 1
nodes:
  - id: a
    exec: [make]
",
    );
    bomb.push_str(
        "a0: &a0 [x, x, x, x, x, x, x, x, x, x]
",
    );
    for level in 1..10 {
        let items = vec![format!("*a{}", level - 1); 10].join(", ");
        bomb.push_str(&format!(
            "a{level}: &a{level} [{items}]
"
        ));
    }

    // when
    let ignored = serde_saphyr::from_str_with_options::<IgnoredAny>(&bomb, yaml_options());
    let result = read(FileFormat::Yaml, &bomb);

    // then
    assert!(ignored.is_err());
    assert!(result.is_err());
}

/// A merge key, a key two times and a second document are refused in YAML.
#[test]
fn the_other_traps_of_yaml_are_refused() {
    // given
    let merge = "version: 1
nodes:
  - &base
    id: a
    exec: [make]
  - <<: *base
    id: b
";
    let twice = "version: 1
version: 1
nodes:
  - id: a
    exec: [make]
";
    let documents = "version: 1
nodes:
  - id: a
    exec: [make]
---
version: 1
";

    // when
    let results = [merge, twice, documents].map(|text| read(FileFormat::Yaml, text));

    // then
    for result in results {
        assert!(result.is_err(), "{result:?}");
    }
}

/// A rule with no value in YAML is the rule with its defaults: each push starts a run.
#[test]
fn an_empty_rule_takes_each_event() {
    // given
    let text = "version: 1\non:\n  push:\n  tag:\nnodes:\n  - id: a\n    exec: [make]\n";

    // when
    let pipeline = read(FileFormat::Yaml, text).unwrap();

    // then
    assert!(pipeline.rules.matches(&TriggerEvent::Push {
        branch: "feature/a",
        message: "Add a",
    }));
    assert!(pipeline.rules.matches(&TriggerEvent::Tag { name: "v1" }));
    assert!(pipeline.rules.pull_request.is_none());
}
