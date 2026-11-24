//! The handlers of `AppService` for the CI identities of an App.

use scylla_domain::identity::apps::{App, AppId};
use scylla_domain::identity::ci_identities::{
    CiIdentityId, CiIdentityInfo, CiProvider, NewCiIdentity,
};
use scylla_proto::apps::v1 as pb;
use tonic::Request;

use super::ci_service;
use super::grpc::AppsGrpc;
use crate::api::parse::{self, ToProto};
use crate::features::identity::api_tokens;
use crate::Error;

impl AppsGrpc {
    /// Handles `AddCiIdentity`: the jobs of a repository get tokens of the App.
    pub(super) async fn add_ci(
        &self,
        request: Request<pb::AddCiIdentityRequest>,
    ) -> Result<pb::AddCiIdentityResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let provider = match pb::CiProvider::try_from(request.provider) {
            Ok(pb::CiProvider::Github) => CiProvider::GitHub,
            Ok(pb::CiProvider::Gitlab) => CiProvider::GitLab,
            _ => return Err(Error::invalid("provider", "must be GitHub or GitLab")),
        };
        let limits = api_tokens::limits(request.limits)?;
        let new = NewCiIdentity::new(
            provider,
            &request.issuer,
            &request.repository,
            &request.git_ref,
            &request.environment,
            limits,
        )
        .map_err(|(field, error)| Error::invalid(field, error.to_string()))?;
        self.authorize(&caller, App::ADD_CI_IDENTITY, id).await?;
        let identity = ci_service::add(&self.pool, id, &new, caller.actor.id).await?;
        Ok(pb::AddCiIdentityResponse {
            identity: Some(identity.to_proto()),
        })
    }

    /// Handles `ListCiIdentities`: the CI identities of an App, the oldest first.
    pub(super) async fn list_ci(
        &self,
        request: Request<pb::ListCiIdentitiesRequest>,
    ) -> Result<pb::ListCiIdentitiesResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::LIST_CI_IDENTITIES, id).await?;
        let identities = ci_service::list(&self.pool, id).await?;
        Ok(pb::ListCiIdentitiesResponse {
            identities: identities.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `RemoveCiIdentity`: no new token comes from it.
    pub(super) async fn remove_ci(
        &self,
        request: Request<pb::RemoveCiIdentityRequest>,
    ) -> Result<pb::RemoveCiIdentityResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let identity: CiIdentityId = parse::required(request.identity_id, "identity_id")?;
        self.authorize(&caller, App::REMOVE_CI_IDENTITY, id).await?;
        ci_service::remove(&self.pool, id, identity).await?;
        Ok(pb::RemoveCiIdentityResponse {})
    }
}

impl ToProto<pb::CiIdentity> for CiIdentityInfo {
    fn to_proto(self) -> pb::CiIdentity {
        let provider = match self.provider {
            CiProvider::GitHub => pb::CiProvider::Github,
            CiProvider::GitLab => pb::CiProvider::Gitlab,
        };
        pb::CiIdentity {
            id: Some(self.id.to_proto()),
            app_id: Some(self.app_id.to_proto()),
            provider: provider.into(),
            issuer: self.issuer,
            repository: self.repository,
            git_ref: self.git_ref.unwrap_or_default(),
            environment: self.environment.unwrap_or_default(),
            limits: Some(self.limits.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            created_at: Some(parse::timestamp(self.created_at)),
            last_used_at: self.last_used_at.map(parse::timestamp),
        }
    }
}
