//! The `TestApp` helpers of the rules that block a merge, one for each RPC of
//! `MergeBlockService`, the rows that a rule reads, and the answers of the fake provider for the
//! GitHub ruleset of Scylla and for "Pipelines must succeed" on GitLab.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::merge_block_service_client::MergeBlockServiceClient;
use scylla_proto::integrations::v1::{ListMergeBlocksRequest, MergeBlock, SetMergeBlockRequest};
use serde_json::{json, Value};
use tonic::transport::Channel;
use tonic::Status;

use super::FakeProvider;
use crate::{id, TestApp};

impl TestApp {
    /// A client of `MergeBlockService`.
    fn merge_blocks(&self) -> MergeBlockServiceClient<Channel> {
        MergeBlockServiceClient::new(self.channel.clone())
    }

    /// Calls `ListMergeBlocks` as `actor`.
    pub async fn list_merge_blocks(
        &self,
        actor: ActorId,
        project: &str,
    ) -> Result<Vec<MergeBlock>, Status> {
        let request = ListMergeBlocksRequest {
            project_id: Some(id(project)),
        };
        let request = self.authed(actor, request).await;
        let response = self.merge_blocks().list_merge_blocks(request).await?;
        Ok(response.into_inner().blocks)
    }

    /// Calls `SetMergeBlock` as `actor`, on a pipeline or on the project for `None`.
    pub async fn set_merge_block(
        &self,
        actor: ActorId,
        project: &str,
        pipeline: Option<&str>,
        enabled: bool,
    ) -> Result<MergeBlock, Status> {
        let request = SetMergeBlockRequest {
            project_id: Some(id(project)),
            pipeline_id: pipeline.map(id),
            enabled,
        };
        let request = self.authed(actor, request).await;
        let response = self.merge_blocks().set_merge_block(request).await?;
        Ok(response.into_inner().block.expect("a merge rule"))
    }

    /// Sets the permissions of a GitHub connection, for example `{"administration": "write"}`.
    pub async fn set_connection_permissions(&self, connection: &str, permissions: Value) {
        sqlx::query("UPDATE vcs_connections SET permissions = $2 WHERE id = $1::uuid")
            .bind(connection)
            .bind(permissions)
            .execute(&self.pool)
            .await
            .expect("set the permissions of a connection");
    }

    /// Sets the state of a connection and its reason.
    pub async fn set_connection_state(&self, connection: &str, state: &str, reason: &str) {
        sqlx::query("UPDATE vcs_connections SET state = $2, state_reason = $3 WHERE id = $1::uuid")
            .bind(connection)
            .bind(state)
            .bind(reason)
            .execute(&self.pool)
            .await
            .expect("set the state of a connection");
    }

    /// Sets the privacy of the repository of a project, and the role of the GitLab service
    /// account (`developer`, `maintainer` or `owner`).
    pub async fn set_link_access(&self, project: &str, private: bool, role: Option<&str>) {
        sqlx::query(
            "UPDATE repository_links SET private = $2, role = $3 WHERE project_id = $1::uuid",
        )
        .bind(project)
        .bind(private)
        .bind(role)
        .execute(&self.pool)
        .await
        .expect("set the access of a link");
    }

    /// Returns the id of the GitHub ruleset of the link of a project.
    pub async fn ruleset_of(&self, project: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT ruleset_id FROM repository_links WHERE project_id = $1::uuid")
            .bind(project)
            .fetch_one(&self.pool)
            .await
            .expect("read a link")
    }
}

impl FakeProvider {
    /// Answers the installation tokens of an installation with the token `ghs_rules`.
    pub fn answer_github_rules_token(&self, installation: u64) {
        let token = json!({ "token": "ghs_rules", "expires_at": "2099-01-01T00:00:00Z" });
        let route = format!("/app/installations/{installation}/access_tokens");
        self.respond("POST", &route, 201, token, &[]);
    }

    /// Answers the rulesets of the GitHub repository `full_name`: an empty list, the creation
    /// of the ruleset `id`, its change and its delete.
    pub fn answer_github_rulesets(&self, full_name: &str, id: i64) {
        let route = format!("/repos/{full_name}/rulesets");
        self.respond("GET", &route, 200, json!([]), &[]);
        let made = json!({ "id": id, "name": "Scylla required checks" });
        self.respond("POST", &route, 201, made.clone(), &[]);
        let route = format!("{route}/{id}");
        self.respond("PUT", &route, 200, made, &[]);
        self.respond("DELETE", &route, 204, Value::Null, &[]);
    }

    /// Answers a read of the ruleset `id` of the GitHub repository `full_name`, with its
    /// enforcement and its required contexts.
    pub fn answer_github_ruleset(
        &self,
        full_name: &str,
        id: i64,
        enforcement: &str,
        contexts: &[&str],
    ) {
        let checks: Vec<Value> = contexts
            .iter()
            .map(|context| json!({ "context": context, "integration_id": 1 }))
            .collect();
        let ruleset = json!({
            "id": id,
            "name": "Scylla required checks",
            "source_type": "Repository",
            "enforcement": enforcement,
            "conditions": { "ref_name": { "include": ["~DEFAULT_BRANCH"], "exclude": [] } },
            "rules": [
                { "type": "deletion" },
                {
                    "type": "required_status_checks",
                    "parameters": {
                        "strict_required_status_checks_policy": false,
                        "required_status_checks": checks,
                    },
                },
            ],
        });
        let route = format!("/repos/{full_name}/rulesets/{id}");
        self.respond("GET", &route, 200, ruleset, &[]);
    }

    /// Answers a change of "Pipelines must succeed" of the GitLab project `project` with the
    /// status `status`.
    pub fn answer_gitlab_merge_rule(&self, project: u64, status: u16) {
        let route = format!("/api/v4/projects/{project}");
        self.respond("PUT", &route, status, json!({ "id": project }), &[]);
    }

    /// Answers a read of the GitLab project `project` where "Pipelines must succeed" is `on`.
    pub fn answer_gitlab_merge_setting(&self, project: u64, on: bool) {
        let found = json!({ "id": project, "only_allow_merge_if_pipeline_succeeds": on });
        let route = format!("/api/v4/projects/{project}");
        self.respond("GET", &route, 200, found, &[]);
    }
}
