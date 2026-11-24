//! The gRPC routes `Grant` and `Revoke` of `AccessService` on the platform: the roles of the whole
//! instance.

use scylla_domain::authz::Check;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::{Platform, PlatformId};
use scylla_domain::tenancy::roles::RoleAdmin;
use scylla_extension::{AccessError, Principal};
use scylla_proto::access::v1 as pb;
use scylla_proto::common::v1 as common;

use super::grpc::{not_an_id, GrantsGrpc};
use crate::api::parse;
use crate::features::tenancy::roles;
use crate::Error;

impl GrantsGrpc {
    /// Handles `Grant` on the platform.
    pub(super) async fn give_on_platform(
        &self,
        caller: &Principal,
        object: pb::ObjectRef,
        role: String,
        actor_id: Option<common::Id>,
    ) -> Result<pb::GrantResponse, Error> {
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        self.authorize_on_platform(caller, object, RoleAdmin::GRANT.on(Platform::ID))
            .await?;
        let gate = self.gate.as_ref();
        roles::give_on_platform(&self.pool, gate, caller, &role, actor).await?;
        Ok(pb::GrantResponse {})
    }

    /// Handles `Revoke` on the platform.
    pub(super) async fn take_back_on_platform(
        &self,
        caller: &Principal,
        object: pb::ObjectRef,
        role: String,
        actor_id: Option<common::Id>,
    ) -> Result<pb::RevokeResponse, Error> {
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        self.authorize_on_platform(caller, object, RoleAdmin::REVOKE.on(Platform::ID))
            .await?;
        let gate = self.gate.as_ref();
        roles::take_back_on_platform(&self.pool, gate, caller, &role, actor).await?;
        Ok(pb::RevokeResponse {})
    }

    /// Checks the route permission on the platform. Another id gets the answer of a refusal.
    async fn authorize_on_platform(
        &self,
        caller: &Principal,
        object: pb::ObjectRef,
        check: Check,
    ) -> Result<(), Error> {
        let id = object
            .id
            .ok_or_else(|| Error::invalid("object.id", "is required"))?;
        let id = PlatformId::parse(&id.value).map_err(|_| not_an_id())?;
        if id != Platform::ID {
            return Err(AccessError::denied(caller, &check).into());
        }
        self.gate.authorize(caller, check).await?;
        Ok(())
    }
}
