//! The gate: verifies the Biscuit of each request, checks its revocation, then asks SpiceDB.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use biscuit_auth::builder::Term;
use biscuit_auth::{AuthorizerBuilder, Biscuit};
use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_extension::{
    async_trait, AccessError, Check, Gate, IssuedToken, Object, Principal, Relation,
};
use spicedb_grpc_tonic::v1::relationship_update::Operation;
use sqlx::PgPool;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;

use super::{store, SpiceDb, TokenKeys};
use crate::{Config, Error};

/// The header of the token, as `Bearer <token>`.
const HEADER: &str = "authorization";

/// The settings of the gate.
pub struct AccessConfig {
    /// The keys that sign and verify the tokens.
    pub keys: TokenKeys,
    /// The lifetime of a token.
    pub token_ttl: Duration,
    /// The SpiceDB client.
    pub spicedb: SpiceDb,
}

/// The gate of the server.
pub struct Access {
    /// The database pool, for the revocations.
    pool: PgPool,
    /// The keys of the tokens.
    keys: TokenKeys,
    /// The lifetime of a token.
    token_ttl: Duration,
    /// The SpiceDB client.
    spicedb: SpiceDb,
}

impl Access {
    /// Makes the gate.
    pub fn new(pool: PgPool, config: AccessConfig) -> Self {
        Self {
            pool,
            keys: config.keys,
            token_ttl: config.token_ttl,
            spicedb: config.spicedb,
        }
    }

    /// Makes the gate from the settings. The token key and the SpiceDB key are required.
    pub fn from_config(config: &Config, pool: PgPool) -> Result<Self, Error> {
        let private = Config::secret(&config.token_private_key, "SCYLLA_TOKEN_PRIVATE_KEY")?;
        let keys = TokenKeys::new(private, config.token_key_id, &config.token_public_keys)
            .map_err(|reason| Error::Config {
                name: "SCYLLA_TOKEN_PRIVATE_KEY",
                reason,
            })?;
        let key = Config::secret(&config.spicedb_key, "SCYLLA_SPICEDB_KEY")?;
        let spicedb =
            SpiceDb::new(&config.spicedb_endpoint, key).map_err(|reason| Error::Config {
                name: "SCYLLA_SPICEDB_ENDPOINT",
                reason,
            })?;
        let config = AccessConfig {
            keys,
            token_ttl: config.token_ttl,
            spicedb,
        };
        Ok(Self::new(pool, config))
    }

    /// Returns the SpiceDB client.
    pub fn spicedb(&self) -> &SpiceDb {
        &self.spicedb
    }

    /// Revokes a token and each token made from it. Returns the revocation ids in hex.
    pub async fn revoke(&self, text: &str) -> Result<Vec<String>, Error> {
        let token =
            Biscuit::from_base64(text, &self.keys).map_err(|_| unauthenticated("invalid token"))?;
        let ids = token.revocation_identifiers();
        let first = ids
            .first()
            .ok_or_else(|| unauthenticated("invalid token"))?;
        let mut tx = self.pool.begin().await?;
        store::revoke(&mut tx, first).await?;
        tx.commit().await?;
        Ok(ids.iter().take(1).map(hex::encode).collect())
    }

    /// Makes a token for an actor, valid for `ttl`.
    pub fn issue_for(&self, actor: &Actor, ttl: Duration) -> Result<IssuedToken, AccessError> {
        let now = OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap_or(OffsetDateTime::now_utc());
        let expires_at = now + ttl;
        let params = HashMap::from([
            ("id".to_owned(), Term::Str(actor.id.to_string())),
            ("kind".to_owned(), Term::Str(actor.kind.as_str().to_owned())),
            (
                "exp".to_owned(),
                Term::Date(expires_at.unix_timestamp() as u64),
            ),
            (
                "until".to_owned(),
                Term::Integer(expires_at.unix_timestamp()),
            ),
        ]);
        let token = Biscuit::builder()
            .code_with_params(
                "actor({id}, {kind}); expires({until}); check if time($t), $t <= {exp};",
                params,
                HashMap::new(),
            )
            .and_then(|builder| {
                builder
                    .root_key_id(self.keys.key_id())
                    .build(self.keys.signer())
            })
            .and_then(|token| token.to_base64())
            .map_err(|error| AccessError::Unavailable(format!("token signing failed: {error}")))?;
        Ok(IssuedToken {
            value: token,
            expires_at,
        })
    }

