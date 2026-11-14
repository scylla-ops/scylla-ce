//! The tests of the check and the export of the pipeline files, and of the JSON Schema of the
//! files.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{Project, ProjectId};
use scylla_proto::pipelines::v1::{
    env_var, node, Container, EnvVar, Node, PipelineFileFormat, Script, Shell,
};
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A file with a node that names a secret.
const WITH_SECRET: &str = "version = 1\n\n[[nodes]]\nid = \"deploy\"\nexec = [\"make\"]\nenv = { TOKEN = { secret = \"deploy-token\" } }\n";

/// Makes a node with each kind of value: a script for bash, a literal and a secret, a container,
/// a timeout, retries and a work folder.
fn full_node(node_id: &str, needs: &[&str]) -> Node {
    Node {
        step: Some(node::Step::Script(Script {
            shell: Shell::Bash.into(),
            source: "set -x\necho \"$TARGET\" # yes: no\n".to_owned(),
        })),
        env: vec![
            EnvVar {
                name: "TARGET".to_owned(),
                value: Some(env_var::Value::Literal("no".to_owned())),
            },
            EnvVar {
                name: "TOKEN".to_owned(),
                value: Some(env_var::Value::Secret("deploy-token".to_owned())),
            },
        ],
        timeout_seconds: 900,
        retries: 2,
        working_dir: "app/src".to_owned(),
        container: Some(Container {
            image: "rust:1.97".to_owned(),
        }),
        ..exec_node(node_id, needs)
    }
}

/// Makes a project with the secret `deploy-token` and a pipeline, and returns the owner, the
/// project and the pipeline.
async fn project_with_pipeline(app: &TestApp) -> (ActorId, String, String) {
    let owner = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    app.create_secret(owner, &project, "deploy-token", "s3cret")
        .await
        .unwrap();
    let nodes = vec![exec_node("build", &[]), full_node("deploy", &["build"])];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let id = text(&pipeline.id).to_owned();
    app.update_pipeline(owner, &id, pipeline.version, Some("Build & Deploy!"), None)
        .await
        .unwrap();
    (owner, project, id)
}

/// An editor pipeline exported in each format gives a file of the folder of the pipelines that
/// validates with no error and no warning, with the same name and nodes.
#[sqlx::test(migrations = false)]
async fn export_pipeline_gives_a_file_that_validates(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = project_with_pipeline(&app).await;
    let saved = app.get_pipeline(owner, &pipeline).await.unwrap();
    let formats = [
        (PipelineFileFormat::Toml, "toml"),
        (PipelineFileFormat::Yaml, "yaml"),
        (PipelineFileFormat::Json, "json"),
    ];

    for (format, extension) in formats {
        // when
        let exported = app.export_pipeline(owner, &pipeline, format).await.unwrap();
        let file = app
            .validate_pipeline_file(owner, &project, &exported.path, &exported.content)
            .await
            .unwrap();

        // then
        assert_eq!(
            exported.path,
            format!(".scylla/pipelines/build-deploy.{extension}")
        );
        assert!(file.errors.is_empty(), "{format:?}: {:?}", file.errors);
        assert_eq!(file.key, "build-deploy");
        assert_eq!(file.format, format as i32);
        assert_eq!(file.name, saved.name);
        assert_eq!(file.nodes, saved.nodes, "{format:?}");
        assert_eq!(file.file.unwrap().path, exported.path);
        assert!(file.pipeline_id.is_none());
    }
}

/// An error has the path of the file, its line and its column, and the file has no nodes.
#[sqlx::test(migrations = false)]
async fn validate_pipeline_file_gives_each_error_with_its_place(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, _) = project_with_pipeline(&app).await;
    let path = ".scylla/pipelines/build.yaml";
    let text = "version: 1\nnodes:\n  - id: build\n    exec: [make]\n    timeout: 60\n";

    // when
    let file = app
        .validate_pipeline_file(owner, &project, path, text)
        .await
        .unwrap();

    // then
    assert_eq!(file.errors.len(), 1, "{:?}", file.errors);
    let error = &file.errors[0];
    assert_eq!(
        (error.path.as_str(), error.line, error.column),
        (path, 5, 5)
    );
    assert!(error.message.contains("timeout"), "{}", error.message);
    assert!(!error.warning);
    assert!(file.nodes.is_empty());
    assert_eq!((file.key.as_str(), file.name.as_str()), ("build", "build"));
}

