//! The logic of the settings of the instance.

use scylla_domain::identity::api_tokens::PERSONAL_TOKEN_PREFIX;
use scylla_domain::platform::{InstanceSettings, InstanceSettingsChange};
use scylla_extension::Tx;
use sqlx::PgPool;

use super::store;
use crate::features::identity::api_tokens;
use crate::Error;

/// Returns the settings of the instance and the number of active personal tokens.
pub(super) async fn get(pool: &PgPool) -> Result<(InstanceSettings, u32), Error> {
    let mut tx = pool.begin().await?;
    let settings = store::get(&mut tx).await?;
    let active = api_tokens::count_active(&mut tx, PERSONAL_TOKEN_PREFIX).await?;
    tx.commit().await?;
    Ok((settings, active))
}

/// Changes the settings of the instance, and returns them with the number of active personal
/// tokens.
pub(super) async fn update(
    pool: &PgPool,
    change: InstanceSettingsChange,
) -> Result<(InstanceSettings, u32), Error> {
    let mut tx = pool.begin().await?;
    let settings = store::update(&mut tx, change).await?;
    let active = api_tokens::count_active(&mut tx, PERSONAL_TOKEN_PREFIX).await?;
    tx.commit().await?;
    tracing::info!(
        event = "instance.settings_changed",
        personal_tokens = settings.personal_tokens_enabled,
        workspace_limit = settings.workspace_limit
    );
    Ok((settings, active))
}

/// Returns the number of workspaces that one user can create, in the transaction of the caller.
pub(crate) async fn workspace_limit(tx: &mut Tx<'_>) -> Result<u32, sqlx::Error> {
    Ok(store::get(tx).await?.workspace_limit)
}

/// Returns `true` when the users can create and use personal tokens, in the transaction of the
/// caller.
pub(crate) async fn personal_tokens_enabled(tx: &mut Tx<'_>) -> Result<bool, sqlx::Error> {
    Ok(store::get(tx).await?.personal_tokens_enabled)
}
