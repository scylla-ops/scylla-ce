//! The tests of the limits of the API tokens: the presets, a custom list of methods, a list of
//! projects, and the check of the limits of a new token.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::auth::v1::{TokenLimits, TokenPreset};
use scylla_proto::pipelines::v1::pipeline_service_client::PipelineServiceClient;
use scylla_proto::pipelines::v1::RunPipelineRequest;
use scylla_proto::projects::v1::{CreateProjectRequest, GetProjectRequest, ListProjectsRequest};
use scylla_testkit::{exec_node, id, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::{app, assert_code, assert_invalid_field};

/// The calls of a test token: a read of the projects, a read of one project, a new project and
/// a run of a pipeline.
struct Calls<'a> {
    /// The server.
    app: &'a TestApp,
    /// The short token.
    token: String,
}

impl Calls<'_> {
    /// Calls `ListProjects`.
    async fn list(&self) -> Result<(), Status> {
        let request = ListProjectsRequest {
            workspace_id: None,
            page: None,
        };
        let mut client = self.app.projects.clone();
        client
            .list_projects(with_token(&self.token, request))
            .await?;
        Ok(())
    }

    /// Calls `GetProject`.
    async fn get(&self, project: &str) -> Result<(), Status> {
        let request = GetProjectRequest {
            project_id: Some(id(project)),
        };
        let mut client = self.app.projects.clone();
        client.get_project(with_token(&self.token, request)).await?;
        Ok(())
    }

    /// Calls `CreateProject`.
    async fn create(&self, workspace: &str) -> Result<(), Status> {
        let request = CreateProjectRequest {
            workspace_id: Some(id(workspace)),
            name: "beta".to_owned(),
            description: String::new(),
        };
        let mut client = self.app.projects.clone();
        client
            .create_project(with_token(&self.token, request))
            .await?;
        Ok(())
    }

    /// Calls `RunPipeline`.
    async fn run(&self, pipeline: &str) -> Result<(), Status> {
        let request = RunPipelineRequest {
            pipeline_id: Some(id(pipeline)),
        };
        let mut client = PipelineServiceClient::new(self.app.channel.clone());
        client
            .run_pipeline(with_token(&self.token, request))
            .await?;
        Ok(())
    }
}

/// A workspace with an App that is an admin of it, two projects, and a pipeline in the first.
struct Place {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: String,
    /// The App.
    app: String,
    /// The first project.
    first: String,
    /// The second project.
    second: String,
    /// The pipeline of the first project.
    pipeline: String,
}

/// Makes the place of a test.
async fn place(app: &TestApp) -> Place {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    app.grant(&[Workspace::ADMIN.of(workspace, key.actor())])
        .await;
    let first = app.create_project(owner, "alpha").await.unwrap();
    let first = text(&first.id).to_owned();
    let second = app.create_project(owner, "gamma").await.unwrap();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(owner, &first, nodes).await.unwrap();
    Place {
        owner,
        workspace: workspace.to_string(),
        app: key.app,
        first,
        second: text(&second.id).to_owned(),
        pipeline: text(&pipeline.id).to_owned(),
    }
}

/// Creates a token of the App with limits, and returns its calls.
async fn calls<'a>(app: &'a TestApp, place: &Place, limits: Option<TokenLimits>) -> Calls<'a> {
    let created = app.create_limited_app_token(place.owner, &place.app, "ci", 0, limits);
    let secret = created.await.unwrap().secret;
    let token = app.exchange_api_token(&secret).await.unwrap();
    Calls { app, token }
}

/// Makes limits.
fn limits(preset: TokenPreset, actions: &[&str], projects: &[&str]) -> Option<TokenLimits> {
    Some(TokenLimits {
        preset: preset as i32,
        actions: actions.iter().map(|action| (*action).to_owned()).collect(),
        project_ids: projects.iter().map(|project| id(*project)).collect(),
    })
}

