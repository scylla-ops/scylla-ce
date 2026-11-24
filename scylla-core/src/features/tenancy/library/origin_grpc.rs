//! The handlers of the routes of `FragmentService` on the fragments of a package: authenticate,
//! parse, authorize, call the service, answer.

use scylla_proto::library::v1 as pb;
use tonic::Request;

use super::grpc::LibraryGrpc;
use super::{access, convert, origin_service};
use crate::api::parse::ToProto;
use crate::Error;

impl LibraryGrpc {
    /// Handles `AcceptOriginVersion`: the content comes from the client, which read the registry.
    pub(super) async fn accept_origin(
        &self,
        request: Request<pb::AcceptOriginVersionRequest>,
    ) -> Result<pb::AcceptOriginVersionResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let version = convert::version(request.nodes, request.parameters, request.changes)?;

        // Check the route on the fragment, then add the version.
        let id = self
            .fragment_of(&caller, request.fragment_id, access::ACCEPT_ORIGIN)
            .await?;
        let fragment = origin_service::accept(
            &self.pool,
            &self.hooks,
            caller.actor,
            id,
            request.version,
            version,
        )
        .await?;
        Ok(pb::AcceptOriginVersionResponse {
            fragment: Some(fragment.to_proto()),
        })
    }

    /// Handles `SkipOriginVersion`.
    pub(super) async fn skip_origin(
        &self,
        request: Request<pb::SkipOriginVersionRequest>,
    ) -> Result<pb::SkipOriginVersionResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id = self
            .fragment_of(&caller, request.fragment_id, access::SKIP_ORIGIN)
            .await?;
        let fragment = origin_service::skip(&self.pool, caller.actor, id, request.version).await?;
        Ok(pb::SkipOriginVersionResponse {
            fragment: Some(fragment.to_proto()),
        })
    }

    /// Handles `DismissOriginVersion`.
    pub(super) async fn dismiss_origin(
        &self,
        request: Request<pb::DismissOriginVersionRequest>,
    ) -> Result<pb::DismissOriginVersionResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id = self
            .fragment_of(&caller, request.fragment_id, access::DISMISS_ORIGIN)
            .await?;
        let fragment =
            origin_service::dismiss(&self.pool, caller.actor, id, request.version).await?;
        Ok(pb::DismissOriginVersionResponse {
            fragment: Some(fragment.to_proto()),
        })
    }

    /// Handles `DetachOrigin`.
    pub(super) async fn detach_origin_of(
        &self,
        request: Request<pb::DetachOriginRequest>,
    ) -> Result<pb::DetachOriginResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id = request.into_inner().fragment_id;
        let id = self.fragment_of(&caller, id, access::DETACH_ORIGIN).await?;
        let fragment = origin_service::detach(&self.pool, caller.actor, id).await?;
        Ok(pb::DetachOriginResponse {
            fragment: Some(fragment.to_proto()),
        })
    }
}
