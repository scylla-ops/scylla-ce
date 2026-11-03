//! The gRPC route `CheckPermissions` of `AccessService`: the permissions of the caller on one
//! object, in one call.

use scylla_domain::authz::{Object, Resource};
use scylla_domain::platform::{Platform, PlatformId};
use scylla_domain::tenancy::grants::{AccessCaller, MAX_CHECKS};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::access::v1 as pb;
use tonic::Request;

use super::grpc::{not_an_id, GrantsGrpc};
use crate::Error;

impl GrantsGrpc {
    /// Handles `CheckPermissions`: for each permission, `true` if the caller has it on the
    /// object, as a route check finds it. An unknown object gives `false` for each one.
    pub(super) async fn check(
        &self,
        request: Request<pb::CheckPermissionsRequest>,
    ) -> Result<pb::CheckPermissionsResponse, Error> {
        // Authenticate the caller, then read the object.
        let caller = self.gate.caller(&request).await?;
        let pb::CheckPermissionsRequest {
            object,
            permissions,
        } = request.into_inner();
        let object = object.ok_or_else(|| Error::invalid("object", "is required"))?;
        let id = object
            .id
            .map(|id| id.value)
            .ok_or_else(|| Error::invalid("object.id", "is required"))?;
        if object.kind != Platform::NAME && self.grants.get(&object.kind).is_none() {
            return Err(Error::invalid("object.kind", "has no permissions to check"));
        }

        // Read the names: each one is a permission of the definition of the object.
        if permissions.len() > MAX_CHECKS {
            let reason = format!("must have at most {MAX_CHECKS} names");
            return Err(Error::invalid("permissions", reason));
        }
        let names = permissions
            .iter()
            .map(|name| {
                self.schema.permission(&object.kind, name).ok_or_else(|| {
                    let reason = format!("has {name}, not a permission of {}", object.kind);
                    Error::invalid("permissions", reason)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Check the route permission on the caller itself.
        self.gate
            .authorize(&caller, AccessCaller::CHECK_PERMISSIONS.on(caller.actor.id))
            .await?;

        // Ask for each permission on the object. An unknown object has none.
        let allowed = match self.locate(&object.kind, &id).await? {
            Some((object, workspace)) => {
                self.gate
                    .permissions(&caller, &object, workspace, &names)
                    .await?
            }
            None => vec![false; names.len()],
        };
        let answers = names
            .into_iter()
            .zip(allowed)
            .map(|(permission, allowed)| pb::PermissionAnswer {
                permission: permission.to_owned(),
                allowed,
            })
            .collect();
        Ok(pb::CheckPermissionsResponse { answers })
    }

    /// Finds an object and its workspace, or returns `None` for an unknown object. The kind is
    /// the platform, or a kind with roles.
    async fn locate(
        &self,
        kind: &str,
        id: &str,
    ) -> Result<Option<(Object, Option<WorkspaceId>)>, Error> {
        // The platform has one object, in no workspace.
        if kind == Platform::NAME {
            let id = PlatformId::parse(id).map_err(|_| not_an_id())?;
            return Ok((id == Platform::ID).then(|| (Object::of(id), None)));
        }

        // An object of another kind is in a workspace, from Postgres.
        let Some(grantable) = self.grants.get(kind) else {
            return Ok(None);
        };
        let object = grantable.object(id).ok_or_else(not_an_id)?;
        let search = grantable
            .workspace(self.pool.clone(), id)
            .ok_or_else(not_an_id)?;
        Ok(search.await?.map(|workspace| (object, Some(workspace))))
    }
}
