//! The gate: verifies the Biscuit of each request and its revocation, then finds the rights.

use std::sync::Arc;
use std::time::Duration;

use biscuit_auth::Biscuit;
use scylla_domain::authz::{Check, Object, Relationship, Schema};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::{apps, users};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{
    async_trait, AccessError, Binding, Gate, IssuedToken, Operation, Principal, RolePage,
};
use spicedb_grpc_tonic::v1::relationship_update::Operation as Update;
use sqlx::PgPool;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;

use super::claims::{allows, claims, sign, unauthenticated, Claims};
use super::rights::Rights;
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
    /// The permission model of the edition.
    pub schema: Schema,
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
    /// The owner chains and the grants.
    rights: Rights,
}

impl Access {
    /// Makes the gate.
    pub fn new(pool: PgPool, config: AccessConfig) -> Self {
        let rights = Rights {
            pool: pool.clone(),
            spicedb: config.spicedb.clone(),
            schema: config.schema,
        };
        Self {
            pool,
            keys: config.keys,
            token_ttl: config.token_ttl,
            spicedb: config.spicedb,
            rights,
        }
    }

    /// Makes the gate from the settings and the permission model. The token key and the
    /// SpiceDB key are required.
    pub fn from_config(config: &Config, pool: PgPool, schema: Schema) -> Result<Self, Error> {
        // Read the keys that sign and verify the tokens.
        let private = Config::secret(&config.token_private_key, "SCYLLA_TOKEN_PRIVATE_KEY")?;
        let keys = TokenKeys::new(private, config.token_key_id, &config.token_public_keys)
            .map_err(|reason| Error::Config {
                name: "SCYLLA_TOKEN_PRIVATE_KEY",
                reason,
            })?;

        // Make the SpiceDB client, then the gate.
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
            schema,
        };
        Ok(Self::new(pool, config))
    }

    /// Returns the SpiceDB client.
    pub fn spicedb(&self) -> &SpiceDb {
        &self.spicedb
    }

    /// Revokes a token and each token made from it. Returns the revocation ids in hex.
    pub async fn revoke(&self, text: &str) -> Result<Vec<String>, Error> {
        // Read the first revocation id: the tokens made from this token have it too.
        let token =
            Biscuit::from_base64(text, &self.keys).map_err(|_| unauthenticated("invalid token"))?;
        let ids = token.revocation_identifiers();
        let first = ids
            .first()
            .ok_or_else(|| unauthenticated("invalid token"))?;

        // Store it as revoked.
        let mut tx = self.pool.begin().await?;
        store::revoke(&mut tx, first).await?;
        tx.commit().await?;
        Ok(ids.iter().take(1).map(hex::encode).collect())
    }

    /// Makes a token for an actor, valid for `ttl`, bound to a session or a key.
    pub fn issue_for(
        &self,
        actor: &Actor,
        ttl: Duration,
        binding: Binding,
    ) -> Result<IssuedToken, AccessError> {
        // Set the expiry, in whole seconds, then sign the claims.
        let now = OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap_or(OffsetDateTime::now_utc());
        let expires_at = now + ttl;
        let value = sign(&self.keys, actor, expires_at, binding)?;
        Ok(IssuedToken { value, expires_at })
    }

    /// Returns `true` if a token cannot pass: a revocation id of it is revoked, its session or
    /// its key is revoked, its session is expired, or its user or App is gone or disabled.
    async fn is_refused(&self, token: &Biscuit, claims: &Claims) -> Result<bool, AccessError> {
        let ids = token.revocation_identifiers();
        let facts = store::TokenFacts {
            ids: &ids,
            actor: claims.actor.id,
            user: claims.actor.kind == users::KIND,
            app: claims.actor.kind == apps::KIND,
            session: claims.session(),
            key: claims.key(),
        };
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let refused = store::is_refused(&mut tx, &facts)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(refused)
    }
}

