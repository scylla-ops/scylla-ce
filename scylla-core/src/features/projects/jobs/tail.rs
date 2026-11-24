//! The handler of `TailJobLogs`: the logs after a cursor, then the new logs while the job runs.
//! The task reads the table again each half second, so it needs no shared memory.

use std::time::Duration;

use scylla_domain::projects::jobs::{JobId, LogBatchId};
use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_proto::pipelines::v1 as pb;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Status};
use tracing::Instrument;

use super::grpc::JobsGrpc;
use super::service;
use crate::api::parse::ToProto;
use crate::Error;

/// The stream of the batches of a tail.
pub(crate) type Tail = ReceiverStream<Result<pb::TailJobLogsResponse, Status>>;

/// The most batches in one read.
const PAGE: i64 = 100;

/// The time between two reads of a running job.
const POLL: Duration = Duration::from_millis(500);

impl JobsGrpc {
    /// Handles `TailJobLogs`: checks the caller, then follows the logs in a task.
    pub(super) async fn tail(
        &self,
        request: Request<pb::TailJobLogsRequest>,
    ) -> Result<Tail, Error> {
        // Authenticate the caller, read the request, and check the route permission.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let node = Some(request.node_id).filter(|node| !node.is_empty());
        let cursor = match request.after.as_str() {
            "" => None,
            text => {
                Some(LogBatchId::parse(text).map_err(|e| Error::invalid("after", e.to_string()))?)
            }
        };
        let id = self
            .job_of(&caller, request.job_id, PipelineAccess::TAIL_JOB_LOGS)
            .await?;

        // Follow the logs in a task, in the span of the call.
        let (out, batches) = mpsc::channel(PAGE as usize);
        let pool = self.pool.clone();
        tokio::spawn(follow(pool, id, node, cursor, out).in_current_span());
        Ok(ReceiverStream::new(batches))
    }
}

/// Sends the batches after the cursor until the job ended and no batch is left, or the client
/// went away.
async fn follow(
    pool: PgPool,
    id: JobId,
    node: Option<String>,
    mut cursor: Option<LogBatchId>,
    out: mpsc::Sender<Result<pb::TailJobLogsResponse, Status>>,
) {
    let mut ended = false;
    loop {
        // Read the next batches, and send them.
        let batches = match service::logs(&pool, id, node.as_deref(), cursor, PAGE).await {
            Ok(batches) => batches,
            Err(error) => {
                let _ = out.send(Err(error.into())).await;
                return;
            }
        };
        let full = batches.len() as i64 == PAGE;
        for batch in batches {
            cursor = Some(batch.id);
            if out.send(Ok(batch.to_proto())).await.is_err() {
                return;
            }
        }
        if full {
            continue;
        }

        // After the last read of a job that ended, stop. Else wait, and read again.
        if ended {
            return;
        }
        ended = match service::get(&pool, id).await {
            Ok(Some(job)) => job.status.is_final(),
            Ok(None) => true,
            Err(error) => {
                let _ = out.send(Err(error.into())).await;
                return;
            }
        };
        if !ended {
            tokio::time::sleep(POLL).await;
        }
    }
}