    /// Returns `true` if a revocation id of the token is revoked.
    async fn is_revoked(&self, token: &Biscuit) -> Result<bool, AccessError> {
        let ids = token.revocation_identifiers();
        let mut tx = self.pool.begin().await.map_err(database)?;
        let revoked = store::is_revoked(&mut tx, &ids).await.map_err(database)?;
        tx.commit().await.map_err(database)?;
        Ok(revoked)
    }
}

#[async_trait]
impl Gate for Access {
    async fn authenticate(&self, metadata: &MetadataMap) -> Result<Principal, AccessError> {
        let text = metadata
            .get(HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| unauthenticated("a Bearer token is required"))?;
        let token =
            Biscuit::from_base64(text, &self.keys).map_err(|_| unauthenticated("invalid token"))?;
        let (actor, expires) = claims(&token)?;
        if expires <= OffsetDateTime::now_utc().unix_timestamp() {
            return Err(unauthenticated("expired token"));
        }
        if self.is_revoked(&token).await? {
            return Err(unauthenticated("revoked token"));
        }
        Ok(Principal::new(actor, Arc::new(token)))
    }

    async fn authorize(&self, principal: &Principal, check: Check<'_>) -> Result<(), AccessError> {
        let token = principal
            .token()
            .downcast_ref::<Biscuit>()
            .ok_or_else(|| unauthenticated("unknown token type"))?;
        let params = HashMap::from([
            ("op".to_owned(), Term::Str(check.operation.to_owned())),
            ("type".to_owned(), Term::Str(check.object.kind.to_owned())),
            ("object".to_owned(), Term::Str(check.object.id.clone())),
        ]);
        AuthorizerBuilder::new()
            .time()
            .code_with_params(
                "operation({op}); resource({type}, {object}); allow if actor($id, $kind);",
                params,
                HashMap::new(),
            )
            .and_then(|builder| builder.build(token))
            .and_then(|mut authorizer| authorizer.authorize())
            .map_err(|_| {
                AccessError::Denied("the token does not allow this operation".to_owned())
            })?;
        let subject = Object::new("actor", principal.actor.id);
        let allowed = self
            .spicedb
            .check(
                &subject,
                check.object,
                check.permission,
                check.at_least_as_fresh,
            )
            .await?;
        if allowed {
            Ok(())
        } else {
            Err(AccessError::Denied(format!(
                "{} on {}:{} is denied",
                check.permission, check.object.kind, check.object.id
            )))
        }
    }

    fn issue(&self, actor: &Actor) -> Result<IssuedToken, AccessError> {
        self.issue_for(actor, self.token_ttl)
    }

    async fn write(&self, relations: &[Relation]) -> Result<String, AccessError> {
        self.spicedb.update(relations, Operation::Touch).await
    }

    async fn delete(&self, relations: &[Relation]) -> Result<(), AccessError> {
        self.spicedb.update(relations, Operation::Delete).await?;
        Ok(())
    }

    async fn delete_object(&self, object: &Object) -> Result<(), AccessError> {
        self.spicedb.delete_object(object).await
    }
}

/// Reads the actor and the expiry from the first block of a token. Later blocks cannot add them.
fn claims(token: &Biscuit) -> Result<(Actor, i64), AccessError> {
    let mut authorizer = AuthorizerBuilder::new()
        .build(token)
        .map_err(|_| unauthenticated("invalid token"))?;
    let actors: Vec<(String, String)> = authorizer
        .query("data($id, $kind) <- actor($id, $kind)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let expiries: Vec<(i64,)> = authorizer
        .query("data($exp) <- expires($exp)")
        .unwrap_or_default();
    let [(id, kind)] = actors.as_slice() else {
        return Err(unauthenticated("the token must name one actor"));
    };
    let id = ActorId::parse(id).map_err(|_| unauthenticated("invalid actor in the token"))?;
    let expires = expiries
        .first()
        .map(|(exp,)| *exp)
        .ok_or_else(|| unauthenticated("the token has no expiry"))?;
    let actor = Actor {
        id,
        kind: ActorKind::from_name(kind.clone()),
    };
    Ok((actor, expires))
}

/// Makes an `Unauthenticated` error.
fn unauthenticated(reason: &str) -> AccessError {
    AccessError::Unauthenticated(reason.to_owned())
}

/// Logs a database failure of the gate.
fn database(error: sqlx::Error) -> AccessError {
    tracing::error!(%error, "revocation check failed");
    AccessError::Unavailable("token check unavailable".to_owned())
}
