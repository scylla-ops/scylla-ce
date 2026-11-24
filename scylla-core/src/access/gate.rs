//! The gate: verifies the Biscuit of each request and its revocation, then finds the rights.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use biscuit_auth::Biscuit;
use scylla_domain::authz::{Check, Object, Relationship, Schema};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{
    async_trait, AccessError, Binding, Gate, IssuedToken, ObjectRole, Operation, Principal,
    RolePage,
};
use spicedb_grpc_tonic::v1::relationship_update::Operation as Update;
use sqlx::PgPool;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;

use super::claims::{allows, claims, unauthenticated};
use super::rights::Rights;
use super::{SpiceDb, TokenKeys};

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
    pub(super) pool: PgPool,
    /// The keys of the tokens.
    pub(super) keys: TokenKeys,
    /// The lifetime of a token.
    pub(super) token_ttl: Duration,
    /// The SpiceDB client.
    pub(super) spicedb: SpiceDb,
    /// The owner chains and the grants.
    pub(super) rights: Rights,
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

        // Refuse a personal token in a workspace that refuses them.
        if self.refuses_personal_token(principal, &check).await? {
            let reason = "the workspace refuses the personal tokens".to_owned();
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

    async fn holds(
        &self,
        principal: &Principal,
        object: &Object,
        workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        let caller = principal.actor.id;
        self.rights
            .holds(caller, object, workspace, permissions)
            .await
    }

    async fn lookup(
        &self,
        actor: ActorId,
        definition: &'static str,
        permission: &'static str,
    ) -> Result<Vec<String>, AccessError> {
        // Find the objects from the roles of the schema, then from the roles that are rows.
        let subject = Object::of(actor);
        let mut ids = self
            .spicedb
            .lookup(&subject, definition, permission)
            .await?;
        ids.extend(
            self.rights
                .row_objects(actor, definition, permission)
                .await?,
        );
        Ok(ids)
    }

    async fn role_counts(
        &self,
        definition: &'static str,
    ) -> Result<BTreeMap<String, u32>, AccessError> {
        self.spicedb.role_counts(definition).await
    }

    async fn roles_of(
        &self,
        actor: ActorId,
        definition: &'static str,
    ) -> Result<Vec<ObjectRole>, AccessError> {
        self.spicedb.roles_of(actor, definition).await
    }

    async fn holders_of(
        &self,
        definition: &'static str,
        relation: &str,
    ) -> Result<Vec<ObjectRole>, AccessError> {
        self.spicedb.holders_of(definition, relation).await
    }
}
