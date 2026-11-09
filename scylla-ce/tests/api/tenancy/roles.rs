//! The tests of `RoleService`: the catalog, the built-in roles, and the changes of a custom role.

use scylla_domain::authz::Resource;
use scylla_domain::identity::apps::App;
use scylla_domain::platform::Platform;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::RoleInput;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// Each route permission of the schema is in the catalog. The list names the groups, and the
/// permissions that only an owner or an instance admin passes.
#[test]
fn each_route_of_a_role_is_in_the_catalog() {
    // given
    let edition = scylla_ce::edition();
    let schema = edition.authz_model();
    const OUT: [&str; 9] = [
        "manage",
        "work",
        "read",
        "delete",
        "edit",
        "view",
        "manage_owners",
        "grant_instance_admin",
        "revoke_instance_admin",
    ];

    // when
    let missing: Vec<String> = [Platform::NAME, Workspace::NAME, Project::NAME, App::NAME]
        .into_iter()
        .flat_map(|definition| {
            schema
                .permissions(definition)
                .into_iter()
                .filter(|name| !OUT.contains(name))
                .filter(move |name| catalog::covering(definition, name).is_empty())
                .map(move |name| format!("{definition}.{name}"))
        })
        .collect();

    // then
    assert!(missing.is_empty(), "not in the catalog: {missing:?}");
}

/// Each caller reads the catalog: the areas of the three levels.
#[sqlx::test(migrations = false)]
async fn list_permission_catalog_gives_the_areas(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let areas = app.permission_catalog(user).await.unwrap();

    // then
    let pipelines = areas.iter().find(|area| area.id == "pipelines").unwrap();
    assert_eq!(pipelines.level, "project");
    let edit = pipelines
        .permissions
        .iter()
        .find(|permission| permission.id == "pipelines.edit")
        .unwrap();
    assert_eq!(edit.depends_on, ["pipelines.view"]);
    let view = areas.iter().find(|area| area.id == "workspace").unwrap();
    assert!(view.permissions[0].implicit);
}

/// `ListRoles` on the instance needs a role, and tells the built-in roles with their facts.
#[sqlx::test(migrations = false)]
async fn list_roles_gives_the_built_in_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);

    // when
    let refused = app.list_roles(user, None).await;
    let roles = app.list_roles(admin, None).await.unwrap();

    // then
    assert_code(refused, Code::PermissionDenied);
    let find = |key: &str| roles.iter().find(|role| role.key == key).unwrap();
    let owner = find("instance-admin");
    assert!(owner.builtin && owner.full_control);
    assert_eq!(owner.holder_count, 1);
    assert_eq!(owner.permissions.len(), 38);
    assert!(!roles.iter().any(|role| role.key == "workspace-creator"));
    let viewer = find("project-viewer");
    assert_eq!(viewer.grant_name, "viewer");
    assert!(!viewer.full_control);
    assert!(viewer.permissions.contains(&"pipelines.view".to_owned()));
    assert!(!viewer.permissions.contains(&"pipelines.edit".to_owned()));
    assert!(find("project-admin").full_control);
    assert_eq!(find("agent").holder, "apps");
}

/// The built-in roles of the agents give the area of the machines, on a workspace and on a project.
#[sqlx::test(migrations = false)]
async fn list_roles_gives_job_execution_to_the_agent_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let roles = app.list_roles(admin, None).await.unwrap();

    // then
    let find = |key: &str| roles.iter().find(|role| role.key == key).unwrap();
    let machine = ["job_execution.edit", "job_execution.run"];
    assert_eq!(find("agent").permissions, machine);
    assert_eq!(find("project-agent").permissions, machine);
}

/// `CreateRole` gives the area of the machines to a role for apps on a project.
#[sqlx::test(migrations = false)]
async fn create_role_gives_job_execution_to_a_project_role_for_apps(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let input = RoleInput {
        name: "Runner",
        level: "project",
        holder: "apps",
        permissions: &["job_execution.run"],
        workspace: Some(&workspace),
    };

    // when
    let role = app.create_role(owner, input).await.unwrap();

    // then
    assert_eq!(
        role.permissions,
        ["job_execution.edit", "job_execution.run"]
    );
}

/// A role of the schema counts its holders on all the objects, and a list for a workspace counts
/// only the holders in it.
#[sqlx::test(migrations = false)]
async fn list_roles_counts_the_holders_of_a_relation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, owner, member) = (app.admin().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let role = scylla_testkit::Role::on("workspace", &workspace, "member");
    app.grant_role(owner, role, &member.to_string())
        .await
        .unwrap();

    // when
    let on_instance = app.get_role(admin, "workspace-member", None).await.unwrap();
    let on_workspace = app
        .get_role(owner, "workspace-member", Some(&workspace))
        .await
        .unwrap();

    // then
    assert_eq!(on_instance.holder_count, 1);
    assert_eq!(on_workspace.holder_count, 1);
}

/// `CreateRole` makes a role of the instance, and adds what its permissions depend on.
#[sqlx::test(migrations = false)]
async fn create_role_makes_a_role_of_the_instance(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let input = RoleInput::on_platform("  Auditor ", &["users.edit"]);
    let role = app.create_role(admin, input).await.unwrap();
    let listed = app.list_roles(admin, None).await.unwrap();

    // then
    assert_eq!(role.name, "Auditor");
    assert_eq!(role.grant_name, role.key);
    assert!(!role.builtin && !role.full_control);
    assert_eq!(role.permissions, ["users.view", "users.edit"]);
    assert!(listed.iter().any(|other| other.key == role.key));
}

/// `CreateRole` refuses a bad level or permission, a used name, and a role with no right.
#[sqlx::test(migrations = false)]
async fn create_role_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let good = RoleInput::on_platform("Auditor", &["users.view"]);
    app.create_role(admin, good).await.unwrap();

    // when
    let bad_level = RoleInput {
        level: "galaxy",
        ..RoleInput::on_platform("A", &[])
    };
    let apps_on_platform = RoleInput {
        holder: "apps",
        ..RoleInput::on_platform("B", &[])
    };
    let above = RoleInput {
        level: "workspace",
        ..RoleInput::on_platform("C", &["users.view"])
    };
    let unknown = RoleInput::on_platform("D", &["pipelines.fly"]);
    let machine = RoleInput::on_platform("E", &["job_execution.run"]);
    let used = RoleInput::on_platform("  AUDITOR ", &[]);

    // then
    for input in [bad_level, apps_on_platform, above, unknown, machine] {
        assert_code(app.create_role(admin, input).await, Code::InvalidArgument);
    }
    assert_refused(app.create_role(admin, used).await, "ROLE_NAME_IN_USE");
    let refused = app
        .create_role(user, RoleInput::on_platform("F", &[]))
        .await;
    assert_code(refused, Code::PermissionDenied);
}

/// The holder of a role can make a role only with the permissions that it holds.
#[sqlx::test(migrations = false)]
async fn create_role_refuses_a_permission_that_the_caller_does_not_hold(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, keeper) = (app.admin().await, app.user().await);
    let input = RoleInput::on_platform("Keeper", &["instance_roles.manage", "users.view"]);
    let role = app.create_role(admin, input).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, keeper)
        .await
        .unwrap();

    // when
    let wider = RoleInput::on_platform("Wider", &["users.edit"]);
    let refused = app.create_role(keeper, wider).await;
    let same = RoleInput::on_platform("Same", &["users.view"]);
    let accepted = app.create_role(keeper, same).await;

    // then
    assert_refused(refused, "ESCALATION");
    assert!(accepted.is_ok());
}
