//! The list of the secrets of a workspace in `SecretService`: the secrets of the projects where
//! the caller can list them, with no value.

use scylla_domain::projects::secrets::{ListWorkspaceSecrets, WorkspaceSecret};
use scylla_domain::projects::{Project, ProjectCaller, ProjectHolder};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::SearchText;
use scylla_proto::projects::v1 as pb;
use tonic::Request;

use super::grpc::SecretsGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::projects::pipelines::access;
use crate::Error;

impl SecretsGrpc {
    /// Handles `ListWorkspaceSecrets`: one page of the secrets of a workspace, newest first, in
    /// the projects where the caller can list them. A filter never causes a refusal.
    pub(super) async fn list_workspace(
        &self,
        request: Request<pb::ListWorkspaceSecretsRequest>,
    ) -> Result<pb::ListWorkspaceSecretsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let query = SearchText::parse(request.query)
            .map_err(|error| Error::invalid("query", error.to_string()))?;
        let page = parse::optional(request.page, "page")?;

        // Check the route permission, then find the projects where the caller lists secrets.
        let check = ProjectCaller::LIST_WORKSPACE_SECRETS.on(caller.actor.id);
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;
        let visible = access::visible_projects(
            self.gate.as_ref(),
            &caller,
            workspace,
            ProjectHolder::LIST_WORKSPACE_SECRETS,
            Project::LIST_WORKSPACE_SECRETS,
        )
        .await;

        // Read one page of the secrets of these projects.
        let list = ListWorkspaceSecrets {
            workspace,
            visible,
            query,
            page,
        };
        let page = service::list_workspace(&self.pool, list).await?;
        Ok(pb::ListWorkspaceSecretsResponse {
            secrets: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }
}

impl ToProto<pb::WorkspaceSecret> for WorkspaceSecret {
    fn to_proto(self) -> pb::WorkspaceSecret {
        pb::WorkspaceSecret {
            secret: Some(self.secret.to_proto()),
            project_name: self.project_name,
        }
    }
}
