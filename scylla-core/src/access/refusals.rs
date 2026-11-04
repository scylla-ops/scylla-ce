//! The refusals of the gate that read Postgres: a token that a revocation, a closed session,
//! a gone actor or a switch stops, and a personal token in a workspace that refuses them.

use biscuit_auth::Biscuit;
use scylla_domain::authz::Check;
use scylla_domain::identity::{apps, users};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Principal};

use super::claims::Claims;
use super::gate::Access;
use super::store;

impl Access {
    /// Returns `true` if a token cannot pass: a revocation id of it is revoked, its session, its
    /// key or its API token is revoked, its session or its API token is expired, or its user or
    /// App is gone or disabled.
    pub(super) async fn is_refused(
        &self,
        token: &Biscuit,
        claims: &Claims,
    ) -> Result<bool, AccessError> {
        let ids = token.revocation_identifiers();
        let facts = store::TokenFacts {
            ids: &ids,
            actor: claims.actor.id,
            user: claims.actor.kind == users::KIND,
            app: claims.actor.kind == apps::KIND,
            session: claims.session(),
            key: claims.key(),
            api_token: claims.api_token(),
        };
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let refused = store::is_refused(&mut tx, &facts)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(refused)
    }

    /// Returns `true` if the token of a user came from a personal token and the workspace of the
    /// check refuses them. A check with no workspace passes.
    pub(super) async fn refuses_personal_token(
        &self,
        principal: &Principal,
        check: &Check,
    ) -> Result<bool, AccessError> {
        if principal.api_token().is_none() || principal.actor.kind != users::KIND {
            return Ok(false);
        }
        let object = check.object();
        let own = (object.kind() == "workspace")
            .then(|| WorkspaceId::parse(object.id()).ok())
            .flatten();
        let Some(workspace) = check.workspace().or(own) else {
            return Ok(false);
        };
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let refused = store::refuses_personal_tokens(&mut tx, workspace)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(refused)
    }
}
