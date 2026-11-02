//! The logic of the projects: one transaction for each call, with the hooks inside.

use scylla_domain::actors::Actor;
use scylla_domain::page::Page;
use scylla_domain::projects::{ListProjects, NewProject, Project, ProjectId, ProjectPage, KIND};
use scylla_extension::{CreateContext, DeleteContext, Hooks};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates a project if each `BeforeCreate` hook accepts it.
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

/// Returns one page of the projects of an owner, oldest first.
pub(super) async fn list(pool: &PgPool, query: ListProjects) -> Result<ProjectPage, Error> {
    let mut tx = pool.begin().await?;
    let projects = store::list_by_owner(&mut tx, query.owner_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(projects, query.page.size, |project| {
        project.id
    }))
}

/// Deletes a project, then runs the `AfterDelete` hooks before the commit.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: ProjectId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let owner_id = store::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    hooks
        .after_delete(&mut tx, &DeleteContext::new(KIND, actor, owner_id))
        .await?;
    tx.commit().await?;
    Ok(())
}
