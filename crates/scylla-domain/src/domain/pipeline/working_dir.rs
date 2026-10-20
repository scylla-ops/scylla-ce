use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::text::{Rule, Text};

pub enum WorkingDirRule {}

impl Rule for WorkingDirRule {
    const LABEL: &'static str = "Working directory";
    const MAX: usize = 4096;

    fn check(s: &str) -> DomainResult<()> {
        if s.starts_with('/') || s.starts_with('\\') {
            return Err(DomainError::validation(
                "Working directory must be relative to the job workspace",
            ));
        }
        if s.split(['/', '\\']).any(|c| c == "..") {
            return Err(DomainError::validation(
                "Working directory must not contain `..`",
            ));
        }
        Ok(())
    }
}

pub type WorkingDir = Text<WorkingDirRule>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_paths() {
        for p in ["crates/api", "build", "a/b/c"] {
            assert!(WorkingDir::new(p).is_ok(), "{p} should be valid");
        }
    }

    #[test]
    fn rejects_absolute_and_traversal() {
        for p in ["", "/etc", "../escape", "a/../b", "/", "\\windows"] {
            assert!(WorkingDir::new(p).is_err(), "{p} should be rejected");
        }
    }
}
