//! The tests of the contract of the Git integration while the server does not serve it: the RPCs
//! of the pipeline files with no link, and the events of a checkout.

use std::time::Duration;

use scylla_proto::agents::v1::EventKind;
use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::pipeline_service_client::PipelineServiceClient;
use scylla_proto::pipelines::v1::{
    GetFilePipelineRequest, JobStatus, ListPipelineFilesRequest, SetPipelineSourceRequest,
};
use scylla_testkit::{exec_node, id, text};
use sqlx::PgPool;
use tonic::Code;

use super::runs::{agent, pipeline};
use crate::{app, assert_code};

/// With no linked repository, the RPCs that read the pipeline files and move to the files give
/// `FAILED_PRECONDITION`, and a pipeline comes from the editor, with the default checkout.
#[sqlx::test(migrations = false)]
async fn the_rpcs_of_the_pipeline_files_need_a_link(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, project, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut client = PipelineServiceClient::new(app.channel.clone());
    let get = GetFilePipelineRequest {
        pipeline_id: Some(id(&pipeline)),
        r#ref: None,
    };
    let list = ListPipelineFilesRequest {
        project_id: Some(id(&project)),
        r#ref: None,
    };
    let source = SetPipelineSourceRequest {
        project_id: Some(id(&project)),
        source: PipelineSource::File.into(),
    };

    // when
    let got = client.get_file_pipeline(app.authed(owner, get).await).await;
    let listed = client
        .list_pipeline_files(app.authed(owner, list).await)
        .await;
    let moved = client
        .set_pipeline_source(app.authed(owner, source).await)
        .await;

    // then
    assert_code(got, Code::FailedPrecondition);
    assert_code(listed, Code::FailedPrecondition);
    assert_code(moved, Code::FailedPrecondition);
    let read = app.get_pipeline(owner, &pipeline).await.unwrap();
    assert_eq!(read.source, PipelineSource::Editor as i32);
    assert!(read.file_key.is_empty() && read.archived_at.is_none());
    let checkout = read.checkout.expect("the default checkout");
    assert!(checkout.enabled && checkout.depth == 1 && !checkout.submodules && !checkout.lfs);
}

/// A job with no source gets no checkout, and the events of a checkout change nothing: the job
/// runs on.
#[sqlx::test(migrations = false)]
async fn the_events_of_a_checkout_change_nothing(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut agent = agent(&app, owner, workspace, true, 1, "").await;
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let job = text(&job.id).to_owned();
    let dispatch = agent.dispatch().await;

    // when
    agent.event(&job, EventKind::CheckoutStarted, "").await;
    agent.event(&job, EventKind::CheckoutFailed, "").await;
    agent.event(&job, EventKind::JobStarted, "").await;
    let mut read = app.get_job(owner, &job).await.unwrap();
    for _ in 0..50 {
        if read.status != JobStatus::Pending as i32 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        read = app.get_job(owner, &job).await.unwrap();
    }

    // then
    assert!(dispatch.checkout.is_none());
    assert_eq!(read.status, JobStatus::Running as i32);
    assert!(read.error.is_empty() && read.checkout.is_none());
}
