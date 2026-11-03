//! The validation errors of the domain types.

/// A value that breaks a rule. The message is the reason only: the caller adds the field.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    /// The text is not an id.
    #[error("must be an id")]
    NotAnId,
    /// The text is empty after trim.
    #[error("must not be empty")]
    Empty,
    /// The text is too short.
    #[error("must be at least {min} characters")]
    TooShort {
        /// The minimum number of characters.
        min: usize,
    },
    /// The text is not an email address.
    #[error("must be an email address")]
    NotAnEmail,
    /// The text is not a handle.
    #[error(
        "must be 2 to 39 lower case letters, digits, dots, dashes or underscores, \
         and start with a letter or a digit"
    )]
    NotAHandle,
    /// The text is too long after trim.
    #[error("must be at most {max} characters")]
    TooLong {
        /// The maximum number of characters.
        max: usize,
    },
    /// The number is below zero.
    #[error("must not be negative")]
    Negative,
}
