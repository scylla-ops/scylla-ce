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
    /// The text is not the name of a port.
    #[error("must be 1 to 128 lower case letters, digits, dashes or underscores")]
    NotAPortName,
    /// A switch names a port `error`: this name is the port of a failed run node.
    #[error("must not be error: this name is the port of a failure")]
    ReservedPort,
    /// A need names a port that its node does not have.
    #[error("must be a port of the node: a case or the otherwise port of a switch, or error")]
    UnknownPort,
    /// An output names a node that is not a run node before the switch.
    #[error("must be a run node that the switch reaches through its needs")]
    NotAnAncestor,
    /// The text is not a finite number.
    #[error("must be a number")]
    NotANumber,
    /// The value is missing.
    #[error("is required")]
    Required,
    /// The value is set where the node or the test has no use for it.
    #[error("must not be set")]
    NotExpected,
    /// The text is not a glob of branches or tags.
    #[error("must be a glob of 1 to 255 characters, for example release/*")]
    NotAGlob,
    /// The text is not the key of a pipeline file.
    #[error("must be 1 to 64 lower case letters, digits, dashes or underscores")]
    NotAFileKey,
    /// The text is not the label of an agent.
    #[error(
        "must be 1 to 32 lower case letters, digits, dots, dashes or underscores, and start with \
         a letter or a digit"
    )]
    NotALabel,
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
    /// The text is not a JSON pointer into a body.
    #[error("must start with /, for example /ref")]
    NotABodyPointer,
    /// The text is not the name of an HTTP header.
    #[error(
        "must be a header name of 1 to 256 characters with no space, for example X-GitHub-Event"
    )]
    NotAHeaderName,
    /// The path is not a relative path inside the work folder.
    #[error("must be a relative path with no .. part")]
    NotRelative,
    /// The text is not the name of a gRPC method.
    #[error("must be <package>.<Service>/<Method>")]
    NotAMethod,
    /// The value is only for the custom preset.
    #[error("must be empty for a preset that is not custom")]
    NotCustom,
    /// The text is not an http or https URL.
    #[error("must be an http or https URL")]
    NotAnUrl,
    /// The text is not a host name or an IP address.
    #[error("must be a host name or an IP address: letters, digits, dots, dashes or colons")]
    NotAHost,
    /// The text is not the path of a repository.
    #[error("must be owner/repo or group/project")]
    NotARepository,
    /// The text is not the key of an entry of the state of a user.
    #[error("must be 1 to 128 lower case letters, digits, dots, colons, dashes or underscores")]
    NotAStateKey,
    /// The place is outside the canvas of a pipeline.
    #[error("must be from -{max} to {max}")]
    OffCanvas {
        /// The largest distance from the origin.
        max: i32,
    },
    /// The end of a period is before its start.
    #[error("must not be before the start of the period")]
    BeforeStart,
    /// The end of a period is not after its start.
    #[error("must be after the start of the period")]
    NotAfterStart,
    /// A period is too long.
    #[error("must be at most {days} days after the start of the period")]
    PeriodTooLong {
        /// The most days of a period.
        days: i64,
    },
    /// The text is not the name of a branch or of a tag.
    #[error("must be a branch or tag name of 1 to 255 characters, with no space")]
    NotARefName,
    /// The text is not a commit.
    #[error("must be 7 to 40 hexadecimal characters")]
    NotASha,
    /// The text is not the name of a zone of the IANA database.
    #[error("must be an IANA time zone, for example Europe/Paris")]
    NotATimeZone,
    /// The text is not a date.
    #[error("must be a date YYYY-MM-DD")]
    NotADate,
    /// The text is not the name of a purpose of the consent.
    #[error("must be 1 to 32 lower case letters, digits or dashes")]
    NotAPurpose,
    /// The text is not a SHA-256.
    #[error("must be 64 lower case hexadecimal characters")]
    NotASha256,
    /// The text is not the name of a glyph of the icon set.
    #[error(
        "must be 1 to 64 lower case letters, digits or dashes, with a letter or a digit first"
    )]
    NotAGlyphName,
    /// The text is not the color of an icon.
    #[error(
        "must be #rrggbb, or 1 to 32 lower case letters, digits or dashes with a letter first"
    )]
    NotAColor,
    /// The text is not a shape of a glyph avatar.
    #[error("must be variant01 to variant35, or scylla")]
    NotAGlyphShape,
    /// The text is not a hex color.
    #[error("must be #rrggbb")]
    NotAHexColor,
    /// The number is not an angle in degrees.
    #[error("must be from -360 to 360")]
    NotAnAngle,
    /// The paint of a glyph avatar has a wrong number of colors for its fill.
    #[error("must have 1 color for a solid fill, or 2 to 4 colors for a gradient")]
    GlyphColorCount,
    /// The bytes are not an image of a format that the server reads.
    #[error("must be a PNG, JPEG or WebP image")]
    NotAnImage,
    /// The image has too many pixels on a side.
    #[error("must be at most {max} pixels wide and high")]
    ImageTooLarge {
        /// The maximum number of pixels on a side.
        max: u32,
    },
    /// The choices of a consent do not fit its action.
    #[error("must accept each purpose for accept all, and refuse each purpose for reject all")]
    NotTheAction,
    /// A parameter of a fragment names no environment variable of its nodes.
    #[error("must be an environment variable of a node")]
    UnknownVariable,
    /// A block of a pipeline names a node that is not in the pipeline, or in another block.
    #[error("must name a node of the pipeline that no other block holds")]
    NotAFreeNode,
}
