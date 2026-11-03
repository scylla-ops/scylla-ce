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
    /// The text is not a slug.
    #[error(
        "must be 2 to 39 lower case letters, digits or dashes, and start and end with a letter \
         or a digit"
    )]
    NotASlug,
    /// The text is a reserved name, for example a route of the web app.
    #[error("is a reserved name")]
    ReservedSlug,
    /// The text is too long after trim.
    #[error("must be at most {max} characters")]
    TooLong {
        /// The maximum number of characters.
        max: usize,
    },
    /// The number is below zero.
    #[error("must not be negative")]
    Negative,
    /// The text has a line break or another control character.
    #[error("must be one line, with no control characters")]
    NotOneLine,
    /// The text has a control character that is not a line break or a tab.
    #[error("must have no control characters other than line breaks and tabs")]
    ControlCharacter,
    /// The text is not the name of a secret.
    #[error("must be 1 to 128 letters, digits, dots, dashes or underscores")]
    NotASecretName,
    /// The text has too many bytes.
    #[error("must be at most {max} bytes")]
    TooLarge {
        /// The maximum number of bytes.
        max: usize,
    },
}
