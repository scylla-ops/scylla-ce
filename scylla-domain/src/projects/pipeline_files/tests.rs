//! Unit tests of the pipeline files: the read, the errors and their places, the warnings, the
//! export and the lookup.

mod errors;
mod export;
mod lookup;
mod read;
mod warnings;

use super::{parse, FileError, FileFormat, FileKey, FilePipeline, Location};

/// The example of a TOML file in the spec.
const TOML: &str = include_str!("tests/build.toml");

/// The same file in YAML.
const YAML: &str = include_str!("tests/build.yaml");

/// The same file in JSON.
const JSON: &str = include_str!("tests/build.json");

/// Returns the path of the file `build` in a format.
fn path(format: FileFormat) -> String {
    format!(".scylla/pipelines/build.{}", format.extension())
}

/// Reads a text as the file `build` in a format.
fn read(format: FileFormat, text: &str) -> Result<FilePipeline, Vec<FileError>> {
    let key = FileKey::parse("build").unwrap();
    parse(&path(format), &key, format, text)
}

/// Returns the one error of a text that does not read.
fn error(format: FileFormat, text: &str) -> FileError {
    let mut errors = read(format, text).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.remove(0)
}

/// Returns a place.
fn at(line: u32, column: u32) -> Option<Location> {
    Some(Location { line, column })
}