/// A token with no limits reads only. A deploy token also runs a pipeline, and a full token
/// does each thing that the roles of its App allow.
#[sqlx::test(migrations = false)]
async fn the_presets_limit_the_methods(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let read = calls(&app, &place, None).await;
    let deploy = calls(&app, &place, limits(TokenPreset::Deploy, &[], &[])).await;
    let full = calls(&app, &place, limits(TokenPreset::Full, &[], &[])).await;

    // when
    let read_calls = (read.list().await, read.create(&place.workspace).await);
    let read_run = read.run(&place.pipeline).await;
    let deploy_calls = (
        deploy.run(&place.pipeline).await,
        deploy.create(&place.workspace).await,
    );
    let full_create = full.create(&place.workspace).await;

    // then
    assert!(read_calls.0.is_ok());
    assert_code(read_calls.1, Code::PermissionDenied);
    assert_code(read_run, Code::PermissionDenied);
    assert!(deploy_calls.0.is_ok());
    assert_code(deploy_calls.1, Code::PermissionDenied);
    assert!(full_create.is_ok());
}

/// A custom token calls its methods only.
#[sqlx::test(migrations = false)]
async fn a_custom_token_calls_its_methods_only(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let get = "scylla.projects.v1.ProjectService/GetProject";
    let custom = calls(&app, &place, limits(TokenPreset::Custom, &[get], &[])).await;

    // when
    let got = custom.get(&place.first).await;
    let listed = custom.list().await;

    // then
    assert!(got.is_ok());
    assert_code(listed, Code::PermissionDenied);
}

/// A token of some projects reaches these projects only, and no route on its caller or on the
/// workspace.
#[sqlx::test(migrations = false)]
async fn a_token_of_some_projects_reaches_them_only(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let first = [place.first.as_str()];
    let limited = calls(&app, &place, limits(TokenPreset::Full, &[], &first)).await;

    // when
    let mine = limited.get(&place.first).await;
    let run = limited.run(&place.pipeline).await;
    let other = limited.get(&place.second).await;
    let listed = limited.list().await;
    let created = limited.create(&place.workspace).await;

    // then
    assert!(mine.is_ok() && run.is_ok());
    assert_code(other, Code::PermissionDenied);
    assert_code(listed, Code::PermissionDenied);
    assert_code(created, Code::PermissionDenied);
}

/// The limits of a new token are checked, and the list shows them.
#[sqlx::test(migrations = false)]
async fn the_limits_of_a_new_token_are_checked(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let run = "scylla.pipelines.v1.PipelineService/RunPipeline";
    let new = |limits| app.create_limited_app_token(place.owner, &place.app, "ci", 0, limits);

    // when
    let no_action = new(limits(TokenPreset::Custom, &[], &[])).await;
    let not_custom = new(limits(TokenPreset::Deploy, &[run], &[])).await;
    let bad_method = new(limits(TokenPreset::Custom, &["RunPipeline"], &[])).await;
    let bad_preset = new(Some(TokenLimits {
        preset: 99,
        ..TokenLimits::default()
    }))
    .await;
    let made = new(limits(TokenPreset::Custom, &[run, run], &[&place.first])).await;

    // then
    assert_invalid_field(&no_action.unwrap_err(), "limits.actions");
    assert_invalid_field(&not_custom.unwrap_err(), "limits.actions");
    assert_invalid_field(&bad_method.unwrap_err(), "limits.actions");
    assert_invalid_field(&bad_preset.unwrap_err(), "limits.preset");
    let shown = made.unwrap().token.unwrap().limits.unwrap();
    assert_eq!(shown.preset, TokenPreset::Custom as i32);
    assert_eq!(shown.actions, vec![run.to_owned()]);
    assert_eq!(shown.project_ids[0].value, place.first);
    let listed = app.list_app_tokens(place.owner, &place.app).await.unwrap();
    assert_eq!(
        listed[0].limits.as_ref().unwrap().actions,
        vec![run.to_owned()]
    );
}
