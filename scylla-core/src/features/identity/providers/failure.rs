//! The codes of a failed sign-in in a browser. The web app has one card for each code, and a code
//! that it does not know gets a general card.

use scylla_extension::AccessError;

use crate::Error;

/// Returns the code of a refused or a failed sign-in, and logs it. A server failure goes to the
/// log as an error.
pub(super) fn report(error: &Error) -> &'static str {
    let code = code_of(error);
    match error {
        Error::Rejected(_)
        | Error::NotFound(_)
        | Error::InvalidArgument { .. }
        | Error::Access(_) => {
            tracing::info!(event = "request.refused", code, message = %error);
        }
        error => tracing::error!(event = "request.failed", %error, "a sign-in failed"),
    }
    code
}

/// Returns the code of an error that a provider sends to the callback. The text of the provider
/// never goes further: it comes from the URL.
pub(super) fn code_of_provider(error: &str) -> &'static str {
    match error {
        "access_denied" => "access_denied",
        "redirect_uri_mismatch" => "redirect_uri_mismatch",
        _ => "provider_refused",
    }
}

/// Returns the code of an error of the service.
fn code_of(error: &Error) -> &'static str {
    match error {
        Error::Rejected(rejection) => match rejection.code {
            "STATE_EXPIRED" => "state_expired",
            "EMAIL_REQUIRED" => "email_unverified",
            "NO_ACCOUNT" | "SIGN_UP_CLOSED" | "INVITATION_REQUIRED" => "signup_disabled",
            "EMAIL_IN_USE" => "email_in_use",
            "IDENTITY_IN_USE" => "identity_in_use",
            "ACCOUNT_INACTIVE" => "account_inactive",
            "SESSION_REQUIRED" => "session_required",
            "REDIRECT_URI_MISMATCH" => "redirect_uri_mismatch",
            "PROVIDER_REFUSED" => "provider_refused",
            _ => "sign_in_failed",
        },
        Error::NotFound(_) => "provider_off",
        Error::Access(AccessError::Unavailable(_)) => "provider_unavailable",
        Error::Access(AccessError::Unauthenticated(_)) => "invalid_token",
        _ => "sign_in_failed",
    }
}
