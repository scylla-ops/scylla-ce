//! The logic of the agents: registration, and the check of a signed assertion.

use ed25519_dalek::{Signature, VerifyingKey};
use prost::Message;
use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::agents::{AgentKeyId, KIND};
use scylla_extension::{AccessError, Gate};
use scylla_proto::auth::v1::AgentAssertion;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::store;
use crate::features::actors;
use crate::parse;
use crate::{access, Error};

/// The audience that each assertion must name.
pub const AUDIENCE: &str = "scylla";

/// The longest life of an assertion.
const MAX_LIFETIME: Duration = Duration::minutes(2);

/// The shortest nonce.
const MIN_NONCE_LEN: usize = 16;

/// Registers an agent for an actor, with the public key of the agent.
pub async fn register(
    pool: &PgPool,
    gate: &dyn Gate,
    owner: ActorId,
    public_key: &[u8],
) -> Result<(ActorId, AgentKeyId), Error> {
    let bytes: [u8; 32] = public_key
        .try_into()
        .map_err(|_| Error::invalid("public_key", "must be an Ed25519 public key"))?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| Error::invalid("public_key", "must be an Ed25519 public key"))?;
    let mut tx = pool.begin().await?;
    let agent = actors::insert(&mut tx, KIND).await?;
    store::insert_agent(&mut tx, agent.id, owner).await?;
    let key_id = AgentKeyId::generate();
    store::insert_key(&mut tx, key_id, agent.id, public_key).await?;
    let grants = [
        Actor::IDENTITY.of(agent.id, agent.id),
        Actor::DELEGATE.of(agent.id, owner),
    ];
    access::commit(tx, gate, &grants).await?;
    Ok((agent.id, key_id))
}

/// Checks an assertion that an agent signed, and returns the agent.
pub async fn verify_assertion(
    pool: &PgPool,
    assertion: &[u8],
    signature: &[u8],
) -> Result<Actor, Error> {
    let claims = AgentAssertion::decode(assertion).map_err(|_| refused())?;
    let agent_id: ActorId = parse::required(claims.agent_id, "agent_id").map_err(|_| refused())?;
    let key_id: AgentKeyId = parse::required(claims.key_id, "key_id").map_err(|_| refused())?;
    let expires_at = claims
        .expires_at
        .and_then(|at| OffsetDateTime::from_unix_timestamp(at.seconds).ok())
        .ok_or_else(refused)?;
    let now = OffsetDateTime::now_utc();
    if claims.audience != AUDIENCE
        || expires_at <= now
        || expires_at > now + MAX_LIFETIME
        || claims.nonce.len() < MIN_NONCE_LEN
    {
        return Err(refused());
    }
    let mut tx = pool.begin().await?;
    let key = store::find_key(&mut tx, key_id)
        .await?
        .filter(|key| key.agent_id == agent_id && !key.revoked)
        .ok_or_else(refused)?;
    let public_key: [u8; 32] = key.public_key.try_into().map_err(|_| refused())?;
    let public_key = VerifyingKey::from_bytes(&public_key).map_err(|_| refused())?;
    let signature = Signature::from_slice(signature).map_err(|_| refused())?;
    public_key
        .verify_strict(assertion, &signature)
        .map_err(|_| refused())?;
    if !store::use_nonce(&mut tx, &claims.nonce, expires_at).await? {
        return Err(refused());
    }
    tx.commit().await?;
    Ok(Actor {
        id: agent_id,
        kind: KIND,
    })
}

/// The error of an assertion that does not pass. It gives no detail to the client.
fn refused() -> Error {
    AccessError::Unauthenticated("invalid assertion".to_owned()).into()
}
