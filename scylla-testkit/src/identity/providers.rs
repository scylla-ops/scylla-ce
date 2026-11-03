//! The `TestApp` helpers of the identity providers: a provider written as the commands of the
//! operator write it.

use scylla_core::features::identity::providers::{self, GitHubSettings, OidcSettings, ProviderRow};
use scylla_domain::identity::users::ProviderId;

use crate::TestApp;

impl TestApp {
    /// Adds an OIDC provider, as `provider add-oidc` does.
    pub async fn add_oidc_provider(
        &self,
        slug: &str,
        issuer: &str,
        client_id: &str,
        secret: Option<&str>,
        allow_sign_up: bool,
    ) {
        let settings = OidcSettings {
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            scopes: ["openid", "email", "profile"].map(str::to_owned).to_vec(),
        };
        let settings = serde_json::to_value(settings).expect("settings");
        self.add_provider(slug, "oidc", settings, secret, allow_sign_up)
            .await;
    }

    /// Adds a GitHub provider at a base URL for the web and the API, as `provider add-github`
    /// does.
    pub async fn add_github_provider(&self, slug: &str, url: &str, client_id: &str, secret: &str) {
        let settings = GitHubSettings {
            client_id: client_id.to_owned(),
            web_url: url.to_owned(),
            api_url: url.to_owned(),
        };
        let settings = serde_json::to_value(settings).expect("settings");
        self.add_provider(slug, "github", settings, Some(secret), true)
            .await;
    }

    /// Writes a provider that is on.
    async fn add_provider(
        &self,
        slug: &str,
        kind: &str,
        settings: serde_json::Value,
        secret: Option<&str>,
        allow_sign_up: bool,
    ) {
        let row = ProviderRow {
            id: ProviderId::generate(),
            slug: slug.to_owned(),
            kind: kind.to_owned(),
            display_name: slug.to_owned(),
            settings,
            client_secret: secret.map(|secret| self.sealer.seal(secret)),
            allow_sign_up,
            enabled: true,
        };
        let mut tx = self.pool.begin().await.expect("transaction");
        providers::upsert(&mut tx, &row).await.expect("provider");
        tx.commit().await.expect("commit");
    }
}
