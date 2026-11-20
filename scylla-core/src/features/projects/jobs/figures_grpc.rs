//! The handler of `GetJobStats`: the figures of the jobs of a workspace, in the projects that the
//! caller can see.

use scylla_domain::projects::pipelines::{PipelineAccess, PipelineCaller, PipelineHolder};
use scylla_proto::pipelines::v1 as pb;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::JobsGrpc;
use super::{figures, figures_convert};
use crate::api::parse::ToProto;
use crate::features::projects::pipelines::access;
use crate::Error;

impl JobsGrpc {
    /// Handles `GetJobStats`: the figures of the jobs of a workspace for a period, in the
    /// projects that the caller can see. A filter never causes a refusal.
    pub(super) async fn figures(
        &self,
        request: Request<pb::GetJobStatsRequest>,
    ) -> Result<pb::GetJobStatsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let mut query = figures_convert::query(request.into_inner(), now)?;
        let workspace = query.jobs.workspace;

        // Check the route permission of the caller, then find the projects that it can see.
        let check = PipelineCaller::GET_JOB_STATS.on(caller.actor.id);
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;
        query.jobs.visible = access::visible_projects(
            self.gate.as_ref(),
            &caller,
            workspace,
            PipelineHolder::GET_JOB_STATS,
            PipelineAccess::GET_JOB_STATS,
        )
        .await;

        // Count the jobs of the visible projects.
        let figures = figures::get(&self.pool, query).await?;
        Ok(figures.to_proto())
    }
}
