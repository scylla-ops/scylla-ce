//! The `TestApp` helpers of the GitHub App of an instance, one for each RPC of `VcsAppService`,
//! the answer of the fake provider for the conversion of a manifest code, and the App of a
//! connection.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::platform::v1::vcs_app_service_client::VcsAppServiceClient;
use scylla_proto::platform::v1::{
    CompleteGitHubManifestRequest, DeleteVcsAppRequest, GetVcsAppRequest,
    StartGitHubManifestRequest, StartGitHubManifestResponse, VcsApp,
};
use serde_json::json;
use tonic::transport::Channel;
use tonic::Status;

use super::keys::TEST_APP_PRIVATE_KEY;
use super::FakeProvider;
use crate::TestApp;

/// The client secret of the App of a manifest test.
pub const MANIFEST_CLIENT_SECRET: &str = "manifest-client-secret";

/// The webhook secret of the App of a manifest test.
pub const MANIFEST_WEBHOOK_SECRET: &str = "manifest-webhook-secret";

impl TestApp {
    /// A client of `VcsAppService`.
    fn vcs_apps(&self) -> VcsAppServiceClient<Channel> {
        VcsAppServiceClient::new(self.channel.clone())
    }

    /// Calls `GetVcsApp` as `actor`.
    pub async fn get_vcs_app(&self, actor: ActorId) -> Result<Option<VcsApp>, Status> {
        let request = self.authed(actor, GetVcsAppRequest {}).await;
        let response = self.vcs_apps().get_vcs_app(request).await?;
        Ok(response.into_inner().app)
    }

    /// Calls `StartGitHubManifest` as `actor`, for an organization or for the account of the
    /// person with an empty text.
    pub async fn start_github_manifest(
        &self,
        actor: ActorId,
        organization: &str,
    ) -> Result<StartGitHubManifestResponse, Status> {
        let request = StartGitHubManifestRequest {
            organization: organization.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.vcs_apps().start_git_hub_manifest(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `CompleteGitHubManifest` as `actor`, with the values of a redirect of GitHub.
    pub async fn complete_github_manifest(
        &self,
        actor: ActorId,
        code: &str,
        state: &str,
    ) -> Result<VcsApp, Status> {
        let request = CompleteGitHubManifestRequest {
            code: code.to_owned(),
            state: state.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.vcs_apps().complete_git_hub_manifest(request).await?;
        Ok(response.into_inner().app.expect("an App"))
    }

    /// Calls `DeleteVcsApp` as `actor`.
    pub async fn delete_vcs_app(&self, actor: ActorId) -> Result<(), Status> {
        let request = self.authed(actor, DeleteVcsAppRequest {}).await;
        self.vcs_apps().delete_vcs_app(request).await?;
        Ok(())
    }

    /// Makes the App of the instance with this id the App of a connection.
    pub async fn set_connection_app(&self, connection: &str, app: &str) {
        sqlx::query("UPDATE vcs_connections SET app_id = $2::uuid WHERE id = $1::uuid")
            .bind(connection)
            .bind(app)
            .execute(&self.pool)
            .await
            .expect("give the App to a connection");
    }

    /// Returns the sealed client secret, private key and webhook secret of the App of the
    /// instance.
    pub async fn sealed_app_secrets(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        sqlx::query_as(
            "SELECT sealed_client_secret, sealed_private_key, sealed_webhook_secret
             FROM vcs_apps WHERE provider = 'github'",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the App of the instance")
    }
}

impl FakeProvider {
    /// Answers the conversion of a manifest code with the App 77 `scylla-acme` of `acme`, the
    /// key of the test App and the secrets of the manifest tests.
    pub fn answer_manifest_code(&self, code: &str) {
        let body = json!({
            "id": 77,
            "slug": "scylla-acme",
            "name": "Scylla acme",
            "owner": { "login": "acme", "type": "Organization" },
            "html_url": "https://github.com/apps/scylla-acme",
            "client_id": "Iv1.manifest",
            "client_secret": MANIFEST_CLIENT_SECRET,
            "webhook_secret": MANIFEST_WEBHOOK_SECRET,
            "pem": TEST_APP_PRIVATE_KEY,
        });
        let route = format!("/app-manifests/{code}/conversions");
        self.respond("POST", &route, 201, body, &[]);
    }
}
