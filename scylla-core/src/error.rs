use std::collections::HashMap;

use scylla_domain::DomainError;
use scylla_extension::{GuardError, Rejection};
use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};

const ERROR_DOMAIN: &str = "scylla";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("{field} {reason}")]
    InvalidArgument {
        field: &'static str,
        reason: &'static str,
    },
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{}", .0.message)]
    Rejected(Rejection),
    #[error("missing environment variable {0}")]
    MissingEnv(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),
}

impl From<GuardError> for Error {
    fn from(error: GuardError) -> Self {
        match error {
            GuardError::Rejected(rejection) => Self::Rejected(rejection),
            GuardError::Database(error) => Self::Database(error),
        }
    }
}

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        match error {
            Error::Domain(_) | Error::InvalidArgument { .. } => {
                Status::invalid_argument(error.to_string())
            }
            Error::NotFound(_) => Status::not_found(error.to_string()),
            Error::Rejected(rejection) => rejected(rejection),
            error => {
                tracing::error!(%error, "request failed");
                Status::internal("internal error")
            }
        }
    }
}

fn rejected(rejection: Rejection) -> Status {
    let metadata: HashMap<String, String> = rejection
        .details
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.to_string()))
        .collect();
    let details = ErrorDetails::with_error_info(rejection.code, ERROR_DOMAIN, metadata);
    Status::with_error_details(Code::FailedPrecondition, rejection.message, details)
}
