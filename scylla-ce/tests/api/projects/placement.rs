//! The tests of the conditional jobs on the streams of the agents: an agent of v0.7.0-rc.1 gets
//! the needs as node ids, and no job with a switch, a merge or an `error` need.

use std::time::Duration;

use scylla_domain::projects::pipelines::PipelineHolder;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::pipelines::v1::{JobStatus, Need};
use scylla_testkit::{exec_node, text};
use sqlx::PgPool;

use super::runs::{connect, pipeline};
use crate::app;

/// The dispatch gives each need as a node id in `needs`, and with its port in `links`. A job
/// with an `error` need stays pending for an agent with no `conditional_nodes`, and an agent with
/// the flag takes it. A plain job goes to both agents.
#[sqlx::test(migrations = false)]
async fn open_stream_gives_a_conditional_job_to_a_new_agent_only(pool: PgPool) {
    // given
    let app = app(pool).await;
    let plain = vec![exec_node("build", &[]), exec_node("test", &["build"])];
    let (owner, workspace, project, plain_id) = pipeline(&app, plain).await;
    let conditional = vec![exec_node("test", &[]), exec_node("notify", &["test:error"])];
    let created = app.create_pipeline(owner, &project, conditional).await;
    let conditional_id = text(&created.unwrap().id).to_owned();
    let (old_key, old_token) = app.app_in(owner, workspace, true).await;
    let (new_key, new_token) = app.app_in(owner, workspace, true).await;
    app.grant(&[
        PipelineHolder::RUNNER.of(workspace, old_key.actor()),
        PipelineHolder::RUNNER.of(workspace, new_key.actor()),
    ])
    .await;
    let mut old = connect(&app, old_key, &old_token, 1, "", false).await;

    // when
    let waiting = app.run_pipeline(owner, &conditional_id).await.unwrap();
    let nothing = old.next(Duration::from_secs(1)).await;
    let first = app.run_pipeline(owner, &plain_id).await.unwrap();
    let to_old = old.dispatch().await;
    let pending = app.get_job(owner, text(&waiting.id)).await.unwrap();
    let mut new = connect(&app, new_key, &new_token, 2, "", true).await;
    let to_new = new.dispatch().await;
    let second = app.run_pipeline(owner, &plain_id).await.unwrap();
    let plain_to_new = new.dispatch().await;

    // then
    assert!(!matches!(nothing, Some(Down::Dispatch(_))));
    assert_eq!(
        (pending.status(), pending.agent_id),
        (JobStatus::Pending, None)
    );
    assert_eq!(to_old.job_id, first.id);
    assert_eq!(to_old.nodes[1].needs, ["build"]);
    let build = Need {
        node: "build".to_owned(),
        port: String::new(),
    };
    assert_eq!(to_old.nodes[1].links, [build]);
    assert_eq!(to_new.job_id, waiting.id);
    assert_eq!(to_new.nodes[1].needs, ["test"]);
    assert_eq!(to_new.nodes[1].links[0].port, "error");
    assert_eq!(plain_to_new.job_id, second.id);
}
