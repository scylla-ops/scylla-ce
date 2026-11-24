//! The SQL of the `instance_settings` table: one row.

use scylla_domain::platform::{InstanceSettings, InstanceSettingsChange};
use scylla_extension::Tx;

/// A row of the settings.
struct Row {
    /// `true` when the users can create and use personal tokens.
    personal_tokens_enabled: bool,
    /// The number of workspaces that one user can create.
    workspace_limit: i32,
}

impl From<Row> for InstanceSettings {
    fn from(row: Row) -> Self {
        Self {
            personal_tokens_enabled: row.personal_tokens_enabled,
            workspace_limit: u32::try_from(row.workspace_limit).unwrap_or(0),
        }
    }
}

/// Returns the settings of the instance.
pub(super) async fn get(tx: &mut Tx<'_>) -> Result<InstanceSettings, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT personal_tokens_enabled, workspace_limit FROM instance_settings"
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into())
}

/// Changes the settings of the instance: a value that is `None` does not change. Returns the
/// settings. The limit fits in an `i32`: the route checks it.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    change: InstanceSettingsChange,
) -> Result<InstanceSettings, sqlx::Error> {
    let limit = change
        .workspace_limit
        .and_then(|limit| i32::try_from(limit).ok());
    let row = sqlx::query_as!(
        Row,
        "UPDATE instance_settings
         SET personal_tokens_enabled = coalesce($1, personal_tokens_enabled),
             workspace_limit = coalesce($2, workspace_limit)
         RETURNING personal_tokens_enabled, workspace_limit",
        change.personal_tokens_enabled,
        limit,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into())
}
