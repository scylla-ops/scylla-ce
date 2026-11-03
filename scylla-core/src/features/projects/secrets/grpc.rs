//! The gRPC handler of `SecretService`: authenticate, parse, authorize on the project, call the
//! service, answer. No answer holds a value.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::projects::secrets::{ListSecrets, NewSecret, Secret, SecretId, SecretValue};
use scylla_domain::projects::{Project, ProjectId};
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::projects::v1 as pb;
use scylla_proto::projects::v1::secret_service_server::{SecretService, SecretServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::projects::service as projects;
use crate::infra::secret::Sealer;
use crate::Error;

/// Makes the service. With no key, a creation and a new value give `SECRET_KEY_MISSING`.
pub(crate) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    sealer: Option<Arc<Sealer>>,
) -> SecretServiceServer<SecretsGrpc> {
    SecretServiceServer::new(SecretsGrpc { pool, gate, sealer })
}

/// The handler.
pub(crate) struct SecretsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The key of the secrets, if the settings give one.
    sealer: Option<Arc<Sealer>>,
}

#[tonic::async_trait]
impl SecretService for SecretsGrpc {
    async fn create_secret(
        &self,
        request: Request<pb::CreateSecretRequest>,
    ) -> Result<Response<pb::CreateSecretResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn update_secret(
        &self,
        request: Request<pb::UpdateSecretRequest>,
    ) -> Result<Response<pb::UpdateSecretResponse>, Status> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn delete_secret(
        &self,
        request: Request<pb::DeleteSecretRequest>,
    ) -> Result<Response<pb::DeleteSecretResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }

    async fn list_secrets(
        &self,
        request: Request<pb::ListSecretsRequest>,
    ) -> Result<Response<pb::ListSecretsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }
}

impl SecretsGrpc {
    /// Handles `CreateSecret`, in a project.
    async fn create(
        &self,
        request: Request<pb::CreateSecretRequest>,
    ) -> Result<pb::CreateSecretResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let new = NewSecret {
            project_id: parse::required(request.project_id, "project_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
            value: parse::field(request.value, "value")?,
        };

        // Check the route permission, then create the secret.
        self.authorize_in(&caller, Project::CREATE_SECRET, new.project_id)
            .await?;
        let secret = service::create(&self.pool, self.sealer.as_deref(), new).await?;
        Ok(pb::CreateSecretResponse {
            secret: Some(secret.to_proto()),
        })
    }

    /// Handles `UpdateSecret`: a new value. The name does not change.
    async fn update(
        &self,
        request: Request<pb::UpdateSecretRequest>,
    ) -> Result<pb::UpdateSecretResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: SecretId = parse::required(request.secret_id, "secret_id")?;
        let value: SecretValue = parse::field(request.value, "value")?;

        // Check the route permission, then seal the new value.
        self.authorize_on(&caller, Project::UPDATE_SECRET, id)
            .await?;
        let secret = service::update(&self.pool, self.sealer.as_deref(), id, value).await?;
        Ok(pb::UpdateSecretResponse {
            secret: Some(secret.to_proto()),
        })
    }

    /// Handles `DeleteSecret`. A pipeline can still name it: its next run fails.
    async fn delete(
        &self,
        request: Request<pb::DeleteSecretRequest>,
    ) -> Result<pb::DeleteSecretResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: SecretId = parse::required(request.into_inner().secret_id, "secret_id")?;

        // Check the route permission, then delete the secret.
        self.authorize_on(&caller, Project::DELETE_SECRET, id)
            .await?;
        service::delete(&self.pool, id).await?;
        Ok(pb::DeleteSecretResponse {})
    }

    /// Handles `ListSecrets`: the secrets of a project, in name order, with no value.
    async fn list(
        &self,
        request: Request<pb::ListSecretsRequest>,
    ) -> Result<pb::ListSecretsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListSecrets {
            project_id: parse::required(request.project_id, "project_id")?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission, then read one page.
        self.authorize_in(&caller, Project::LIST_SECRETS, query.project_id)
            .await?;
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListSecretsResponse {
            secrets: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Checks a route permission on a project and on its workspace. An unknown project gets the
    /// answer of a refusal.
    async fn authorize_in(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        id: ProjectId,
    ) -> Result<(), Error> {
        let check = permission.on(id);
        let Some(workspace) = projects::workspace(&self.pool, id).await? else {
            return Err(AccessError::denied(caller, &check).into());
        };
        self.gate.authorize(caller, check.within(workspace)).await?;
        Ok(())
    }

    /// Checks a route permission on the project of a secret and on its workspace. An unknown
    /// secret and a refusal get the same answer, which names the secret, not its project.
    async fn authorize_on(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        id: SecretId,
    ) -> Result<(), Error> {
        // Find the project of the secret. An unknown secret gets the answer of a refusal.
        let object = format!("secret:{id}");
        let reason = format!("{} on {object} is denied", permission.name());
        let Some((project, workspace)) = service::place(&self.pool, id).await? else {
            let error = AccessError::refused(caller, permission.name(), &object, reason);
            return Err(error.into());
        };

        // Check the permission on the project, and hide the project in a refusal.
        let check = permission.on(project).within(workspace);
        match self.gate.authorize(caller, check).await {
            Ok(()) => Ok(()),
            Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
            Err(error) => Err(error.into()),
        }
    }
}

impl ToProto<pb::Secret> for Secret {
    fn to_proto(self) -> pb::Secret {
        pb::Secret {
            id: Some(self.id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            name: self.name.as_str().to_owned(),
            created_by: Some(self.created_by.to_proto()),
            created_at: Some(parse::timestamp(self.created_at)),
            updated_at: Some(parse::timestamp(self.updated_at)),
        }
    }
}
