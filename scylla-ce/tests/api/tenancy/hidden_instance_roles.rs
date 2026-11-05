//! The tests of the rule that a read of the people of a workspace or of a project shows no role of
//! the instance. See decision 92.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::access::v1::{GetMemberAccessResponse, Member};
use scylla_testkit::{text, Role, RoleInput, TestApp};
use sqlx::PgPool;

use crate::app;

/// Makes a workspace of a new owner with one project, and a viewer of the workspace. Returns the
/// owner, the viewer, the id of the workspace and the id of the project.
async fn workspace_with_viewer(app: &TestApp) -> (ActorId, ActorId, String, String) {
    let (owner, viewer) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    let project = app.create_project(owner, "alpha").await.unwrap();
    let role = Role::on("workspace", &workspace, "viewer");
    app.grant_role(owner, role, &viewer.to_string())
        .await
        .unwrap();
    (owner, viewer, workspace, text(&project.id).to_owned())
}

/// Returns the member with an id in a list.
fn find(members: &[Member], actor: ActorId) -> Option<&Member> {
    let id = actor.to_string();
    members
        .iter()
        .find(|member| member.actor_id.as_ref().is_some_and(|one| one.value == id))
}

/// Returns the roles of an answer as `kind:key`, in order.
fn roles_in(member: &Member) -> Vec<String> {
    let mut roles: Vec<String> = member
        .roles
        .iter()
        .map(|role| format!("{}:{}", role.scope_kind, role.role_key))
        .collect();
    roles.sort();
    roles
}

/// Returns `true` if an answer names a role of the instance, in the member or in a source.
fn names_the_instance(answer: &GetMemberAccessResponse) -> bool {
    let in_member = answer
        .member
        .iter()
        .flat_map(|member| &member.roles)
        .any(|role| role.scope_kind == "platform" || role.role_key == "instance-admin");
    let in_sources = answer
        .permissions
        .iter()
        .flat_map(|permission| &permission.sources)
        .any(|source| source.scope_kind == "platform" || source.role_key == "instance-admin");
    in_member || in_sources
}

/// An instance admin with no role here is not in the lists, for each caller, and also for itself.
#[sqlx::test(migrations = false)]
async fn list_members_leaves_out_an_instance_admin_with_no_role_here(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (owner, viewer, workspace, project) = workspace_with_viewer(&app).await;

    for caller in [owner, viewer, admin] {
        // when
        let on_workspace = app
            .members_of(caller, "workspace", &workspace)
            .await
            .unwrap();
        let on_project = app.members_of(caller, "project", &project).await.unwrap();

        // then
        assert!(find(&on_workspace, admin).is_none());
        assert!(find(&on_project, admin).is_none());
        assert!(find(&on_workspace, owner).is_some());
        assert!(find(&on_project, viewer).is_some());
    }
}

/// An instance admin with a role here is a normal member, and the list does not tell the role of
/// the instance.
#[sqlx::test(migrations = false)]
async fn list_members_shows_an_instance_admin_with_a_role_here_as_a_member(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (owner, viewer, workspace, project) = workspace_with_viewer(&app).await;
    let to = admin.to_string();
    let on_workspace = Role::on("workspace", &workspace, "member");
    app.grant_role(owner, on_workspace, &to).await.unwrap();
    let on_project = Role::on("project", &project, "editor");
    app.grant_role(owner, on_project, &to).await.unwrap();

    // when
    let in_workspace = app
        .members_of(viewer, "workspace", &workspace)
        .await
        .unwrap();
    let in_project = app.members_of(viewer, "project", &project).await.unwrap();

    // then
    let expected = ["project:project-editor", "workspace:workspace-member"];
    assert_eq!(roles_in(find(&in_workspace, admin).unwrap()), expected);
    assert_eq!(roles_in(find(&in_project, admin).unwrap()), expected);
}

/// A person with only a role of the instance gets the empty answer of an actor with no role: an
/// instance admin, and a holder of a custom role of the instance.
#[sqlx::test(migrations = false)]
async fn get_member_access_gives_nothing_for_a_role_of_the_instance_alone(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, auditor) = (app.admin().await, app.user().await);
    let role = RoleInput::on_platform("Auditor", &["users.view", "project.view"]);
    let role = app.create_role(admin, role).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, auditor)
        .await
        .unwrap();
    let (owner, viewer, workspace, project) = workspace_with_viewer(&app).await;

    for caller in [owner, viewer, admin] {
        for person in [admin, auditor] {
            // when
            let person = person.to_string();
            let on_workspace = app
                .member_access(caller, &person, "workspace", &workspace)
                .await
                .unwrap();
            let on_project = app
                .member_access(caller, &person, "project", &project)
                .await
                .unwrap();

            // then
            assert_eq!(on_workspace, GetMemberAccessResponse::default());
            assert_eq!(on_project, GetMemberAccessResponse::default());
        }
    }
}

/// An instance admin with a role here shows only that role and what it gives.
#[sqlx::test(migrations = false)]
async fn get_member_access_shows_an_instance_admin_only_by_its_role_here(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (owner, viewer, workspace, project) = workspace_with_viewer(&app).await;
    let to = admin.to_string();
    let role = Role::on("workspace", &workspace, "viewer");
    app.grant_role(owner, role, &to).await.unwrap();

    // when
    let on_workspace = app
        .member_access(viewer, &to, "workspace", &workspace)
        .await
        .unwrap();
    let on_project = app
        .member_access(viewer, &to, "project", &project)
        .await
        .unwrap();

    // then
    assert!(!names_the_instance(&on_workspace));
    assert!(!names_the_instance(&on_project));
    let member = on_workspace.member.as_ref().unwrap();
    assert_eq!(roles_in(member), ["workspace:workspace-viewer"]);
    let has = |id: &str| {
        on_workspace
            .permissions
            .iter()
            .any(|permission| permission.permission == id)
    };
    assert!(has("project.view"));
    assert!(!has("workspace.delete"));
}

/// The other reads of a workspace do not show an instance admin, and the holders of the role on
/// the instance still do.
#[sqlx::test(migrations = false)]
async fn the_other_reads_leave_out_an_instance_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (owner, viewer, workspace, project) = workspace_with_viewer(&app).await;
    let id = admin.to_string();

    // when
    let on_workspace = app
        .access_of(viewer, "workspace", &workspace, 100)
        .await
        .unwrap();
    let on_project = app
        .access_of(owner, "project", &project, 100)
        .await
        .unwrap();
    let roles = app.list_roles(viewer, Some(&workspace)).await.unwrap();
    let refused = app
        .role_holders(viewer, "instance-admin", Some(&workspace))
        .await;
    let on_instance = app
        .role_holders(admin, "instance-admin", None)
        .await
        .unwrap();

    // then
    assert!(on_workspace.iter().all(|(actor, _)| *actor != id));
    assert!(on_project.iter().all(|(actor, _)| *actor != id));
    assert!(roles.iter().all(|role| role.key != "instance-admin"));
    assert!(refused.is_err());
    assert_eq!(on_instance.total, 1);
    assert_eq!(on_instance.holders[0].actor_id.as_ref().unwrap().value, id);
}
