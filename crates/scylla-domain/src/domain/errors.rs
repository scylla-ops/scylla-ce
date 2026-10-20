use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("{0}")]
    NotFound(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Business rule violation: {0}")]
    BusinessRule(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("{0}")]
    Stale(String),

    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("Infrastructure error: {0}")]
    Infrastructure(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type DomainResult<T> = Result<T, DomainError>;

impl DomainError {
    pub fn not_found(entity_type: impl Display, key: impl Display) -> Self {
        Self::NotFound(format!("{entity_type} '{key}' not found"))
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn business_rule(message: impl Into<String>) -> Self {
        Self::BusinessRule(message.into())
    }

    /// A request names a resource that does not exist, as a reference and not as its target.
    pub fn missing_reference() -> Self {
        Self::business_rule("a referenced resource does not exist")
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn stale(entity_type: impl Display, key: impl Display) -> Self {
        Self::Stale(format!(
            "{entity_type} '{key}' changed since it was read; read it again and retry"
        ))
    }

    pub fn quota_exceeded(message: impl Into<String>) -> Self {
        Self::QuotaExceeded(message.into())
    }

    pub fn infrastructure(message: impl Into<String>) -> Self {
        Self::Infrastructure(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}
