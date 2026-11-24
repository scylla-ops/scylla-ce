//! The gRPC handler of `FragmentService`: authenticate, parse, authorize, call the service,
//! answer.

use std::sync::Arc;

use scylla_domain::projects::pipelines::NodeId;
use scylla_domain::tenancy::library::{FragmentHolder, FragmentId, ListFragments, NewFragment};
use scylla_extension::{Gate, Hooks, Principal};
use scylla_proto::library::v1 as pb;
use scylla_proto::library::v1::fragment_service_server::{FragmentService, FragmentServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::access::{self, Route};
use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
) -> FragmentServiceServer<LibraryGrpc> {
    FragmentServiceServer::new(LibraryGrpc { pool, hooks, gate })
}

/// The handler.
pub(super) struct LibraryGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The hooks of the edition.
    pub(super) hooks: Arc<Hooks>,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl FragmentService for LibraryGrpc {
    async fn list_fragments(
        &self,
        request: Request<pb::ListFragmentsRequest>,
    ) -> Result<Response<pb::ListFragmentsResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let query = ListFragments {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            reader: caller.actor.id,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the workspace, then read one page.
        let check = FragmentHolder::LIST_FRAGMENTS.on(query.workspace_id);
        self.gate
            .authorize(&caller, check)
            .await
            .map_err(Error::from)?;
        let page = service::list(&self.pool, query).await?;
        Ok(Response::new(pb::ListFragmentsResponse {
            fragments: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn get_fragment(
        &self,
        request: Request<pb::GetFragmentRequest>,
    ) -> Result<Response<pb::GetFragmentResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .fragment_of(&caller, request.into_inner().fragment_id, access::GET)
            .await?;
        let fragment = service::get(&self.pool, id, caller.actor.id)
            .await?
            .ok_or(Error::NotFound("fragment"))?;
        Ok(Response::new(pb::GetFragmentResponse {
            fragment: Some(fragment.to_proto()),
        }))
    }

    async fn create_fragment(
        &self,
        request: Request<pb::CreateFragmentRequest>,
    ) -> Result<Response<pb::CreateFragmentResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let new = NewFragment {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            created_by: caller.actor.id,
            name: NodeId::parse(request.name)
                .map_err(|error| Error::invalid("name", error.to_string()))?,
            description: convert::description(request.description)?,
            tags: convert::tags(request.tags)?,
            origin: request.origin.map(convert::origin).transpose()?,
            version: convert::version(request.nodes, request.parameters, Vec::new())?,
        };

        // Check the route permission on the workspace, then create the fragment.
        let check = FragmentHolder::CREATE_FRAGMENT.on(new.workspace_id);
        self.gate
            .authorize(&caller, check)
            .await
            .map_err(Error::from)?;
        let fragment = service::create(&self.pool, &self.hooks, caller.actor, new).await?;
        Ok(Response::new(pb::CreateFragmentResponse {
            fragment: Some(fragment.to_proto()),
        }))
    }

    async fn publish_fragment_version(
        &self,
        request: Request<pb::PublishFragmentVersionRequest>,
    ) -> Result<Response<pb::PublishFragmentVersionResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let version = convert::version(request.nodes, request.parameters, request.changes)?;

        // Check the route on the fragment, then add the version.
        let id = self
            .fragment_of(&caller, request.fragment_id, access::PUBLISH)
            .await?;
        let fragment =
            service::publish_version(&self.pool, &self.hooks, caller.actor, id, version).await?;
        Ok(Response::new(pb::PublishFragmentVersionResponse {
            fragment: Some(fragment.to_proto()),
        }))
    }

    async fn set_fragment_favorite(
        &self,
        request: Request<pb::SetFragmentFavoriteRequest>,
    ) -> Result<Response<pb::SetFragmentFavoriteResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let id = self
            .fragment_of(&caller, request.fragment_id, access::FAVORITE)
            .await?;
        service::set_favorite(&self.pool, id, caller.actor.id, request.favorite).await?;
        Ok(Response::new(pb::SetFragmentFavoriteResponse {}))
    }

    async fn delete_fragment(
        &self,
        request: Request<pb::DeleteFragmentRequest>,
    ) -> Result<Response<pb::DeleteFragmentResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .fragment_of(&caller, request.into_inner().fragment_id, access::DELETE)
            .await?;
        service::delete(&self.pool, &self.hooks, caller.actor, id).await?;
        Ok(Response::new(pb::DeleteFragmentResponse {}))
    }

    async fn accept_origin_version(
        &self,
        request: Request<pb::AcceptOriginVersionRequest>,
    ) -> Result<Response<pb::AcceptOriginVersionResponse>, Status> {
        Ok(Response::new(self.accept_origin(request).await?))
    }

    async fn skip_origin_version(
        &self,
        request: Request<pb::SkipOriginVersionRequest>,
    ) -> Result<Response<pb::SkipOriginVersionResponse>, Status> {
        Ok(Response::new(self.skip_origin(request).await?))
    }

    async fn dismiss_origin_version(
        &self,
        request: Request<pb::DismissOriginVersionRequest>,
    ) -> Result<Response<pb::DismissOriginVersionResponse>, Status> {
        Ok(Response::new(self.dismiss_origin(request).await?))
    }

    async fn detach_origin(
        &self,
        request: Request<pb::DetachOriginRequest>,
    ) -> Result<Response<pb::DetachOriginResponse>, Status> {
        Ok(Response::new(self.detach_origin_of(request).await?))
    }
}

impl LibraryGrpc {
    /// Reads the fragment id of a request and checks a route on the fragment. An unknown
    /// fragment gets the answer of a refusal.
    pub(super) async fn fragment_of(
        &self,
        caller: &Principal,
        id: Option<scylla_proto::common::v1::Id>,
        route: Route,
    ) -> Result<FragmentId, Error> {
        let id: FragmentId = parse::required(id, "fragment_id")?;
        let found = service::find(&self.pool, id).await?;
        access::authorize(self.gate.as_ref(), caller, route, id, found.as_ref()).await?;
        Ok(id)
    }
}
