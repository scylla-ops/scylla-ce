//! The logic of the projects: one transaction for each call, with the hooks.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::{
    ListProjects, NewProject, Project, ProjectId, ProjectPage, Visible, KIND,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Gate, Hooks, WorkspaceFuture};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates a project if each `BeforeCreate` hook accepts it. Its workspace is in its row only.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewProject,
) -> Result<Project, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.workspace_id))
        .await?;
    let project = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(project)
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(super) async fn workspace(pool: &PgPool, id: ProjectId) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::workspace(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Finds the workspace of a project, for the roles of `AccessService`.
pub(super) fn find_workspace(pool: PgPool, id: ProjectId) -> WorkspaceFuture {
    Box::pin(async move {
        let mut tx = pool.begin().await?;
        let workspace = store::workspace(&mut tx, id).await?;
        tx.commit().await?;
        Ok(workspace)
    })
}

/// Returns a project, or `None` if no project has this id.
pub(super) async fn get(pool: &PgPool, id: ProjectId) -> Result<Option<Project>, Error> {
    let mut tx = pool.begin().await?;
    let project = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(project)
}

/// Returns one page of the projects that a caller can see, oldest first.
pub(super) async fn list(
    pool: &PgPool,
    query: ListProjects,
    visible: Visible,
) -> Result<ProjectPage, Error> {
    let mut tx = pool.begin().await?;
    let projects = store::list_visible(&mut tx, &visible, query.workspace_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(projects, query.page.size, |project| {
        project.id
    }))
}

/// Deletes a project, runs the `AfterDelete` hooks, then removes the roles on it.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    gate: &dyn Gate,
    actor: Actor,
    id: ProjectId,
) -> Result<(), Error> {
    // Delete the row and run the hooks in one transaction.
    let mut tx = pool.begin().await?;
    let workspace_id = store::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    hooks
        .after_delete(&mut tx, &DeleteContext::new(KIND, actor, workspace_id))
        .await?;
    tx.commit().await?;

    // Remove the roles after the commit. A role that stays reaches no project.
    if let Err(error) = gate.delete_object(&Object::of(id)).await {
        tracing::error!(%error, project = %id, "roles stay after a delete");
    }
    Ok(())
}
