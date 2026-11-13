//! The `TestApp` helpers of the webhooks of the providers: the GitLab webhook of a link with its
//! signing token, the deliveries that the server kept, and the modes and the states of the links
//! and of the connections that the webhooks change.

use std::time::Duration;

use serde_json::{json, Value};

use super::FakeProvider;
use crate::TestApp;

/// The longest wait for the work of a delivery.
const WAIT: Duration = Duration::from_secs(15);

/// The time between two reads of a wait.
const POLL: Duration = Duration::from_millis(100);

/// A row of `vcs_deliveries`: the provider, the key, the event, the action, the end and the
/// error.
type Delivered = (String, String, String, Option<String>, bool, Option<String>);

/// A delivery that the server kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryRow {
    /// `github` or `gitlab`.
    pub provider: String,
    /// The id of the delivery, or the key of a poll.
    pub key: String,
    /// The event, such as `push`.
    pub event: String,
    /// The action, or an empty text.
    pub action: String,
    /// `true` when its work ended.
    pub processed: bool,
    /// The error of its work, or an empty text.
    pub error: String,
}

/// The delivery mode and the state of a link or of a connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeRow {
    /// `webhook` or `polling`.
    pub delivery_mode: String,
    /// The state, such as `active`.
    pub state: String,
    /// The reason of the state or of the mode, or an empty text.
    pub reason: String,
    /// `true` when a webhook arrived.
    pub has_event: bool,
}

impl TestApp {
    /// Gives the link of a project the GitLab webhook `hook` with a signing token, sealed with
    /// the id of the project as the server does.
    pub async fn set_link_signing_token(&self, project: &str, hook: &str, signing_token: &str) {
        let id = uuid::Uuid::parse_str(project).expect("a project id");
        let sealer = self.sealer.as_ref().expect("a server with a key");
        let sealed = sealer.seal_with(signing_token.as_bytes(), id.as_bytes());
        sqlx::query(
            "UPDATE repository_links SET hook_id = $2, sealed_hook_secret = $3
             WHERE project_id = $1",
        )
        .bind(id)
        .bind(hook)
        .bind(sealed)
        .execute(&self.pool)
        .await
        .expect("set the signing token of a link");
    }

    /// Removes the GitLab webhook of the link of a project.
    pub async fn set_link_hook_off(&self, project: &str) {
        sqlx::query(
            "UPDATE repository_links SET hook_id = NULL, sealed_hook_secret = NULL
             WHERE project_id = $1::uuid",
        )
        .bind(project)
        .execute(&self.pool)
        .await
        .expect("remove the hook of a link");
    }

    /// Returns the deliveries that the server kept, in the order of their keys.
    pub async fn deliveries(&self) -> Vec<DeliveryRow> {
        let rows: Vec<Delivered> = sqlx::query_as(
            "SELECT provider, delivery_id, event, action, processed_at IS NOT NULL, error
                 FROM vcs_deliveries ORDER BY delivery_id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read the deliveries");
        rows.into_iter()
            .map(|row| DeliveryRow {
                provider: row.0,
                key: row.1,
                event: row.2,
                action: row.3.unwrap_or_default(),
                processed: row.4,
                error: row.5.unwrap_or_default(),
            })
            .collect()
    }

    /// Waits until the work of a delivery ended, and returns it. Fails after 15 seconds.
    pub async fn wait_for_delivery(&self, key: &str) -> DeliveryRow {
        let start = tokio::time::Instant::now();
        loop {
            let found = self
                .deliveries()
                .await
                .into_iter()
                .find(|row| row.key == key);
            if let Some(found) = found.filter(|row| row.processed) {
                return found;
            }
            assert!(start.elapsed() < WAIT, "the delivery {key} did not end");
            tokio::time::sleep(POLL).await;
        }
    }

    /// Returns the delivery mode and the state of the link of a project.
    pub async fn link_mode(&self, project: &str) -> ModeRow {
        let row: (String, String, Option<String>, bool) = sqlx::query_as(
            "SELECT delivery_mode, state, state_reason, last_event_at IS NOT NULL
             FROM repository_links WHERE project_id = $1::uuid",
        )
        .bind(project)
        .fetch_one(&self.pool)
        .await
        .expect("read a link");
        mode_row(row)
    }

    /// Returns the delivery mode and the state of a connection.
    pub async fn connection_mode(&self, connection: &str) -> ModeRow {
        let row: (String, String, Option<String>, bool) = sqlx::query_as(
            "SELECT delivery_mode, state, state_reason, last_event_at IS NOT NULL
             FROM vcs_connections WHERE id = $1::uuid",
        )
        .bind(connection)
        .fetch_one(&self.pool)
        .await
        .expect("read a connection");
        mode_row(row)
    }

    /// Sets the delivery mode of the link of a project and its reason.
    pub async fn set_link_mode(&self, project: &str, mode: &str, reason: Option<&str>) {
        sqlx::query(
            "UPDATE repository_links SET delivery_mode = $2, state_reason = $3
             WHERE project_id = $1::uuid",
        )
        .bind(project)
        .bind(mode)
        .bind(reason)
        .execute(&self.pool)
        .await
        .expect("set the mode of a link");
    }

    /// Returns the number of repositories of a connection, and `true` when it sees all of them.
    pub async fn connection_repositories(&self, connection: &str) -> (i32, bool) {
        sqlx::query_as(
            "SELECT repository_count, all_repositories FROM vcs_connections WHERE id = $1::uuid",
        )
        .bind(connection)
        .fetch_one(&self.pool)
        .await
        .expect("read a connection")
    }
}

impl FakeProvider {
    /// Answers the deliveries of the webhook of the App (the last one first) and a new delivery
    /// of each one. `deliveries` has the id and the installation of each delivery.
    pub fn answer_github_deliveries(&self, deliveries: &[(u64, u64)]) {
        let listed: Vec<Value> = deliveries
            .iter()
            .map(|(id, installation)| {
                json!({ "id": id, "guid": format!("guid-{id}"), "installation_id": installation })
            })
            .collect();
        self.respond(
            "GET",
            "/app/hook/deliveries",
            200,
            Value::Array(listed),
            &[],
        );
        for (id, _) in deliveries {
            let route = format!("/app/hook/deliveries/{id}/attempts");
            self.respond("POST", &route, 202, json!({}), &[]);
        }
    }

    /// Answers a test push on the GitLab webhook `hook` of the project `project`.
    pub fn answer_gitlab_hook_test(&self, project: u64, hook: u64) {
        let route = format!("/api/v4/projects/{project}/hooks/{hook}/test/push_events");
        self.respond(
            "POST",
            &route,
            201,
            json!({ "message": "201 Created" }),
            &[],
        );
    }
}

/// Makes the mode of a row of a link or of a connection.
fn mode_row(row: (String, String, Option<String>, bool)) -> ModeRow {
    ModeRow {
        delivery_mode: row.0,
        state: row.1,
        reason: row.2.unwrap_or_default(),
        has_event: row.3,
    }
}
