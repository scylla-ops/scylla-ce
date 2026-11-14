//! The tests of the permission model of the Git integration: what the built-in roles and the
//! custom roles give on the connections, the repository of a project, the approval of a job from
//! a fork, and the GitHub App of the instance.

use scylla_domain::integrations::apps::VcsAppHolder;
use scylla_domain::integrations::connections::VcsConnectionHolder;
use scylla_domain::integrations::links::RepositoryAccess;
use scylla_domain::platform::Platform;
use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::{text, Role, RoleInput};
use sqlx::PgPool;

use crate::app;

/// A project editor approves the jobs from a fork and links a repository. A project viewer reads
/// the link, and does not change it.
#[sqlx::test(migrations = false)]
async fn a_project_editor_approves_jobs_and_links_a_repository(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (editor, viewer) = (app.user().await, app.user().await);
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = ProjectId::parse(text(&project.id)).unwrap();
    let workspace = WorkspaceId::parse(text(&project.workspace_id)).unwrap();
    app.grant(&[
        Project::EDITOR.of(id, editor),
        Project::VIEWER.of(id, viewer),
    ])
    .await;
    let approve = || PipelineAccess::APPROVE_JOB.on(id).within(workspace);
    let link = || RepositoryAccess::LINK_REPOSITORY.on(id).within(workspace);
    let read = || {
        RepositoryAccess::GET_REPOSITORY_LINK
            .on(id)
            .within(workspace)
    };

    // when
    let by_editor = [
        app.can(editor, approve()).await,
        app.can(editor, link()).await,
        app.can(editor, read()).await,
    ];
    let by_viewer = [
        app.can(viewer, approve()).await,
        app.can(viewer, link()).await,
        app.can(viewer, read()).await,
    ];

    // then
    assert_eq!(by_editor, [true, true, true]);
    assert_eq!(by_viewer, [false, false, true]);
}

/// A custom role of a project with the edit of the repository links a repository and sets the
/// source of the pipelines. It does not approve a job.
#[sqlx::test(migrations = false)]
async fn a_custom_project_role_gives_the_edit_of_the_repository(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, bob) = (app.user().await, app.user().await);
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = ProjectId::parse(text(&project.id)).unwrap();
    let workspace = app.workspace_of(owner).await;
    let owned_by = workspace.to_string();
    let input = RoleInput {
        name: "Repository keeper",
        level: "project",
        holder: "people",
        permissions: &["repository.edit"],
        workspace: Some(&owned_by),
    };
    let role = app.create_role(owner, input).await.unwrap();
    let to = bob.to_string();
    let member = Role::on("workspace", &owned_by, "viewer");
    app.grant_role(owner, member, &to).await.unwrap();
    let keeper = Role::on("project", text(&project.id), &role.grant_name);
    app.grant_role(owner, keeper, &to).await.unwrap();

    // when
    let link = app
        .can(
            bob,
            RepositoryAccess::LINK_REPOSITORY.on(id).within(workspace),
        )
        .await;
    let source = app
        .can(
            bob,
            PipelineAccess::SET_PIPELINE_SOURCE.on(id).within(workspace),
        )
        .await;
    let approve = app
        .can(bob, PipelineAccess::APPROVE_JOB.on(id).within(workspace))
        .await;

    // then
    assert!(link, "the role links a repository");
    assert!(source, "the role sets the source of the pipelines");
    assert!(!approve, "the role does not approve a job");
}

/// A member of a workspace lists the connections, and only an admin of the workspace connects a
/// provider.
#[sqlx::test(migrations = false)]
async fn the_admins_of_a_workspace_manage_its_connections(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (admin, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
    ])
    .await;
    let list = || VcsConnectionHolder::LIST_VCS_CONNECTIONS.on(workspace);
    let connect = || VcsConnectionHolder::CONNECT_GIT_LAB.on(workspace);

    // when
    let by_admin = [
        app.can(admin, list()).await,
        app.can(admin, connect()).await,
    ];
    let by_member = [
        app.can(member, list()).await,
        app.can(member, connect()).await,
    ];

    // then
    assert_eq!(by_admin, [true, true]);
    assert_eq!(by_member, [true, false]);
}

/// The instance admins and a role of the instance with the Git providers manage the GitHub App of
/// the instance. Another user does not.
#[sqlx::test(migrations = false)]
async fn a_role_of_the_instance_manages_the_git_providers(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, keeper, user) = (app.admin().await, app.user().await, app.user().await);
    let input = RoleInput::on_platform("Git keeper", &["git_providers.manage"]);
    let role = app.create_role(admin, input).await.unwrap();
    app.grant_on_platform(admin, &role.grant_name, keeper)
        .await
        .unwrap();
    let manifest = || VcsAppHolder::START_GIT_HUB_MANIFEST.on(Platform::ID);

    // when
    let answers = [
        app.can(admin, manifest()).await,
        app.can(keeper, manifest()).await,
        app.can(user, manifest()).await,
    ];

    // then
    assert_eq!(answers, [true, true, false]);
}

/// The built-in roles show the cells of the integration: a project editor approves and edits the
/// repository, a project viewer reads it, and a workspace member reads the connections.
#[sqlx::test(migrations = false)]
async fn list_roles_gives_the_cells_of_the_integration(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let roles = app.list_roles(admin, None).await.unwrap();

    // then
    let holds = |key: &str, id: &str| {
        let role = roles.iter().find(|role| role.key == key).unwrap();
        role.permissions.iter().any(|held| held == id)
    };
    assert!(holds("project-editor", "jobs.approve"));
    assert!(holds("project-editor", "repository.edit"));
    assert!(holds("project-viewer", "repository.view"));
    assert!(!holds("project-viewer", "repository.edit"));
    assert!(!holds("project-viewer", "jobs.approve"));
    assert!(holds("workspace-member", "integrations.view"));
    assert!(!holds("workspace-member", "integrations.manage"));
    assert!(holds("workspace-admin", "integrations.manage"));
}
