//! The tests of the kinds of the access: the roles, the owners and the instance admins.

use scylla_testkit::{Role, RoleInput};
use sqlx::PgPool;

use super::{kinds_of, row_of};
use crate::app;

/// Returns the list of roles in a parameter.
fn roles(params: &std::collections::HashMap<String, String>, field: &str) -> Vec<String> {
    let text = params.get(field).map_or("[]", String::as_str);
    serde_json::from_str(text).unwrap()
}

/// A person gets one row for the role changes of a short time, with what came and what went. The
/// actor gets no row for the role of another person.
#[sqlx::test(migrations = false)]
async fn role_changes_of_a_short_time_make_one_row(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let user = app.user().await;
    let (id, to) = (workspace.to_string(), user.to_string());

    // when
    app.grant_role(owner, Role::on("workspace", &id, "viewer"), &to)
        .await
        .unwrap();
    let first = app.inbox(user).await;
    app.grant_role(owner, Role::on("workspace", &id, "member"), &to)
        .await
        .unwrap();
    app.revoke_role(owner, Role::on("workspace", &id, "viewer"), &to)
        .await
        .unwrap();
    app.grant_role(owner, Role::on("workspace", &id, "admin"), &to)
        .await
        .unwrap();
    app.revoke_role(owner, Role::on("workspace", &id, "admin"), &to)
        .await
        .unwrap();
    let inbox = app.inbox(user).await;

    // then
    assert_eq!(first.len(), 1);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].id, first[0].id);
    assert_eq!(roles(&inbox[0].params, "added"), ["member"]);
    assert!(roles(&inbox[0].params, "removed").is_empty());
    assert_eq!(inbox[0].actor_name, "Test User");
    assert!(app.inbox(owner).await.is_empty());
}

/// A role change after the window makes a new row.
#[sqlx::test(migrations = false)]
async fn a_role_change_after_the_window_makes_a_new_row(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let user = app.user().await;
    let (id, to) = (workspace.to_string(), user.to_string());
    app.grant_role(owner, Role::on("workspace", &id, "viewer"), &to)
        .await
        .unwrap();
    app.inbox(user).await;
    sqlx::query!("UPDATE notifications SET last_at = now() - interval '6 minutes'")
        .execute(&pool)
        .await
        .unwrap();

    // when
    app.grant_role(owner, Role::on("workspace", &id, "member"), &to)
        .await
        .unwrap();
    let inbox = app.inbox(user).await;

    // then
    assert_eq!(inbox.len(), 2);
}

/// The new owner and the other owners get a row, and the actor gets none. The old owner keeps its
/// row after it loses the workspace.
#[sqlx::test(migrations = false)]
async fn owner_changes_reach_the_person_and_the_other_owners(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.user().await;
    let workspace = app.workspace_of(actor).await;
    let id = workspace.to_string();
    let (other, new) = (app.user().await, app.user().await);
    let owner = Role::owner_of(&id);
    app.grant_role(actor, owner, &other.to_string())
        .await
        .unwrap();
    app.dispatch_notifications().await;

    // when
    app.grant_role(actor, owner, &new.to_string())
        .await
        .unwrap();
    app.grant_role(actor, owner, &new.to_string())
        .await
        .unwrap();
    let person = row_of(&app, new, "owner.added").await;
    let others = kinds_of(&app, other).await;
    app.revoke_role(actor, owner, &new.to_string())
        .await
        .unwrap();
    let old = kinds_of(&app, new).await;

    // then
    assert_eq!(person.reason, "subject");
    assert_eq!(others, ["owner.added", "owner.added"]);
    assert_eq!(old, ["owner.added", "owner.removed"]);
    assert!(app.inbox(actor).await.is_empty());
}

/// A person that gets the instance admin role gets a row, and so do the other admins. The actor
/// gets none.
#[sqlx::test(migrations = false)]
async fn admin_changes_reach_the_person_and_the_other_admins(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.admin().await;
    let other = app.admin().await;
    let new = app.user().await;

    // when
    app.grant_instance_admin(actor, &new.to_string())
        .await
        .unwrap();
    app.grant_instance_admin(actor, &new.to_string())
        .await
        .unwrap();
    let person = row_of(&app, new, "admin.granted").await;
    let others = row_of(&app, other, "admin.granted").await;
    app.revoke_instance_admin(actor, &new.to_string())
        .await
        .unwrap();
    let after = kinds_of(&app, new).await;

    // then
    assert_eq!(person.reason, "subject");
    assert_eq!(others.reason, "admin");
    assert_eq!(after, ["admin.granted", "admin.revoked"]);
    assert!(app.inbox(actor).await.is_empty());
}

/// A role of the instance that a person gets makes a row with the name of the role. A revoke in
/// the same window takes both changes out, and the row stays. The actor gets no row.
#[sqlx::test(migrations = false)]
async fn an_instance_role_change_reaches_the_person(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let role = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, role).await.unwrap();

    // when
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    let granted = row_of(&app, user, "instance_role.changed").await;
    app.revoke_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    let after = row_of(&app, user, "instance_role.changed").await;

    // then
    assert_eq!(granted.reason, "subject");
    assert_eq!(roles(&granted.params, "added"), ["Auditor"]);
    assert_eq!(after.id, granted.id);
    assert!(roles(&after.params, "added").is_empty());
    assert!(roles(&after.params, "removed").is_empty());
    assert!(app.inbox(admin).await.is_empty());
}
