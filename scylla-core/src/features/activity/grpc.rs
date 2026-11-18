//! The gRPC handler of `ActivityService`: authenticate, parse, authorize, find what the caller
//! reads, call the service, answer.

use std::sync::Arc;

use scylla_domain::activity::{ActivityCaller, ListActivity};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Gate;
use scylla_proto::activity::v1 as pb;
use scylla_proto::activity::v1::activity_service_server::{ActivityService, ActivityServiceServer};
use sqlx::PgPool;
use time::OffsetDateTime;
use tonic::{Request, Response, Status};

use super::{access, service};
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> ActivityServiceServer<ActivityGrpc> {
    ActivityServiceServer::new(ActivityGrpc { pool, gate })
}

/// The handler.
pub(super) struct ActivityGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl ActivityService for ActivityGrpc {
    async fn list_activity(
        &self,
        request: Request<pb::ListActivityRequest>,
    ) -> Result<Response<pb::ListActivityResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }
}

impl ActivityGrpc {
    /// Handles `ListActivity`: one page of the rows of a workspace that the caller can read,
    /// newest first. A caller with no role gets an empty list.
    async fn list(
        &self,
        request: Request<pb::ListActivityRequest>,
    ) -> Result<pb::ListActivityResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let page = parse::optional(request.page, "page")?;
        let since: Option<OffsetDateTime> = request
            .since
            .map(|at| parse::field(at, "since"))
            .transpose()?;

        // Check the route permission of the caller, then find what it reads.
        let check = ActivityCaller::LIST_ACTIVITY.on(caller.actor.id);
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;
        let visible = access::visible(self.gate.as_ref(), &caller, workspace).await;

        // Read one page of the rows that it reads.
        let query = ListActivity {
            workspace,
            visible,
            page,
            since,
        };
        let found = service::list(&self.pool, query).await?;
        Ok(pb::ListActivityResponse {
            activities: found
                .page
                .items
                .into_iter()
                .map(ToProto::to_proto)
                .collect(),
            page: Some(found.page.next_after.to_proto()),
            since_count: found.since_count,
        })
    }
}
