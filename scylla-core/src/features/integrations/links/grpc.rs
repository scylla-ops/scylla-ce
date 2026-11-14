//! The gRPC handler of `RepositoryService`: authenticate, parse, authorize on the project, call
//! the service, answer. The readers of a project read its link and its refs, and the editors
//! change the link.

use scylla_domain::authz::Permission;
use scylla_domain::integrations::links::RepositoryAccess;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageSize;
use scylla_extension::{AccessError, Principal};
use scylla_proto::common::v1::{PageRequest, PageResponse};
use scylla_proto::integrations::v1 as pb;
use scylla_proto::integrations::v1::repository_service_server::{
    RepositoryService, RepositoryServiceServer,
};
use tonic::{Request, Response, Status};

use super::service::Changes;
use super::{convert, link, refs, service, test_delivery};
use crate::api::parse::{self, ToProto};
use crate::features::integrations::connections::store as connections;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The longest search text, in characters.
const MAX_SEARCH: usize = 255;

/// The longest id of a repository on a provider, in digits.
const MAX_EXTERNAL_ID: usize = 20;

/// The shortest and the longest time between two polls, in seconds.
const POLL_INTERVAL: std::ops::RangeInclusive<u32> = 60..=900;

/// Makes the service.
pub(super) fn server(ctx: IntegrationContext) -> RepositoryServiceServer<LinksGrpc> {
    RepositoryServiceServer::new(LinksGrpc { ctx })
}

/// The handler.
pub(super) struct LinksGrpc {
    /// What the parts of the integration share.
    ctx: IntegrationContext,
}

#[tonic::async_trait]
impl RepositoryService for LinksGrpc {
    async fn get_repository_link(
        &self,
        request: Request<pb::GetRepositoryLinkRequest>,
    ) -> Result<Response<pb::GetRepositoryLinkResponse>, Status> {
        let caller = self.ctx.gate.caller(&request).await.map_err(Error::from)?;
        let project = parse::required(request.into_inner().project_id, "project_id")?;
        let permission = RepositoryAccess::GET_REPOSITORY_LINK;
        self.authorize(&caller, permission, project).await?;
        let linked = service::get(&self.ctx, project).await?;
        Ok(Response::new(pb::GetRepositoryLinkResponse {
            link: linked.map(ToProto::to_proto),
        }))
    }

    async fn link_repository(
        &self,
        request: Request<pb::LinkRepositoryRequest>,
    ) -> Result<Response<pb::LinkRepositoryResponse>, Status> {
        Ok(Response::new(self.link(request).await?))
    }

    async fn update_repository_link(
        &self,
        request: Request<pb::UpdateRepositoryLinkRequest>,
    ) -> Result<Response<pb::UpdateRepositoryLinkResponse>, Status> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn unlink_repository(
        &self,
        request: Request<pb::UnlinkRepositoryRequest>,
    ) -> Result<Response<pb::UnlinkRepositoryResponse>, Status> {
        let caller = self.ctx.gate.caller(&request).await.map_err(Error::from)?;
        let project = parse::required(request.into_inner().project_id, "project_id")?;
        let permission = RepositoryAccess::UNLINK_REPOSITORY;
        self.authorize(&caller, permission, project).await?;
        service::unlink(&self.ctx, project).await?;
        Ok(Response::new(pb::UnlinkRepositoryResponse {}))
    }

    async fn test_delivery(
        &self,
        request: Request<pb::TestDeliveryRequest>,
    ) -> Result<Response<pb::TestDeliveryResponse>, Status> {
        let caller = self.ctx.gate.caller(&request).await.map_err(Error::from)?;
        let project = parse::required(request.into_inner().project_id, "project_id")?;
        let permission = RepositoryAccess::TEST_DELIVERY;
        self.authorize(&caller, permission, project).await?;
        let linked = service::require(&self.ctx, project).await?;
        let delivery = test_delivery::test(&self.ctx, &linked).await?;
        let linked = service::require(&self.ctx, project).await?;
        Ok(Response::new(pb::TestDeliveryResponse {
            delivered: delivery.delivered,
            error: delivery.error,
            link: Some(linked.to_proto()),
        }))
    }

    async fn list_refs(
        &self,
        request: Request<pb::ListRefsRequest>,
    ) -> Result<Response<pb::ListRefsResponse>, Status> {
        Ok(Response::new(self.refs(request).await?))
    }
}

impl LinksGrpc {
    /// Handles `LinkRepository`: a repository of a connection of the workspace of the project.
    async fn link(
        &self,
        request: Request<pb::LinkRepositoryRequest>,
    ) -> Result<pb::LinkRepositoryResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let id = parse::required(request.connection_id, "connection_id")?;
        let external_id = request.external_id.trim();
        let digits = external_id.chars().all(|c| c.is_ascii_digit());
        if external_id.is_empty() || external_id.len() > MAX_EXTERNAL_ID || !digits {
            return Err(Error::invalid(
                "external_id",
                "is not an id of a repository",
            ));
        }

