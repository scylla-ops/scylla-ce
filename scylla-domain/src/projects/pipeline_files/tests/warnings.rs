//! Tests of the warnings of a pipeline file: a secret that the project does not have.

use super::{path, read, JSON, TOML, YAML};
use crate::projects::pipeline_files::{missing_secrets, FileFormat};

/// A file names each of its secrets one time, in the order of the file.
#[test]
fn a_file_names_its_secrets() {
    // given
    let pipeline = read(FileFormat::Toml, TOML).unwrap();

    // when
    let names = pipeline.secret_names();

    // then
    assert_eq!(names, ["deploy-token"]);
}

/// A secret that the project does not have gives a warning at its value, in each format, and a
/// known secret gives none.
#[test]
fn a_missing_secret_is_a_warning_at_its_value() {
    for (format, text) in [
        (FileFormat::Toml, TOML),
        (FileFormat::Yaml, YAML),
        (FileFormat::Json, JSON),
    ] {
        // given
        let pipeline = read(format, text).unwrap();
        let file = path(format);
        let line = text
            .lines()
            .position(|line| line.contains("DEPLOY_TOKEN"))
            .map(|index| u32::try_from(index + 1).unwrap());

        // when
        let warnings = missing_secrets(&file, format, text, &pipeline, &[]);
        let none = missing_secrets(&file, format, text, &pipeline, &["deploy-token".to_owned()]);

        // then
        assert_eq!(warnings.len(), 1, "{format:?}");
        let warning = &warnings[0];
        assert_eq!(warning.path, file);
        assert_eq!(warning.field, "nodes[3].env[0].value");
        assert!(
            warning.message.contains("\"deploy-token\""),
            "{}",
            warning.message
        );
        assert_eq!(warning.location.map(|place| place.line), line, "{format:?}");
        assert!(none.is_empty(), "{format:?}");
    }
}
