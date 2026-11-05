//! The tests of `DeleteWorkspace`: only an owner deletes, only an empty workspace goes, its
//! invitations and roles go with it, and a creation at the same time cannot slip in.

use scylla_domain::authz::Object;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::Gate;
use scylla_testkit::{text, Role, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Counts the rows of a workspace in `workspaces`, `workspace_owners` and `invitations`.
async fn rows_of(app: &TestApp, workspace: WorkspaceId) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM workspaces WHERE id = $1)
                + (SELECT count(*) FROM workspace_owners WHERE workspace_id = $1)
                + (SELECT count(*) FROM invitations WHERE workspace_id = $1) AS "count!""#,
        workspace.to_db(),
    )
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Only an owner deletes a workspace: an admin, a stranger and an unknown workspace get the
/// same refusal. The workspace goes with its owners, its invitations and its roles.
#[sqlx::test(migrations = false)]
async fn delete_workspace_needs_an_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin, stranger) = (app.user().await, app.user().await, app.user().await);
    let team = app.create_workspace(owner, "team").await.unwrap();
    let workspace = WorkspaceId::parse(text(&team.id)).unwrap();
    let id = workspace.to_string();
    app.grant(&[Workspace::ADMIN.of(workspace, admin)]).await;
    app.create_invitation(owner, workspace, "eve@example.com", "viewer")
        .await
        .unwrap();

    // when
    let by_admin = app.delete_workspace(admin, &id).await;
    let by_stranger = app.delete_workspace(stranger, &id).await;
    let unknown = app.delete_workspace(owner, UNKNOWN_ID).await;
    let not_an_id = app.delete_workspace(owner, "nope").await;
    let before = rows_of(&app, workspace).await;
    app.delete_workspace(owner, &id).await.unwrap();
    let again = app.delete_workspace(owner, &id).await;

    // then
    assert_code(by_admin, Code::PermissionDenied);
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "workspace_id");
    assert_eq!(before, 3);
    assert_eq!(rows_of(&app, workspace).await, 0);
    assert_code(again, Code::PermissionDenied);
    let holders = Gate::subjects(app.access.as_ref(), &Object::of(workspace)).await;
    assert!(holders.unwrap().is_empty());
    assert_eq!(app.visible_workspaces(admin).await.unwrap(), ["Personal"]);
}

/// A workspace with a project or an agent stays: `WORKSPACE_NOT_EMPTY`. With no project left,
/// it goes.
#[sqlx::test(migrations = false)]
async fn delete_workspace_needs_an_empty_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let with_project = app.create_workspace(owner, "projects").await.unwrap();
    let with_agent = app.create_workspace(owner, "agents").await.unwrap();
    let (projects, agents) = (text(&with_project.id), text(&with_agent.id));
    let project = app
        .create_project_in(owner, WorkspaceId::parse(projects).unwrap(), "alpha")
        .await
        .unwrap();
    app.app_in(owner, WorkspaceId::parse(agents).unwrap(), true)
        .await;

    // when
    let full = app.delete_workspace(owner, projects).await;
    let agent = app.delete_workspace(owner, agents).await;
    app.delete_project(owner, text(&project.id)).await.unwrap();
    let emptied = app.delete_workspace(owner, projects).await;

    // then
    assert_refused(full, "WORKSPACE_NOT_EMPTY");
    assert_refused(agent, "WORKSPACE_NOT_EMPTY");
    assert!(emptied.is_ok(), "{emptied:?}");
}

/// A delete and a project creation at the same time never both pass, and neither fails with an
/// internal error: the lock on the workspace makes one wait for the other. Ten rounds, so that a
/// race shows.
#[sqlx::test(migrations = false)]
async fn a_creation_cannot_slip_into_a_deleted_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let mut workspaces = Vec::new();
    for round in 0..10 {
        let created = app.create_workspace(owner, &format!("race {round}")).await;
        workspaces.push(WorkspaceId::parse(text(&created.unwrap().id)).unwrap());
    }

    // when
    let mut outcomes = Vec::new();
    for workspace in &workspaces {
        let id = workspace.to_string();
        let (deleted, created) = tokio::join!(
            app.delete_workspace(owner, &id),
            app.create_project_in(owner, *workspace, "late"),
        );
        outcomes.push((deleted, created));
    }

    // then
    for ((deleted, created), workspace) in outcomes.into_iter().zip(&workspaces) {
        let projects = app.project_count(*workspace).await;
        match (deleted, created) {
            (Ok(()), Err(status)) => {
                let refused = [Code::NotFound, Code::PermissionDenied];
                assert!(refused.contains(&status.code()), "{status:?}");
                assert_eq!(rows_of(&app, *workspace).await, 0);
            }
            (Err(status), Ok(_)) => {
                let info = tonic_types::StatusExt::get_details_error_info(&status);
                assert_eq!(
                    info.map(|info| info.reason).as_deref(),
                    Some("WORKSPACE_NOT_EMPTY")
                );
                assert_eq!(projects, 1);
            }
            other => panic!("one of the two must pass: {other:?}"),
        }
    }
}

/// A new owner or an invitation at the same time as the delete of the workspace never fails
/// with an internal error: it passes before the delete, or finds no workspace. Ten rounds, so
/// that a race shows.
#[sqlx::test(migrations = false)]
async fn a_change_that_races_a_delete_finds_no_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, partner) = (app.user().await, app.user().await);
    let partner_id = partner.to_string();
    let mut workspaces = Vec::new();
    for round in 0..10 {
        let created = app.create_workspace(owner, &format!("race {round}")).await;
        workspaces.push(WorkspaceId::parse(text(&created.unwrap().id)).unwrap());
    }

    // when
    let mut codes = Vec::new();
    for (round, workspace) in workspaces.iter().enumerate() {
        let id = workspace.to_string();
        let email = format!("race{round}@example.com");
        let (deleted, added, invited) = tokio::join!(
            app.delete_workspace(owner, &id),
            app.grant_role(owner, Role::owner_of(&id), &partner_id),
            app.create_invitation(owner, *workspace, &email, "viewer"),
        );
        assert!(deleted.is_ok(), "{deleted:?}");
        codes.push(added.err().map(|status| status.code()));
        codes.push(invited.err().map(|status| status.code()));
    }

    // then
    let allowed = [None, Some(Code::NotFound), Some(Code::PermissionDenied)];
    assert!(codes.iter().all(|code| allowed.contains(code)), "{codes:?}");
}
