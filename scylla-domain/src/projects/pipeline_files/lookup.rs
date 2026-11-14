//! The lookup of the pipeline files in the files of a commit: `.scylla/pipelines/<key>.<ext>`
//! when that folder has a pipeline file, else `.scylla/pipeline.<ext>`. A case with two answers is
//! an error, never a silent choice.

use std::collections::BTreeMap;

use super::{FileError, FileFormat, FileKey, MAX_BYTES};
use crate::DomainError;

/// The most pipeline files at one commit.
pub const MAX_FILES: usize = 32;

/// The folder of the one pipeline file, `.scylla/pipeline.<ext>`.
const ROOT: &str = ".scylla/";

/// The folder of many pipeline files, `.scylla/pipelines/<key>.<ext>`.
const FOLDER: &str = ".scylla/pipelines/";

/// The key of the one pipeline file.
const ONE_KEY: &str = "pipeline";

/// A file of a commit, as the tree of the provider lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// The path from the root of the repository, for example `.scylla/pipelines/build.toml`.
    pub path: String,
    /// The size in bytes, if the provider gives it.
    pub size: Option<u64>,
}

/// A pipeline file to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    /// The key of the pipeline.
    pub key: FileKey,
    /// The path of the file.
    pub path: String,
    /// The format of the file.
    pub format: FileFormat,
}

/// A file that the lookup refuses, with the key of its pipeline when it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    /// The key of the pipeline. `None` when the name of the file is not a key.
    pub key: Option<FileKey>,
    /// The error.
    pub error: FileError,
}

/// The result of a lookup, in the order of the keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lookup {
    /// The files to read, one for each key.
    pub files: Vec<FileRef>,
    /// The files that the lookup refuses. No file of their keys is read.
    pub rejected: Vec<Rejected>,
}

/// Finds the pipeline files in the files of a commit. Other files are ignored.
pub fn lookup(entries: &[TreeEntry]) -> Lookup {
    // Group the candidates of the folder by key, and keep the candidates at the root.
    let mut many: BTreeMap<String, Vec<(&TreeEntry, FileFormat)>> = BTreeMap::new();
    let mut one = Vec::new();
    for entry in entries {
        if let Some((stem, format)) = entry.path.strip_prefix(FOLDER).and_then(candidate) {
            many.entry(stem.to_owned())
                .or_default()
                .push((entry, format));
        } else if let Some((stem, format)) = entry.path.strip_prefix(ROOT).and_then(candidate) {
            if stem == ONE_KEY {
                one.push((entry, format));
            }
        }
    }

    // Use the folder when it has a file, and refuse a file at the root next to it.
    let mut lookup = Lookup::default();
    if many.is_empty() {
        if !one.is_empty() {
            let key = FileKey::parse(ONE_KEY).expect("a valid key");
            lookup.add(key, one);
        }
        return lookup;
    }
    for (entry, _) in one {
        let message = "a file .scylla/pipeline.* must not be next to the folder .scylla/pipelines";
        lookup.rejected.push(Rejected {
            key: None,
            error: FileError::of_file(&entry.path, message),
        });
    }

    // Take the files of each key, up to the limit.
    let mut keys = 0;
    for (stem, files) in many {
        match FileKey::parse(&stem) {
            Ok(key) if keys < MAX_FILES => {
                keys += 1;
                lookup.add(key, files);
            }
            Ok(key) => {
                let message =
                    format!("more than {MAX_FILES} pipeline files: this file is not read");
                lookup
                    .rejected
                    .extend(files.into_iter().map(|(entry, _)| Rejected {
                        key: Some(key.clone()),
                        error: FileError::of_file(&entry.path, message.clone()),
                    }));
            }
            Err(error) => {
                let message = format!("the name of the file {error}");
                lookup
                    .rejected
                    .extend(files.into_iter().map(|(entry, _)| Rejected {
                        key: None,
                        error: FileError::of_file(&entry.path, message.clone()),
                    }));
            }
        }
    }
    lookup
}

impl Lookup {
    /// Adds the files of one key: one file to read, or a refusal for two files or a large file.
    fn add(&mut self, key: FileKey, mut files: Vec<(&TreeEntry, FileFormat)>) {
        // Refuse two files for one key, or a large file.
        files.sort_by_key(|(entry, _)| rank(&entry.path));
        let (entry, format) = files[0];
        let error = if files.len() > 1 {
            let paths: Vec<_> = files.iter().map(|(entry, _)| entry.path.as_str()).collect();
            Some(format!(
                "two definitions of \"{}\": {}",
                key.as_str(),
                paths.join(", ")
            ))
        } else if entry.size.is_some_and(|size| size > MAX_BYTES as u64) {
            let error = DomainError::TooLarge { max: MAX_BYTES };
            Some(format!("the file {error}"))
        } else {
            None
        };

        // Keep the file to read, or its refusal.
        match error {
            Some(message) => self.rejected.push(Rejected {
                key: Some(key),
                error: FileError::of_file(&entry.path, message),
            }),
            None => self.files.push(FileRef {
                key,
                path: entry.path.clone(),
                format,
            }),
        }
    }
}

/// Returns the name with no extension and the format of a pipeline file in a folder, or `None`
/// for a file in a sub folder or with another extension.
fn candidate(name: &str) -> Option<(&str, FileFormat)> {
    let (stem, extension) = name.rsplit_once('.')?;
    let format = FileFormat::of_extension(extension)?;
    (!name.contains('/')).then_some((stem, format))
}

/// Returns the place of the extension of a path in the order of the lookup.
fn rank(path: &str) -> usize {
    let extension = path.rsplit_once('.').map_or("", |(_, extension)| extension);
    FileFormat::EXTENSIONS
        .iter()
        .position(|known| *known == extension)
        .unwrap_or(usize::MAX)
}
