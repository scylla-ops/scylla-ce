//! The call of the manifest flow to GitHub: the conversion of the code of a new App into its id,
//! its key and its secrets.

use reqwest::Method;
use serde::Deserialize;

use crate::infra::vcs::{Hidden, ProviderError, VcsClients};

/// A new App, from `POST /app-manifests/{code}/conversions`. `Debug` hides its secrets.
#[derive(Debug, Deserialize)]
pub(super) struct Converted {
    /// The id of the App on GitHub.
    pub(super) id: i64,
    /// The slug of the App.
    pub(super) slug: String,
    /// The display name.
    pub(super) name: String,
    /// The account that owns the App.
    pub(super) owner: Option<Owner>,
    /// The page of the App.
    pub(super) html_url: String,
    /// The client id of the App.
    pub(super) client_id: String,
    /// The client secret of the App.
    pub(super) client_secret: Hidden,
    /// The secret of the webhooks of the App.
    pub(super) webhook_secret: Option<Hidden>,
    /// The private key of the App, in PEM.
    pub(super) pem: Hidden,
}

/// The account that owns an App.
#[derive(Debug, Deserialize)]
pub(super) struct Owner {
    /// The login, for example `acme`.
    pub(super) login: String,
}

/// Converts the code of a new App, one time, with no credentials. The code lives one hour.
pub(super) async fn convert(clients: &VcsClients, code: &str) -> Result<Converted, ProviderError> {
    clients
        .github
        .call(Method::POST, &format!("/app-manifests/{code}/conversions"))
        .read()
        .await
}
