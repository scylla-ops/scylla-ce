//! The tests of the pipelines: the checks of the nodes, the reads, the change with a version,
//! the roles, the run and the delete.

use scylla_domain::projects::Project;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::pipelines::v1::{env_var, EnvVar, Node};
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Creates a user and a project in its personal workspace. Returns the user and the project id.
async fn project(app: &TestApp) -> (scylla_domain::identity::actors::ActorId, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    (owner, text(&project.id).to_owned())
}

/// Makes a node that reads a secret.
fn with_secret(node_id: &str, secret: &str) -> Node {
    let mut node = exec_node(node_id, &[]);
    node.env = vec![EnvVar {
        name: "TOKEN".to_owned(),
        value: Some(env_var::Value::Secret(secret.to_owned())),
    }];
    node
}

/// A pipeline keeps its nodes, gets the default timeout, and is in the lists of its project and
/// of its workspace.
#[sqlx::test(migrations = false)]
async fn create_pipeline_keeps_its_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let nodes = vec![exec_node("build", &[]), exec_node("test", &["build"])];

    // when
    let created = app.create_pipeline(owner, &project, nodes).await.unwrap();

    // then
    let id = text(&created.id).to_owned();
    let read = app.get_pipeline(owner, &id).await.unwrap();
    assert_eq!(read.version, 1);
    assert_eq!(read.nodes[1].needs, vec!["build".to_owned()]);
    assert_eq!(read.nodes[0].timeout_seconds, 3600);
    let in_project = app
        .list_project_pipelines(owner, &project, 0, "")
        .await
        .unwrap();
    let workspace = app.workspace_of(owner).await;
    let in_workspace = app
        .list_workspace_pipelines(owner, workspace)
        .await
        .unwrap();
    assert_eq!(text(&in_project.pipelines[0].id), id);
    assert_eq!(text(&in_workspace.pipelines[0].id), id);
}

/// A bad graph or a bad node is refused with the path of the value.
#[sqlx::test(migrations = false)]
async fn create_pipeline_checks_the_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let cycle = vec![exec_node("a", &["b"]), exec_node("b", &["a"])];
    let mut reserved = exec_node("a", &[]);
    reserved.env = vec![EnvVar {
        name: "SCYLLA_TOKEN".to_owned(),
        value: Some(env_var::Value::Literal("1".to_owned())),
    }];
    let mut no_step = exec_node("a", &[]);
    no_step.step = None;
    let bad_id = exec_node("Build", &[]);

    // when
    let refusals = [
        app.create_pipeline(owner, &project, cycle).await,
        app.create_pipeline(owner, &project, vec![reserved]).await,
        app.create_pipeline(owner, &project, vec![no_step]).await,
        app.create_pipeline(owner, &project, vec![bad_id]).await,
        app.create_pipeline(owner, &project, Vec::new()).await,
    ];

    // then
    let fields = [
        "nodes",
        "nodes[0].env[0].name",
        "nodes[0].step",
        "nodes[0].id",
        "nodes",
    ];
    for (result, field) in refusals.into_iter().zip(fields) {
        assert_invalid_field(&result.unwrap_err(), field);
    }
}

/// A change needs the version that the client read: another version gives `ABORTED`.
#[sqlx::test(migrations = false)]
async fn update_pipeline_needs_the_current_version(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let created = app
        .create_pipeline(owner, &project, vec![exec_node("build", &[])])
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let nodes = vec![exec_node("lint", &[]), exec_node("build", &["lint"])];

    // when
    let changed = app
        .update_pipeline(owner, &id, 1, Some("ci"), Some(nodes))
        .await
        .unwrap();
    let stale = app.update_pipeline(owner, &id, 1, Some("old"), None).await;
    let renamed = app
        .update_pipeline(owner, &id, 2, Some("ci 2"), None)
        .await
        .unwrap();

    // then
    assert_eq!((changed.version, changed.nodes.len()), (2, 2));
    assert_code(stale, Code::Aborted);
    assert_eq!((renamed.name.as_str(), renamed.version), ("ci 2", 3));
    assert_eq!(renamed.nodes.len(), 2);
}

/// The members of the workspace and the editors of the project write and run the pipelines. A
/// viewer reads them. Another user and an unknown pipeline get `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn the_pipeline_routes_follow_the_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let (member, editor, viewer, stranger) = (
        app.user().await,
        app.user().await,
        app.user().await,
        app.user().await,
    );
    let workspace = app.workspace_of(owner).await;
    let project_id = scylla_domain::projects::ProjectId::parse(&project).unwrap();
    app.grant(&[
        Workspace::MEMBER.of(workspace, member),
        Project::EDITOR.of(project_id, editor),
        Project::VIEWER.of(project_id, viewer),
    ])
    .await;
    let nodes = || vec![exec_node("build", &[])];
    let id = text(
        &app.create_pipeline(member, &project, nodes())
            .await
            .unwrap()
            .id,
    )
    .to_owned();

    // when
    let by_editor = app.create_pipeline(editor, &project, nodes()).await;
    let by_viewer = app.create_pipeline(viewer, &project, nodes()).await;
    let read = app.get_pipeline(viewer, &id).await;
    let run = app.run_pipeline(viewer, &id).await;
    let hidden = app.get_pipeline(stranger, &id).await;
    let unknown = app.get_pipeline(owner, UNKNOWN_ID).await;
    let listed = app.list_workspace_pipelines(stranger, workspace).await;

    // then
    assert!(by_editor.is_ok() && read.is_ok());
    for refused in [by_viewer.map(|_| ()), run.map(|_| ()), hidden.map(|_| ())] {
        assert_code(refused, Code::PermissionDenied);
    }
    assert_code(unknown, Code::PermissionDenied);
    assert_code(listed, Code::PermissionDenied);
}

/// A run needs each secret that the nodes name. The job keeps the nodes of the run: a later
/// change of the pipeline does not change them.
#[sqlx::test(migrations = false)]
async fn run_pipeline_keeps_a_snapshot(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let created = app
        .create_pipeline(owner, &project, vec![with_secret("deploy", "API_TOKEN")])
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let missing = app.run_pipeline(owner, &id).await;
    app.create_secret(owner, &project, "API_TOKEN", "s3cr3t")
        .await
        .unwrap();

    // when
    let job = app.run_pipeline(owner, &id).await.unwrap();
    app.update_pipeline(owner, &id, 1, None, Some(vec![exec_node("other", &[])]))
        .await
        .unwrap();

    // then
    assert_code(missing, Code::NotFound);
    let read = app.get_job(owner, text(&job.id)).await.unwrap();
    assert_eq!(read.nodes[0].id, "deploy");
    assert_eq!(
        read.status,
        scylla_proto::pipelines::v1::JobStatus::Pending as i32
    );
    assert_eq!(read.runs[0].node_id, "deploy");
}

/// A deleted pipeline goes with its jobs.
#[sqlx::test(migrations = false)]
async fn delete_pipeline_deletes_its_jobs(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project) = project(&app).await;
    let created = app
        .create_pipeline(owner, &project, vec![exec_node("build", &[])])
        .await
        .unwrap();
    let id = text(&created.id).to_owned();
    let job = app.run_pipeline(owner, &id).await.unwrap();

    // when
    app.delete_pipeline(owner, &id).await.unwrap();

    // then
    assert_code(app.get_pipeline(owner, &id).await, Code::PermissionDenied);
    assert_code(
        app.get_job(owner, text(&job.id)).await,
        Code::PermissionDenied,
    );
}
