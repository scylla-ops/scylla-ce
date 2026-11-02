use scylla_domain::projects::{NewProject, Project, ProjectId};
use scylla_domain::users::UserId;
use uuid::Uuid;

use crate::db::Tx;
use crate::Error;

pub async fn insert(tx: &mut Tx<'_>, new: &NewProject) -> Result<Project, Error> {
    let id = Uuid::now_v7();
    let row = sqlx::query!(
        "INSERT INTO projects (id, owner_id, name) VALUES ($1, $2, $3) RETURNING created_at",
        id,
        new.owner_id.0,
        new.name.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => Error::NotFound("owner"),
        error => error.into(),
    })?;

    Ok(Project {
        id: ProjectId(id),
        owner_id: new.owner_id,
        name: new.name.as_str().to_owned(),
        created_at: row.created_at,
    })
}

pub async fn list_by_owner(
    tx: &mut Tx<'_>,
    owner_id: UserId,
    after: Option<ProjectId>,
    limit: i64,
) -> Result<Vec<Project>, Error> {
    let rows = sqlx::query!(
        "SELECT id, owner_id, name, created_at FROM projects
         WHERE owner_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id
         LIMIT $3",
        owner_id.0,
        after.map(|id| id.0),
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Project {
            id: ProjectId(row.id),
            owner_id: UserId(row.owner_id),
            name: row.name,
            created_at: row.created_at,
        })
        .collect())
}

pub async fn delete(tx: &mut Tx<'_>, id: ProjectId) -> Result<Option<UserId>, Error> {
    let owner_id = sqlx::query_scalar!(
        "DELETE FROM projects WHERE id = $1 RETURNING owner_id",
        id.0
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(owner_id.map(UserId))
}