/// A secret that the project does not have is a warning at its value: the file keeps its nodes.
/// A secret of the project gives no warning.
#[sqlx::test(migrations = false)]
async fn validate_pipeline_file_warns_about_a_missing_secret(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, _) = project_with_pipeline(&app).await;
    let other = app.create_project(owner, "beta").await.unwrap();
    let path = ".scylla/pipeline.toml";

    // when
    let missing = app
        .validate_pipeline_file(owner, text(&other.id), path, WITH_SECRET)
        .await
        .unwrap();
    let known = app
        .validate_pipeline_file(owner, &project, path, WITH_SECRET)
        .await
        .unwrap();

    // then
    assert_eq!(missing.errors.len(), 1, "{:?}", missing.errors);
    let warning = &missing.errors[0];
    assert!(warning.warning);
    assert_eq!(
        (warning.line, warning.field.as_str()),
        (6, "nodes[0].env[0].value")
    );
    assert!(
        warning.message.contains("deploy-token"),
        "{}",
        warning.message
    );
    assert_eq!(missing.nodes.len(), 1);
    assert_eq!(missing.key, "pipeline");
    assert!(known.errors.is_empty(), "{:?}", known.errors);
}

/// A path that is not the path of a pipeline file, a key that is not valid and a text larger
/// than 1 MiB give `INVALID_ARGUMENT`. An export with no format too.
#[sqlx::test(migrations = false)]
async fn the_pipeline_files_refuse_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, project, pipeline) = project_with_pipeline(&app).await;
    let large = format!("version = 1\n#{}", "x".repeat(1 << 20));

    // when
    let outside = app
        .validate_pipeline_file(owner, &project, "build.toml", "version = 1")
        .await;
    let bad_key = app
        .validate_pipeline_file(owner, &project, ".scylla/pipelines/Build.toml", "")
        .await;
    let too_large = app
        .validate_pipeline_file(owner, &project, ".scylla/pipeline.toml", &large)
        .await;
    let no_format = app
        .export_pipeline(owner, &pipeline, PipelineFileFormat::Unspecified)
        .await;

    // then
    assert_invalid_field(&outside.unwrap_err(), "path");
    assert_invalid_field(&bad_key.unwrap_err(), "path");
    assert_invalid_field(&too_large.unwrap_err(), "content");
    assert_invalid_field(&no_format.unwrap_err(), "format");
}

/// A viewer of the project validates a file and exports a pipeline. Another user and an unknown
/// pipeline get `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn a_project_viewer_validates_and_exports(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, project, pipeline) = project_with_pipeline(&app).await;
    let (viewer, stranger) = (app.user().await, app.user().await);
    let id = ProjectId::parse(&project).unwrap();
    app.grant(&[Project::VIEWER.of(id, viewer)]).await;
    let path = ".scylla/pipeline.toml";
    let toml = PipelineFileFormat::Toml;

    // when
    let validated = app
        .validate_pipeline_file(viewer, &project, path, WITH_SECRET)
        .await;
    let exported = app.export_pipeline(viewer, &pipeline, toml).await;
    let refused = app
        .validate_pipeline_file(stranger, &project, path, WITH_SECRET)
        .await;
    let hidden = app.export_pipeline(stranger, &pipeline, toml).await;
    let unknown = app.export_pipeline(viewer, UNKNOWN_ID, toml).await;

    // then
    assert!(validated.unwrap().errors.is_empty());
    assert!(exported.unwrap().content.starts_with("version = 1\n"));
    assert_code(refused, Code::PermissionDenied);
    assert_code(hidden, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// The JSON Schema of the files is public, with its URL on the API as its id.
#[sqlx::test(migrations = false)]
async fn the_schema_of_the_files_is_public(pool: PgPool) {
    // given
    let app = app(pool).await;
    let url = format!("http://{}/schemas/pipeline-v1.json", app.addr);

    // when
    let response = reqwest::get(&url).await.unwrap();
    let status = response.status().as_u16();
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let schema: serde_json::Value = response.json().await.unwrap();

    // then
    assert_eq!(status, 200);
    assert_eq!(kind, "application/schema+json");
    assert_eq!(schema["$id"], url.as_str());
    assert_eq!(schema["title"], "Scylla pipeline file");
    assert!(schema["properties"]["nodes"].is_object());
    assert!(schema["properties"]["on"].is_object());
}
