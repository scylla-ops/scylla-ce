//! The read of a pipeline file: the parser of its format, the version, then the checks of the
//! nodes. Each error has the line and the column of its value.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_saphyr::{DuplicateKeyPolicy, MergeKeyPolicy, MessageFormatter, UserMessageFormatter};

use super::document::PipelineFile;
use super::positions::{locate, Positions};
use super::{convert, FileError, FileFormat, FileKey, FilePipeline, Location, MAX_BYTES, VERSION};
use crate::DomainError;

/// The version of a file, read alone when the whole document does not read.
#[derive(Deserialize)]
struct Probe {
    /// The version, if the file has one.
    #[serde(default)]
    version: Option<u64>,
}

/// An error of a parser: its place, when the parser knows it, and its text.
struct Syntax {
    /// The place of the error.
    location: Option<Location>,
    /// What is wrong.
    message: String,
}

/// Reads and checks the text of the pipeline file at `path`, with the key of the file. Returns
/// each error that the read finds.
pub fn parse(
    path: &str,
    key: &FileKey,
    format: FileFormat,
    text: &str,
) -> Result<FilePipeline, Vec<FileError>> {
    // Refuse a file that is too large, and drop a byte order mark.
    if text.len() > MAX_BYTES {
        let error = DomainError::TooLarge { max: MAX_BYTES };
        return Err(vec![FileError::of_file(path, format!("the file {error}"))]);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    // Read the document. A version that is not supported comes before each other error.
    let file = match read::<PipelineFile>(format, text) {
        Ok(file) => file,
        Err(syntax) => {
            let version = read::<Probe>(format, text)
                .ok()
                .and_then(|probe| probe.version);
            let error = match version.filter(|version| *version != u64::from(VERSION)) {
                Some(version) => unsupported(path, format, text, version),
                None => FileError {
                    path: path.to_owned(),
                    location: syntax.location,
                    field: String::new(),
                    message: syntax.message,
                },
            };
            return Err(vec![error]);
        }
    };
    if file.version != VERSION {
        return Err(vec![unsupported(path, format, text, file.version.into())]);
    }

    // Check the document, and place each refusal at its value.
    convert::pipeline(file, key).map_err(|refusals| {
        let positions = Positions::of(format, text);
        refusals
            .into_iter()
            .map(|(field, message)| FileError {
                path: path.to_owned(),
                location: positions.find(&field),
                field,
                message,
            })
            .collect()
    })
}

/// Returns the options of the YAML parser: only `true` and `false` are booleans, a key comes
/// once, no merge key, no unknown tag, one document, and limits on the aliases and the size.
pub(super) fn yaml_options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_reader_input_bytes: Some(MAX_BYTES),
            max_aliases: 1_000,
            max_anchors: 1_000,
            max_depth: 32,
            max_documents: 1,
            max_total_scalar_bytes: 2 * MAX_BYTES,
        },
        alias_limits: serde_saphyr::alias_limits! {
            max_total_replayed_events: 10_000,
            max_replay_stack_depth: 8,
            max_alias_expansions_per_anchor: 100,
        },
        duplicate_keys: DuplicateKeyPolicy::Error,
        merge_keys: MergeKeyPolicy::Error,
        strict_booleans: true,
        reject_unsupported_tags: true,
        with_snippet: false,
    }
}

/// Reads a text in a format, with the error of its parser.
fn read<T: DeserializeOwned>(format: FileFormat, text: &str) -> Result<T, Syntax> {
    match format {
        FileFormat::Toml => toml::from_str(text).map_err(|error| Syntax {
            location: error.span().map(|span| locate(text, span.start)),
            message: error.message().trim().to_owned(),
        }),
        FileFormat::Yaml => {
            serde_saphyr::from_str_with_options(text, yaml_options()).map_err(|error| {
                let location = error.location().filter(|place| place.line() > 0);
                Syntax {
                    location: location.map(|place| Location {
                        line: u32::try_from(place.line()).unwrap_or(u32::MAX),
                        column: u32::try_from(place.column()).unwrap_or(u32::MAX),
                    }),
                    message: UserMessageFormatter.format_message(&error).into_owned(),
                }
            })
        }
        FileFormat::Json => serde_json::from_str(text).map_err(|error| {
            let suffix = format!(" at line {} column {}", error.line(), error.column());
            let message = error.to_string();
            Syntax {
                location: (error.line() > 0).then(|| Location {
                    line: u32::try_from(error.line()).unwrap_or(u32::MAX),
                    column: u32::try_from(error.column().max(1)).unwrap_or(u32::MAX),
                }),
                message: message.strip_suffix(&suffix).unwrap_or(&message).to_owned(),
            }
        }),
    }
}

/// Makes the error of a version that is not supported, at the value of `version`.
fn unsupported(path: &str, format: FileFormat, text: &str, version: u64) -> FileError {
    FileError {
        path: path.to_owned(),
        location: Positions::of(format, text).find("version"),
        field: "version".to_owned(),
        message: format!("{version} is not supported: the only version is {VERSION}"),
    }
}