#[async_trait]
impl Gate for Access {
    async fn authenticate(
        &self,
        metadata: &MetadataMap,
        operation: &Operation,
    ) -> Result<Principal, AccessError> {
        // Read the Bearer token and verify its signature.
        let text = metadata
            .get(HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(|| unauthenticated("a Bearer token is required"))?;
        let token =
            Biscuit::from_base64(text, &self.keys).map_err(|_| unauthenticated("invalid token"))?;

        // Refuse an expired token, and a token that a revocation or a sign-out stopped.
        let claims = claims(&token)?;
        if claims.expires <= OffsetDateTime::now_utc().unix_timestamp() {
            return Err(unauthenticated("expired token"));
        }
        if self.is_refused(&token, &claims).await? {
            return Err(unauthenticated("revoked token"));
        }

        // Record the token on the operation, for the route rule, and the actor on the call.
        operation.set_authenticated();
        let call = tracing::Span::current();
        call.record("actor", tracing::field::display(claims.actor.id));
        call.record("actor_kind", claims.actor.kind.as_str());
        let token = Arc::new(token);
        let expires_at = OffsetDateTime::from_unix_timestamp(claims.expires)
            .map_err(|_| unauthenticated("invalid expiry"))?;
        Ok(Principal::new(
            claims.actor,
            operation.clone(),
            token,
            claims.binding,
            expires_at,
        ))
    }

    async fn authorize(&self, principal: &Principal, check: Check) -> Result<(), AccessError> {
        // Refuse a check of a permission that is not the route permission.
        let operation = principal.operation();
        if check.permission() != operation.permission() {
            tracing::error!(
                operation = operation.as_str(),
                permission = check.permission(),
                "a route checks a permission that is not its own"
            );
            return Err(AccessError::Denied(
                "the route checks a permission that is not its own".to_owned(),
            ));
        }

        // Ask the token: its checks see the operation and the object.
        let token = principal
            .token()
            .downcast_ref::<Biscuit>()
            .ok_or_else(|| unauthenticated("unknown token type"))?;
        if !allows(token, operation, &check) {
            let reason = "the token does not allow this operation".to_owned();
            let object = check.object();
            return Err(AccessError::refused(
                principal,
                check.permission(),
                object,
                reason,
            ));
        }

        // Pass an owner or a grant, and record a passed check for the route rule.
        if self.rights.allow(principal.actor.id, &check).await? {
            operation.set_authorized();
            Ok(())
        } else {
            Err(AccessError::denied(principal, &check))
        }
    }

    fn issue(&self, actor: &Actor, binding: Binding) -> Result<IssuedToken, AccessError> {
        self.issue_for(actor, self.token_ttl, binding)
    }

    async fn write(&self, relationships: &[Relationship]) -> Result<String, AccessError> {
        self.spicedb.update(relationships, Update::Touch).await
    }

    async fn delete(&self, relationships: &[Relationship]) -> Result<(), AccessError> {
        self.spicedb.update(relationships, Update::Delete).await?;
        Ok(())
    }

    async fn delete_object(&self, object: &Object) -> Result<(), AccessError> {
        self.spicedb.delete_object(object).await
    }

    async fn delete_subject(&self, subject: &Object) -> Result<(), AccessError> {
        self.spicedb.delete_subject(subject).await
    }

    async fn subjects(&self, object: &Object) -> Result<Vec<ActorId>, AccessError> {
        let ids = self.spicedb.subjects(object).await?;
        Ok(ids
            .iter()
            .filter_map(|id| ActorId::parse(id).ok())
            .collect())
    }

    async fn roles(
        &self,
        object: &Object,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Option<RolePage>, AccessError> {
        self.spicedb.roles(object, cursor, count).await
    }

    async fn permissions(
        &self,
        principal: &Principal,
        object: &Object,
        workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        let caller = principal.actor.id;
        self.rights
            .answers(caller, object, workspace, permissions, None)
            .await
    }

    async fn lookup(
        &self,
        principal: &Principal,
        definition: &'static str,
        permission: &'static str,
    ) -> Result<Vec<String>, AccessError> {
        let subject = Object::of(principal.actor.id);
        self.spicedb.lookup(&subject, definition, permission).await
    }
}
