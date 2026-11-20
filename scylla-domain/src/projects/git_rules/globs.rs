//! The globs of the branches and of the tags: their checks and their matches.

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use super::{MAX_GLOBS, MAX_GLOB_LEN};
use crate::DomainError;

/// Checks a list of globs at the path `at`: at most `MAX_GLOBS`, each one valid.
pub(super) fn check_globs(at: &str, globs: &[String]) -> Vec<(String, DomainError)> {
    if globs.len() > MAX_GLOBS {
        return vec![(at.to_owned(), DomainError::TooMany { max: MAX_GLOBS })];
    }
    globs
        .iter()
        .enumerate()
        .filter(|(_, glob)| {
            let bad_text = glob.trim().is_empty()
                || glob.chars().count() > MAX_GLOB_LEN
                || glob.chars().any(char::is_control);
            bad_text || compile(glob).is_err()
        })
        .map(|(index, _)| (format!("{at}[{index}]"), DomainError::NotAGlob))
        .collect()
}

/// Compiles a glob: `*` stops at a `/`, `**` does not, and `\` escapes the next character on
/// each platform.
fn compile(glob: &str) -> Result<Glob, globset::Error> {
    GlobBuilder::new(glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
}

/// Returns the set of the globs that compile.
fn set(globs: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for glob in globs.iter().filter_map(|glob| compile(glob).ok()) {
        builder.add(glob);
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Returns `true` if a glob of the list takes the name.
pub(super) fn any(globs: &[String], name: &str) -> bool {
    !globs.is_empty() && set(globs).is_match(name)
}

/// Returns `true` if the list is empty, or if a glob of the list takes the name.
pub(super) fn any_or_all(globs: &[String], name: &str) -> bool {
    globs.is_empty() || set(globs).is_match(name)
}
