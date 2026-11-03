//! The SQL of the `secrets` table. Each function runs in the transaction of the caller. A value
//! is sealed before it comes here: the store never sees it in clear.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::secrets::{NewSecret, Secret, SecretId, SecretName};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;

use crate::Error;

/// The unique constraint of the names in a project.
const NAME_KEY: &str = "secrets_project_id_name_key";

/// The foreign key from a secret to its project.
const PROJECT_FOREIGN_KEY: &str = "secrets_project_id_fkey";

/// Inserts a secret with its sealed value, and returns it. A name in use in the project gives
/// `SECRET_NAME_IN_USE`, and an unknown project gives `NotFound("project")`.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    id: SecretId,
    new: &NewSecret,
    sealed: &[u8],
) -> Result<Secret, Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO secrets (id, project_id, name, sealed_value, created_by)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, project_id, name, created_by, created_at, updated_at",
        id.to_db(),
        new.project_id.to_db(),
        new.name.as_str(),
        sealed,
        new.created_by.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => name_in_use(),
        sqlx::Error::Database(db) if db.constraint() == Some(PROJECT_FOREIGN_KEY) => {
            Error::NotFound("project")
        }
        error => error.into(),
    })?;
    row.into_secret()
}

/// Replaces the sealed value of a secret, and returns the secret, or `None` if no secret has
/// this id.
pub(super) async fn update_value(
    tx: &mut Tx<'_>,
    id: SecretId,
    sealed: &[u8],
) -> Result<Option<Secret>, Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE secrets SET sealed_value = $2, updated_at = now()
         WHERE id = $1
         RETURNING id, project_id, name, created_by, created_at, updated_at",
        id.to_db(),
        sealed,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_secret).transpose()
}

/// Deletes a secret. Returns `false` if no secret has this id.
pub(super) async fn delete(tx: &mut Tx<'_>, id: SecretId) -> Result<bool, Error> {
    let deleted = sqlx::query!("DELETE FROM secrets WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

/// Returns the secrets of a project after the name of the page, in name order, with one extra
/// row.
pub(super) async fn list(
    tx: &mut Tx<'_>,
    project: ProjectId,
    page: &PageRequest<SecretName>,
) -> Result<Vec<Secret>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, project_id, name, created_by, created_at, updated_at FROM secrets
         WHERE project_id = $1 AND ($2::text IS NULL OR name > $2)
         ORDER BY name
         LIMIT $3",
        project.to_db(),
        page.after.as_ref().map(SecretName::as_str),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_secret).collect()
}

/// Returns the project of a secret and the workspace of that project, or `None` if no secret
/// has this id.
pub(super) async fn place(
    tx: &mut Tx<'_>,
    id: SecretId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    let row = sqlx::query!(
        "SELECT s.project_id, p.workspace_id FROM secrets s
         JOIN projects p ON p.id = s.project_id
         WHERE s.id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        (
            ProjectId::from_db(row.project_id),
            WorkspaceId::from_db(row.workspace_id),
        )
    }))
}

/// Returns the id and the sealed value of the secret of a project with this name, or `None` if
/// the project has no such secret.
pub(super) async fn sealed(
    tx: &mut Tx<'_>,
    project: ProjectId,
    name: &SecretName,
) -> Result<Option<(SecretId, Vec<u8>)>, Error> {
    let row = sqlx::query!(
        "SELECT id, sealed_value FROM secrets WHERE project_id = $1 AND name = $2",
        project.to_db(),
        name.as_str(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| (SecretId::from_db(row.id), row.sealed_value)))
}

/// A row of the `secrets` table, with no value.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The project.
    project_id: uuid::Uuid,
    /// The name.
    name: String,
    /// The author.
    created_by: uuid::Uuid,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The time of the last change of the value.
    updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the secret of the row. The table checks the form of the name, so a bad name is an
    /// internal error.
    fn into_secret(self) -> Result<Secret, Error> {
        let name = SecretName::parse(self.name)
            .map_err(|error| Error::Internal(format!("a stored secret name {error}")))?;
        Ok(Secret {
            id: SecretId::from_db(self.id),
            project_id: ProjectId::from_db(self.project_id),
            name,
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

/// The refusal of a name that another secret of the project uses.
fn name_in_use() -> Error {
    Error::Rejected(Rejection::new(
        "SECRET_NAME_IN_USE",
        "another secret of the project uses this name",
    ))
}
