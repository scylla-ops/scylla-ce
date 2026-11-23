//! The tests of the access to the library: the built-in roles of the workspace, an instance
//! admin, the author of a fragment and an unknown fragment.

use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{env_node, literal_parameter, text, Role, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, UNKNOWN_ID};

/// Each caller gets the routes of its rights on a fragment of another actor: an owner, an admin
/// and an instance admin each route, a member the reads and the creation, a viewer the reads,
/// and another user none.
#[sqlx::test(migrations = false)]
async fn each_role_gets_its_library_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let author = app.user().await;
    let (owner, admin, member) = (app.user().await, app.user().await, app.user().await);
    let (viewer, stranger, instance_admin) =
        (app.user().await, app.user().await, app.admin().await);
    let workspace = app.workspace_of(author).await.to_string();
    let in_workspace = |name| Role::on("workspace", &workspace, name);
    let grants = [
        (Role::owner_of(&workspace), owner),
        (in_workspace("admin"), admin),
        (in_workspace("member"), member),
        (in_workspace("viewer"), viewer),
    ];
    for (role, user) in grants {
        app.grant_role(author, role, &user.to_string())
            .await
            .unwrap();
    }

    // when
    let mut answers = Vec::new();
    for caller in [owner, admin, member, viewer, stranger, instance_admin] {
        answers.push(routes(&app, author, caller, &workspace).await);
    }

    // then
    let expected = [
        [true; 6],
        [true; 6],
        [true, true, true, true, false, false],
        [true, true, true, false, false, false],
        [false; 6],
        [true; 6],
    ];
    assert_eq!(answers, expected);
}

/// Calls each route as `caller` on a new fragment of `author`, and returns which ones pass: list,
/// get, favorite, create, publish a version, delete.
async fn routes(app: &TestApp, author: ActorId, caller: ActorId, workspace: &str) -> [bool; 6] {
    let name = format!(
        "f{}",
        caller
            .to_string()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "")
            .to_lowercase()
    );
    let fragment = text(
        &app.create_fragment(author, workspace, &name)
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let mine = format!("{name}-mine");
    let node = || vec![env_node("build", &[], &[("MODE", "debug")])];
    let passes = |code: Option<Code>| match code {
        None => true,
        Some(Code::PermissionDenied) => false,
        Some(other) => panic!("unexpected code {other:?}"),
    };
    [
        passes(
            app.list_fragments(caller, workspace)
                .await
                .err()
                .map(|s| s.code()),
        ),
        passes(
            app.get_fragment(caller, &fragment)
                .await
                .err()
                .map(|s| s.code()),
        ),
        passes(
            app.set_fragment_favorite(caller, &fragment, true)
                .await
                .err()
                .map(|s| s.code()),
        ),
        passes(
            app.create_fragment(caller, workspace, &mine)
                .await
                .err()
                .map(|s| s.code()),
        ),
        passes(
            app.publish_fragment_version(
                caller,
                &fragment,
                node(),
                vec![literal_parameter("MODE", "")],
                &[],
            )
            .await
            .err()
            .map(|s| s.code()),
        ),
        passes(
            app.delete_fragment(caller, &fragment)
                .await
                .err()
                .map(|s| s.code()),
        ),
    ]
}

/// A member versions and deletes its own fragment, and not the fragment of another member.
#[sqlx::test(migrations = false)]
async fn the_author_versions_and_deletes_its_fragment(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (author, other) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    for user in [author, other] {
        let member = Role::on("workspace", &workspace, "member");
        app.grant_role(owner, member, &user.to_string())
            .await
            .unwrap();
    }
    let fragment = text(
        &app.create_fragment(author, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    let nodes = || vec![env_node("build", &[], &[("MODE", "debug")])];

    // when
    let by_other = app
        .publish_fragment_version(other, &fragment, nodes(), Vec::new(), &[])
        .await;
    let by_author = app
        .publish_fragment_version(author, &fragment, nodes(), Vec::new(), &[])
        .await;
    let delete_by_other = app.delete_fragment(other, &fragment).await;
    let delete_by_author = app.delete_fragment(author, &fragment).await;

    // then
    assert_code(by_other, Code::PermissionDenied);
    assert_eq!(by_author.unwrap().versions.len(), 2);
    assert_code(delete_by_other, Code::PermissionDenied);
    assert!(delete_by_author.is_ok());
}

/// An author that left the workspace has no route on its fragment any more.
#[sqlx::test(migrations = false)]
async fn an_author_that_left_loses_its_fragment(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let author = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let member = || Role::on("workspace", &workspace, "member");
    app.grant_role(owner, member(), &author.to_string())
        .await
        .unwrap();
    let fragment = text(
        &app.create_fragment(author, &workspace, "build")
            .await
            .unwrap()
            .id,
    )
    .to_owned();
    app.revoke_role(owner, member(), &author.to_string())
        .await
        .unwrap();

    // when
    let result = app.delete_fragment(author, &fragment).await;

    // then
    assert_code(result, Code::PermissionDenied);
    assert!(app.get_fragment(owner, &fragment).await.is_ok());
}

/// An unknown fragment gets the answer of a refusal, also for an instance admin.
#[sqlx::test(migrations = false)]
async fn an_unknown_fragment_is_a_refusal(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let read = app.get_fragment(admin, UNKNOWN_ID).await;
    let delete = app.delete_fragment(admin, UNKNOWN_ID).await;

    // then
    assert_code(read, Code::PermissionDenied);
    assert_code(delete, Code::PermissionDenied);
}
