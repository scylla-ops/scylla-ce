//! The handlers of the lists of `TriggerService` for the dashboard: the triggers of a workspace,
//! their fires, and the counts of their fires, in the projects that the caller can see.

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::triggers::{TriggerAccess, TriggerCaller, TriggerHolder};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::Principal;
use scylla_proto::pipelines::v1 as pb;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::TriggersGrpc;
use super::{history, lists_convert, lists_proto, summaries};
use crate::api::parse::ToProto;
use crate::features::projects::pipelines::access;
use crate::Error;

/// The route permissions of a list: on the caller, on the workspace, and on a project.
type Routes = (
    Permission<Actor>,
    Permission<Workspace>,
    Permission<Project>,
);

impl TriggersGrpc {
    /// Handles `ListWorkspaceTriggers`: one page of the triggers of a workspace, in the projects
    /// that the caller can see. A filter never causes a refusal.
    pub(super) async fn list_workspace(
        &self,
        request: Request<pb::ListWorkspaceTriggersRequest>,
    ) -> Result<pb::ListWorkspaceTriggersResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let mut query = lists_convert::summaries_query(request.into_inner(), now)?;

        // Find the projects that the caller can see, then read one page.
        let routes = (
            TriggerCaller::LIST_WORKSPACE_TRIGGERS,
            TriggerHolder::LIST_WORKSPACE_TRIGGERS,
            TriggerAccess::LIST_WORKSPACE_TRIGGERS,
        );
        query.visible = self.visible(&caller, query.workspace, routes).await?;
        let (page, zone) = summaries::list(&self.pool, query, now).await?;
        Ok(lists_proto::summaries_response(page, zone))
    }

    /// Handles `ListTriggerFires`: one page of the fires of a workspace, newest due time first,
    /// in the projects that the caller can see. A filter never causes a refusal.
    pub(super) async fn list_fires(
        &self,
        request: Request<pb::ListTriggerFiresRequest>,
    ) -> Result<pb::ListTriggerFiresResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let mut query = lists_convert::fires_query(request.into_inner())?;

        // Find the projects that the caller can see, then read one page.
        let routes = (
            TriggerCaller::LIST_TRIGGER_FIRES,
            TriggerHolder::LIST_TRIGGER_FIRES,
            TriggerAccess::LIST_TRIGGER_FIRES,
        );
        query.fires.visible = self.visible(&caller, query.fires.workspace, routes).await?;
        let page = history::list(&self.pool, query).await?;
        Ok(lists_proto::fires_response(page))
    }

    /// Handles `GetTriggerFireStats`: the counts of the fires of a workspace for a period, in the
    /// projects that the caller can see. A filter never causes a refusal.
    pub(super) async fn fire_stats(
        &self,
        request: Request<pb::GetTriggerFireStatsRequest>,
    ) -> Result<pb::GetTriggerFireStatsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let mut query = lists_convert::stats_query(request.into_inner(), now)?;

        // Find the projects that the caller can see, then count their fires.
        let routes = (
            TriggerCaller::GET_TRIGGER_FIRE_STATS,
            TriggerHolder::GET_TRIGGER_FIRE_STATS,
            TriggerAccess::GET_TRIGGER_FIRE_STATS,
        );
        query.fires.visible = self.visible(&caller, query.fires.workspace, routes).await?;
        let stats = history::stats(&self.pool, query).await?;
        Ok(stats.to_proto())
    }

    /// Checks the route permission of a list on the caller, and returns the projects of the
    /// workspace that it can see: `None` for each project.
    async fn visible(
        &self,
        caller: &Principal,
        workspace: WorkspaceId,
        (route, holder, access): Routes,
    ) -> Result<Option<Vec<ProjectId>>, Error> {
        let check = route.on(caller.actor.id).within(workspace);
        self.gate.authorize(caller, check).await?;
        let gate = self.gate.as_ref();
        Ok(access::visible_projects(gate, caller, workspace, holder, access).await)
    }
}
