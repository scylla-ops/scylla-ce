use scylla_domain::projects::{ListProjects, NewProject, Project, ProjectId, ProjectPage, KIND};
use scylla_extension::{CreateContext, DeleteContext, ResourceGuard};
use sqlx::PgPool;

use super::store;
use crate::Error;

pub async fn create(
    pool: &PgPool,
    guard: &dyn ResourceGuard,
    new: NewProject,
) -> Result<Project, Error> {
    let mut tx = pool.begin().await?;
    guard
        .before_create(&mut tx, &CreateContext::new(KIND, new.owner_id))
        .await?;
    let project = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(project)
}

pub async fn list(pool: &PgPool, query: ListProjects) -> Result<ProjectPage, Error> {
    let mut tx = pool.begin().await?;
    let fetch = query.limit as i64 + 1;
    let mut projects = store::list_by_owner(&mut tx, query.owner_id, query.after, fetch).await?;
    tx.commit().await?;

    let next_after = if projects.len() > query.limit {
        projects.truncate(query.limit);
        projects.last().map(|project| project.id)
    } else {
        None
    };
    Ok(ProjectPage {
        projects,
        next_after,
    })
}

pub async fn delete(pool: &PgPool, guard: &dyn ResourceGuard, id: ProjectId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let owner_id = store::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("project"))?;
    guard
        .after_delete(&mut tx, &DeleteContext::new(KIND, owner_id))
        .await?;
    tx.commit().await?;
    Ok(())
}
