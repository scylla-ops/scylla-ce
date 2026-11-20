//! The gRPC handler of `VcsAppService`: authenticate, parse, authorize on the platform, call the
//! service, answer. No answer holds a key or a secret of the App.

use scylla_domain::authz::Permission;
use scylla_domain::integrations::apps::VcsAppHolder;
use scylla_domain::platform::Platform;
use scylla_extension::Principal;
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::vcs_app_service_server::{VcsAppService, VcsAppServiceServer};
use tonic::{Request, Response, Status};

use super::{convert, manifest, service};
use crate::api::parse;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The longest organization of GitHub, in characters.
const MAX_ORGANIZATION: usize = 39;

/// The longest code of GitHub, in characters.
const MAX_CODE: usize = 255;

/// Makes the service.
pub(super) fn server(ctx: IntegrationContext) -> VcsAppServiceServer<AppsGrpc> {
    VcsAppServiceServer::new(AppsGrpc { ctx })
}

/// The handler.
pub(super) struct AppsGrpc {
    /// What the parts of the integration share.
    ctx: IntegrationContext,
}

#[tonic::async_trait]
impl VcsAppService for AppsGrpc {
    async fn get_vcs_app(
        &self,
        request: Request<pb::GetVcsAppRequest>,
    ) -> Result<Response<pb::GetVcsAppResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn start_git_hub_manifest(
        &self,
        request: Request<pb::StartGitHubManifestRequest>,
    ) -> Result<Response<pb::StartGitHubManifestResponse>, Status> {
        Ok(Response::new(self.start(request).await?))
    }

    async fn complete_git_hub_manifest(
        &self,
        request: Request<pb::CompleteGitHubManifestRequest>,
    ) -> Result<Response<pb::CompleteGitHubManifestResponse>, Status> {
        Ok(Response::new(self.complete(request).await?))
    }

    async fn delete_vcs_app(
        &self,
        request: Request<pb::DeleteVcsAppRequest>,
    ) -> Result<Response<pb::DeleteVcsAppResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }
}

impl AppsGrpc {
    /// Handles `GetVcsApp`: the App of the settings, else the App of the instance.
    async fn get(
        &self,
        request: Request<pb::GetVcsAppRequest>,
    ) -> Result<pb::GetVcsAppResponse, Error> {
        let caller = self.ctx.gate.caller(&request).await?;
        self.authorize(&caller, VcsAppHolder::GET_VCS_APP).await?;
        let shown = service::get(&self.ctx).await?;
        Ok(pb::GetVcsAppResponse {
            app: shown.map(|shown| convert::app(&self.ctx, shown)),
        })
    }

    /// Handles `StartGitHubManifest`: the manifest and the page of GitHub, with a new state.
    async fn start(
        &self,
        request: Request<pb::StartGitHubManifestRequest>,
    ) -> Result<pb::StartGitHubManifestResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let organization = organization(&request.organization)?;

        // Check the route permission, then make the state and the manifest.
        self.authorize(&caller, VcsAppHolder::START_GIT_HUB_MANIFEST)
            .await?;
        let started = manifest::start(&self.ctx, organization, caller.actor.id).await?;
        Ok(pb::StartGitHubManifestResponse {
            post_url: started.post_url,
            manifest: started.manifest,
            expires_at: Some(parse::timestamp(started.expires_at)),
        })
    }

    /// Handles `CompleteGitHubManifest`: the new App, from the code of the redirect of GitHub.
    async fn complete(
        &self,
        request: Request<pb::CompleteGitHubManifestRequest>,
    ) -> Result<pb::CompleteGitHubManifestResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let code = code(&request.code)?;
        if request.state.is_empty() {
            return Err(Error::invalid("state", "is required"));
        }

        // Check the route permission, then make the App.
        self.authorize(&caller, VcsAppHolder::COMPLETE_GIT_HUB_MANIFEST)
            .await?;
        let shown = manifest::complete(&self.ctx, code, &request.state, caller.actor.id).await?;
        Ok(pb::CompleteGitHubManifestResponse {
            app: Some(convert::app(&self.ctx, shown)),
        })
    }

    /// Handles `DeleteVcsApp`: the App of the instance goes, and its connections stop.
    async fn delete(
        &self,
        request: Request<pb::DeleteVcsAppRequest>,
    ) -> Result<pb::DeleteVcsAppResponse, Error> {
        let caller = self.ctx.gate.caller(&request).await?;
        self.authorize(&caller, VcsAppHolder::DELETE_VCS_APP)
            .await?;
        service::refuse_settings(&self.ctx)?;
        service::delete(&self.ctx).await?;
        Ok(pb::DeleteVcsAppResponse {})
    }

    /// Checks a route permission on the platform.
    async fn authorize(
        &self,
        caller: &Principal,
        permission: Permission<Platform>,
    ) -> Result<(), Error> {
        self.ctx
            .gate
            .authorize(caller, permission.on(Platform::ID))
            .await?;
        Ok(())
    }
}

/// Reads the organization of a new App: empty for the account of the person, else a login of
/// GitHub (letters, digits and inner hyphens, at most 39 characters).
fn organization(text: &str) -> Result<Option<&str>, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let valid = text.len() <= MAX_ORGANIZATION
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !text.starts_with('-')
        && !text.ends_with('-');
    match valid {
        true => Ok(Some(text)),
        false => Err(Error::invalid(
            "organization",
            "must be a GitHub login: letters, digits and inner hyphens",
        )),
    }
}

/// Reads the code of the redirect of GitHub: required, letters, digits, `-` and `_`.
fn code(text: &str) -> Result<&str, Error> {
    if text.is_empty() {
        return Err(Error::invalid("code", "is required"));
    }
    let valid = text.len() <= MAX_CODE
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    match valid {
        true => Ok(text),
        false => Err(Error::invalid("code", "is not a code of GitHub")),
    }
}
