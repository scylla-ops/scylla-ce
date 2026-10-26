use scylla_core::grpc::convert::Parse;
use tonic::Status;

pub mod apps;
pub mod grants;
pub mod organizations;
pub mod pipelines;
pub mod projects;
pub mod roles;
pub mod secrets;
pub mod users;

pub use apps::{AppAuthServiceClient, AppAuthServiceServer, AppServiceClient, AppServiceServer};
pub use grants::{GrantServiceClient, GrantServiceServer};
pub use organizations::{OrganizationServiceClient, OrganizationServiceServer};
pub use pipelines::{PipelineServiceClient, PipelineServiceServer};
pub use projects::{ProjectServiceClient, ProjectServiceServer};
pub use roles::{RoleServiceClient, RoleServiceServer};
pub use secrets::{SecretServiceClient, SecretServiceServer};
pub use users::{UserServiceClient, UserServiceServer};

#[allow(clippy::all, clippy::pedantic)]
pub mod service {
    pub mod auth {
        tonic::include_proto!("scylla.auth.v1");
    }
    pub mod user {
        tonic::include_proto!("scylla.user.v1");
    }
    pub mod organization {
        tonic::include_proto!("scylla.organization.v1");
    }
    pub mod project {
        tonic::include_proto!("scylla.project.v1");
    }
    pub mod authz {
        tonic::include_proto!("scylla.authz.v1");
    }
    pub mod pipeline {
        tonic::include_proto!("scylla.pipeline.v1");
    }
    pub mod secret {
        tonic::include_proto!("scylla.secret.v1");
    }
    pub mod job {
        tonic::include_proto!("scylla.job.v1");
    }
    pub mod app {
        tonic::include_proto!("scylla.app.v1");
    }
    pub mod agent {
        tonic::include_proto!("scylla.agent.v1");
    }
    pub mod trigger {
        tonic::include_proto!("scylla.trigger.v1");
    }
}

pub(crate) fn parse<R: Parse>(req: R) -> Result<R::Into, Status> {
    req.parse()
}

pub(crate) async fn authed<A: crate::Access>(call: &strata_tonic::Call<A>) -> Result<(), Status> {
    crate::AccessRpc::authenticated(&call.scoped())
        .await
        .map_err(scylla_core::grpc::domain_error_to_status)
}
