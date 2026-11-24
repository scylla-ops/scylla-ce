//! The gRPC route `ListAccess` of `AccessService`: the owners of a workspace and the roles on
//! an object, one page at a time.

use scylla_domain::authz::Resource;
use scylla_domain::tenancy::grants::{AccessCursor, AccessEntry};
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_domain::PageSize;
use scylla_proto::access::v1 as pb;
use scylla_proto::common::v1::PageResponse;
use tonic::Request;

use super::grpc::{not_an_id, GrantsGrpc};
use super::service;
use crate::api::parse::ToProto;
use crate::Error;

impl GrantsGrpc {
    /// Handles `ListAccess`: for a workspace, its owners first, then the roles on the object.
    pub(super) async fn list(
        &self,
        request: Request<pb::ListAccessRequest>,
    ) -> Result<pb::ListAccessResponse, Error> {
        // Authenticate the caller, then read the object and the page.
        let caller = self.gate.caller(&request).await?;
        let pb::ListAccessRequest { object, page } = request.into_inner();
        let (kind, _, id) = self.target(object)?;
        let check = kind.list_access(&id).ok_or_else(not_an_id)?;
        let object = kind.object(&id).ok_or_else(not_an_id)?;
        let page = page.unwrap_or_default();
        let size = PageSize::parse(page.size)
            .map_err(|error| Error::invalid("page.size", error.to_string()))?;
        let start = match page.token.as_str() {
            "" => None,
            token => Some(AccessCursor::parse(token).ok_or_else(bad_token)?),
        };

        // Check the route permission on the object, with its workspace.
        let (check, workspace) = self.within(&caller, kind, &id, check).await?;
        self.gate.authorize(&caller, check).await?;

        // Read one page: the owners of a workspace first, then the roles.
        let owners_of = (object.kind() == Workspace::NAME).then_some(workspace);
        let gate = self.gate.as_ref();
        let page = service::list_access(&self.pool, gate, &object, owners_of, start, size)
            .await?
            .ok_or_else(bad_token)?;
        Ok(pb::ListAccessResponse {
            entries: page.entries.into_iter().map(ToProto::to_proto).collect(),
            page: Some(PageResponse {
                next_token: page.next.map(|next| next.token()).unwrap_or_default(),
            }),
        })
    }
}

/// The error of a page token that this list did not make.
fn bad_token() -> Error {
    Error::invalid("page.token", "is not a token of this list")
}

impl ToProto<pb::AccessEntry> for AccessEntry {
    fn to_proto(self) -> pb::AccessEntry {
        pb::AccessEntry {
            actor_id: Some(self.actor_id.to_proto()),
            actor_kind: self.actor_kind.as_str().to_owned(),
            display_name: self.display_name,
            handle: self.handle.unwrap_or_default(),
            role: self.role,
        }
    }
}
