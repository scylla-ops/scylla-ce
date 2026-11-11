//! The tests of the contract of the Git integration while the server does not serve it: the new
//! RPCs, the Git kinds of trigger, the events of a checkout and the new services.

use std::time::Duration;

use scylla_proto::agents::v1::EventKind;
use scylla_proto::integrations::v1::merge_block_service_client::MergeBlockServiceClient;
use scylla_proto::integrations::v1::vcs_report_service_client::VcsReportServiceClient;
use scylla_proto::integrations::v1::{
    ListJobReportsRequest, ListMergeBlocksRequest, PipelineSource,
};
use scylla_proto::pipelines::v1::job_service_client::JobServiceClient;
use scylla_proto::pipelines::v1::pipeline_service_client::PipelineServiceClient;
use scylla_proto::pipelines::v1::trigger_service_client::TriggerServiceClient;
use scylla_proto::pipelines::v1::{
    create_trigger_request, update_trigger_request, ApproveJobRequest, CreateTriggerRequest,
    GetFilePipelineRequest, JobStatus, ListPipelineFilesRequest, Push, RejectJobRequest,
    SetPipelineSourceRequest, Tag, UpdateTriggerRequest,
};
use scylla_proto::platform::v1::vcs_app_service_client::VcsAppServiceClient;
use scylla_proto::platform::v1::GetVcsAppRequest;
use scylla_testkit::{exec_node, id, text};
use sqlx::PgPool;
use tonic::Code;
use tonic_types::StatusExt;

use super::runs::{agent, pipeline};
use crate::{app, assert_code, assert_invalid_field};

/// `ApproveJob` and `RejectJob` need an authenticated actor, then answer `UNIMPLEMENTED`. The job
/// does not change, and it has no source.
#[sqlx::test(migrations = false)]
async fn approve_job_and_reject_job_are_not_served(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let job = app.run_pipeline(owner, &pipeline).await.unwrap();
    let mut jobs = JobServiceClient::new(app.channel.clone());
    let approve = || ApproveJobRequest {
        job_id: job.id.clone(),
    };
    let reject = RejectJobRequest {
        job_id: job.id.clone(),
    };

    // when
    let anonymous = jobs.approve_job(approve()).await;
    let approved = jobs.approve_job(app.authed(owner, approve()).await).await;
    let rejected = jobs.reject_job(app.authed(owner, reject).await).await;

    // then
    assert_code(anonymous, Code::Unauthenticated);
    assert_code(approved, Code::Unimplemented);
    assert_code(rejected, Code::Unimplemented);
    let read = app.get_job(owner, text(&job.id)).await.unwrap();
    assert_eq!(read.status, JobStatus::Pending as i32);
    assert!(read.source.is_none() && read.approval.is_none() && read.checkout.is_none());
}

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

/// A push, pull request or tag trigger gives `INVALID_ARGUMENT` with the reason
/// `TRIGGER_KIND_NOT_SUPPORTED`. Git rules on a cron trigger give `INVALID_ARGUMENT` on rules.
#[sqlx::test(migrations = false)]
async fn create_trigger_refuses_a_git_kind(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let cron = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 3 * * *"), Vec::new())
        .await
        .unwrap()
        .trigger
        .unwrap();
    let mut client = TriggerServiceClient::new(app.channel.clone());
    let push = CreateTriggerRequest {
        pipeline_id: Some(id(&pipeline)),
        name: "push".to_owned(),
        kind: Some(create_trigger_request::Kind::Push(Push::default())),
        inputs: Vec::new(),
    };
    let rules = UpdateTriggerRequest {
        trigger_id: cron.id.clone(),
        name: None,
        schedule: None,
        inputs: None,
        rules: Some(update_trigger_request::Rules::Tag(Tag::default())),
    };

    // when
    let created = client.create_trigger(app.authed(owner, push).await).await;
    let updated = client.update_trigger(app.authed(owner, rules).await).await;

    // then
    let status = created.unwrap_err();
    assert_invalid_field(&status, "kind");
    let violations = status.get_details_bad_request().unwrap().field_violations;
    assert_eq!(violations[0].description, "TRIGGER_KIND_NOT_SUPPORTED");
    assert_invalid_field(&updated.unwrap_err(), "rules");
    let listed = app.list_pipeline_triggers(owner, &pipeline).await;
    assert_eq!(listed.unwrap().triggers.len(), 1);
    assert!(!cron.from_file);
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

/// The services of the integrations that are not done yet and of the GitHub App answer
/// `UNIMPLEMENTED`.
#[sqlx::test(migrations = false)]
async fn the_services_of_the_integrations_are_not_served(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _, project, _) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let admin = app.admin().await;
    let blocks = ListMergeBlocksRequest {
        project_id: Some(id(&project)),
    };
    let reports = ListJobReportsRequest {
        job_ids: Vec::new(),
    };

    // when
    let blocks = MergeBlockServiceClient::new(app.channel.clone())
        .list_merge_blocks(app.authed(owner, blocks).await)
        .await;
    let reports = VcsReportServiceClient::new(app.channel.clone())
        .list_job_reports(app.authed(owner, reports).await)
        .await;
    let vcs_app = VcsAppServiceClient::new(app.channel.clone())
        .get_vcs_app(app.authed(admin, GetVcsAppRequest {}).await)
        .await;

    // then
    assert_code(blocks, Code::Unimplemented);
    assert_code(reports, Code::Unimplemented);
    assert_code(vcs_app, Code::Unimplemented);
}
