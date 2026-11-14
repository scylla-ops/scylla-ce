//! Tests of the export of a pipeline to a file, and of the JSON Schema of the files.

use super::{read, TOML};
use crate::projects::git_rules::{GitRules, PullRequestAction, PullRequestRule, PushRule, TagRule};
use crate::projects::pipeline_files::{json_schema, CheckoutSettings, FileFormat, FilePipeline};
use crate::projects::pipelines::{
    EnvValue, EnvVar, Node, NodeId, Nodes, PipelineName, Runtime, Shell, Step,
};
use crate::projects::secrets::SecretName;

/// Makes a node with each kind of value, and texts that a format could read as another type.
fn node(id: &str, needs: &[&str]) -> Node {
    let literal = |name: &str, value: &str| EnvVar {
        name: name.to_owned(),
        value: EnvValue::Literal(value.to_owned()),
    };
    Node {
        id: NodeId::parse(id.to_owned()).unwrap(),
        needs: needs
            .iter()
            .map(|need| NodeId::parse((*need).to_owned()).unwrap())
            .collect(),
        step: Step::Script {
            shell: Shell::Bash,
            source: "set -x\necho \"$A\" 'b' # c\n\ttab: yes\n".to_owned(),
        },
        env: vec![
            literal("Z", "no"),
            literal("A", "123"),
            literal("ON", "true"),
            literal("EMPTY", ""),
            literal("MAP", "a: b, [c]"),
            EnvVar {
                name: "TOKEN".to_owned(),
                value: EnvValue::Secret(SecretName::parse("deploy-token".to_owned()).unwrap()),
            },
        ],
        timeout_seconds: 60,
        retries: 3,
        working_dir: Some("app/src".to_owned()),
        runtime: Runtime::Container {
            image: "rust:1.97".to_owned(),
        },
    }
}

/// Makes a pipeline with each field of a file.
fn pipeline() -> FilePipeline {
    let mut lint = node("lint", &["build"]);
    lint.step = Step::Exec(vec!["cargo".to_owned(), "--".to_owned(), "-D".to_owned()]);
    lint.env = Vec::new();
    lint.runtime = Runtime::Process;
    FilePipeline {
        name: PipelineName::parse("Build: and deploy".to_owned()).unwrap(),
        rules: GitRules {
            push: Some(PushRule {
                branches: vec!["main".to_owned(), "release/*".to_owned()],
                exclude: vec!["release/old".to_owned()],
                skip_ci: false,
            }),
            pull_request: Some(PullRequestRule {
                branches: vec!["main".to_owned()],
                events: vec![PullRequestAction::Opened, PullRequestAction::NewCommits],
                drafts: true,
            }),
            tag: Some(TagRule {
                patterns: vec!["v*".to_owned()],
            }),
        },
        checkout: CheckoutSettings {
            enabled: true,
            depth: 0,
            submodules: true,
            lfs: true,
        },
        nodes: Nodes::parse(vec![node("build", &[]), lint]).unwrap(),
    }
}

/// An export reads back to the same pipeline, in each format.
#[test]
fn an_export_reads_back_to_the_same_pipeline() {
    // given
    let pipeline = pipeline();

    // when
    let read_back: Vec<_> = [FileFormat::Toml, FileFormat::Yaml, FileFormat::Json]
        .into_iter()
        .map(|format| read(format, &pipeline.export(format)))
        .collect();

    // then
    for result in read_back {
        assert_eq!(result.unwrap(), pipeline);
    }
}

/// The example of the spec goes through each format and comes back the same.
#[test]
fn the_example_goes_through_each_format() {
    // given
    let example = read(FileFormat::Toml, TOML).unwrap();

    // when
    let yaml = read(FileFormat::Yaml, &example.export(FileFormat::Yaml)).unwrap();
    let json = read(FileFormat::Json, &yaml.export(FileFormat::Json)).unwrap();
    let toml = read(FileFormat::Toml, &json.export(FileFormat::Toml)).unwrap();

    // then
    assert_eq!(toml, example);
}

/// An export does not write the defaults.
#[test]
fn an_export_does_not_write_the_defaults() {
    // given
    let text = "version = 1\n[[nodes]]\nid = \"build\"\nscript = \"make\"\n";
    let pipeline = read(FileFormat::Toml, text).unwrap();

    // when
    let json = pipeline.export(FileFormat::Json);

    // then
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let expected = serde_json::json!({
        "version": 1,
        "name": "build",
        "nodes": [{ "id": "build", "script": "make" }],
    });
    assert_eq!(value, expected);
}

/// The JSON Schema has the keys of the file, and refuses an unknown key.
#[test]
fn the_schema_has_the_keys_of_the_file() {
    // when
    let schema = json_schema();

    // then
    assert_eq!(schema["title"], "Scylla pipeline file");
    assert_eq!(schema["additionalProperties"], false);
    let keys: Vec<_> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, ["checkout", "name", "nodes", "on", "version"]);
    assert_eq!(schema["required"], serde_json::json!(["version", "nodes"]));
}
