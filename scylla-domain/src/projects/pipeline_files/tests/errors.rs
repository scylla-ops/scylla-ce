//! Tests of the errors of a pipeline file: each error has the path of the file, and its line and
//! column when the error is on a value.

use super::{at, error, read};
use crate::projects::pipeline_files::{FileFormat, MAX_BYTES};

/// A syntax error has the place that its parser gives, in each format.
#[test]
fn a_syntax_error_has_its_place() {
    // given
    let toml = "version = 1\n[[nodes]]\nid \"build\"\n";
    let yaml = "version: 1\nnodes:\n  - id: [build\n";
    let json = "{\n  \"version\": 1,\n  \"nodes\": [\n";

    // when
    let errors = [
        error(FileFormat::Toml, toml),
        error(FileFormat::Yaml, yaml),
        error(FileFormat::Json, json),
    ];

    // then
    let lines: Vec<_> = errors
        .iter()
        .map(|error| error.location.map(|place| place.line))
        .collect();
    assert_eq!(lines, [Some(3), Some(3), Some(4)]);
    assert!(errors
        .iter()
        .all(|error| error.field.is_empty() && !error.message.is_empty()));
}

/// An unknown key has the place of the key, as in the example of the spec.
#[test]
fn an_unknown_key_has_its_place() {
    // given
    let text = "version: 1\nnodes:\n  - id: build\n    exec: [make]\n  - id: test\n    needs: [build]\n    exec: [make]\n    timeout: 60\n";

    // when
    let error = error(FileFormat::Yaml, text);

    // then
    assert_eq!(error.location, at(8, 5));
    assert!(error.message.contains("timeout"), "{}", error.message);
}

/// A need of an unknown node has the place of the need, in each format.
#[test]
fn a_rule_of_the_nodes_has_the_place_of_its_value() {
    // given
    let toml = "version = 1\n\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n\n[[nodes]]\nid = \"test\"\nneeds = [\"tset\"]\nexec = [\"make\"]\n";
    let yaml = "version: 1\nnodes:\n  - id: build\n    exec: [make]\n  - id: test\n    needs: [tset]\n    exec: [make]\n";
    let json = "{\n  \"version\": 1,\n  \"nodes\": [\n    { \"id\": \"build\", \"exec\": [\"make\"] },\n    { \"id\": \"test\", \"needs\": [\"tset\"], \"exec\": [\"make\"] }\n  ]\n}\n";

    // when
    let errors = [
        error(FileFormat::Toml, toml),
        error(FileFormat::Yaml, yaml),
        error(FileFormat::Json, json),
    ];

    // then
    let places: Vec<_> = errors.iter().map(|error| error.location).collect();
    assert_eq!(places, [at(9, 10), at(6, 13), at(5, 31)]);
    assert!(errors
        .iter()
        .all(|error| error.field == "nodes[1].needs[0]"));
    assert_eq!(
        errors[0].to_string(),
        ".scylla/pipelines/build.toml:9:10: nodes[1].needs[0] must name another node of the pipeline"
    );
}

/// A rule of a variable has the place of the name of the variable in its table.
#[test]
fn a_rule_of_a_variable_has_the_place_of_its_name() {
    // given
    let text = "version = 1\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\nenv = { A = \"1\", SCYLLA_X = \"2\" }\n";

    // when
    let error = error(FileFormat::Toml, text);

    // then
    assert_eq!(error.field, "nodes[0].env[1].name");
    assert_eq!(error.location, at(5, 18));
}

/// A version that is not 1 is refused at its key, also when the file has keys of that version.
#[test]
fn a_version_that_is_not_1_is_refused() {
    // given
    let same_keys = "version = 2\n[[nodes]]\nid = \"build\"\nexec = [\"make\"]\n";
    let new_keys = "version = 2\nfuture = true\n";

    // when
    let errors = [same_keys, new_keys].map(|text| error(FileFormat::Toml, text));

    // then
    for error in errors {
        assert_eq!(
            (error.field.as_str(), error.location),
            ("version", at(1, 1))
        );
        assert!(error.to_string().contains("version 2 is not supported"));
    }
}

/// The checks of the file go on after an error: each error comes back with its place.
#[test]
fn each_error_of_a_file_comes_back() {
    // given
    let name = "n".repeat(101);
    let text = format!(
        "version: 1\nname: {name}\non:\n  push:\n    branches: [\"[main\"]\nnodes:\n  - id: Build\n    exec: [make]\n  - id: test\n    exec: [make]\n    shell: bash\n  - id: lint\n    timeout_seconds: 0\n    script: make\n"
    );

    // when
    let errors = read(FileFormat::Yaml, &text).unwrap_err();

    // then
    let fields: Vec<_> = errors.iter().map(|error| error.field.as_str()).collect();
    assert_eq!(
        fields,
        [
            "name",
            "on.push.branches[0]",
            "nodes[0].id",
            "nodes[1].shell",
            "nodes[2].timeout_seconds"
        ]
    );
    let lines: Vec<_> = errors
        .iter()
        .map(|error| error.location.map(|place| place.line))
        .collect();
    assert_eq!(lines, [Some(2), Some(5), Some(7), Some(11), Some(13)]);
}

/// A node with both `exec` and `script`, or with neither, is refused at the node.
#[test]
fn a_node_has_exactly_one_step() {
    // given
    let both = "version = 1\n[[nodes]]\nid = \"a\"\nexec = [\"make\"]\nscript = \"make\"\n";
    let neither = "version = 1\n[[nodes]]\nid = \"a\"\n";

    // when
    let errors = [both, neither].map(|text| error(FileFormat::Toml, text));

    // then
    for error in errors {
        assert_eq!(
            (error.field.as_str(), error.location),
            ("nodes[0]", at(2, 1))
        );
    }
}

/// A file larger than 1 MiB is refused as a whole, with no place.
#[test]
fn a_large_file_is_refused() {
    // given
    let text = format!("version = 1\n#{}\n", "x".repeat(MAX_BYTES));

    // when
    let error = error(FileFormat::Toml, &text);

    // then
    assert_eq!(error.location, None);
    assert_eq!(
        error.message,
        format!("the file must be at most {MAX_BYTES} bytes")
    );
}
