//! The rights of an actor on an object: ownership from Postgres, then grants from SpiceDB.

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::authz::{Check, Object, Resource, Schema};
use scylla_extension::AccessError;
use sqlx::PgPool;

use super::{store, SpiceDb};

/// Finds the rights of an actor: the owner chain in Postgres, and the grants in SpiceDB.
pub(super) struct Rights {
    /// The database pool, for the owner chains.
    pub(super) pool: PgPool,
    /// The SpiceDB client, for the grants.
    pub(super) spicedb: SpiceDb,
    /// The permission model: it tells which definitions have a permission.
    pub(super) schema: Schema,
}

impl Rights {
    /// Returns `true` if the caller is an owner of the object, through the owner chain, or has a
    /// grant of the permission on the object or on one of its owners.
    pub(super) async fn allow(&self, caller: ActorId, check: &Check) -> Result<bool, AccessError> {
        // Find the first owner: the object itself for an actor, or the owner of the check.
        let object = check.object();
        let first = if object.kind() == Actor::NAME {
            ActorId::parse(object.id()).ok()
        } else {
            check.owner()
        };

        // Pass the owner and each actor above it, with no SpiceDB call.
        let chain = match first {
            Some(first) if first == caller => return Ok(true),
            Some(first) => self.chain(first).await?,
            None => Vec::new(),
        };
        if chain.contains(&caller) {
            return Ok(true);
        }

        // Ask SpiceDB for a grant on the object or on an owner, in one call.
        let permission = check.permission();
        let mut objects = Vec::new();
        if object.kind() != Actor::NAME && self.schema.has(object.kind(), permission) {
            objects.push(object.clone());
        }
        if self.schema.has(Actor::NAME, permission) {
            objects.extend(chain.iter().map(|id| Object::of(*id)));
        }
        if objects.is_empty() {
            return Ok(false);
        }
        let subject = Object::of(caller);
        self.spicedb
            .check_any(&subject, &objects, permission, check.freshness())
            .await
    }

    /// Returns an actor and the actors above it, nearest first.
    async fn chain(&self, actor: ActorId) -> Result<Vec<ActorId>, AccessError> {
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let chain = store::owner_chain(&mut tx, actor)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(chain)
    }
}
