//! The `TestApp` helpers of the polling of the linked repositories: the poll state of a link, a
//! poll that is due at once, and the answers of the fake provider for the heads of a repository.

use std::time::Duration;

use serde_json::{json, Value};

use super::FakeProvider;
use crate::TestApp;

/// The longest wait for a poll, longer than the passes of 2 seconds of the task.
const WAIT: Duration = Duration::from_secs(15);

/// The time between two reads of a wait.
const POLL: Duration = Duration::from_millis(100);

impl TestApp {
    /// Keeps a row of the poll state of the link of a project: the head of a ref, or the cursor
    /// of a list (`branches`, `pulls`) with no head.
    pub async fn insert_poll_state(&self, project: &str, name: &str, sha: Option<&str>) {
        sqlx::query(
            "INSERT INTO vcs_poll_state (project_id, ref, last_sha) VALUES ($1::uuid, $2, $3)",
        )
        .bind(project)
        .bind(name)
        .bind(sha)
        .execute(&self.pool)
        .await
        .expect("insert a row of the poll state");
    }

    /// Returns the heads of the poll state of the link of a project: `(ref, sha)`, in the order
    /// of the refs. The cursors of the lists are not in it.
    pub async fn poll_heads(&self, project: &str) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT ref, last_sha FROM vcs_poll_state
             WHERE project_id = $1::uuid AND last_sha IS NOT NULL ORDER BY ref",
        )
        .bind(project)
        .fetch_all(&self.pool)
        .await
        .expect("read the poll state")
    }

    /// Makes the poll of the link of a project due at once. Returns the time of the database
    /// before the poll, for `wait_for_poll`.
    pub async fn poll_now(&self, project: &str) -> String {
        sqlx::query_scalar(
            "UPDATE repository_links SET next_poll_at = now() - interval '1 second'
             WHERE project_id = $1::uuid
             RETURNING now()::text",
        )
        .bind(project)
        .fetch_one(&self.pool)
        .await
        .expect("make a poll due")
    }

    /// Waits until a poll of the link of a project kept its lists after the time `since`. Fails
    /// after 15 seconds.
    pub async fn wait_for_poll(&self, project: &str, since: &str) {
        let start = tokio::time::Instant::now();
        loop {
            let polled: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM vcs_poll_state
                                WHERE project_id = $1::uuid AND ref = 'pulls'
                                  AND polled_at > $2::timestamptz)",
            )
            .bind(project)
            .bind(since)
            .fetch_one(&self.pool)
            .await
            .expect("read the poll state");
            if polled {
                return;
            }
            assert!(
                start.elapsed() < WAIT,
                "the link of {project} was not polled"
            );
            tokio::time::sleep(POLL).await;
        }
    }

    /// Waits until the link of a project is in a delivery mode, and returns its mode. Fails
    /// after 15 seconds.
    pub async fn wait_for_link_mode(&self, project: &str, mode: &str) -> super::ModeRow {
        let start = tokio::time::Instant::now();
        loop {
            let found = self.link_mode(project).await;
            if found.delivery_mode == mode {
                return found;
            }
            assert!(
                start.elapsed() < WAIT,
                "the link of {project} is not in {mode} mode"
            );
            tokio::time::sleep(POLL).await;
        }
    }
}

impl FakeProvider {
    /// Answers the heads of the GitHub repository `full_name` of the installation 42: its
    /// branches (`(name, sha)`) with the ETag `etag`, its open pull requests, and the commit of
    /// each branch with the message `Commit <sha>`.
    pub fn answer_github_heads(
        &self,
        full_name: &str,
        branches: &[(&str, &str)],
        pulls: Value,
        etag: &str,
    ) {
        let token = json!({ "token": "ghs_poll", "expires_at": "2099-01-01T00:00:00Z" });
        self.respond(
            "POST",
            "/app/installations/42/access_tokens",
            201,
            token,
            &[],
        );
        let items: Vec<Value> = branches
            .iter()
            .map(|(name, sha)| json!({ "name": name, "commit": { "sha": sha } }))
            .collect();
        let route = format!("/repos/{full_name}/branches");
        self.respond("GET", &route, 200, Value::Array(items), &[("etag", etag)]);
        self.respond("GET", &format!("/repos/{full_name}/pulls"), 200, pulls, &[]);
        for (_, sha) in branches {
            let commit = json!({
                "sha": sha,
                "commit": { "message": format!("Commit {sha}") },
                "author": { "login": "octocat", "avatar_url": null },
            });
            let route = format!("/repos/{full_name}/commits/{sha}");
            self.respond("GET", &route, 200, commit, &[]);
        }
    }

    /// Answers the heads of the GitLab project `project`: its branches (`(name, sha)`) and its
    /// open merge requests, and the state of its webhook `hook` (`executable` or `disabled`).
    pub fn answer_gitlab_heads(
        &self,
        project: u64,
        branches: &[(&str, &str)],
        merge_requests: Value,
        hook: (u64, &str),
    ) {
        let items: Vec<Value> = branches
            .iter()
            .map(|(name, sha)| json!({ "name": name, "commit": { "id": sha, "message": "Fix" } }))
            .collect();
        let base = format!("/api/v4/projects/{project}");
        let route = format!("{base}/repository/branches");
        self.respond("GET", &route, 200, Value::Array(items), &[]);
        let route = format!("{base}/merge_requests");
        self.respond("GET", &route, 200, merge_requests, &[]);
        let (hook, state) = hook;
        let found = json!({ "id": hook, "alert_status": state });
        self.respond("GET", &format!("{base}/hooks/{hook}"), 200, found, &[]);
    }
}
