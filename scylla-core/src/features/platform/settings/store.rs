//! The SQL of the `instance_settings` table: one row.

use scylla_domain::platform::InstanceSettings;
use scylla_extension::Tx;

/// Returns the settings of the instance.
pub(super) async fn get(tx: &mut Tx<'_>) -> Result<InstanceSettings, sqlx::Error> {
    let enabled = sqlx::query_scalar!("SELECT personal_tokens_enabled FROM instance_settings")
        .fetch_one(&mut **tx)
        .await?;
    Ok(InstanceSettings {
        personal_tokens_enabled: enabled,
    })
}

/// Changes the settings of the instance: a value that is `None` does not change. Returns the
/// settings.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    personal_tokens_enabled: Option<bool>,
) -> Result<InstanceSettings, sqlx::Error> {
    let enabled = sqlx::query_scalar!(
        "UPDATE instance_settings
         SET personal_tokens_enabled = coalesce($1, personal_tokens_enabled)
         RETURNING personal_tokens_enabled",
        personal_tokens_enabled,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(InstanceSettings {
        personal_tokens_enabled: enabled,
    })
}
