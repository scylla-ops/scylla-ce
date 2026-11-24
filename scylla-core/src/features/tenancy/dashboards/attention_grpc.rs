//! The handler of `ListAttentionItems`: the open items of a workspace that need a person, of the
//! kinds that the caller reads.

use scylla_domain::authz::Object;
use scylla_domain::identity::apps::AppHolder;
use scylla_domain::projects::pipelines::{PipelineAccess, PipelineHolder};
use scylla_domain::projects::triggers::{TriggerAccess, TriggerHolder};
use scylla_domain::tenancy::dashboards::attention::{AttentionReach, Reach};
use scylla_domain::tenancy::dashboards::DashboardCaller;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Principal;
use scylla_proto::dashboards::v1 as pb;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::DashboardsGrpc;
use super::{attention, attention_convert};
use crate::features::projects::pipelines::access::visible_projects;
use crate::Error;

impl DashboardsGrpc {
    /// Handles `ListAttentionItems`: one page of the open items of a workspace, with the counts
    /// of each kind. The list drops each kind that the caller does not read, and a filter never
    /// causes a refusal.
    pub(super) async fn attention(
        &self,
        request: Request<pb::ListAttentionItemsRequest>,
    ) -> Result<pb::ListAttentionItemsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let mut query = attention_convert::query(request.into_inner())?;
        let workspace = query.workspace;

        // Check the route permission of the caller, then find what it reads of each kind.
        let check = DashboardCaller::LIST_ATTENTION_ITEMS.on(caller.actor.id);
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;
        query.reach = self.reach(&caller, workspace).await;

        // Read one page of the open items.
        let connected = self.registry.connected_in(workspace);
        let now = OffsetDateTime::now_utc();
        let list = attention::list(&self.pool, query, &connected, now).await?;
        Ok(attention_convert::response(list))
    }

    /// Returns the projects of a workspace in which the caller reads each kind: the jobs with
    /// the route `ListWorkspaceJobs`, the fires with `ListTriggerFires`, the approvals with
    /// `ApproveJob`, and the agents with `ListApps` on the workspace. It is not a route check.
    async fn reach(&self, caller: &Principal, workspace: WorkspaceId) -> AttentionReach {
        let gate = self.gate.as_ref();
        let (object, apps) = (Object::of(workspace), [AppHolder::LIST_APPS.name()]);
        let (jobs, fires, approvals, agents) = tokio::join!(
            visible_projects(
                gate,
                caller,
                workspace,
                PipelineHolder::LIST_WORKSPACE_JOBS,
                PipelineAccess::LIST_WORKSPACE_JOBS,
            ),
            visible_projects(
                gate,
                caller,
                workspace,
                TriggerHolder::LIST_TRIGGER_FIRES,
                TriggerAccess::LIST_TRIGGER_FIRES,
            ),
            visible_projects(
                gate,
                caller,
                workspace,
                PipelineHolder::APPROVE_JOB,
                PipelineAccess::APPROVE_JOB,
            ),
            gate.permissions(caller, &object, Some(workspace), &apps),
        );
        let agents = match agents {
            Ok(answers) if answers.first().copied().unwrap_or(false) => Reach::Workspace,
            Ok(_) => Reach::Nothing,
            Err(error) => {
                tracing::warn!(%error, "a check failed: the list shows no agent");
                Reach::Nothing
            }
        };
        AttentionReach {
            jobs: Reach::of(jobs),
            fires: Reach::of(fires),
            approvals: Reach::of(approvals),
            agents,
        }
    }
}
