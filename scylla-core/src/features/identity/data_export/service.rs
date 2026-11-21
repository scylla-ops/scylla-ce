//! The export of the data of a user: one read-only transaction reads the rows of the core, the
//! hooks of the edition add their parts, and the rows become one JSON document.

use scylla_domain::identity::actors::ActorId;
use scylla_extension::{Hooks, UserContext};
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store;
use crate::Error;

/// The name of the structure of the document. A change of the structure changes it.
const FORMAT: &str = "scylla.account-data.v1";

/// Returns the data of a user as one JSON document, and the time of the export. An unknown user
/// gives `NotFound("user")`.
pub(crate) async fn export(
    pool: &PgPool,
    hooks: &Hooks,
    user: ActorId,
) -> Result<(String, OffsetDateTime), Error> {
    // Read the rows of the core in one snapshot.
    let mut tx = pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await?;
    let account = store::user(&mut tx, user)
        .await?
        .ok_or(Error::NotFound("user"))?;
    let emails = store::emails(&mut tx, user).await?;
    let identities = store::identities(&mut tx, user).await?;
    let sessions = store::sessions(&mut tx, user).await?;
    let personal_tokens = store::personal_tokens(&mut tx, user).await?;
    let interface_state = store::interface_state(&mut tx, user).await?;
    let workspaces = store::owned_workspaces(&mut tx, user).await?;
    let roles = store::roles(&mut tx, user).await?;

    // Add the parts of the edition.
    let parts = hooks
        .export_user(&mut tx, &UserContext::new(user, user))
        .await?;
    tx.commit().await?;
    let edition: Map<String, Value> = parts
        .into_iter()
        .map(|part| (part.name.to_owned(), part.data))
        .collect();

    // Write the document.
    let exported_at = OffsetDateTime::now_utc();
    let document = json!({
        "format": FORMAT,
        "exported_at": store::time(exported_at),
        "account": account,
        "emails": emails,
        "identities": identities,
        "sessions": sessions,
        "personal_tokens": personal_tokens,
        "interface_state": interface_state,
        "owned_workspaces": workspaces,
        "roles": roles,
        "edition": edition,
    });
    Ok((format!("{document:#}"), exported_at))
}
