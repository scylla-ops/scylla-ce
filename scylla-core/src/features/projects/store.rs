//! The SQL of the `projects` table. Each function runs in the transaction of the caller.

use scylla_domain::actors::ActorId;
use scylla_domain::page::PageRequest;
use scylla_domain::projects::{NewProject, Project, ProjectId};
use scylla_extension::Tx;

use crate::Error;

/// The foreign key from a project to its owner.
const OWNER_FOREIGN_KEY: &str = "projects_owner_id_fkey";

/// Inserts a project and returns it. An unknown owner gives `NotFound("owner")`.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewProject) -> Result<Project, Error> {
    let id = ProjectId::generate();
    let row = sqlx::query!(
        "INSERT INTO projects (id, owner_id, created_by, name) VALUES ($1, $2, $3, $4)
         RETURNING created_at",
        id.to_db(),
        new.owner_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(OWNER_FOREIGN_KEY) => {
            Error::NotFound("owner")
        }
        error => error.into(),
    })?;

    Ok(Project {
        id,
        owner_id: new.owner_id,
        created_by: new.created_by,
        name: new.name.as_str().to_owned(),
        created_at: row.created_at,
    })
}

/// Returns the projects of an owner after the page id, in id order, with one extra row.
pub(super) async fn list_by_owner(
    tx: &mut Tx<'_>,
    owner_id: ActorId,
    page: PageRequest<ProjectId>,
) -> Result<Vec<Project>, Error> {
    let rows = sqlx::query!(
        "SELECT id, owner_id, created_by, name, created_at FROM projects
         WHERE owner_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id
         LIMIT $3",
        owner_id.to_db(),
        page.after.map(ProjectId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Project {
            id: ProjectId::from_db(row.id),
            owner_id: ActorId::from_db(row.owner_id),
            created_by: ActorId::from_db(row.created_by),
            name: row.name,
            created_at: row.created_at,
        })
        .collect())
}

/// Deletes a project and returns its owner, or `None` if no project has this id.
pub(super) async fn delete(tx: &mut Tx<'_>, id: ProjectId) -> Result<Option<ActorId>, Error> {
    let owner_id = sqlx::query_scalar!(
        "DELETE FROM projects WHERE id = $1 RETURNING owner_id",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(owner_id.map(ActorId::from_db))
}
