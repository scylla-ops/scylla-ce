//! The tests of `FragmentService`: the creation, the versions, the reads, the favorites, the
//! usages and the delete of the fragments of the library, and the bad values.

use scylla_proto::library::v1::{FragmentParameter, ParameterKind};
use scylla_testkit::{
    env_node, exec_node, fragment_link, fragment_request, literal_parameter, text, TestApp,
};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code, assert_invalid_field};

/// Checks that `status` is a refusal of the library with its code.
fn assert_refused<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(info.reason, reason);
}

/// Makes a user with a project in its workspace, and returns the user, the workspace and the
/// project as text.
async fn owner_with_project(
    app: &TestApp,
) -> (scylla_domain::identity::actors::ActorId, String, String) {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "web").await.unwrap();
    (owner, workspace, text(&project.id).to_owned())
}

/// A new fragment has its version 1 with its nodes and its parameters, the caller as its author
/// with its display name, and no favorite and no usage.
#[sqlx::test(migrations = false)]
async fn create_fragment_makes_version_one(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let created = app
        .create_fragment(owner, &workspace, "build")
        .await
        .unwrap();
    let read = app.get_fragment(owner, text(&created.id)).await.unwrap();

    // then
    assert_eq!(created, read);
    assert_eq!(
        (created.name.as_str(), created.description.as_str()),
        ("build", "Builds the code")
    );
    assert_eq!(created.tags, ["build"]);
    let author = created.author.unwrap();
    assert_eq!(text(&author.id), owner.to_string());
    assert!(!author.name.is_empty());
    assert_eq!(created.versions.len(), 1);
    let version = &created.versions[0];
    assert_eq!(version.number, 1);
    assert_eq!(version.nodes[0].id, "build");
    assert_eq!(version.parameters, [literal_parameter("MODE", "release")]);
    assert!(!created.favorite && created.usages.is_empty());
}

/// A name that the workspace has is refused with `FRAGMENT_NAME_IN_USE`, and another workspace
/// can use it.
#[sqlx::test(migrations = false)]
async fn create_fragment_refuses_a_name_in_use(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (first, second) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(first).await.to_string();
    let other = app.workspace_of(second).await.to_string();
    app.create_fragment(first, &workspace, "build")
        .await
        .unwrap();

    // when
    let again = app.create_fragment(first, &workspace, "build").await;
    let elsewhere = app.create_fragment(second, &other, "build").await;

    // then
    assert_refused(again, "FRAGMENT_NAME_IN_USE");
    assert!(elsewhere.is_ok());
}

