//! The tests of the permission model: the roles in a workspace and on one project, and the
//! parts that an edition adds.

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::authz;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::workspaces::{Workspace, WorkspaceId};
use scylla_domain::Id;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;

use crate::app;

authz! {
    /// An auditor of the projects, as an edition adds it.
    extend Project as ProjectAudit {
        /// An actor that audits the project.
        relation auditor: Actor;
        /// Read the audit log: the auditors and the project admins.
        permission audit = auditor + delete;
    }
}

/// A resource that an edition adds.
struct Widget;

authz! {
    /// The roles on one widget.
    definition widget for Widget {
        /// An actor that can use the widget.
        relation operator: Actor;
        /// Use the widget: its operators.
        permission operate = operator;
    }
}

/// Creates a project in the personal workspace of an owner, and returns the two ids.
async fn project_of(app: &TestApp, owner: ActorId) -> (ProjectId, WorkspaceId) {
    let project = app.create_project(owner, "alpha").await.unwrap();
    let id = ProjectId::parse(text(&project.id)).unwrap();
    (id, WorkspaceId::parse(text(&project.workspace_id)).unwrap())
}

/// The owner of the workspace, a workspace admin and a project admin delete a project. A
/// workspace member and a project viewer only read it, and another actor can do nothing.
#[sqlx::test(migrations = false)]
async fn the_roles_give_their_permissions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (workspace_admin, member) = (app.user().await, app.user().await);
    let (admin, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let (project, workspace) = project_of(&app, owner).await;
    let roles = [
        Workspace::ADMIN.of(workspace, workspace_admin),
        Workspace::MEMBER.of(workspace, member),
        Project::ADMIN.of(project, admin),
        Project::VIEWER.of(project, viewer),
    ];
    app.grant(&roles).await;
    let delete = || Project::DELETE_PROJECT.on(project).within(workspace);
    let read = || Project::GET_PROJECT.on(project).within(workspace);

    // when
    let owner_deletes = app.can(owner, delete()).await;
    let workspace_admin_deletes = app.can(workspace_admin, delete()).await;
    let admin_deletes = app.can(admin, delete()).await;
    let member_deletes = app.can(member, delete()).await;
    let viewer_deletes = app.can(viewer, delete()).await;
    let member_reads = app.can(member, read()).await;
    let viewer_reads = app.can(viewer, read()).await;
    let stranger_reads = app.can(stranger, read()).await;

    // then
    assert!(owner_deletes && workspace_admin_deletes && admin_deletes);
    assert!(member_reads && viewer_reads);
    assert!(!member_deletes && !viewer_deletes && !stranger_reads);
}

/// An edition adds a relation and a permission to the core definition of the projects.
#[sqlx::test(migrations = false)]
async fn an_edition_adds_a_permission_to_a_core_definition(pool: PgPool) {
    // given
    let edition = scylla_core::edition("test").authz::<ProjectAudit>().build();
    let app = TestApp::spawn(pool, edition).await;
    let owner = app.user().await;
    let auditor = app.user().await;
    let stranger = app.user().await;
    let (project, workspace) = project_of(&app, owner).await;
    app.grant(&[ProjectAudit::AUDITOR.of(project, auditor)])
        .await;
    let audit = || ProjectAudit::AUDIT.on(project).within(workspace);
    let delete = Project::DELETE_PROJECT.on(project).within(workspace);

    // when
    let auditor_audits = app.can(auditor, audit()).await;
    let owner_audits = app.can(owner, audit()).await;
    let auditor_deletes = app.can(auditor, delete).await;
    let stranger_audits = app.can(stranger, audit()).await;

    // then
    assert!(auditor_audits && owner_audits);
    assert!(!auditor_deletes && !stranger_audits);
}

/// An edition adds a definition: the owner of the workspace of a widget passes, and an operator
/// passes on its widget only.
#[sqlx::test(migrations = false)]
async fn an_edition_adds_a_definition(pool: PgPool) {
    // given
    let edition = scylla_core::edition("test").authz::<Widget>().build();
    let app = TestApp::spawn(pool, edition).await;
    let owner = app.user().await;
    let operator = app.user().await;
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let widget = Id::<Widget>::generate();
    let other = Id::<Widget>::generate();
    app.grant(&[Widget::OPERATOR.of(widget, operator)]).await;
    let operate = |id: Id<Widget>| Widget::OPERATE.on(id).within(workspace);

    // when
    let owner_operates = app.can(owner, operate(widget)).await;
    let operator_operates = app.can(operator, operate(widget)).await;
    let operator_on_other = app.can(operator, operate(other)).await;
    let stranger_operates = app.can(stranger, operate(widget)).await;

    // then
    assert!(owner_operates && operator_operates);
    assert!(!operator_on_other && !stranger_operates);
}

/// An edition with a part that does not fit stops at start.
#[test]
#[should_panic(expected = "permission model: the definition `project` is declared twice")]
fn a_part_that_does_not_fit_stops_the_edition() {
    // given
    let edition = scylla_core::edition("test");

    // when
    let _ = edition.authz::<Project>();
}
