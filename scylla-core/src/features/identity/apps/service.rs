//! The logic of the Apps: one transaction for each call. A delete or a key revocation also
//! closes the stream of the agent.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::apps::{
    App, AppId, AppKeyId, AppKeyInfo, AppPage, Host, JoinCode, KeyLabel, ListApps, NewApp, KIND,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{AccessError, Gate, Rejection, Tx};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::assertion;
use super::store::{apps, codes, keys};
use crate::features::identity::actors;
use crate::features::identity::agents::Registry;
use crate::features::identity::api_tokens;
use crate::features::projects::jobs::runs;
use crate::infra::secret;
use crate::Error;

/// The life of a join code.
pub const JOIN_CODE_TTL: Duration = Duration::minutes(15);

/// Creates an App as an actor of kind `app` in a workspace, with no key and no role, and gives
/// its first join code.
pub(crate) async fn create(pool: &PgPool, new: NewApp) -> Result<(App, JoinCode), Error> {
    let mut tx = pool.begin().await?;
    let actor = actors::insert(&mut tx, KIND).await?;
    let app = apps::insert(&mut tx, App::of_actor(actor.id), &new).await?;
    let code = new_code(&mut tx, app.id, new.created_by).await?;
    tx.commit().await?;
    let (id, workspace, runner) = (app.id, app.workspace_id, app.runner);
    tracing::info!(event = "app.created", app = %id, %workspace, runner);
    Ok((app, code))
}

