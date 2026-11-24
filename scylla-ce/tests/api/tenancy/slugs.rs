//! The tests of the slugs of the workspaces: the form, one workspace for each slug, the slug
//! that a name or a handle gives, and `GetWorkspace` with a slug.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field};

/// A workspace with no slug gets the slug of its name. A name that a workspace uses already
/// gets the suffix `-2`, then `-3`.
#[sqlx::test(migrations = false)]
async fn create_workspace_derives_a_slug_from_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);

    // when
    let first = app.create_workspace(alice, "Équipe Été!").await.unwrap();
    let second = app.create_workspace(bob, "equipe ete").await.unwrap();
    let third = app.create_workspace(alice, "Equipe-Ete").await.unwrap();
    let symbols = app.create_workspace(alice, "!!!").await.unwrap();

    // then
    let slugs = [first.slug, second.slug, third.slug, symbols.slug];
    assert_eq!(
        slugs,
        ["equipe-ete", "equipe-ete-2", "equipe-ete-3", "workspace"]
    );
}

/// A chosen slug is trimmed and in lower case. A chosen slug in use gives `SLUG_IN_USE`, and a
/// slug from a name in use gets a suffix.
#[sqlx::test(migrations = false)]
async fn create_workspace_takes_a_chosen_slug(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);

    // when
    let chosen = app
        .create_workspace_with(alice, "One", Some(" Acme "))
        .await;
    let taken = app.create_workspace_with(bob, "Two", Some("acme")).await;
    let derived = app.create_workspace(bob, "ACME").await;

    // then
    assert_eq!(chosen.unwrap().slug, "acme");
    assert_refused(taken, "SLUG_IN_USE");
    assert_eq!(derived.unwrap().slug, "acme-2");
}

/// A slug has 2 to 39 lower case letters, digits or dashes, and starts and ends with a letter
/// or a digit.
#[sqlx::test(migrations = false)]
async fn create_workspace_checks_the_slug(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (max, too_long) = ("x".repeat(39), "x".repeat(40));

    // when
    let shortest = app.create_workspace_with(user, "a", Some("ab")).await;
    let longest = app.create_workspace_with(user, "b", Some(&max)).await;
    let mut refused = Vec::new();
    for slug in ["a", &too_long, "-ab", "ab-", "a_b", "a.b", "a b", ""] {
        refused.push(app.create_workspace_with(user, "c", Some(slug)).await);
    }

    // then
    assert!(shortest.is_ok() && longest.is_ok());
    for result in refused {
        assert_invalid_field(&result.unwrap_err(), "slug");
    }
}

/// The personal workspace of a new user takes the handle as its slug, and with no handle, the
/// slug of the display name, with a suffix when it is in use.
#[sqlx::test(migrations = false)]
async fn the_personal_workspace_takes_the_handle(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (alice, bob) = (app.user().await, app.user().await);

    // when
    let created = app
        .create_user(admin, "ada@example.com", "ada.l", PASSWORD)
        .await
        .unwrap();
    let ada = ActorId::parse(text(&created.id)).unwrap();
    let slugs = [
        slug_of(&app, ada).await,
        slug_of(&app, alice).await,
        slug_of(&app, bob).await,
    ];

    // then
    assert_eq!(slugs[0], "ada-l");
    assert!(slugs[1].starts_with("test-user"), "{slugs:?}");
    assert!(slugs[2].starts_with("test-user"), "{slugs:?}");
    assert_ne!(slugs[1], slugs[2]);
}

/// Returns the slug of the personal workspace of a user.
async fn slug_of(app: &TestApp, user: ActorId) -> String {
    let id = app.workspace_of(user).await.to_string();
    app.get_workspace(user, &id).await.unwrap().slug
}

/// `GetWorkspace` finds a workspace by its slug, in any case, for a reader. Another user and an
/// unknown slug get the same refusal, and a request with no id and no slug is a request error.
#[sqlx::test(migrations = false)]
async fn get_workspace_by_slug_needs_a_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let team = app.create_workspace(owner, "Team").await.unwrap();
    let id = WorkspaceId::parse(text(&team.id)).unwrap();
    app.grant(&[Workspace::VIEWER.of(id, viewer)]).await;

    // when
    let by_owner = app.get_workspace_by_slug(owner, "TEAM").await;
    let by_viewer = app.get_workspace_by_slug(viewer, "team").await;
    let by_stranger = app.get_workspace_by_slug(stranger, "team").await;
    let unknown = app.get_workspace_by_slug(owner, "nothing-here").await;
    let bad = app.get_workspace_by_slug(owner, "-bad").await;

    // then
    assert_eq!(by_owner.unwrap().id, team.id);
    assert_eq!(by_viewer.unwrap().slug, "team");
    let (stranger, unknown) = (by_stranger.unwrap_err(), unknown.unwrap_err());
    assert_eq!(stranger.code(), Code::PermissionDenied);
    assert_eq!(unknown.code(), Code::PermissionDenied);
    assert!(!stranger.message().contains(text(&team.id)), "{stranger:?}");
    assert_invalid_field(&bad.unwrap_err(), "slug");
}

/// An admin of the workspace changes its name and its slug, and the old slug finds nothing. A
/// slug in use gives `SLUG_IN_USE`. A member and an unknown workspace get a refusal.
#[sqlx::test(migrations = false)]
async fn update_workspace_changes_the_name_and_the_slug(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin, member) = (app.user().await, app.user().await, app.user().await);
    let team = app.create_workspace(owner, "Team").await.unwrap();
    app.create_workspace(owner, "Other").await.unwrap();
    let id = text(&team.id).to_owned();
    let workspace = WorkspaceId::parse(&id).unwrap();
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
    ])
    .await;

    // when
    let changed = app
        .update_workspace(admin, &id, Some(" Core "), Some("Core-Team"))
        .await;
    let old_slug = app.get_workspace_by_slug(owner, "team").await;
    let unchanged = app.update_workspace(owner, &id, None, None).await;
    let in_use = app.update_workspace(owner, &id, None, Some("other")).await;
    let by_member = app.update_workspace(member, &id, Some("Mine"), None).await;
    let unknown = app
        .update_workspace(owner, crate::UNKNOWN_ID, Some("x"), None)
        .await;
    let empty_name = app.update_workspace(owner, &id, Some("  "), None).await;

    // then
    let changed = changed.unwrap();
    assert_eq!(
        (changed.name.as_str(), changed.slug.as_str()),
        ("Core", "core-team")
    );
    assert_code(old_slug, Code::PermissionDenied);
    assert_eq!(unchanged.unwrap().slug, "core-team");
    assert_refused(in_use, "SLUG_IN_USE");
    assert_code(by_member, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&empty_name.unwrap_err(), "name");
}

/// A slug that is a route of the web app or of the server is refused, and a name or a handle
/// that gives one gets a suffix.
#[sqlx::test(migrations = false)]
async fn a_reserved_slug_is_refused(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);

    // when
    let chosen = app.create_workspace_with(user, "x", Some(" Login ")).await;
    let derived = app.create_workspace(user, "Settings").await.unwrap();
    let renamed = app
        .update_workspace(user, text(&derived.id), None, Some("webhooks"))
        .await;
    let created = app
        .create_user(admin, "api@example.com", "api", PASSWORD)
        .await
        .unwrap();
    let personal = slug_of(&app, ActorId::parse(text(&created.id)).unwrap()).await;

    // then
    assert_invalid_field(&chosen.unwrap_err(), "slug");
    assert_eq!(derived.slug, "settings-2");
    assert_invalid_field(&renamed.unwrap_err(), "slug");
    assert_eq!(personal, "api-2");
}
