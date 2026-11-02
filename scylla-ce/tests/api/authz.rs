//! The tests of the permission model: the core roles, and the parts that an edition adds.

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::authz;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::Id;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;

use crate::app;

authz! {
    /// An auditor of the projects, as an edition adds it.
    extend Project as ProjectAudit {
        /// An actor that audits the project.
        relation auditor: Actor;
        /// Read the audit log: the auditors, and the actors that can delete the project.
        permission audit = auditor->act_for + delete;
    }
}

/// A resource that an edition adds.
struct Widget;

authz! {
    /// The permission model of the widgets.
    definition widget for Widget {
        /// The project of the widget.
        relation project: Project;
        /// Use the widget: the actors that can change its project.
        permission operate = project->edit;
    }
}

/// Creates a project for an owner and returns its id.
async fn project_of(app: &TestApp, owner: ActorId) -> ProjectId {
    let project = app.create_project(owner, "alpha").await.unwrap();
    ProjectId::parse(text(&project.id)).unwrap()
}

/// The owner and its delegates can delete a project, a viewer can only read it, and another
/// actor can do nothing.
#[sqlx::test(migrations = false)]
async fn the_core_roles_give_their_permissions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let delegate = app.user().await;
    let viewer = app.user().await;
    let stranger = app.user().await;
    let project = project_of(&app, owner).await;
    let grants = [
        Actor::DELEGATE.of(owner, delegate),
        Project::VIEWER.of(project, viewer),
    ];
    app.grant(&grants).await;

    // when
    let owner_deletes = app.can(owner, Project::DELETE.on(project)).await;
    let delegate_deletes = app.can(delegate, Project::DELETE.on(project)).await;
    let viewer_reads = app.can(viewer, Project::VIEW.on(project)).await;
    let viewer_edits = app.can(viewer, Project::EDIT.on(project)).await;
    let stranger_reads = app.can(stranger, Project::VIEW.on(project)).await;

    // then
    assert!(owner_deletes && delegate_deletes && viewer_reads);
    assert!(!viewer_edits && !stranger_reads);
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
    let project = project_of(&app, owner).await;
    app.grant(&[ProjectAudit::AUDITOR.of(project, auditor)])
        .await;

    // when
    let auditor_audits = app.can(auditor, ProjectAudit::AUDIT.on(project)).await;
    let owner_audits = app.can(owner, ProjectAudit::AUDIT.on(project)).await;
    let auditor_deletes = app.can(auditor, Project::DELETE.on(project)).await;
    let stranger_audits = app.can(stranger, ProjectAudit::AUDIT.on(project)).await;

    // then
    assert!(auditor_audits && owner_audits);
    assert!(!auditor_deletes && !stranger_audits);
}

/// An edition adds a definition, with an arrow to a core definition.
#[sqlx::test(migrations = false)]
async fn an_edition_adds_a_definition(pool: PgPool) {
    // given
    let edition = scylla_core::edition("test").authz::<Widget>().build();
    let app = TestApp::spawn(pool, edition).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let project = project_of(&app, owner).await;
    let widget = Id::<Widget>::generate();
    app.grant(&[Widget::PROJECT.of(widget, project)]).await;

    // when
    let owner_operates = app.can(owner, Widget::OPERATE.on(widget)).await;
    let stranger_operates = app.can(stranger, Widget::OPERATE.on(widget)).await;

    // then
    assert!(owner_operates);
    assert!(!stranger_operates);
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
