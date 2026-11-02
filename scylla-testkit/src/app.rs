//! `TestApp`: the real server on a free port, with a real gate, and one helper per core RPC.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use scylla_core::access::{Access, AccessConfig, SpiceDb, TokenKeys};
use scylla_core::features::actors;
use scylla_core::server::{self, ServeOptions};
use scylla_domain::actors::{ActorId, ActorKind};
use scylla_domain::authz::{Check, Relationship, Schema};
use scylla_extension::{AccessError, Edition, Gate, Operation};
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

/// The SpiceDB of the tests (`spicedb serve-testing`), unless `SCYLLA_TEST_SPICEDB_ENDPOINT`
/// names another. It gives an empty store to each key.
const SPICEDB_ENDPOINT: &str = "http://127.0.0.1:55052";

/// An endpoint where no SpiceDB answers: the system refuses a connection to port 0 at once.
const NO_SPICEDB: &str = "http://127.0.0.1:0";

/// The service of the routes that `TestApp::can` makes for its checks.
const TEST_SERVICE: &str = "/scylla.test.v1.TestService";

/// A running server, its gate and its clients.
pub struct TestApp {
    /// The pool of the test database.
    pub pool: PgPool,
    /// The gate of the server, to sign tokens and write relationships in a test.
    pub access: Arc<Access>,
    /// The channel to the server, to make more clients.
    pub channel: Channel,
    /// The address of the server.
    pub addr: std::net::SocketAddr,
    /// The client of `ProjectService`, for requests that no helper makes.
    pub projects: ProjectServiceClient<Channel>,
}

impl TestApp {
    /// Starts an edition with the default options.
    pub async fn spawn(pool: PgPool, edition: Edition) -> Self {
        Self::spawn_with(pool, edition, ServeOptions::default()).await
    }

    /// Writes the schema of the edition to a new SpiceDB store, then starts its server.
    pub async fn spawn_with(pool: PgPool, edition: Edition, options: ServeOptions) -> Self {
        let access = test_access(pool.clone(), edition.authz_model().clone());
        access
            .spicedb()
            .write_schema(edition.authz_schema())
            .await
            .expect("schema applies");
        Self::start(pool, edition, options, access).await
    }

    /// Starts an edition with a SpiceDB that never answers: each SpiceDB call fails.
    pub async fn spawn_without_spicedb(pool: PgPool, edition: Edition) -> Self {
        let access = access_at(pool.clone(), edition.authz_model().clone(), NO_SPICEDB);
        Self::start(pool, edition, ServeOptions::default(), access).await
    }

    /// Applies the migrations of the edition, then starts its server with this gate.
    async fn start(pool: PgPool, edition: Edition, options: ServeOptions, access: Access) -> Self {
        // Apply the migrations to the test database.
        scylla_core::crypto::install();
        scylla_core::db::migrate(&pool, edition.migrators())
            .await
            .expect("migrations apply");

        // Start the server on a free port, and connect to it.
        let access = Arc::new(access);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(server::serve(
            listener,
            pool.clone(),
            edition,
            Arc::clone(&access),
            options,
            std::future::pending(),
        ));
        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .expect("endpoint")
            .connect()
            .await
            .expect("connect");
        Self {
            pool,
            access,
            addr,
            projects: ProjectServiceClient::new(channel.clone()),
            channel,
        }
    }

    /// Creates an actor of a kind, with no owner, and returns its id.
    pub async fn actor_of(&self, kind: ActorKind) -> ActorId {
        actors::create(&self.pool, kind)
            .await
            .expect("create actor")
            .id
    }

    /// Creates a user and returns its id.
    pub async fn user(&self) -> ActorId {
        self.actor_of(scylla_domain::users::KIND).await
    }

    /// Signs a token for an actor.
    pub async fn token(&self, actor: ActorId) -> String {
        let actor = actors::find(&self.pool, actor).await.expect("actor");
        self.gate().issue(&actor).expect("token").value
    }

    /// Makes a request sent by an actor, with a fresh token.
    pub async fn authed<M>(&self, actor: ActorId, message: M) -> Request<M> {
        with_token(&self.token(actor).await, message)
    }

    /// Writes relationships, for example `Actor::DELEGATE.of(owner, delegate)`.
    pub async fn grant(&self, relationships: &[Relationship]) {
        self.gate()
            .write(relationships)
            .await
            .expect("write relationships");
    }

    /// Returns `true` if an actor passes a check, with a fresh token, as in a request to the
    /// route of the permission: `delete_project` is the route `DeleteProject`.
    pub async fn can(&self, actor: ActorId, check: Check) -> bool {
        // Make the route of the permission: `delete_project` gives `DeleteProject`.
        let method: String = check
            .permission()
            .split('_')
            .map(|word| word[..1].to_uppercase() + &word[1..])
            .collect();
        let operation = Operation::from_path(&format!("{TEST_SERVICE}/{method}"));

        // Authenticate a fresh token, then check, as the route would.
        let request = with_token(&self.token(actor).await, ());
        let principal = self
            .gate()
            .authenticate(request.metadata(), &operation)
            .await
            .expect("valid token");
        match self.gate().authorize(&principal, check).await {
            Ok(()) => true,
            Err(AccessError::Denied(_)) => false,
            Err(error) => panic!("the check fails: {error}"),
        }
    }

    /// Calls `CreateProject` as `actor`, for the actor itself.
    pub async fn create_project(&self, actor: ActorId, name: &str) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            owner_id: None,
            name: name.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().create_project(request).await?;
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
        let request = self.authed(actor, request).await;
        Ok(self
            .projects
            .clone()
            .list_projects(request)
            .await?
            .into_inner())
    }

    /// Calls `DeleteProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_project(&self, actor: ActorId, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        self.projects.clone().delete_project(request).await?;
        Ok(())
    }

    /// Calls `GetCapabilities`, with no token.
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

    /// Returns the gate as the trait that the features use.
    fn gate(&self) -> &dyn Gate {
        self.access.as_ref()
    }
}

/// Makes the gate of a test: a new signing key, and a new key of the SpiceDB of the tests, so
/// each test starts with an empty SpiceDB store and runs in parallel with the others.
pub fn test_access(pool: PgPool, schema: Schema) -> Access {
    let endpoint =
        env::var("SCYLLA_TEST_SPICEDB_ENDPOINT").unwrap_or_else(|_| SPICEDB_ENDPOINT.into());
    access_at(pool, schema, &endpoint)
}

/// Makes a gate with a new signing key and a new key of the SpiceDB at `endpoint`.
fn access_at(pool: PgPool, schema: Schema, endpoint: &str) -> Access {
    let key = format!("test-{}", uuid::Uuid::now_v7());
    let config = AccessConfig {
        keys: TokenKeys::generate(1),
        token_ttl: Duration::from_secs(600),
        spicedb: SpiceDb::new(endpoint, &key).expect("SpiceDB client"),
        schema,
    };
    Access::new(pool, config)
}

/// Makes a request with a token in `authorization: Bearer`.
pub fn with_token<M>(token: &str, message: M) -> Request<M> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {token}")).expect("ascii token");
    request.metadata_mut().insert("authorization", value);
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
