//! A check run on GitHub: `POST /repos/{owner}/{repo}/check-runs` one time, then `PATCH` of the
//! check run with each new state, with an installation token narrowed to the repository and to
//! `checks: write`. The external id of a check run is the id of its job, so a re-run that a person
//! asks for on GitHub finds the job.

use reqwest::Method;
use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::ProjectId;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

use super::failures::Failure;
use super::output::{self, Output, Tail};
use super::sender::Sent;
use super::store::checks;
use super::store::queue::{Claimed, Facts};
use super::{github, states};
use crate::features::integrations::IntegrationContext;
use crate::features::projects::jobs;
use crate::Error;

/// The answer of a create or a change of a check run.
#[derive(Deserialize)]
struct Answer {
    /// The id of the check run.
    id: u64,
    /// The page of the check run.
    html_url: Option<String>,
}

/// Returns the output of the check run of a report: from its job, and from the last log lines
/// of the node that failed.
pub(super) async fn output(
    ctx: &IntegrationContext,
    claimed: &Claimed,
    facts: &Facts,
    link: &str,
) -> Result<Output, Error> {
    // Read the job.
    let job = jobs::get(&ctx.pool, claimed.job).await?;
    let job = job.ok_or(Error::NotFound("job"))?;

    // Read the last lines of the node that failed.
    let failed = output::failed_node(&job).filter(|_| claimed.status == ReportedStatus::Failed);
    let tail = match failed {
        Some(node) => {
            let mut tx = ctx.pool.begin().await?;
            let lines = checks::tail(&mut tx, job.id, node, output::TAIL_LINES).await?;
            tx.commit().await?;
            Some(Tail {
                node: node.to_owned(),
                lines,
            })
        }
        None => None,
    };
    let name = &facts.pipeline_name;
    Ok(output::of(name, claimed.status, &job, link, tail.as_ref()))
}

/// Creates the check run of a report, or changes the check run that the provider gave.
pub(super) async fn send(
    ctx: &IntegrationContext,
    claimed: &Claimed,
    facts: &Facts,
    target_url: &str,
    output: &Output,
) -> Result<Sent, Failure> {
    // Get a token of the repository for the check runs.
    let token = github::token(ctx, facts, "checks").await?;

    // Make the body: the name, the link, the job, the state and the output.
    let (status, conclusion) = states::check_run(claimed.status);
    let mut body = json!({
        "name": claimed.context,
        "details_url": target_url,
        "external_id": claimed.job.to_string(),
        "status": status,
        "output": { "title": output.title, "summary": output.summary },
    });
    if let Some(conclusion) = conclusion {
        body["conclusion"] = json!(conclusion);
    }
    if let Some(text) = &output.text {
        body["output"]["text"] = json!(text);
    }

    // Create the check run one time, then change it.
    let repository = &facts.repository;
    let call = match &claimed.external_id {
        Some(id) => {
            let path = format!("/repos/{repository}/check-runs/{id}");
            ctx.clients.github.call(Method::PATCH, &path)
        }
        None => {
            body["head_sha"] = json!(facts.sha);
            let path = format!("/repos/{repository}/check-runs");
            ctx.clients.github.call(Method::POST, &path)
        }
    };
    let answer: Answer = token.read(call.json(body)).await?;
    let page = format!("{}/commit/{}", facts.url, facts.sha);
    Ok(Sent {
        state: conclusion.unwrap_or(status).to_owned(),
        external_id: Some(answer.id.to_string()),
        external_url: answer.html_url.unwrap_or(page),
    })
}

/// Returns the jobs of a project to run again for a commit, as a person asked on GitHub: the job
/// of one check run, or the newest job of each check run of the commit. A job that did not end is
/// left out.
pub(crate) async fn reruns(
    pool: &PgPool,
    project: ProjectId,
    sha: &str,
    check: Option<JobId>,
) -> Result<Vec<JobId>, Error> {
    let mut tx = pool.begin().await?;
    let jobs = checks::reruns(&mut tx, project, sha, check).await?;
    tx.commit().await?;
    Ok(jobs)
}
