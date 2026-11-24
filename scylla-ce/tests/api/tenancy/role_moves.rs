//! The tests of the move of the holders in `DeleteRole`.

use scylla_proto::access::v1::Member;
use scylla_testkit::{Role, RoleInput};
use sqlx::PgPool;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_invalid_field};

/// Returns the keys of the roles of an actor in a list of members, as `kind:key`.
fn roles_of(members: &[Member], actor: &str) -> Vec<String> {
    let member = members
        .iter()
        .find(|member| member.actor_id.as_ref().is_some_and(|id| id.value == actor));
    let mut roles: Vec<String> = member
        .map(|member| {
            member
                .roles
                .iter()
                .map(|role| format!("{}:{}", role.scope_kind, role.role_key))
                .collect()
        })
        .unwrap_or_default();
    roles.sort();
    roles
}

/// The holders get the new role and keep their other roles, then the role goes.
#[sqlx::test(migrations = false)]
async fn delete_role_moves_the_holders(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let to = bob.to_string();
    let old = RoleInput {
        name: "Old",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let old = app.create_role(owner, old).await.unwrap();
    let next = RoleInput {
        name: "Next",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let next = app.create_role(owner, next).await.unwrap();
    let other = RoleInput {
        name: "Other",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let other = app.create_role(owner, other).await.unwrap();
    for role in [&old, &other] {
        app.grant_role(
            owner,
            Role::on("workspace", &workspace, &role.grant_name),
            &to,
        )
        .await
        .unwrap();
    }

    // when
    let refused = app.delete_role(owner, &old.key).await;
    let bad = app.delete_role_moving(owner, &old.key, "nope").await;
    let to_itself = app
        .delete_role_moving(owner, &old.key, &old.grant_name)
        .await;
    app.delete_role_moving(owner, &old.key, &next.grant_name)
        .await
        .unwrap();
    let members = app
        .members_of(owner, "workspace", &workspace)
        .await
        .unwrap();

    // then
    assert_refused(refused, "ROLE_IN_USE");
    assert_invalid_field(&bad.unwrap_err(), "move_to");
    assert_invalid_field(&to_itself.unwrap_err(), "move_to");
    let mut expected = vec![
        format!("workspace:{}", next.key),
        format!("workspace:{}", other.key),
    ];
    expected.sort();
    assert_eq!(roles_of(&members, &to), expected);
}

/// A move to a built-in role writes the role of the schema.
#[sqlx::test(migrations = false)]
async fn delete_role_moves_the_holders_to_a_built_in_role(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let to = bob.to_string();
    let input = RoleInput {
        name: "Old",
        level: "workspace",
        holder: "people",
        permissions: &["project.view"],
        workspace: Some(&workspace),
    };
    let old = app.create_role(owner, input).await.unwrap();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, &old.grant_name),
        &to,
    )
    .await
    .unwrap();

    // when
    app.delete_role_moving(owner, &old.key, "viewer")
        .await
        .unwrap();
    let access = app
        .access_of(owner, "workspace", &workspace, 10)
        .await
        .unwrap();

    // then
    assert!(access.contains(&(to, "viewer".to_owned())));
}

/// A move to a role above the caller gives `ESCALATION`, and the role stays.
#[sqlx::test(migrations = false)]
async fn delete_role_refuses_a_move_above_the_caller(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, manager, bob) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let manage = RoleInput {
        name: "Roles",
        level: "workspace",
        holder: "people",
        permissions: &["workspace_roles.manage", "workspace.view"],
        workspace: Some(&workspace),
    };
    let manage = app.create_role(owner, manage).await.unwrap();
    let small = RoleInput {
        name: "Small",
        level: "workspace",
        holder: "people",
        permissions: &["workspace.view"],
        workspace: Some(&workspace),
    };
    let small = app.create_role(owner, small).await.unwrap();
    let grant = |role: &str| Role::on("workspace", &workspace, role).name.to_owned();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, &manage.grant_name),
        &manager.to_string(),
    )
    .await
    .unwrap();
    app.grant_role(
        owner,
        Role::on("workspace", &workspace, &small.grant_name),
        &bob.to_string(),
    )
    .await
    .unwrap();

    // when
    let refused = app
        .delete_role_moving(manager, &small.key, &grant("admin"))
        .await;
    let holders = app
        .role_holders(owner, &small.key, Some(&workspace))
        .await
        .unwrap();

    // then
    assert_refused(refused, "ESCALATION");
    assert_eq!(holders.total, 1);
}
