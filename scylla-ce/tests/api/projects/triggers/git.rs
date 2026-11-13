//! The tests of the push, pull request and tag triggers of an editor pipeline: their rules, the
//! linked repository that they need, and the `on` table of the export of their pipeline.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_proto::pipelines::v1::create_trigger_request::Kind;
use scylla_proto::pipelines::v1::update_trigger_request::Rules;
use scylla_proto::pipelines::v1::{
    trigger, PipelineFileFormat, PullRequest, PullRequestEvent, Push, Tag,
};
use scylla_testkit::{text, LinkRow, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use super::pipeline;
use crate::{app, assert_code, assert_invalid_field};

/// Links the project to `octo/app` through an installation of the GitHub App.
async fn link(app: &TestApp, owner: ActorId, project: &str) {
    let workspace = app.workspace_of(owner).await;
    let connection = app
        .insert_github_connection(workspace, owner, 42, "octo")
        .await;
    let link = LinkRow {
        project,
        connection: &connection,
        external_id: "7",
        full_name: "octo/app",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
}

/// Returns a list of texts.
fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// Returns the rule of a push to `main` and `release/*`, with no `wip/*`.
fn push() -> Push {
    Push {
        branches: texts(&["main", "release/*"]),
        exclude_branches: texts(&["wip/*"]),
        skip_ci: true,
    }
}

/// Returns the rule of a pull request into `main`, when it opens or gets new commits.
fn pull_request() -> PullRequest {
    PullRequest {
        branches: texts(&["main"]),
        events: vec![
            PullRequestEvent::Opened.into(),
            PullRequestEvent::Synchronized.into(),
        ],
        drafts: false,
    }
}

/// Checks that a call fails with `FAILED_PRECONDITION` and a reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}

/// A linked project gets push, pull request and tag triggers with their rules. They have no
/// schedule, the list shows them, and the export of the pipeline puts them in its `on` table.
#[sqlx::test(migrations = false)]
async fn create_trigger_makes_the_git_triggers_of_a_linked_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = pipeline(&app).await;
    link(&app, owner, &project).await;
    let tag = Tag {
        patterns: texts(&["v*"]),
    };

    // when
    let pushed = app
        .create_git_trigger(owner, &pipeline, "push", Kind::Push(push()))
        .await
        .unwrap();
    let pull = app
        .create_git_trigger(owner, &pipeline, "pr", Kind::PullRequest(pull_request()))
        .await
        .unwrap();
    let tagged = app
        .create_git_trigger(owner, &pipeline, "tag", Kind::Tag(tag.clone()))
        .await
        .unwrap();
    let read = app.get_trigger(owner, text(&pull.id)).await.unwrap();
    let listed = app.list_pipeline_triggers(owner, &pipeline).await.unwrap();
    let exported = app
        .export_pipeline(owner, &pipeline, PipelineFileFormat::Toml)
        .await
        .unwrap();

    // then
    assert_eq!(pushed.kind, Some(trigger::Kind::Push(push())));
    assert_eq!(read.kind, Some(trigger::Kind::PullRequest(pull_request())));
    assert_eq!(tagged.kind, Some(trigger::Kind::Tag(tag)));
    assert!(pushed.enabled && pushed.next_fire_at.is_none() && !pushed.from_file);
    let names: Vec<_> = listed.triggers.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["push", "pr", "tag"]);
    let content = exported.content;
    assert!(content.contains("[on.push]"), "{content}");
    assert!(content.contains(r#"exclude = ["wip/*"]"#), "{content}");
    assert!(content.contains("[on.pull_request]"), "{content}");
    assert!(content.contains(r#""new_commits""#), "{content}");
    assert!(content.contains("[on.tag]"), "{content}");
}

/// A Git trigger of a project with no linked repository gives `NO_REPOSITORY_LINK`, and one of a
/// file pipeline gives `PIPELINE_READ_ONLY`. A schedule needs no link.
#[sqlx::test(migrations = false)]
async fn create_trigger_needs_a_link_and_an_editor_pipeline(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = pipeline(&app).await;
    let unlinked = app
        .create_git_trigger(owner, &pipeline, "push", Kind::Push(push()))
        .await;
    link(&app, owner, &project).await;
    sqlx::query(
        "UPDATE pipelines SET source = 'file', file_key = 'build',
                              file_path = '.scylla/pipelines/build.toml'
         WHERE id = $1::uuid",
    )
    .bind(&pipeline)
    .execute(&app.pool)
    .await
    .unwrap();

    // when
    let from_file = app
        .create_git_trigger(owner, &pipeline, "push", Kind::Push(push()))
        .await;
    let cron = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 3 * * *"), Vec::new())
        .await;

    // then
    assert_reason(unlinked, "NO_REPOSITORY_LINK");
    assert_reason(from_file, "PIPELINE_READ_ONLY");
    assert!(cron.is_ok());
    let listed = app.list_pipeline_triggers(owner, &pipeline).await.unwrap();
    assert_eq!(listed.triggers.len(), 1);
}

/// A bad glob, more than 32 globs in a list, or an unknown event of a pull request gives
/// `INVALID_ARGUMENT` on its field, and makes no trigger.
#[sqlx::test(migrations = false)]
async fn create_trigger_checks_the_rules(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = pipeline(&app).await;
    link(&app, owner, &project).await;
    let bad_glob = Push {
        exclude_branches: texts(&["wip/*", "release/["]),
        ..push()
    };
    let too_many = Tag {
        patterns: (0..33).map(|index| format!("v{index}.*")).collect(),
    };
    let unknown_event = PullRequest {
        events: vec![PullRequestEvent::Unspecified.into()],
        ..pull_request()
    };

    // when
    let glob = app
        .create_git_trigger(owner, &pipeline, "push", Kind::Push(bad_glob))
        .await;
    let many = app
        .create_git_trigger(owner, &pipeline, "tag", Kind::Tag(too_many))
        .await;
    let event = app
        .create_git_trigger(owner, &pipeline, "pr", Kind::PullRequest(unknown_event))
        .await;

    // then
    assert_invalid_field(&glob.unwrap_err(), "push.exclude_branches[1]");
    assert_invalid_field(&many.unwrap_err(), "tag.patterns");
    assert_invalid_field(&event.unwrap_err(), "pull_request.events[0]");
    let listed = app.list_pipeline_triggers(owner, &pipeline).await.unwrap();
    assert!(listed.triggers.is_empty());
}

/// New rules of the kind of a Git trigger replace its rules. Rules of another kind, rules of a
/// cron trigger, a schedule of a Git trigger and a bad glob give `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn update_trigger_replaces_the_rules_of_a_git_trigger(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = pipeline(&app).await;
    link(&app, owner, &project).await;
    let created = app
        .create_git_trigger(owner, &pipeline, "push", Kind::Push(push()))
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let cron = app
        .create_trigger(owner, &pipeline, "nightly", Some("0 3 * * *"), Vec::new())
        .await
        .unwrap();
    let cron = text(&cron.trigger.unwrap().id).to_owned();
    let develop = Push {
        branches: texts(&["develop"]),
        exclude_branches: Vec::new(),
        skip_ci: false,
    };
    let bad = Push {
        branches: texts(&[" "]),
        ..push()
    };

    // when
    let updated = app
        .update_trigger_rules(owner, &id, Rules::Push(develop.clone()))
        .await
        .unwrap();
    let other_kind = app
        .update_trigger_rules(owner, &id, Rules::Tag(Tag::default()))
        .await;
    let on_cron = app
        .update_trigger_rules(owner, &cron, Rules::Push(push()))
        .await;
    let schedule = app
        .update_trigger(owner, &id, Some("0 3 * * *"), None)
        .await;
    let bad_glob = app.update_trigger_rules(owner, &id, Rules::Push(bad)).await;

    // then
    assert_eq!(updated.kind, Some(trigger::Kind::Push(develop.clone())));
    assert_invalid_field(&other_kind.unwrap_err(), "rules");
    assert_invalid_field(&on_cron.unwrap_err(), "rules");
    assert_invalid_field(&schedule.unwrap_err(), "schedule");
    assert_invalid_field(&bad_glob.unwrap_err(), "push.branches[0]");
    let read = app.get_trigger(owner, &id).await.unwrap();
    assert_eq!(read.kind, Some(trigger::Kind::Push(develop)));
}

/// A viewer of the project cannot make a Git trigger.
#[sqlx::test(migrations = false)]
async fn create_trigger_of_a_git_kind_needs_the_editor_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = pipeline(&app).await;
    link(&app, owner, &project).await;
    let viewer = app.user().await;
    let project_id = ProjectId::parse(&project).unwrap();
    app.grant(&[Project::VIEWER.of(project_id, viewer)]).await;

    // when
    let created = app
        .create_git_trigger(viewer, &pipeline, "push", Kind::Push(push()))
        .await;

    // then
    assert_code(created, Code::PermissionDenied);
}
