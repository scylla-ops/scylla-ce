//! The errors of a hook.

use std::fmt;

/// Why a hook stops a write.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// The hook refuses the write. The client gets `FAILED_PRECONDITION`.
    #[error("{}", .0.message)]
    Rejected(Rejection),
    /// The hook failed on the database. The client gets `INTERNAL`.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// A refusal, sent as `FAILED_PRECONDITION` with a `google.rpc.ErrorInfo` detail.
#[derive(Debug)]
pub struct Rejection {
    /// A stable code in upper snake case, for example `QUOTA_EXCEEDED`. It becomes `reason`.
    pub code: &'static str,
    /// A message for developers and logs.
    pub message: String,
    /// The values that a client shows, for example `used` and `limit`. They become `metadata`.
    pub details: Vec<(&'static str, String)>,
}

impl Rejection {
    /// Makes a refusal with no detail.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: Vec::new(),
        }
    }

    /// Adds one detail: a number or a text.
    pub fn detail(mut self, key: &'static str, value: impl fmt::Display) -> Self {
        self.details.push((key, value.to_string()));
        self
    }
}

impl From<Rejection> for HookError {
    fn from(rejection: Rejection) -> Self {
        Self::Rejected(rejection)
    }
}
