//! The tests of the deletion of an account: its rows, its empty workspaces, its ownerships and
//! its roles go, its actor stays, and the last owner of a workspace with content stays.

use scylla_domain::authz::Object;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::Gate;
use scylla_testkit::{text, Role, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field};

/// Signs in a user with a password, and returns its access token.
async fn token_of(app: &TestApp, email: &str) -> String {
    app.sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value
}

/// Returns `true` if the workspace row exists.
async fn exists(app: &TestApp, workspace: WorkspaceId) -> bool {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = $1) AS "found!""#,
        workspace.to_db()
    )
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Returns the ids in the metadata `workspace_ids` of a `LAST_OWNER` refusal, sorted.
fn blocked(status: &Status) -> Vec<String> {
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(info.reason, "LAST_OWNER");
    let mut ids: Vec<String> = info.metadata["workspace_ids"]
        .split(',')
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids
}

/// A deletion needs the current password. It removes the user and its empty personal
/// workspace, stops its tokens, and keeps its actor.
#[sqlx::test(migrations = false)]
async fn delete_account_removes_the_user_and_its_empty_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "ada@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let personal = app.workspace_of(user).await;
    let token = token_of(&app, email).await;

    // when
    let wrong = app.delete_account(&token, "a wrong password").await;
    app.delete_account(&token, PASSWORD).await.unwrap();

    // then
    assert_invalid_field(&wrong.unwrap_err(), "password");
    assert!(!exists(&app, personal).await);
    assert_code(app.get_me(&token).await, Code::Unauthenticated);
    assert_code(
        app.get_me(&app.token(user).await).await,
        Code::Unauthenticated,
    );
    assert_code(app.sign_in(email, PASSWORD).await, Code::Unauthenticated);
    let actor = sqlx::query_scalar!("SELECT kind FROM actors WHERE id = $1", user.to_db())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(actor, "user");
}

/// A workspace where the user is the only owner stops the deletion if it has a project or a
/// role of another actor. The refusal names these workspaces, and nothing changes.
#[sqlx::test(migrations = false)]
async fn delete_account_keeps_the_last_owner_of_a_full_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "ben@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let other = app.user().await;
    let personal = app.workspace_of(user).await;
    app.create_project(user, "alpha").await.unwrap();
    let shared = app.create_workspace(user, "shared").await.unwrap();
    let shared = WorkspaceId::parse(text(&shared.id)).unwrap();
    app.grant(&[Workspace::VIEWER.of(shared, other)]).await;
    let empty = app.create_workspace(user, "empty").await.unwrap();
    let empty = WorkspaceId::parse(text(&empty.id)).unwrap();

    // when
    let refused = app
        .delete_account(&token_of(&app, email).await, PASSWORD)
        .await;

    // then
    let mut expected = vec![personal.to_string(), shared.to_string()];
    expected.sort();
    assert_eq!(blocked(&refused.unwrap_err()), expected);
    assert!(exists(&app, empty).await);
    assert!(app.sign_in(email, PASSWORD).await.is_ok());
}

/// A workspace with another owner keeps its content and its other owner: the user leaves it.
/// The roles of the user go, and its projects keep their author.
#[sqlx::test(migrations = false)]
async fn delete_account_leaves_a_shared_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "cal@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let owner = app.user().await;
    let team = app.workspace_of(owner).await;
    let team_id = team.to_string();
    app.grant_role(owner, Role::owner_of(&team_id), &user.to_string())
        .await
        .unwrap();
    let project = app.create_project_in(user, team, "beta").await.unwrap();
    let other = app.workspace_of(app.user().await).await;
    app.grant(&[Workspace::VIEWER.of(other, user)]).await;

    // when
    app.delete_account(&token_of(&app, email).await, PASSWORD)
        .await
        .unwrap();

    // then
    assert_eq!(app.owner_count(team).await, 1);
    let kept = app.get_project(owner, text(&project.id)).await.unwrap();
    assert_eq!(text(&kept.created_by), user.to_string());
    let holders = Gate::subjects(app.access.as_ref(), &Object::of(other)).await;
    assert!(!holders.unwrap().contains(&user));
}

/// The last active instance admin cannot delete its account.
#[sqlx::test(migrations = false)]
async fn delete_account_keeps_the_last_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let token = app.token(admin).await;

    // when
    let refused = app.delete_account(&token, "").await;

    // then
    assert_refused(refused, "LAST_ADMIN");
    assert!(app.get_me(&token).await.is_ok());
}

/// Waits until a statement of the test database waits for a lock.
async fn wait_for_a_lock(app: &TestApp) {
    for _ in 0..500 {
        let waiting = sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM pg_stat_activity
               WHERE datname = current_database() AND wait_event_type = 'Lock'"#
        )
        .fetch_one(&app.pool)
        .await
        .unwrap();
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no statement waits for a lock");
}

/// A user that leaves a workspace while its deletion waits for the lock of the workspace is
/// not its last owner: the workspace of the other owner stays.
#[sqlx::test(migrations = false)]
async fn delete_user_keeps_a_workspace_that_the_user_left(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user, owner) = (app.admin().await, app.user().await, app.user().await);
    let team = app.create_workspace(owner, "team").await.unwrap();
    let team = WorkspaceId::parse(text(&team.id)).unwrap();
    let team_id = team.to_string();
    app.grant_role(owner, Role::owner_of(&team_id), &user.to_string())
        .await
        .unwrap();
    let mut removal = app.pool.begin().await.unwrap();
    sqlx::query!(
        "SELECT id FROM workspaces WHERE id = $1 FOR UPDATE",
        team.to_db()
    )
    .fetch_one(&mut *removal)
    .await
    .unwrap();
    sqlx::query!(
        "DELETE FROM workspace_owners WHERE workspace_id = $1 AND user_id = $2",
        team.to_db(),
        user.to_db(),
    )
    .execute(&mut *removal)
    .await
    .unwrap();

    // when
    let target = user.to_string();
    let (deleted, ()) = tokio::join!(app.delete_user(admin, &target), async {
        wait_for_a_lock(&app).await;
        removal.commit().await.unwrap();
    });

    // then
    assert!(deleted.is_ok(), "{deleted:?}");
    assert!(exists(&app, team).await);
    assert_eq!(app.owner_count(team).await, 1);
}
