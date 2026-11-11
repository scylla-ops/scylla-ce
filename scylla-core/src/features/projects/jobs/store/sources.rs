//! The SQL of the `job_sources` table: the commit of a job in the linked repository, and the event
//! that started the job.

use std::collections::HashMap;

use scylla_domain::integrations::sources::{JobSource, PullRequestRef, SourceEvent};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_extension::Tx;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

/// A pull request in JSON.
#[derive(Serialize, Deserialize)]
struct PullJson {
    /// The number.
    number: u32,
    /// The title.
    title: String,
    /// The page on the provider.
    url: String,
    /// The source branch.
    source_branch: String,
    /// The target branch.
    target_branch: String,
}

/// A row of the `job_sources` table.
struct Row {
    /// The job.
    job_id: uuid::Uuid,
    /// The provider.
    provider: String,
    /// The full name of the repository.
    repository: String,
    /// The page of the repository.
    url: String,
    /// The commit.
    sha: String,
    /// The full Git ref.
    git_ref: Option<String>,
    /// The branch.
    branch: Option<String>,
    /// The tag.
    tag: Option<String>,
    /// The pull requests in JSON.
    pull_requests: Value,
    /// The full name of the fork.
    fork_repository: Option<String>,
    /// The login of the author.
    author_login: Option<String>,
    /// The avatar of the author.
    avatar: Option<String>,
    /// The title of the commit.
    message_title: String,
    /// The event.
    event: String,
    /// `true` for an automatic event.
    automatic: bool,
    /// `true` when the pull request changes a pipeline file.
    changes_pipeline_files: bool,
}

/// Inserts the source of a job.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    job: JobId,
    pipeline: PipelineId,
    source: &JobSource,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO job_sources (job_id, pipeline_id, provider, repository, url, sha, ref, branch,
                                  tag, pull_requests, from_fork, fork_repository, author_login,
                                  avatar, message_title, event, automatic, changes_pipeline_files)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        job.to_db(),
        pipeline.to_db(),
        source.provider.as_str(),
        source.repository,
        source.url,
        source.sha,
        source.git_ref,
        source.branch,
        source.tag,
        pulls(&source.pull_requests),
        source.from_fork(),
        source.fork_repository,
        source.author_login,
        source.avatar,
        source.message_title,
        source.event.as_str(),
        source.automatic,
        source.changes_pipeline_files,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the source of a job, or `None` for a job with no source.
pub(crate) async fn find(tx: &mut Tx<'_>, job: JobId) -> Result<Option<JobSource>, Error> {
    Ok(of_jobs(tx, &[job]).await?.remove(&job))
}

/// Returns the sources of jobs, by job. A job with no source is not in the map.
pub(crate) async fn of_jobs(
    tx: &mut Tx<'_>,
    jobs: &[JobId],
) -> Result<HashMap<JobId, JobSource>, Error> {
    let ids: Vec<_> = jobs.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT job_id, provider, repository, url, sha, ref AS git_ref, branch, tag,
                  pull_requests, fork_repository, author_login, avatar, message_title, event,
                  automatic, changes_pipeline_files
           FROM job_sources WHERE job_id = ANY($1)"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| Ok((JobId::from_db(row.job_id), row.into_source()?)))
        .collect()
}

impl Row {
    /// Makes the source of the row.
    fn into_source(self) -> Result<JobSource, Error> {
        let pulls: Vec<PullJson> = serde_json::from_value(self.pull_requests).map_err(bad)?;
        Ok(JobSource {
            provider: VcsProvider::parse(&self.provider).ok_or_else(|| bad("provider"))?,
            repository: self.repository,
            url: self.url,
            sha: self.sha,
            git_ref: self.git_ref,
            branch: self.branch,
            tag: self.tag,
            pull_requests: pulls
                .into_iter()
                .map(|pull| PullRequestRef {
                    number: pull.number,
                    title: pull.title,
                    url: pull.url,
                    source_branch: pull.source_branch,
                    target_branch: pull.target_branch,
                })
                .collect(),
            fork_repository: self.fork_repository,
            author_login: self.author_login,
            avatar: self.avatar,
            message_title: self.message_title,
            event: SourceEvent::parse(&self.event).ok_or_else(|| bad("event"))?,
            automatic: self.automatic,
            changes_pipeline_files: self.changes_pipeline_files,
        })
    }
}

/// Returns the JSON of the pull requests of a source.
fn pulls(pulls: &[PullRequestRef]) -> Value {
    let items: Vec<PullJson> = pulls
        .iter()
        .map(|pull| PullJson {
            number: pull.number,
            title: pull.title.clone(),
            url: pull.url.clone(),
            source_branch: pull.source_branch.clone(),
            target_branch: pull.target_branch.clone(),
        })
        .collect();
    serde_json::to_value(items).unwrap_or_else(|_| Value::Array(Vec::new()))
}

/// The error of bad data of a source in the database.
fn bad(error: impl std::fmt::Display) -> Error {
    Error::Internal(format!("bad job source in the database: {error}"))
}
