//! The `TestApp` helpers of the reports of the runs on the commits: the two routes of
//! `VcsReportService`, the waits for the sender, and the rows of a report or a token that a test
//! changes.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::vcs_report_service_client::VcsReportServiceClient;
use scylla_proto::integrations::v1::{
    JobReports, ListJobReportsRequest, ListJobReportsResponse, ReportState, RetryJobReportRequest,
    VcsReport,
};
use tonic::Status;

use super::{FakeProvider, Recorded};
use crate::{id, TestApp};

/// The longest wait for the sender, longer than its passes of 2 seconds.
const WAIT: Duration = Duration::from_secs(15);

/// The time between two reads of a wait.
const POLL: Duration = Duration::from_millis(100);

impl TestApp {
    /// Calls `ListJobReports` as `actor`.
    pub async fn list_job_reports(
        &self,
        actor: ActorId,
        jobs: &[&str],
    ) -> Result<ListJobReportsResponse, Status> {
        let request = ListJobReportsRequest {
            job_ids: jobs.iter().map(|job| id(*job)).collect(),
        };
        let request = self.authed(actor, request).await;
        let mut client = VcsReportServiceClient::new(self.channel.clone());
        Ok(client.list_job_reports(request).await?.into_inner())
    }

    /// Calls `RetryJobReport` as `actor`, and returns the report.
    pub async fn retry_job_report(
        &self,
        actor: ActorId,
        job: &str,
        context: &str,
    ) -> Result<VcsReport, Status> {
        let request = RetryJobReportRequest {
            job_id: Some(id(job)),
            context: context.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let mut client = VcsReportServiceClient::new(self.channel.clone());
        let response = client.retry_job_report(request).await?.into_inner();
        Ok(response.report.expect("a report"))
    }

    /// Returns the reports of one job, as `actor` reads them.
    pub async fn job_reports(&self, actor: ActorId, job: &str) -> JobReports {
        let response = self.list_job_reports(actor, &[job]).await;
        let mut jobs = response.expect("the reports of a job").jobs;
        assert_eq!(jobs.len(), 1, "one job");
        jobs.remove(0)
    }

    /// Waits until the first report of a job is in a delivery state, and returns it. Fails after
    /// 15 seconds.
    pub async fn wait_for_report(
        &self,
        actor: ActorId,
        job: &str,
        state: ReportState,
    ) -> VcsReport {
        let start = tokio::time::Instant::now();
        loop {
            let reports = self.job_reports(actor, job).await.reports;
            if let Some(report) = reports.into_iter().find(|report| report.state() == state) {
                return report;
            }
            assert!(start.elapsed() < WAIT, "no report of {job} is {state:?}");
            tokio::time::sleep(POLL).await;
        }
    }

    /// Sets the failed calls of each report of a job, and makes it due at once.
    pub async fn set_report_attempts(&self, job: &str, attempts: i32) {
        sqlx::query(
            "UPDATE vcs_reports SET attempts = $2, next_attempt_at = now() WHERE job_id = $1::uuid",
        )
        .bind(job)
        .bind(attempts)
        .execute(&self.pool)
        .await
        .expect("set the attempts of a report");
    }

    /// Sets the expiry of the token of a connection to now and a number of days, which can be
    /// less than zero.
    pub async fn set_token_expiry(&self, connection: &str, days: i32) {
        sqlx::query(
            "UPDATE vcs_connections
             SET token_expires_at = now() + make_interval(days => $2), token_warned_at = NULL
             WHERE id = $1::uuid",
        )
        .bind(connection)
        .bind(days)
        .execute(&self.pool)
        .await
        .expect("set the expiry of a token");
    }

    /// Turns the reports of the link of a project on or off.
    pub async fn set_link_reports(&self, project: &str, on: bool) {
        sqlx::query("UPDATE repository_links SET reports_enabled = $2 WHERE project_id = $1::uuid")
            .bind(project)
            .bind(on)
            .execute(&self.pool)
            .await
            .expect("turn the reports of a link on or off");
    }
}

/// Waits until a fake provider got `count` requests to a method and a path, and returns them.
/// Fails after 15 seconds.
pub async fn wait_for_requests(
    fake: &FakeProvider,
    method: &str,
    path: &str,
    count: usize,
) -> Vec<Recorded> {
    let start = tokio::time::Instant::now();
    loop {
        let requests = fake.requests_to(method, path);
        if requests.len() >= count {
            return requests;
        }
        assert!(
            start.elapsed() < WAIT,
            "{} of {count} requests to {method} {path}",
            requests.len()
        );
        tokio::time::sleep(POLL).await;
    }
}