/// Returns the workspace of an App, or `None` if no App has this id.
pub(crate) async fn workspace(pool: &PgPool, id: AppId) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = apps::workspace(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Returns an App, or `None` if no App has this id.
pub(crate) async fn get(pool: &PgPool, id: AppId) -> Result<Option<App>, Error> {
    let mut tx = pool.begin().await?;
    let app = apps::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(app)
}

/// Returns the keys of an App, oldest first.
pub(crate) async fn keys(pool: &PgPool, id: AppId) -> Result<Vec<AppKeyInfo>, Error> {
    let mut tx = pool.begin().await?;
    let keys = keys::list(&mut tx, id).await?;
    tx.commit().await?;
    Ok(keys)
}

/// Returns one page of the Apps of a workspace, oldest first.
pub(crate) async fn list(pool: &PgPool, query: ListApps) -> Result<AppPage, Error> {
    let mut tx = pool.begin().await?;
    let apps = apps::list(&mut tx, query.workspace_id, query.runner, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(apps, query.page.size, |app| app.id))
}

/// Deletes an App with its keys, its codes and its API tokens, then removes its roles and closes
/// its stream.
/// Its actor stays, so each `created_by` stays valid.
pub(crate) async fn delete(
    pool: &PgPool,
    gate: &dyn Gate,
    registry: &Registry,
    id: AppId,
) -> Result<(), Error> {
    // Delete the row in one transaction.
    let mut tx = pool.begin().await?;
    if !apps::delete(&mut tx, id).await? {
        return Err(Error::NotFound("app"));
    }
    api_tokens::delete_all(&mut tx, App::actor(id)).await?;
    tx.commit().await?;

    // Close the stream, then remove the roles. A role that stays gives nothing to a token.
    registry.stop(id, "the app was deleted");
    if let Err(error) = gate.delete_subject(&Object::of(App::actor(id))).await {
        tracing::error!(%error, app = %id, "roles stay after a delete");
    }
    tracing::info!(event = "app.deleted", app = %id);
    Ok(())
}

/// Disables an App and releases the jobs of its agent in one transaction, then closes its
/// stream. A second call keeps the first time. An unknown App gives `NotFound("app")`.
pub(crate) async fn disable(pool: &PgPool, registry: &Registry, id: AppId) -> Result<App, Error> {
    let mut tx = pool.begin().await?;
    let app = apps::set_disabled(&mut tx, id, true)
        .await?
        .ok_or(Error::NotFound("app"))?;
    let pooled = runs::release_all(&mut tx, id, "the agent was disabled").await?;
    tx.commit().await?;
    registry.stop(id, "the app was disabled");
    if pooled {
        registry.wake_all();
    }
    tracing::info!(event = "app.disabled", app = %id);
    Ok(app)
}

/// Enables a disabled App again. An unknown App gives `NotFound("app")`.
pub(crate) async fn enable(pool: &PgPool, id: AppId) -> Result<App, Error> {
    let mut tx = pool.begin().await?;
    let app = apps::set_disabled(&mut tx, id, false)
        .await?
        .ok_or(Error::NotFound("app"))?;
    tx.commit().await?;
    tracing::info!(event = "app.enabled", app = %id);
    Ok(app)
}

/// Gives a new join code for an App. The previous open codes of the App stop.
pub(crate) async fn join_code(
    pool: &PgPool,
    id: AppId,
    created_by: ActorId,
) -> Result<JoinCode, Error> {
    let mut tx = pool.begin().await?;
    let code = new_code(&mut tx, id, created_by).await?;
    tx.commit().await?;
    Ok(code)
}

/// Adds a public key to an App.
pub(crate) async fn add_key(
    pool: &PgPool,
    id: AppId,
    public_key: &[u8],
    label: Option<&KeyLabel>,
) -> Result<AppKeyInfo, Error> {
    assertion::check_public_key(public_key)?;
    let mut tx = pool.begin().await?;
    let key = keys::insert(&mut tx, id, public_key, label).await?;
    tx.commit().await?;
    tracing::info!(event = "app.key_added", app = %id, key = %key.id);
    Ok(key)
}

/// Gives a new name to a key of an App. An unknown key gives `NotFound("key")`.
pub(crate) async fn rename_key(
    pool: &PgPool,
    id: AppId,
    key: AppKeyId,
    label: &KeyLabel,
) -> Result<AppKeyInfo, Error> {
    let mut tx = pool.begin().await?;
    let renamed = keys::rename(&mut tx, id, key, label)
        .await?
        .ok_or(Error::NotFound("key"))?;
    tx.commit().await?;
    tracing::info!(event = "app.key_renamed", app = %id, %key);
    Ok(renamed)
}

/// Revokes a key of an App and closes its stream. An unknown key gives `NotFound("key")`.
pub(crate) async fn revoke_key(
    pool: &PgPool,
    registry: &Registry,
    id: AppId,
    key: AppKeyId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !keys::revoke(&mut tx, id, key).await? {
        return Err(Error::NotFound("key"));
    }
    tx.commit().await?;
    registry.stop(id, "a key of the app was revoked");
    tracing::info!(event = "app.key_revoked", app = %id, %key);
    Ok(())
}

/// Uses a join code: adds the public key of the machine to the App of the code. An unknown,
/// used or expired code gives `UNAUTHENTICATED`.
pub async fn join(
    pool: &PgPool,
    code: &str,
    public_key: &[u8],
    label: Option<&KeyLabel>,
) -> Result<(AppId, AppKeyId), Error> {
    assertion::check_public_key(public_key)?;
    let mut tx = pool.begin().await?;
    let app = codes::take(&mut tx, &secret::hash(code))
        .await?
        .ok_or_else(|| AccessError::Unauthenticated("invalid join code".to_owned()))?;
    let key = keys::insert(&mut tx, app, public_key, label).await?;
    tx.commit().await?;
    tracing::info!(event = "app.joined", %app, key = %key.id);
    Ok((app, key.id))
}

/// Returns the App of an actor that is an agent. Another actor gives `NOT_A_RUNNER`, and a
/// deleted App gives `UNAUTHENTICATED`.
pub(crate) async fn runner(pool: &PgPool, actor: &Actor) -> Result<AppId, Error> {
    let not_a_runner = || {
        let message = "only an app that runs jobs opens a stream";
        Error::Rejected(Rejection::new("NOT_A_RUNNER", message))
    };
    if actor.kind != KIND {
        return Err(not_a_runner());
    }
    let id = App::of_actor(actor.id);
    match get(pool, id).await? {
        Some(app) if app.runner => Ok(id),
        Some(_) => Err(not_a_runner()),
        None => Err(AccessError::Unauthenticated("the app was deleted".to_owned()).into()),
    }
}

/// Stores the host of the hello of an agent, and its last seen time.
pub(crate) async fn hello(pool: &PgPool, id: AppId, host: &Host) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    apps::set_host(&mut tx, id, host).await?;
    tx.commit().await?;
    Ok(())
}

/// Sets the last seen time of an agent to now.
pub(crate) async fn touch(pool: &PgPool, id: AppId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    apps::touch(&mut tx, id).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns the workspace of an actor that is an App, or `None` for another actor.
pub(crate) async fn workspace_of(
    pool: &PgPool,
    actor: ActorId,
) -> Result<Option<WorkspaceId>, Error> {
    workspace(pool, App::of_actor(actor)).await
}

/// Makes a join code for an App, in the transaction of the caller, and keeps only its hash.
async fn new_code(tx: &mut Tx<'_>, id: AppId, created_by: ActorId) -> Result<JoinCode, Error> {
    let code = secret::token();
    let expires_at = OffsetDateTime::now_utc() + JOIN_CODE_TTL;
    codes::replace(tx, id, &secret::hash(&code), created_by, expires_at).await?;
    Ok(JoinCode { code, expires_at })
}
