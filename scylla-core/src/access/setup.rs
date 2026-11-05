//! How the gate starts: its settings, its keys, and the tokens of the command line.

use std::time::Duration;

use biscuit_auth::Biscuit;

use scylla_domain::authz::Schema;
use scylla_domain::identity::actors::Actor;
use scylla_extension::{AccessError, Binding, IssuedToken};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::claims::{sign, unauthenticated};
use super::gate::{Access, AccessConfig};
use super::rights::Rights;
use super::{store, SpiceDb, TokenKeys};
use crate::{Config, Error};

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
}
