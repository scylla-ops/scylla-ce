//! The identity of a person that a provider gives.

/// The identity of a person that a provider gives.
pub(crate) struct ExternalIdentity {
    /// The id of the person at the provider. It never changes.
    pub(crate) subject: String,
    /// The email, if the provider gave one.
    pub(crate) email: Option<String>,
    /// `true` if the provider verified the email.
    pub(crate) email_verified: bool,
    /// The name of the person, if the provider gave one.
    pub(crate) name: Option<String>,
}
