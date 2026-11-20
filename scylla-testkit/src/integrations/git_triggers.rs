//! The `TestApp` helpers of the runs of the push, pull request and tag events: the runner of the
//! triggers of a workspace, and the answers of the fake provider for a token of the installation,
//! a commit and the files of a pull request.

use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde_json::{json, Value};

use super::FakeProvider;
use crate::TestApp;

impl TestApp {
    /// Returns the runner of the triggers of a workspace as the text of its id, or `None` before
    /// its first fire.
    pub async fn trigger_runner(&self, workspace: WorkspaceId) -> Option<String> {
        sqlx::query_scalar("SELECT actor_id::text FROM trigger_runners WHERE workspace_id = $1")
            .bind(workspace.to_db())
            .fetch_optional(&self.pool)
            .await
            .expect("read the runner of the triggers")
    }
}

impl FakeProvider {
    /// Answers the tokens of the installation 42 of the test GitHub App with `ghs_runs`.
    pub fn answer_installation_token(&self) {
        let token = json!({ "token": "ghs_runs", "expires_at": "2099-01-01T00:00:00Z" });
        let route = "/app/installations/42/access_tokens";
        self.respond("POST", route, 201, token, &[]);
    }

    /// Answers the commit `sha` of the GitHub repository `full_name` with a message.
    pub fn answer_github_commit(&self, full_name: &str, sha: &str, message: &str) {
        let commit = json!({
            "sha": sha,
            "commit": { "message": message },
            "author": { "login": "octocat", "avatar_url": null },
        });
        let route = format!("/repos/{full_name}/commits/{sha}");
        self.respond("GET", &route, 200, commit, &[]);
    }

    /// Adds an answer for the files of the pull request `number` of the GitHub repository
    /// `full_name`. The answers serve the requests in order.
    pub fn answer_pull_files(&self, full_name: &str, number: u32, files: &[&str]) {
        let items: Vec<Value> = files
            .iter()
            .map(|file| json!({ "filename": file, "status": "modified" }))
            .collect();
        let route = format!("/repos/{full_name}/pulls/{number}/files");
        self.respond("GET", &route, 200, Value::Array(items), &[]);
    }

    /// Answers the files of the merge request `iid` of the GitLab project `project`.
    pub fn answer_merge_request_diffs(&self, project: u64, iid: u32, files: &[&str]) {
        let items: Vec<Value> = files
            .iter()
            .map(|file| json!({ "old_path": file, "new_path": file }))
            .collect();
        let route = format!("/api/v4/projects/{project}/merge_requests/{iid}/diffs");
        self.respond("GET", &route, 200, Value::Array(items), &[]);
    }
}
