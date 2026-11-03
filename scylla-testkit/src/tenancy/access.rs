//! The `TestApp` helpers of `AccessService`: the roles, the list of the access, and the
//! permissions of a caller.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::access::v1::access_service_client::AccessServiceClient;
use scylla_proto::access::v1::{
    CheckPermissionsRequest, GrantRequest, ListAccessRequest, ListAccessResponse, ObjectRef,
    RevokeRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

/// A role on an object, as text, so a test can send a bad value: `Role::on("project", id,
/// "viewer")`.
#[derive(Clone, Copy, Debug)]
pub struct Role<'a> {
    /// The kind of the object.
    pub kind: &'a str,
    /// The id of the object.
    pub id: &'a str,
    /// The name of the role.
    pub name: &'a str,
}

impl<'a> Role<'a> {
    /// Names a role on an object.
    pub fn on(kind: &'a str, id: &'a str, name: &'a str) -> Self {
        Self { kind, id, name }
    }
}

/// Makes the message of an object.
fn object(kind: &str, object_id: &str) -> Option<ObjectRef> {
    Some(ObjectRef {
        kind: kind.to_owned(),
        id: Some(id(object_id)),
    })
}

impl TestApp {
    /// Calls `Grant` as `actor`: gives the role to the actor `to`, as text.
    pub async fn grant_role(&self, actor: ActorId, role: Role<'_>, to: &str) -> Result<(), Status> {
        let request = GrantRequest {
            object: object(role.kind, role.id),
            role: role.name.to_owned(),
            actor_id: Some(id(to)),
        };
        let request = self.authed(actor, request).await;
        self.access().grant(request).await?;
        Ok(())
    }

    /// Calls `Revoke` as `actor`: takes back the role from the actor `from`, as text.
    pub async fn revoke_role(
        &self,
        actor: ActorId,
        role: Role<'_>,
        from: &str,
    ) -> Result<(), Status> {
        let request = RevokeRequest {
            object: object(role.kind, role.id),
            role: role.name.to_owned(),
            actor_id: Some(id(from)),
        };
        let request = self.authed(actor, request).await;
        self.access().revoke(request).await?;
        Ok(())
    }

    /// Calls `ListAccess` as `actor` for one page of an object.
    pub async fn list_access(
        &self,
        actor: ActorId,
        kind: &str,
        object_id: &str,
        size: i32,
        token: &str,
    ) -> Result<ListAccessResponse, Status> {
        let request = ListAccessRequest {
            object: object(kind, object_id),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.access().list_access(request).await?.into_inner())
    }

    /// Calls `ListAccess` as `actor` for each page of an object, and returns each entry as
    /// `(actor id, role)`, in list order.
    pub async fn access_of(
        &self,
        actor: ActorId,
        kind: &str,
        object_id: &str,
        size: i32,
    ) -> Result<Vec<(String, String)>, Status> {
        let mut entries = Vec::new();
        let mut token = String::new();
        loop {
            let page = self
                .list_access(actor, kind, object_id, size, &token)
                .await?;
            entries.extend(page.entries.into_iter().map(|entry| {
                let actor = entry.actor_id.map(|id| id.value).unwrap_or_default();
                (actor, entry.role)
            }));
            token = page.page.map(|page| page.next_token).unwrap_or_default();
            if token.is_empty() {
                return Ok(entries);
            }
        }
    }

    /// Calls `CheckPermissions` as `actor`, and returns the answers in the order of the names.
    pub async fn check_permissions(
        &self,
        actor: ActorId,
        kind: &str,
        object_id: &str,
        permissions: &[&str],
    ) -> Result<Vec<bool>, Status> {
        let request = CheckPermissionsRequest {
            object: object(kind, object_id),
            permissions: permissions.iter().map(|&name| name.to_owned()).collect(),
        };
        let request = self.authed(actor, request).await;
        let response = self.access().check_permissions(request).await?;
        Ok(response
            .into_inner()
            .answers
            .into_iter()
            .map(|answer| answer.allowed)
            .collect())
    }

    /// Returns a client of `AccessService`.
    fn access(&self) -> AccessServiceClient<Channel> {
        AccessServiceClient::new(self.channel.clone())
    }
}
