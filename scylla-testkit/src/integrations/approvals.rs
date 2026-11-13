//! The `TestApp` helpers of the approval of the runs from a fork: the two routes of the approval,
//! and a job from a fork that waits.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::job_service_client::JobServiceClient;
use scylla_proto::pipelines::v1::{ApproveJobRequest, Job, RejectJobRequest};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `ApproveJob` as `actor`, and returns the job.
    pub async fn approve_job(&self, actor: ActorId, job: &str) -> Result<Job, Status> {
        let request = ApproveJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        let mut client = JobServiceClient::new(self.channel.clone());
        let response = client.approve_job(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Calls `RejectJob` as `actor`, and returns the job.
    pub async fn reject_job(&self, actor: ActorId, job: &str) -> Result<Job, Status> {
        let request = RejectJobRequest {
            job_id: Some(id(job)),
        };
        let request = self.authed(actor, request).await;
        let mut client = JobServiceClient::new(self.channel.clone());
        let response = client.reject_job(request).await?.into_inner();
        Ok(response.job.expect("job in response"))
    }

    /// Makes a pending job with no agent a job from the fork `evil/app` on the pull request 57 at
    /// the commit `sha`, that waits for an approval. No report records the change.
    pub async fn make_fork_job_wait(&self, job: &str, sha: &str) {
        self.insert_job_source(job, sha, "feature", "pull_request")
            .await;
        self.set_job_source(job, "evil", Some(57), Some("evil/app"))
            .await;
        let changed = sqlx::query(
            "UPDATE jobs SET status = 'awaiting_approval', approval = 'waiting'
             WHERE id = $1::uuid AND status = 'pending' AND agent_id IS NULL",
        )
        .bind(job)
        .execute(&self.pool)
        .await
        .expect("make a job wait for an approval");
        assert_eq!(changed.rows_affected(), 1, "a pending job with no agent");
    }
}
