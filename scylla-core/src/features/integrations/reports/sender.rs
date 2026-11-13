//! The sender of the reports: a task that wakes each 2 seconds or after a retry, takes the due
//! reports, and posts the latest state of each one to its provider, as a check run or a commit
//! status.

use std::time::Duration;

use scylla_domain::integrations::links::ReportMode;
use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::integrations::VcsProvider;
use time::OffsetDateTime;
use tokio::task::{JoinHandle, JoinSet};

use super::failures::{Failure, Next};
use super::store::queue::{self, Claimed, Facts};
use super::{checks, github, gitlab, health, states};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The time between two passes.
const TICK: Duration = Duration::from_secs(2);

/// The most reports of one pass.
const BATCH: i64 = 20;

/// The time that a pass holds a report while it calls the provider, in seconds.
const LEASE_SECONDS: f64 = 120.0;

/// The failed calls after which a report stops.
pub(super) const MAX_ATTEMPTS: i32 = 10;

/// What a call posts.
pub(super) struct Outgoing {
    /// The state on the provider.
    pub(super) state: &'static str,
    /// The name of the report.
    pub(super) context: String,
    /// The short text of the report.
    pub(super) description: String,
    /// The page of the run in Scylla.
    pub(super) target_url: String,
}

/// What the provider got.
pub(super) struct Sent {
    /// The state on the provider.
    pub(super) state: String,
    /// The id that the provider gave.
    pub(super) external_id: Option<String>,
    /// The page of the report on the provider.
    pub(super) external_url: String,
}

/// Starts the task.
pub(super) fn spawn(ctx: IntegrationContext) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = ctx.wake.notified() => {}
            }
            if let Err(error) = pass(&ctx).await {
                tracing::warn!(%error, "the reports wait");
            }
        }
    })
}

/// Runs one pass: the health of the connections, then each due report.
async fn pass(ctx: &IntegrationContext) -> Result<(), Error> {
    // Check the tokens that expire.
    health::check(ctx).await?;

    // Stop the reports that cannot go out, and take the due reports.
    let mut tx = ctx.pool.begin().await?;
    queue::stop_inactive(&mut tx).await?;
    let claimed = queue::claim(&mut tx, BATCH, LEASE_SECONDS).await?;
    tx.commit().await?;

    // Send each report.
    let mut calls = JoinSet::new();
    for claimed in claimed {
        let ctx = ctx.clone();
        calls.spawn(async move { send(&ctx, claimed).await });
    }
    while let Some(done) = calls.join_next().await {
        match done {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(%error, "a report waits"),
            Err(error) => tracing::warn!(%error, "a call of a report stopped"),
        }
    }
    Ok(())
}

/// Sends the latest state of one report, and keeps the result.
async fn send(ctx: &IntegrationContext, claimed: Claimed) -> Result<(), Error> {
    // Read what the call needs.
    let mut tx = ctx.pool.begin().await?;
    let facts = queue::facts(&mut tx, &claimed).await?;
    tx.commit().await?;
    let Some(facts) = facts else {
        let failure = Failure::Server("the job, its commit or its link is gone".to_owned());
        return keep(ctx, &claimed, VcsProvider::GitHub, None, &failure).await;
    };
    let provider = VcsProvider::parse(&facts.provider)
        .ok_or_else(|| Error::Internal("unknown provider of a connection".to_owned()))?;

    // Post the state: a check run or a commit status on GitHub, a commit status on GitLab.
    let outgoing = outgoing(ctx, provider, &claimed, &facts);
    let result = match (provider, claimed.mode) {
        (VcsProvider::GitHub, ReportMode::CheckRun) => {
            let output = checks::output(ctx, &claimed, &facts, &outgoing.target_url).await?;
            checks::send(ctx, &claimed, &facts, &outgoing, &output).await
        }
        (VcsProvider::GitHub, ReportMode::CommitStatus) => {
            github::send(ctx, &facts, &outgoing).await
        }
        (VcsProvider::GitLab, _) => {
            let pipeline = claimed.external_id.as_deref();
            gitlab::send(ctx, claimed.connection, &facts, &outgoing, pipeline).await
        }
    };

    // Keep what the provider got, or the failure.
    match result {
        Ok(sent) => {
            let mut tx = ctx.pool.begin().await?;
            let id = sent.external_id.as_deref();
            queue::sent(&mut tx, &claimed, &sent.state, id, Some(&sent.external_url)).await?;
            tx.commit().await?;
            Ok(())
        }
        Err(failure) => keep(ctx, &claimed, provider, Some(&facts), &failure).await,
    }
}

/// Keeps a failed call: a later call, a connection that needs a person, or a report that stops.
async fn keep(
    ctx: &IntegrationContext,
    claimed: &Claimed,
    provider: VcsProvider,
    facts: Option<&Facts>,
    failure: &Failure,
) -> Result<(), Error> {
    // Find what comes next.
    let now = OffsetDateTime::now_utc();
    let expired = facts
        .and_then(|facts| facts.token_expires_at)
        .is_some_and(|at| at <= now);
    let next = failure.next(provider, claimed.attempts, expired, now);
    let error = failure.text(provider);

    // Keep it, and mark the connection when the provider refused the credentials.
    let mut tx = ctx.pool.begin().await?;
    let (stopped, broken) = match next {
        Next::Retry(at) => {
            let stopped = queue::retry(&mut tx, claimed, at, &error, MAX_ATTEMPTS).await?;
            (stopped, None)
        }
        Next::Stop => (queue::stop(&mut tx, claimed, &error).await?, None),
        Next::Broken(reason) => {
            let stopped = queue::stop(&mut tx, claimed, &error).await?;
            let broken = health::broken(&mut tx, claimed.connection, reason).await?;
            (stopped, broken)
        }
    };
    tx.commit().await?;

    // Log the facts for the operators.
    if let Some(broken) = broken {
        health::log_broken(&broken);
    }
    if stopped {
        tracing::warn!(
            event = "vcs.report_failed",
            job = %claimed.job,
            context = claimed.context,
            connection = %claimed.connection,
            attempts = claimed.attempts + 1,
            error,
        );
    }
    Ok(())
}

/// Makes what a call posts: the state of the provider, the description and the page of the run.
fn outgoing(
    ctx: &IntegrationContext,
    provider: VcsProvider,
    claimed: &Claimed,
    facts: &Facts,
) -> Outgoing {
    let run = states::Run {
        started_at: facts.started_at,
        finished_at: facts.finished_at,
        failed_node: facts.failed_node.as_deref(),
        file_path: facts.file_path.as_deref(),
    };
    let status: ReportedStatus = claimed.status;
    let target_url = format!(
        "{}/{}/projects/{}/pipelines/{}/jobs/{}",
        ctx.urls.app_url, facts.slug, facts.project, facts.pipeline, claimed.job
    );
    Outgoing {
        state: match provider {
            VcsProvider::GitHub => states::github(status),
            VcsProvider::GitLab => states::gitlab(status),
        },
        context: claimed.context.clone(),
        description: states::description(provider, status, &run),
        target_url,
    }
}
