//! The acceptances of the terms of service by the users: how a user accepted a version, and the
//! state of the terms for a user.

use time::OffsetDateTime;

/// How a user accepted a version of the terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptanceMethod {
    /// The box of the sign-up form with a password.
    SignUpForm,
    /// The terms step of the web app, with no earlier acceptance of the terms.
    AcceptanceStep,
    /// The terms step of the web app, after the acceptance of an earlier version.
    ReAcceptance,
}

impl AcceptanceMethod {
    /// Returns the name of the method in the database and in the export.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SignUpForm => "sign_up_form",
            Self::AcceptanceStep => "acceptance_step",
            Self::ReAcceptance => "re_acceptance",
        }
    }
}

/// An acceptance of a version of the terms by a user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceptance {
    /// The version that the user accepted.
    pub version: String,
    /// The time of the acceptance.
    pub accepted_at: OffsetDateTime,
}

/// The terms of service of a user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TermsState {
    /// `true` while the terms need an acceptance and the user did not accept their version.
    pub acceptance_required: bool,
    /// The last version that the user accepted.
    pub accepted: Option<Acceptance>,
}
