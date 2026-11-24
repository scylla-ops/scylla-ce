//! The tests of the job lists with the filters of the source: the branch, the event, the author
//! and the pull request.

use scylla_proto::pipelines::v1::{Job, JobFilter, SourceEvent};
use scylla_testkit::{exec_node, text};
use sqlx::PgPool;

use super::linked;
use crate::assert_invalid_field;

/// Returns the ids of jobs, in their order.
fn ids(jobs: &[Job]) -> Vec<&str> {
    jobs.iter().map(|job| text(&job.id)).collect()
}

/// A filter keeps the jobs whose source matches each value that is set, newest first. A job with
/// no source matches no filter, and a bad branch gives `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn list_jobs_filters_on_the_source(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let linked = linked(&app, "github", vec![exec_node("build", &[])]).await;
    let (owner, pipeline) = (linked.owner, linked.pipeline.as_str());
    let mut jobs = Vec::new();
    for (sha, branch, event, author, pull) in [
        (
            "1111111111111111111111111111111111111111",
            "main",
            "push",
            "octocat",
            12,
        ),
        (
            "2222222222222222222222222222222222222222",
            "feature",
            "pull_request",
            "hubot",
            13,
        ),
        (
            "3333333333333333333333333333333333333333",
            "release",
            "tag",
            "octocat",
            0,
        ),
    ] {
        let job = app.run_pipeline(owner, pipeline).await.unwrap();
        let job = text(&job.id).to_owned();
        app.insert_job_source(&job, sha, branch, event).await;
        let pull = (pull > 0).then_some(pull);
        app.set_job_source(&job, author, pull, None).await;
        jobs.push(job);
    }
    let plain = app.run_pipeline(owner, pipeline).await.unwrap();
    let (push, pull, tag) = (jobs[0].as_str(), jobs[1].as_str(), jobs[2].as_str());
    let list = |filter: JobFilter| app.list_pipeline_jobs_with(owner, pipeline, filter);
    let branch = |name: &str| JobFilter {
        branch: name.to_owned(),
        ..JobFilter::default()
    };

    // when
    let by_branch = list(branch("main")).await.unwrap();
    let by_event = list(JobFilter {
        event: SourceEvent::PullRequest.into(),
        ..JobFilter::default()
    });
    let by_event = by_event.await.unwrap();
    let by_author = list(JobFilter {
        author_login: "octocat".to_owned(),
        ..JobFilter::default()
    });
    let by_author = by_author.await.unwrap();
    let by_pull = list(JobFilter {
        pull_request: 13,
        ..JobFilter::default()
    });
    let by_pull = by_pull.await.unwrap();
    let all = list(JobFilter::default()).await.unwrap();
    let none = list(branch("nothing")).await.unwrap();
    let in_workspace = app.list_workspace_jobs_with(owner, linked.workspace, branch("main"));
    let in_workspace = in_workspace.await.unwrap();
    let bad = list(branch("a b")).await;

    // then
    assert_eq!(ids(&by_branch), vec![push]);
    assert_eq!(ids(&by_event), vec![pull]);
    assert_eq!(ids(&by_author), vec![tag, push]);
    assert_eq!(ids(&by_pull), vec![pull]);
    assert_eq!(ids(&all), vec![text(&plain.id), tag, pull, push]);
    assert!(none.is_empty());
    assert_eq!(ids(&in_workspace), vec![push]);
    assert_invalid_field(&bad.unwrap_err(), "filter.branch");
}
