//! Tests of the key `agent` of a pipeline file: the labels that the agent of a job must have.

use super::{at, error, read};
use crate::projects::pipeline_files::FileFormat;

/// The labels of the key `agent` read the same in each format. A file with no key `agent` asks
/// no label.
#[test]
fn the_labels_read_in_each_format() {
    // given
    let toml = "version = 1\n[agent]\nlabels = [\"deploy\", \"linux\"]\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n";
    let yaml =
        "version: 1\nagent:\n  labels: [deploy, linux]\nnodes:\n  - id: build\n    exec: [make]\n";
    let json = "{\"version\": 1, \"agent\": {\"labels\": [\"deploy\", \"linux\"]}, \"nodes\": [{\"id\": \"build\", \"exec\": [\"make\"]}]}";
    let plain = "version = 1\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n";

    // when
    let labels = [
        (FileFormat::Toml, toml),
        (FileFormat::Yaml, yaml),
        (FileFormat::Json, json),
    ]
    .map(|(format, text)| read(format, text).unwrap().agent_labels.texts());
    let none = read(FileFormat::Toml, plain).unwrap().agent_labels;

    // then
    for one in labels {
        assert_eq!(one, ["deploy", "linux"]);
    }
    assert!(none.is_empty());
}

/// A label that does not have the form of a label is refused at its place, and a second copy of
/// a label too.
#[test]
fn a_bad_label_has_its_place() {
    // given
    let bad = "version: 1\nagent:\n  labels:\n    - deploy\n    - Prod Host\nnodes:\n  - id: build\n    exec: [make]\n";
    let twice = "version = 1\n[agent]\nlabels = [\"deploy\", \"deploy\"]\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n";

    // when
    let bad = error(FileFormat::Yaml, bad);
    let twice = error(FileFormat::Toml, twice);

    // then
    assert_eq!(bad.field, "agent.labels[1]");
    assert_eq!(bad.location, at(5, 7));
    assert!(
        bad.message.starts_with("must be 1 to 32"),
        "{}",
        bad.message
    );
    assert_eq!(twice.field, "agent.labels[1]");
    assert_eq!(twice.message, "must be unique");
}

/// A file asks at most 16 labels.
#[test]
fn a_file_asks_at_most_16_labels() {
    // given
    let labels: Vec<String> = (0..17).map(|n| format!("\"l{n}\"")).collect();
    let text = format!(
        "version = 1\n[agent]\nlabels = [{}]\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n",
        labels.join(", ")
    );

    // when
    let error = error(FileFormat::Toml, &text);

    // then
    assert_eq!(error.field, "agent.labels");
    assert_eq!(error.location, at(3, 1));
    assert_eq!(error.message, "must have at most 16 items");
}

/// The key `agent` has no other key than `labels`.
#[test]
fn an_unknown_key_of_the_agent_is_refused() {
    // given
    let text = "version: 1\nagent:\n  os: linux\nnodes:\n  - id: build\n    exec: [make]\n";

    // when
    let error = error(FileFormat::Yaml, text);

    // then
    assert_eq!(error.location, at(3, 3));
    assert!(error.message.contains("os"), "{}", error.message);
}
