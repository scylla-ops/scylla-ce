//! The error type of the core and of the edition routes, and its gRPC status codes.

use std::collections::HashMap;

use scylla_extension::{AuthError, HookError, Rejection};
use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};

/// The `domain` of each `google.rpc.ErrorInfo`.
const ERROR_DOMAIN: &str = "scylla";

/// An error of a request or of a command.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A field is missing or not valid: `INVALID_ARGUMENT` with a `google.rpc.BadRequest`.
    #[error("{field} {reason}")]
    InvalidArgument {
        /// The path of the field, for example `page.size`.
        field: String,
        /// Why the value is not valid.
        reason: String,
    },
    /// The request has no valid actor: `UNAUTHENTICATED`.
    #[error("{0}")]
    Unauthenticated(String),
    /// A named resource does not exist: `NOT_FOUND`.
    #[error("{0} not found")]
    NotFound(&'static str),
    /// A hook refuses the request: `FAILED_PRECONDITION` with a `google.rpc.ErrorInfo`.
    #[error("{}", .0.message)]
    Rejected(Rejection),
    /// The database misses migrations, so the server does not start.
    #[error("{count} migrations of {table} are not applied, run the migrate command")]
    PendingMigrations {
        /// The ledger table.
        table: String,
        /// The number of missing migrations.
        count: usize,
    },
    /// A database error: `INTERNAL`.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// A migration failed.
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The server could not bind its address.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The gRPC server failed.
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
}

impl Error {
    /// Makes an `InvalidArgument` error.
    pub fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidArgument {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

impl From<HookError> for Error {
    fn from(error: HookError) -> Self {
        match error {
            HookError::Rejected(rejection) => Self::Rejected(rejection),
            HookError::Database(error) => Self::Database(error),
        }
    }
}

impl From<AuthError> for Error {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Database(error) => Self::Database(error),
            error => Self::Unauthenticated(error.to_string()),
        }
    }
}

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        match error {
            Error::InvalidArgument { field, reason } => invalid_argument(field, reason),
            Error::Unauthenticated(message) => Status::unauthenticated(message),
            Error::NotFound(_) => Status::not_found(error.to_string()),
            Error::Rejected(rejection) => rejected(rejection),
            error => {
                tracing::error!(%error, "request failed");
                Status::internal("internal error")
            }
        }
    }
}

/// Makes `INVALID_ARGUMENT` with a field violation.
fn invalid_argument(field: String, reason: String) -> Status {
    let message = format!("{field} {reason}");
    let details = ErrorDetails::with_bad_request_violation(field, reason);
    Status::with_error_details(Code::InvalidArgument, message, details)
}

/// Makes `FAILED_PRECONDITION` with an `ErrorInfo` from a refusal.
fn rejected(rejection: Rejection) -> Status {
    let metadata: HashMap<String, String> = rejection
        .details
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.to_string()))
        .collect();
    let details = ErrorDetails::with_error_info(rejection.code, ERROR_DOMAIN, metadata);
    Status::with_error_details(Code::FailedPrecondition, rejection.message, details)
}
