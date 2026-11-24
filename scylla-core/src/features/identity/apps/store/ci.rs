//! The SQL of the `ci_identities` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::{TokenLimits, TokenPreset};
use scylla_domain::identity::apps::AppId;
use scylla_domain::identity::ci_identities::{
    CiIdentityId, CiIdentityInfo, CiProvider, NewCiIdentity,
};
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// Inserts a CI identity of an App, and returns it.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    app: AppId,
    new: &NewCiIdentity,
    created_by: ActorId,
) -> Result<CiIdentityInfo, sqlx::Error> {
    let projects: Vec<uuid::Uuid> = new.limits.projects.iter().map(|id| id.to_db()).collect();
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO ci_identities (id, app_id, provider, issuer, repository, git_ref, environment,
                                    preset, actions, project_ids, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING id, app_id, provider, issuer, repository, git_ref, environment, preset,
                   actions, project_ids, created_by, created_at, last_used_at",
        CiIdentityId::generate().to_db(),
        app.to_db(),
        new.provider.as_str(),
        new.issuer,
        new.repository,
        new.git_ref,
        new.environment,
        new.limits.preset.as_str(),
        &new.limits.actions,
        &projects,
        created_by.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_info())
}

/// Returns the CI identities of an App, the oldest first.
pub(crate) async fn list(tx: &mut Tx<'_>, app: AppId) -> Result<Vec<CiIdentityInfo>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, app_id, provider, issuer, repository, git_ref, environment, preset, actions,
                project_ids, created_by, created_at, last_used_at
         FROM ci_identities WHERE app_id = $1 ORDER BY created_at, id",
        app.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_info).collect())
}

/// Removes a CI identity of an App. Returns `false` if the App has no CI identity with this id.
pub(crate) async fn remove(
    tx: &mut Tx<'_>,
    app: AppId,
    id: CiIdentityId,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM ci_identities WHERE id = $1 AND app_id = $2",
        id.to_db(),
        app.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Sets the last use of a CI identity to now. Returns `false` if it is gone.
pub(crate) async fn touch(tx: &mut Tx<'_>, id: CiIdentityId) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE ci_identities SET last_used_at = now() WHERE id = $1",
        id.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// A row of the `ci_identities` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The App.
    app_id: uuid::Uuid,
    /// The name of the provider.
    provider: String,
    /// The issuer.
    issuer: String,
    /// The repository.
    repository: String,
    /// The Git ref.
    git_ref: Option<String>,
    /// The environment.
    environment: Option<String>,
    /// The name of the preset.
    preset: String,
    /// The methods of a custom preset.
    actions: Vec<String>,
    /// The projects.
    project_ids: Vec<uuid::Uuid>,
    /// The actor that added it.
    created_by: uuid::Uuid,
    /// The time when it was added.
    created_at: OffsetDateTime,
    /// The last exchange.
    last_used_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the CI identity of the row.
    fn into_info(self) -> CiIdentityInfo {
        CiIdentityInfo {
            id: CiIdentityId::from_db(self.id),
            app_id: AppId::from_db(self.app_id),
            provider: CiProvider::from_name(&self.provider),
            issuer: self.issuer,
            repository: self.repository,
            git_ref: self.git_ref,
            environment: self.environment,
            limits: TokenLimits {
                preset: TokenPreset::from_name(&self.preset),
                actions: self.actions,
                projects: self
                    .project_ids
                    .into_iter()
                    .map(ProjectId::from_db)
                    .collect(),
            },
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }
}
