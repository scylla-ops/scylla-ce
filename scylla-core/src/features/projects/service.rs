//! The logic of the projects: one transaction for each call, with the hooks.

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::authz::Object;
use scylla_domain::page::Page;
use scylla_domain::projects::{ListProjects, NewProject, Project, ProjectId, ProjectPage, KIND};
use scylla_extension::{CreateContext, DeleteContext, Gate, Hooks};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates a project if each `BeforeCreate` hook accepts it. Its owner is in its row only.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewProject,
) -> Result<Project, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.owner_id))
        .await?;
    let project = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(project)
}

/// Returns the owner of a project, or `None` if no project has this id.
pub(super) async fn owner(pool: &PgPool, id: ProjectId) -> Result<Option<ActorId>, Error> {
    let mut tx = pool.begin().await?;
    let owner = store::owner(&mut tx, id).await?;
    tx.commit().await?;
    Ok(owner)
}

/// Returns one page of the projects of an owner, oldest first.
pub(super) async fn list(pool: &PgPool, query: ListProjects) -> Result<ProjectPage, Error> {
    let mut tx = pool.begin().await?;
    let projects = store::list_by_owner(&mut tx, query.owner_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(projects, query.page.size, |project| {
        project.id
    }))
}

/// Deletes a project, runs the `AfterDelete` hooks, then removes the grants on it.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    gate: &dyn Gate,
    actor: Actor,
    id: ProjectId,
) -> Result<(), Error> {
    // Delete the row and run the hooks in one transaction.
    let mut tx = pool.begin().await?;
    let owner_id = store::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    hooks
        .after_delete(&mut tx, &DeleteContext::new(KIND, actor, owner_id))
        .await?;
    tx.commit().await?;

    // Remove the grants after the commit. A grant that stays reaches no project.
    if let Err(error) = gate.delete_object(&Object::of(id)).await {
        tracing::error!(%error, project = %id, "grants stay after a delete");
    }
    Ok(())
}
