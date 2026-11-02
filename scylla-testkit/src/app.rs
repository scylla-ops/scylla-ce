//! `TestApp`: the real server on a free port, its clients, and one helper per core RPC.

use std::time::Duration;

use scylla_core::features::actors::{self, ACTOR_HEADER};
use scylla_core::features::api_keys;
use scylla_core::server::{self, ServeOptions};
use scylla_domain::actors::{ActorId, ActorKind};
use scylla_extension::Edition;
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use scylla_proto::common::v1 as common;
use scylla_proto::projects::v1::project_service_client::ProjectServiceClient;
use scylla_proto::projects::v1::{
    CreateProjectRequest, DeleteProjectRequest, ListProjectsRequest, ListProjectsResponse, Project,
};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

/// Returns the server options of the tests: a long timeout, the dev header, no TLS.
fn options() -> ServeOptions {
    ServeOptions {
        request_timeout: Duration::from_secs(30),
        trust_actor_header: true,
        tls: None,
    }
}

/// A running server and its clients.
pub struct TestApp {
    /// The pool of the test database.
    pub pool: PgPool,
    /// The channel to the server, to make more clients.
    pub channel: Channel,
    /// The client of `ProjectService`, for requests that no helper makes.
    pub projects: ProjectServiceClient<Channel>,
}

impl TestApp {
    /// Applies the migrations of the edition, then starts its server on a free port.
    pub async fn spawn(pool: PgPool, edition: Edition) -> Self {
        scylla_core::db::migrate(&pool, edition.migrators())
            .await
            .expect("migrations apply");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(server::serve(
            listener,
            pool.clone(),
            edition,
            options(),
            std::future::pending(),
        ));
        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .expect("endpoint")
            .connect()
            .await
            .expect("connect");
        Self {
            pool,
            projects: ProjectServiceClient::new(channel.clone()),
            channel,
        }
    }

    /// Creates an actor of the kind `test` and returns its id.
    pub async fn actor(&self) -> ActorId {
        actors::create(&self.pool, ActorKind::new("test"))
            .await
            .expect("create actor")
            .id
    }

    /// Creates an API key for an actor and returns the key.
    pub async fn api_key(&self, actor: ActorId) -> String {
        api_keys::create(&self.pool, actor)
            .await
            .expect("create key")
            .secret
    }

    /// Calls `CreateProject` as `actor`, for the actor itself.
    pub async fn create_project(&self, actor: ActorId, name: &str) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            owner_id: None,
            name: name.to_owned(),
        };
        let response = self
            .projects
            .clone()
            .create_project(as_actor(actor, request))
            .await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Calls `ListProjects` as `actor`, for the projects of the actor.
    pub async fn list_projects(
        &self,
        actor: ActorId,
        size: i32,
        token: &str,
    ) -> Result<ListProjectsResponse, Status> {
        let request = ListProjectsRequest {
            owner_id: None,
            page: Some(common::PageRequest {
                size,
                token: token.to_owned(),
            }),
        };
        let response = self
            .projects
            .clone()
            .list_projects(as_actor(actor, request))
            .await?;
        Ok(response.into_inner())
    }

    /// Calls `DeleteProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_project(&self, actor: ActorId, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: Some(id(project_id)),
        };
        self.projects
            .clone()
            .delete_project(as_actor(actor, request))
            .await?;
        Ok(())
    }

    /// Calls `GetCapabilities`, with no actor.
    pub async fn get_capabilities(&self) -> Result<GetCapabilitiesResponse, Status> {
        let mut client = CapabilityServiceClient::new(self.channel.clone());
        let response = client.get_capabilities(GetCapabilitiesRequest {}).await?;
        Ok(response.into_inner())
    }

    /// Calls the standard health check for the whole server.
    pub async fn health_check(&self) -> Result<ServingStatus, Status> {
        let mut client = HealthClient::new(self.channel.clone());
        let request = HealthCheckRequest {
            service: String::new(),
        };
        Ok(client.check(request).await?.into_inner().status())
    }

    /// Counts the projects of an owner in the database.
    pub async fn project_count(&self, owner: ActorId) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM projects WHERE owner_id = $1"#,
            owner.to_db(),
        )
        .fetch_one(&self.pool)
        .await
        .expect("count projects")
    }
}

/// Makes a request sent by `actor`, with the dev header.
pub fn as_actor<M>(actor: ActorId, message: M) -> Request<M> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(actor.to_string()).expect("ascii id");
    request.metadata_mut().insert(ACTOR_HEADER, value);
    request
}

/// Makes a request that sends an API key.
pub fn with_key<M>(key: &str, message: M) -> Request<M> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {key}")).expect("ascii key");
    request.metadata_mut().insert(api_keys::HEADER, value);
    request
}

/// Makes an id message from text.
pub fn id(value: impl Into<String>) -> common::Id {
    common::Id {
        value: value.into(),
    }
}

/// Returns the text of an id of a response.
pub fn text(id: &Option<common::Id>) -> &str {
    &id.as_ref().expect("id in response").value
}
