//! `Grant` and `Revoke` of the role owner of a workspace. Its holders are rows of `workspace_owners`.
//! After the route permission, each call needs `manage_owners`, so only an owner and an instance
//! admin change them.

use scylla_domain::authz::{Check, Resource};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users;
use scylla_domain::tenancy::grants::OWNER;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{AccessError, Grantable, Principal};
use scylla_proto::access::v1 as pb;
use scylla_proto::common::v1 as common;

use super::grpc::{not_an_id, GrantsGrpc};
use super::{membership, service};
use crate::api::parse;
use crate::features::tenancy::workspaces;
use crate::Error;

impl GrantsGrpc {
    /// Gives the role owner of a workspace. The actor must be a user. An owner stays an owner.
    pub(super) async fn give_owner(
        &self,
        caller: Principal,
        kind: &Grantable,
        id: &str,
        actor_id: Option<common::Id>,
    ) -> Result<pb::GrantResponse, Error> {
        // Read the actor, check the rights of the caller, then check who gets the role.
        let check = kind.grant(id).ok_or_else(not_an_id)?;
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        let workspace = self.check_changes_owners(&caller, kind, id, check).await?;
        let grantee = service::check_actor(&self.pool, actor, workspace).await?;
        if grantee.kind != users::KIND {
            return Err(Error::invalid("role", "is for people only"));
        }

        // Give the role, with the hooks of a user that joins.
        workspaces::give_owner(&self.pool, &self.hooks, caller.actor, workspace, actor).await?;
        let object = format!("{}:{id}", Workspace::NAME);
        tracing::info!(event = "role.granted", role = OWNER, object, grantee = %actor);
        Ok(pb::GrantResponse {})
    }

    /// Takes back the role owner of a workspace: the last owner stays. A user with no role left in
    /// the workspace loses the roles on its projects.
    pub(super) async fn take_back_owner(
        &self,
        caller: Principal,
        kind: &Grantable,
        id: &str,
        actor_id: Option<common::Id>,
    ) -> Result<pb::RevokeResponse, Error> {
        // Read the actor, check the rights of the caller, then take back the role.
        let check = kind.revoke(id).ok_or_else(not_an_id)?;
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        let workspace = self.check_changes_owners(&caller, kind, id, check).await?;
        workspaces::take_owner(&self.pool, workspace, actor, caller.actor.id).await?;
        let object = format!("{}:{id}", Workspace::NAME);
        let member = (actor, object.clone(), OWNER);
        let by = caller.actor;
        service::leave(&self.pool, &self.hooks, by.clone(), workspace, member).await?;
        tracing::info!(event = "role.revoked", role = OWNER, object, grantee = %actor);

        // End the membership of a user with no role left.
        let leaving = membership::Leaving {
            pool: &self.pool,
            gate: self.gate.as_ref(),
            grants: &self.grants,
            hooks: &self.hooks,
        };
        membership::end_membership(&leaving, by, actor, workspace).await?;
        Ok(pb::RevokeResponse {})
    }

    /// Checks the route permission, then `manage_owners`, and returns the workspace. A second check
    /// through the route rule is not possible: a route checks only its own permission. An unknown
    /// workspace gets the answer of a refusal.
    async fn check_changes_owners(
        &self,
        caller: &Principal,
        kind: &Grantable,
        id: &str,
        route: Check,
    ) -> Result<WorkspaceId, Error> {
        // Check the route permission on the workspace.
        let (route, workspace) = self.within(caller, kind, id, route).await?;
        self.gate.authorize(caller, route).await?;

        // Refuse a caller that is not an owner or an instance admin.
        let check = Workspace::MANAGE_OWNERS.on(workspace);
        let permissions = [check.permission()];
        let answers = self
            .gate
            .permissions(caller, check.object(), Some(workspace), &permissions)
            .await?;
        if answers.first() == Some(&true) {
            return Ok(workspace);
        }
        Err(AccessError::denied(caller, &check).into())
    }
}