        // Check the route permission on the project.
        let permission = RepositoryAccess::LINK_REPOSITORY;
        let workspace = self.authorize(&caller, permission, project).await?;

        // Refuse a connection of another workspace as an unknown one.
        let mut tx = self.ctx.pool.begin().await?;
        let found = connections::find(&mut tx, id).await?;
        tx.commit().await?;
        let Some(found) = found.filter(|found| found.connection.workspace_id == workspace) else {
            let object = format!("vcs_connection:{id}");
            let reason = format!("{} on {object} is denied", permission.name());
            let error = AccessError::refused(&caller, permission.name(), &object, reason);
            return Err(error.into());
        };

        // Link the repository.
        let connection = &found.connection;
        let linked = link::link(&self.ctx, project, connection, external_id, caller.actor.id);
        Ok(pb::LinkRepositoryResponse {
            link: Some(linked.await?.to_proto()),
        })
    }

    /// Handles `UpdateRepositoryLink`: the reports and the polling of a link.
    async fn update(
        &self,
        request: Request<pb::UpdateRepositoryLinkRequest>,
    ) -> Result<pb::UpdateRepositoryLinkResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let project = parse::required(request.project_id, "project_id")?;
        let report_mode = convert::report_mode_of(request.report_mode)
            .ok_or_else(|| Error::invalid("report_mode", "is not a report mode"))?;
        let poll_interval_seconds = request.poll_interval_seconds;
        if poll_interval_seconds.is_some_and(|seconds| !POLL_INTERVAL.contains(&seconds)) {
            let reason = "must be from 60 to 900 seconds";
            return Err(Error::invalid("poll_interval_seconds", reason));
        }
        let changes = Changes {
            reports_enabled: request.reports_enabled,
            report_mode,
            poll_interval_seconds,
        };

        // Check the route permission, then change the link.
        let permission = RepositoryAccess::UPDATE_REPOSITORY_LINK;
        self.authorize(&caller, permission, project).await?;
        let linked = service::update(&self.ctx, project, changes).await?;
        Ok(pb::UpdateRepositoryLinkResponse {
            link: Some(linked.to_proto()),
        })
    }

    /// Handles `ListRefs`: one page of the branches and the tags of the linked repository.
    async fn refs(
        &self,
        request: Request<pb::ListRefsRequest>,
    ) -> Result<pb::ListRefsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let project = parse::required(request.project_id, "project_id")?;
        let kind = convert::ref_kind(request.kind)
            .ok_or_else(|| Error::invalid("kind", "is not a kind of ref"))?;
        let search = request.search.trim().to_owned();
        if search.chars().count() > MAX_SEARCH {
            return Err(Error::invalid("search", "must have at most 255 characters"));
        }
        let (offset, size) = page(request.page.unwrap_or_default())?;

        // Check the route permission, then read the refs and give one page.
        let permission = RepositoryAccess::LIST_REFS;
        self.authorize(&caller, permission, project).await?;
        let linked = service::require(&self.ctx, project).await?;
        let found = refs::list(&self.ctx, &linked, kind, &search).await?;
        let next = (found.len() > offset + size).then(|| (offset + size).to_string());
        let items = found.into_iter().skip(offset).take(size);
        Ok(pb::ListRefsResponse {
            refs: items.map(ToProto::to_proto).collect(),
            page: Some(PageResponse {
                next_token: next.unwrap_or_default(),
            }),
        })
    }

    /// Checks a route permission on a project, and returns its workspace. An unknown project
    /// gets the answer of a refusal.
    async fn authorize(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        project: ProjectId,
    ) -> Result<WorkspaceId, Error> {
        let check = permission.on(project);
        let mut tx = self.ctx.pool.begin().await?;
        let workspace = connections::project_workspace(&mut tx, project).await?;
        tx.commit().await?;
        let Some(workspace) = workspace else {
            return Err(AccessError::denied(caller, &check).into());
        };
        self.ctx
            .gate
            .authorize(caller, check.within(workspace))
            .await?;
        Ok(workspace)
    }
}

/// Reads the page of a list of refs: the token is the number of items before the page.
fn page(page: PageRequest) -> Result<(usize, usize), Error> {
    let size = PageSize::parse(page.size)
        .map_err(|error| Error::invalid("page.size", error.to_string()))?;
    let offset = match page.token.as_str() {
        "" => 0,
        token => token
            .parse()
            .map_err(|_| Error::invalid("page.token", "is not a token of this list"))?,
    };
    Ok((offset, size.get()))
}
