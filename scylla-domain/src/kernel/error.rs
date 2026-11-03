//! The validation errors of the domain types.

/// A value that breaks a rule. The message is the reason only: the caller adds the field.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
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
    /// The list has too many items.
    #[error("must have at most {max} items")]
    TooMany {
        /// The maximum number of items.
        max: usize,
    },
    /// The number is out of its range.
    #[error("must be from {min} to {max}")]
    OutOfRange {
        /// The smallest value.
        min: u64,
        /// The largest value.
        max: u64,
    },
    /// The text is not the id of a node.
    #[error("must be 1 to 128 lower case letters, digits, dashes or underscores")]
    NotANodeId,
    /// Two items have the same name.
    #[error("must be unique")]
    Duplicate,
    /// A node needs a node that is not in the pipeline, or itself.
    #[error("must name another node of the pipeline")]
    UnknownNode,
    /// The needs of the nodes make a cycle.
    #[error("must not make a cycle")]
    Cycle,
    /// The text is not the name of an environment variable.
    #[error("must be a letter or an underscore, then letters, digits or underscores")]
    NotAnEnvName,
    /// The name starts with a prefix that the server keeps.
    #[error("must not start with SCYLLA_")]
    ReservedName,
    /// The event does not fit the state of the job or of its node.
    #[error("does not fit the state of the job")]
    NotAllowed,
    /// The text is not a JSON pointer.
    #[error("must be empty or start with /")]
    NotAPointer,
    /// The path is not a relative path inside the work folder.
    #[error("must be a relative path with no .. part")]
    NotRelative,
}
