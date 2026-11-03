//! The logic of the secrets: one transaction for each call. Each value is sealed with the key of
//! the server and the id of its secret as associated data, so a sealed value copied to another
//! secret does not open.

use scylla_domain::projects::secrets::{
    ListSecrets, NewSecret, Secret, SecretId, SecretName, SecretPage, SecretValue,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::Rejection;
use sqlx::PgPool;

use super::store;
use crate::infra::secret::Sealer;
use crate::Error;

/// Creates a secret: a new id, then the value sealed with this id. A server with no key gives
/// `SECRET_KEY_MISSING`.
pub(super) async fn create(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    new: NewSecret,
) -> Result<Secret, Error> {
    let id = SecretId::generate();
    let sealed = key(sealer)?.seal_with(new.value.expose().as_bytes(), &id.to_bytes());
    let mut tx = pool.begin().await?;
    let secret = store::insert(&mut tx, id, &new, &sealed).await?;
    tx.commit().await?;
    let (project, name) = (secret.project_id, secret.name.as_str());
    tracing::info!(event = "secret.created", secret = %id, %project, name);
    Ok(secret)
}

/// Gives a secret a new value, and returns it. The last change wins. A server with no key gives
/// `SECRET_KEY_MISSING`.
pub(super) async fn update(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    id: SecretId,
    value: SecretValue,
) -> Result<Secret, Error> {
    let sealed = key(sealer)?.seal_with(value.expose().as_bytes(), &id.to_bytes());
    let mut tx = pool.begin().await?;
    let secret = store::update_value(&mut tx, id, &sealed)
        .await?
        .ok_or(Error::NotFound("secret"))?;
    tx.commit().await?;
    let (project, name) = (secret.project_id, secret.name.as_str());
    tracing::info!(event = "secret.updated", secret = %id, %project, name);
    Ok(secret)
}

/// Deletes a secret. A pipeline that names it fails at its next run.
pub(super) async fn delete(pool: &PgPool, id: SecretId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !store::delete(&mut tx, id).await? {
        return Err(Error::NotFound("secret"));
    }
    tx.commit().await?;
    tracing::info!(event = "secret.deleted", secret = %id);
    Ok(())
}

/// Returns one page of the secrets of a project, in name order, with no value.
pub(super) async fn list(pool: &PgPool, query: ListSecrets) -> Result<SecretPage, Error> {
    let mut tx = pool.begin().await?;
    let secrets = store::list(&mut tx, query.project_id, &query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(secrets, query.page.size, |secret| {
        secret.name.clone()
    }))
}

/// Returns the project of a secret and its workspace, or `None` if no secret has this id.
pub(super) async fn place(
    pool: &PgPool,
    id: SecretId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    let mut tx = pool.begin().await?;
    let place = store::place(&mut tx, id).await?;
    tx.commit().await?;
    Ok(place)
}

/// Opens the value of the secret of a project with this name, for the run of a pipeline. An
/// unknown name gives `NotFound("secret")`, and a server with no key `SECRET_KEY_MISSING`.
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) async fn reveal(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    project: ProjectId,
    name: &SecretName,
) -> Result<SecretValue, Error> {
    // Read the sealed value.
    let sealer = key(sealer)?;
    let mut tx = pool.begin().await?;
    let (id, sealed) = store::sealed(&mut tx, project, name)
        .await?
        .ok_or(Error::NotFound("secret"))?;
    tx.commit().await?;

    // Open it with the id of its secret.
    let plain = sealer
        .open_with(&sealed, &id.to_bytes())
        .map_err(|reason| Error::Internal(format!("secret {id}: {reason}")))?;
    let text = String::from_utf8(plain)
        .map_err(|_| Error::Internal(format!("secret {id}: the value is not UTF-8")))?;
    SecretValue::parse(text).map_err(|error| Error::Internal(format!("secret {id}: {error}")))
}

/// Returns the key of the secrets, or `SECRET_KEY_MISSING` if the server has none.
fn key(sealer: Option<&Sealer>) -> Result<&Sealer, Error> {
    sealer.ok_or_else(|| {
        let message = "the server has no key for the secrets: set SCYLLA_SECRET_KEY";
        Error::Rejected(Rejection::new("SECRET_KEY_MISSING", message))
    })
}
