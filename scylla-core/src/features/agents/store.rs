//! The SQL of the agents, their keys, and the assertion nonces.

use scylla_domain::actors::ActorId;
use scylla_domain::agents::AgentKeyId;
use scylla_domain::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// A public key of an agent.
pub(super) struct AgentKeyRow {
    /// The agent of the key.
    pub(super) agent_id: ActorId,
    /// The Ed25519 public key, 32 bytes.
    pub(super) public_key: Vec<u8>,
    /// `true` if the key is revoked.
    pub(super) revoked: bool,
}

/// Marks an actor as an agent of a workspace, so it can have keys.
pub(super) async fn insert_agent(
    tx: &mut Tx<'_>,
    agent_id: ActorId,
    workspace_id: WorkspaceId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO agents (id, workspace_id) VALUES ($1, $2)",
        agent_id.to_db(),
        workspace_id.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts a public key of an agent.
pub(super) async fn insert_key(
    tx: &mut Tx<'_>,
    key_id: AgentKeyId,
    agent_id: ActorId,
    public_key: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO agent_keys (id, agent_id, public_key) VALUES ($1, $2, $3)",
        key_id.to_db(),
        agent_id.to_db(),
        public_key,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns a key, or `None` if it is unknown.
pub(super) async fn find_key(
    tx: &mut Tx<'_>,
    key_id: AgentKeyId,
) -> Result<Option<AgentKeyRow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT agent_id, public_key, revoked_at IS NOT NULL AS "revoked!"
           FROM agent_keys WHERE id = $1"#,
        key_id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| AgentKeyRow {
        agent_id: ActorId::from_db(row.agent_id),
        public_key: row.public_key,
        revoked: row.revoked,
    }))
}

/// Records a nonce. Returns `false` if the nonce is already known: a replay.
pub(super) async fn use_nonce(
    tx: &mut Tx<'_>,
    nonce: &str,
    expires_at: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    // Forget the nonces of the expired assertions.
    sqlx::query!("DELETE FROM assertion_nonces WHERE expires_at < now()")
        .execute(&mut **tx)
        .await?;

    // Add the nonce. A known nonce adds no row.
    let result = sqlx::query!(
        "INSERT INTO assertion_nonces (nonce, expires_at) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        nonce,
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}
