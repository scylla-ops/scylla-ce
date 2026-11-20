//! The `TestApp` helpers of the install of the GitHub App (the redirects of GitHub, and the
//! whole install flow), and the answers of the fake provider for an install.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::complete_git_hub_install_response::Outcome;
use scylla_proto::integrations::v1::{
    CompleteGitHubInstallRequest, GitHubInstallChoice, VcsConnection,
};
use serde_json::{json, Value};
use tonic::Status;

use super::FakeProvider;
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
        let redirect = CompleteGitHubInstallRequest {
            state: state_of(&started.install_url).to_owned(),
            installation_id: installation.to_string(),
            code: "good-code".to_owned(),
            setup_action: "install".to_owned(),
        };
        self.complete_github_install(actor, redirect).await
    }

    /// Calls `CompleteGitHubInstall` as `actor`, with the values of a redirect of GitHub, and
    /// returns the connection.
    pub async fn complete_github_install(
        &self,
        actor: ActorId,
        redirect: CompleteGitHubInstallRequest,
    ) -> Result<VcsConnection, Status> {
        match self.send_complete_github_install(actor, redirect).await? {
            Outcome::Connection(connection) => Ok(connection),
            Outcome::Choice(choice) => panic!("a connection, not {choice:?}"),
        }
    }

    /// Calls `CompleteGitHubInstall` as `actor` after the authorize page, and returns the
    /// installations to choose.
    pub async fn choose_github_installation(
        &self,
        actor: ActorId,
        redirect: CompleteGitHubInstallRequest,
    ) -> Result<GitHubInstallChoice, Status> {
        match self.send_complete_github_install(actor, redirect).await? {
            Outcome::Choice(choice) => Ok(choice),
            Outcome::Connection(connection) => panic!("a choice, not {connection:?}"),
        }
    }

    /// Sends a request of `CompleteGitHubInstall` as `actor`, and returns its outcome.
    async fn send_complete_github_install(
        &self,
        actor: ActorId,
        redirect: CompleteGitHubInstallRequest,
    ) -> Result<Outcome, Status> {
        let request = self.authed(actor, redirect).await;
        let response = self.connections().complete_git_hub_install(request).await?;
        Ok(response.into_inner().outcome.expect("an outcome"))
    }
}

impl FakeProvider {
    /// Answers the calls of an install of the test App on the organization `account`: the code
    /// `good-code` gives the user token `ghu_user`, the user sees the installation, the
    /// installation has the permissions of `permissions`, and it sees `repositories`.
    pub fn answer_github_install(
        &self,
        installation: u64,
        account: &str,
        permissions: Value,
        repositories: &[(u64, &str, bool)],
    ) {
        self.answer_github_user(&[(installation, account, "Organization")]);
        let found = json!({
            "id": installation,
            "account": {
                "login": account,
                "avatar_url": format!("https://avatars.example/{account}"),
                "type": "Organization",
            },
            "repository_selection": "selected",
            "permissions": permissions,
            "suspended_at": null,
        });
        let path = format!("/app/installations/{installation}");
        self.respond("GET", &path, 200, found, &[]);
        let token = json!({ "token": "ghs_installation", "expires_at": "2099-01-01T00:00:00Z" });
        let path = format!("/app/installations/{installation}/access_tokens");
        self.respond("POST", &path, 201, token, &[]);
        let items: Vec<Value> = repositories
            .iter()
            .map(|(id, name, private)| {
                json!({
                    "id": id,
                    "full_name": name,
                    "html_url": format!("https://github.com/{name}"),
                    "default_branch": "main",
                    "private": private,
                })
            })
            .collect();
        let list = json!({ "total_count": items.len(), "repositories": items });
        self.respond("GET", "/installation/repositories", 200, list, &[]);
    }

    /// Answers the user of the code `good-code`: it gives the user token `ghu_user`, and the user
    /// sees the installations of the test App on the accounts `(id, login, type)`, where the type
    /// is `Organization` or `User`.
    pub fn answer_github_user(&self, installations: &[(u64, &str, &str)]) {
        let user = json!({ "access_token": "ghu_user", "token_type": "bearer" });
        self.respond("POST", "/login/oauth/access_token", 200, user, &[]);
        let items: Vec<Value> = installations
            .iter()
            .map(|(id, login, kind)| {
                json!({
                    "id": id,
                    "account": {
                        "login": login,
                        "avatar_url": format!("https://avatars.example/{login}"),
                        "type": kind,
                    },
                })
            })
            .collect();
        let listed = json!({ "total_count": items.len(), "installations": items });
        self.respond("GET", "/user/installations", 200, listed, &[]);
    }
}

/// Returns the state in a page of GitHub.
fn state_of(page: &str) -> &str {
    let (_, query) = page.split_once('?').expect("a query");
    let mut pairs = query.split('&');
    pairs
        .find_map(|pair| pair.strip_prefix("state="))
        .expect("a state")
}
