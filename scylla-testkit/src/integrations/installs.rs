//! The `TestApp` helper of a new install of the GitHub App: the whole install flow on the fake
//! provider.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::{CompleteGitHubInstallRequest, VcsConnection};
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Installs the GitHub App in a workspace as `actor`: starts the install, then completes it
    /// with the redirect of GitHub for `installation` and the code `good-code`. The fake must
    /// answer the install (`FakeProvider::answer_github_install`).
    pub async fn install_github(
        &self,
        actor: ActorId,
        workspace: &str,
        installation: u64,
    ) -> Result<VcsConnection, Status> {
        let started = self.start_github_install(actor, workspace).await?;
        let (_, state) = started.install_url.split_once("?state=").expect("a state");
        let redirect = CompleteGitHubInstallRequest {
            state: state.to_owned(),
            installation_id: installation.to_string(),
            code: "good-code".to_owned(),
            setup_action: "install".to_owned(),
        };
        self.complete_github_install(actor, redirect).await
    }
}
