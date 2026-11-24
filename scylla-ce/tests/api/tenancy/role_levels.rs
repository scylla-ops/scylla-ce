//! The tests of a change of the level of a role: only with no holder, and never past a grant.

use scylla_testkit::RoleInput;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// The time that a request gets to reach a row that another transaction locks.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

/// A role with no holder changes its level, and takes the permissions of the new level.
#[sqlx::test(migrations = false)]
async fn update_role_changes_the_level_of_a_role_with_no_holder(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let input = RoleInput {
        name: "Release",
        level: "workspace",
        holder: "people",
        permissions: &["pipelines.run"],
        workspace: Some(&workspace),
    };
    let role = app.create_role(owner, input).await.unwrap();
    assert!(role.permissions.iter().any(|id| id == "workspace.view"));

    // when
    let moved = app
        .update_role_level(owner, &role.key, "project", &["pipelines.run"])
        .await
        .unwrap();
    let read = app
        .get_role(owner, &role.key, Some(&workspace))
        .await
        .unwrap();
    let back = app
        .update_role_level(owner, &role.key, "workspace", &["pipelines.run"])
        .await
        .unwrap();

    // then
    assert_eq!(moved.level, "project");
    assert_eq!(read.level, "project");
    assert!(!moved.permissions.iter().any(|id| id == "workspace.view"));
    assert_eq!(back.level, "workspace");
    assert!(back.permissions.iter().any(|id| id == "workspace.view"));
}

/// A role with a holder keeps its level until the holder goes. The same level is not a change.
#[sqlx::test(migrations = false)]
async fn update_role_keeps_the_level_of_a_role_with_a_holder(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let input = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();

    // when
    let refused = app
        .update_role_level(admin, &role.key, "workspace", &[])
        .await;
    let same = app
        .update_role_level(admin, &role.key, "platform", &["users.view"])
        .await;
    app.revoke_on_platform(admin, &role.grant_name, user)
        .await
        .unwrap();
    let moved = app
        .update_role_level(admin, &role.key, "workspace", &[])
        .await;

    // then
    assert_refused(refused, "ROLE_IN_USE");
    assert_eq!(same.unwrap().level, "platform");
    assert_eq!(moved.unwrap().level, "workspace");
}

/// A level that the owner, the holders or the permissions of the role do not fit stays out.
#[sqlx::test(migrations = false)]
async fn update_role_refuses_a_level_that_does_not_fit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, other) = (app.admin().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let of_workspace = RoleInput {
        name: "Release",
        level: "workspace",
        holder: "people",
        permissions: &["pipelines.run"],
        workspace: Some(&workspace),
    };
    let for_apps = RoleInput {
        name: "Machine",
        level: "workspace",
        holder: "apps",
        permissions: &[],
        workspace: None,
    };
    let role = app.create_role(owner, of_workspace).await.unwrap();
    let machine = app.create_role(admin, for_apps).await.unwrap();

    // when
    let unknown = app.update_role_level(owner, &role.key, "galaxy", &[]).await;
    let on_platform = app
        .update_role_level(owner, &role.key, "platform", &[])
        .await;
    let apps_on_platform = app
        .update_role_level(admin, &machine.key, "platform", &[])
        .await;
    let above = app
        .update_role_level(owner, &role.key, "project", &["workspace.view"])
        .await;
    let denied = app
        .update_role_level(other, &role.key, "project", &[])
        .await;

    // then
    assert_code(unknown, Code::InvalidArgument);
    assert_code(on_platform, Code::InvalidArgument);
    assert_code(apps_on_platform, Code::InvalidArgument);
    assert_code(above, Code::InvalidArgument);
    assert_code(denied, Code::PermissionDenied);
}

/// A grant waits for a change of the level that is not committed, then reads the new level.
#[sqlx::test(migrations = false)]
async fn a_grant_after_a_change_of_the_level_is_refused(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let input = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    let mut change = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM roles WHERE id = $1::uuid FOR UPDATE")
        .bind(&role.key)
        .execute(&mut *change)
        .await
        .unwrap();
    sqlx::query("UPDATE roles SET level = 'workspace' WHERE id = $1::uuid")
        .bind(&role.key)
        .execute(&mut *change)
        .await
        .unwrap();

    // when
    let (grant, ()) = tokio::join!(
        app.grant_on_platform(admin, &role.grant_name, user),
        async {
            tokio::time::sleep(LOCK_WAIT).await;
            change.commit().await.unwrap();
        }
    );

    // then
    assert_code(grant, Code::InvalidArgument);
    let read = app.get_role(admin, &role.key, None).await.unwrap();
    assert_eq!(read.holder_count, 0);
}

/// A change of the level waits for a grant that is not committed, then sees the holder.
#[sqlx::test(migrations = false)]
async fn a_change_of_the_level_after_a_grant_is_refused(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let input = RoleInput::on_platform("Auditor", &["users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    let mut grant = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM roles WHERE id = $1::uuid FOR SHARE")
        .bind(&role.key)
        .execute(&mut *grant)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO role_grants (actor_id, role_id, scope_kind, scope_id)
         VALUES ($1, $2::uuid, 'platform', $1)",
    )
    .bind(user.to_db())
    .bind(&role.key)
    .execute(&mut *grant)
    .await
    .unwrap();

    // when
    let (change, ()) = tokio::join!(
        app.update_role_level(admin, &role.key, "workspace", &[]),
        async {
            tokio::time::sleep(LOCK_WAIT).await;
            grant.commit().await.unwrap();
        }
    );

    // then
    assert_refused(change, "ROLE_IN_USE");
    let read = app.get_role(admin, &role.key, None).await.unwrap();
    assert_eq!(read.level, "platform");
}