/// Each bad value is a request error with its path.
#[sqlx::test(migrations = false)]
async fn create_fragment_refuses_bad_values(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let base = fragment_request(&workspace, "build");
    let secret_with_default = FragmentParameter {
        kind: ParameterKind::Secret.into(),
        ..literal_parameter("MODE", "x")
    };
    let no_kind = FragmentParameter {
        kind: ParameterKind::Unspecified.into(),
        ..literal_parameter("MODE", "x")
    };
    let cases = [
        ("name", {
            let mut r = base.clone();
            r.name = "Build It".to_owned();
            r
        }),
        ("tags[1]", {
            let mut r = base.clone();
            r.tags = vec!["ci".to_owned(), " ci".to_owned()];
            r
        }),
        ("nodes", {
            let mut r = base.clone();
            r.nodes.clear();
            r.parameters.clear();
            r
        }),
        ("parameters[0].key", {
            let mut r = base.clone();
            r.parameters = vec![literal_parameter("OTHER", "")];
            r
        }),
        ("parameters[0].default_value", {
            let mut r = base.clone();
            r.parameters = vec![secret_with_default];
            r
        }),
        ("parameters[0].kind", {
            let mut r = base.clone();
            r.parameters = vec![no_kind];
            r
        }),
        ("description", {
            let mut r = base.clone();
            r.description = "x".repeat(2001);
            r
        }),
    ];

    // when
    let mut fields = Vec::new();
    for (field, request) in cases {
        let status = app.create_fragment_with(owner, request).await.unwrap_err();
        assert_invalid_field(&status, field);
        fields.push(field);
    }

    // then
    assert!(app
        .list_fragments(owner, &workspace)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(fields.len(), 7);
}

/// A new version takes the next number and keeps the older versions. A read of one fragment has
/// the nodes of each version, and a list only the nodes of the latest version.
#[sqlx::test(migrations = false)]
async fn publish_fragment_version_adds_the_next_number(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let nodes = vec![
        env_node("build", &[], &[("MODE", "release")]),
        exec_node("test", &["build"]),
    ];

    // when
    let changed = app
        .publish_fragment_version(
            owner,
            &fragment,
            nodes,
            vec![literal_parameter("MODE", "debug")],
            &[" Adds the tests "],
        )
        .await
        .unwrap();
    let listed = app.list_fragments(owner, &workspace).await.unwrap();

    // then
    let numbers: Vec<_> = changed
        .versions
        .iter()
        .map(|version| version.number)
        .collect();
    assert_eq!(numbers, [1, 2]);
    assert_eq!(changed.versions[1].changes, ["Adds the tests"]);
    assert_eq!(changed.versions[1].nodes.len(), 2);
    assert_eq!(changed.versions[0].nodes.len(), 1);
    assert!(listed[0].versions[0].nodes.is_empty());
    assert_eq!(listed[0].versions[1].nodes.len(), 2);
}

/// A version with a need of a node outside the version is refused, and the fragment keeps its
/// versions.
#[sqlx::test(migrations = false)]
async fn publish_fragment_version_refuses_a_need_outside_the_fragment(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();

    // when
    let result = app
        .publish_fragment_version(
            owner,
            &fragment,
            vec![exec_node("test", &["checkout"])],
            Vec::new(),
            &[],
        )
        .await;

    // then
    assert_invalid_field(&result.unwrap_err(), "nodes[0].needs[0].node");
    let read = app.get_fragment(owner, &fragment).await.unwrap();
    assert_eq!(read.versions.len(), 1);
}

/// A favorite belongs to its reader: another reader of the workspace does not see it.
#[sqlx::test(migrations = false)]
async fn set_fragment_favorite_is_for_the_caller(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let reader = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let viewer = scylla_testkit::Role::on("workspace", &workspace, "viewer");
    app.grant_role(owner, viewer, &reader.to_string())
        .await
        .unwrap();
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();

    // when
    app.set_fragment_favorite(reader, &fragment, true)
        .await
        .unwrap();
    app.set_fragment_favorite(reader, &fragment, true)
        .await
        .unwrap();

    // then
    assert!(app.get_fragment(reader, &fragment).await.unwrap().favorite);
    assert!(!app.get_fragment(owner, &fragment).await.unwrap().favorite);
    app.set_fragment_favorite(reader, &fragment, false)
        .await
        .unwrap();
    assert!(!app.list_fragments(reader, &workspace).await.unwrap()[0].favorite);
}

/// A pipeline that links a fragment is a usage of it, with its version and its values.
#[sqlx::test(migrations = false)]
async fn get_fragment_shows_the_pipelines_that_link_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, project) = owner_with_project(&app).await;
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let links = vec![fragment_link("build-block", &fragment, 1, &["build"])];
    let pipeline = app
        .create_linked_pipeline(owner, &project, vec![exec_node("build", &[])], links)
        .await
        .unwrap();

    // when
    let read = app.get_fragment(owner, &fragment).await.unwrap();

    // then
    assert_eq!(read.usages.len(), 1);
    let usage = &read.usages[0];
    assert_eq!(text(&usage.pipeline_id), text(&pipeline.id));
    assert_eq!(
        (usage.pipeline_name.as_str(), usage.project_name.as_str()),
        ("linked", "web")
    );
    assert_eq!(usage.version, 1);
    assert_eq!(usage.values.get("MODE").map(String::as_str), Some("debug"));
}

/// A delete removes the fragment from the library. The pipeline that links it keeps its nodes
/// and its link.
#[sqlx::test(migrations = false)]
async fn delete_fragment_keeps_the_pipelines_that_link_it(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, project) = owner_with_project(&app).await;
    let fragment = text(
        &app.create_fragment(owner, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let links = vec![fragment_link("build-block", &fragment, 1, &["build"])];
    let pipeline = app
        .create_linked_pipeline(owner, &project, vec![exec_node("build", &[])], links)
        .await
        .unwrap();

    // when
    app.delete_fragment(owner, &fragment).await.unwrap();

    // then
    assert_code(
        app.get_fragment(owner, &fragment).await,
        Code::PermissionDenied,
    );
    assert!(app
        .list_fragments(owner, &workspace)
        .await
        .unwrap()
        .is_empty());
    let kept = app.get_pipeline(owner, text(&pipeline.id)).await.unwrap();
    assert_eq!(kept.nodes.len(), 1);
    assert_eq!(text(&kept.fragment_links[0].fragment_id), fragment);
    let saved = app
        .update_pipeline_links(owner, text(&pipeline.id), kept.version, None, None)
        .await;
    assert!(
        saved.is_ok(),
        "a pipeline with a link of a deleted fragment still saves"
    );
}

/// A list gives its fragments in pages, in id order.
#[sqlx::test(migrations = false)]
async fn list_fragments_gives_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    for name in ["a", "b", "c"] {
        app.create_fragment(owner, &workspace, name).await.unwrap();
    }

    // when
    let first = app
        .list_fragments_page(owner, &workspace, 2, "")
        .await
        .unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = app
        .list_fragments_page(owner, &workspace, 2, &token)
        .await
        .unwrap();

    // then
    assert_eq!(first.fragments.len(), 2);
    assert_eq!(second.fragments.len(), 1);
    assert!(second.page.unwrap().next_token.is_empty());
}
