//! `TestApp`: the real server on a free port, with a real gate, its actors and its tokens.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use scylla_core::access::{Access, AccessConfig, SpiceDb, TokenKeys};
use scylla_core::api::{self, ServeOptions};
use scylla_core::features::identity::{accounts, actors};
use scylla_core::infra::secret::Sealer;
use scylla_domain::authz::{Check, Relationship, Schema};
use scylla_domain::identity::actors::{ActorId, ActorKind};
use scylla_domain::identity::users::{Email, NewUser, Password};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Name;
use scylla_extension::{AccessError, Binding, Edition, Gate, NotificationKinds, Operation};
use scylla_proto::projects::v1::project_service_client::ProjectServiceClient;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

use crate::mail::Mailbox;
use crate::rpc::with_token;

/// The SpiceDB of the tests (`spicedb serve-testing`), unless `SCYLLA_TEST_SPICEDB_ENDPOINT`
/// names another. It gives an empty store to each key.
const SPICEDB_ENDPOINT: &str = "http://127.0.0.1:55052";

/// An endpoint where no SpiceDB answers: the system refuses a connection to port 0 at once.
const NO_SPICEDB: &str = "http://127.0.0.1:0";

/// The service of the routes that `TestApp::can` makes for its checks.
const TEST_SERVICE: &str = "/scylla.test.v1.TestService";

/// A running server, its gate and its clients.
pub struct TestApp {
    /// The pool of the server on the test database.
    pub pool: PgPool,
    /// The gate of the server, to sign tokens and write relationships in a test.
    pub access: Arc<Access>,
    /// The channel to the server, to make more clients.
    pub channel: Channel,
    /// The address of the server.
    pub addr: std::net::SocketAddr,
    /// The client of `ProjectService`, for requests that no helper makes.
    pub projects: ProjectServiceClient<Channel>,
    /// The mails that the server sent.
    pub mails: Arc<Mailbox>,
    /// The key of the secrets of the server, or `None` for a server with no key.
    pub sealer: Option<Arc<Sealer>>,
    /// The kinds of notifications of the edition.
    pub notification_kinds: Arc<NotificationKinds>,
}

impl TestApp {
    /// Starts an edition with the default options.
    pub async fn spawn(pool: PgPool, edition: Edition) -> Self {
        Self::spawn_with(pool, edition, ServeOptions::default()).await
    }

    /// Writes the schema of the edition to a new SpiceDB store, then starts its server.
    pub async fn spawn_with(pool: PgPool, edition: Edition, options: ServeOptions) -> Self {
        let pool = server_pool(&pool);
        let access = schema_access(&pool, &edition).await;
        Self::start(pool, edition, options, access, true).await
    }

    /// Starts an edition with no key of the secrets: the server refuses to seal a secret.
    pub async fn spawn_without_secret_key(pool: PgPool, edition: Edition) -> Self {
        let pool = server_pool(&pool);
        let access = schema_access(&pool, &edition).await;
        Self::start(pool, edition, ServeOptions::default(), access, false).await
    }

    /// Starts an edition with a SpiceDB that never answers: each SpiceDB call fails.
    pub async fn spawn_without_spicedb(pool: PgPool, edition: Edition) -> Self {
        let pool = server_pool(&pool);
        let access = access_at(pool.clone(), edition.authz_model().clone(), NO_SPICEDB);
        Self::start(pool, edition, ServeOptions::default(), access, true).await
    }

    /// Applies the migrations of the edition, then starts its server with this gate, a mailbox and,
    /// if `with_key`, the key of the options or a new key of the secrets.
    async fn start(
        pool: PgPool,
        edition: Edition,
        mut options: ServeOptions,
        access: Access,
        with_key: bool,
    ) -> Self {
        // Apply the migrations to the test database.
        scylla_core::infra::tls::install();
        scylla_core::infra::db::migrate(&pool, edition.migrators())
            .await
            .expect("migrations apply");

        // Give the server a mailbox and a key of the secrets.
        let mails = Arc::new(Mailbox::default());
        options.mailer = mails.clone();
        let sealer = with_key.then(|| {
            options
                .sealer
                .clone()
                .unwrap_or_else(|| Arc::new(Sealer::generate().0))
        });
        options.sealer = sealer.clone();

        // Start the server on a free port, and connect to it.
        let access = Arc::new(access);
        let notification_kinds = edition.notification_kinds();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        options.accounts.api_url = format!("http://{addr}");
        tokio::spawn(api::serve(
            listener,
            pool.clone(),
            edition,
            Arc::clone(&access),
            options,
            pool.close_event(),
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
            mails,
            sealer,
            notification_kinds,
        }
    }

    /// Creates an actor of a kind, with no owner, and returns its id.
    pub async fn actor_of(&self, kind: ActorKind) -> ActorId {
        actors::create(&self.pool, kind)
            .await
            .expect("create actor")
            .id
    }

    /// Creates an active user with a new email and its personal workspace, and returns its id.
    pub async fn user(&self) -> ActorId {
        let email = format!("{}@test.example", ActorId::generate());
        self.user_with(&email, None).await
    }

    /// Creates an active user with a verified email, a password if one is given, and its
    /// personal workspace, and returns its id.
    pub async fn user_with(&self, email: &str, password: Option<&str>) -> ActorId {
        let new = NewUser {
            email: Email::parse(email.to_owned()).expect("email"),
            email_verified: true,
            display_name: Name::parse("Test User".to_owned()).expect("name"),
            handle: None,
            password: password.map(|text| Password::parse(text.to_owned()).expect("password")),
        };
        accounts::create_user(&self.pool, new)
            .await
            .expect("create user")
            .id
    }

    /// Returns the first workspace that an actor owns: the personal workspace of a user.
    pub async fn workspace_of(&self, owner: ActorId) -> WorkspaceId {
        let id = sqlx::query_scalar!(
            "SELECT workspace_id FROM workspace_owners WHERE user_id = $1
             ORDER BY workspace_id LIMIT 1",
            owner.to_db(),
        )
        .fetch_one(&self.pool)
        .await
        .expect("a workspace of the owner");
        WorkspaceId::from_db(id)
    }

    /// Signs a token for an actor.
    pub async fn token(&self, actor: ActorId) -> String {
        let actor = actors::find(&self.pool, actor).await.expect("actor");
        self.gate()
            .issue(&actor, Binding::None)
            .expect("token")
            .value
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

    /// Counts the owners of a workspace.
    pub async fn owner_count(&self, workspace: WorkspaceId) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM workspace_owners WHERE workspace_id = $1"#,
            workspace.to_db(),
        )
        .fetch_one(&self.pool)
        .await
        .expect("count owners")
    }

    /// Returns the gate as the trait that the features use.
    fn gate(&self) -> &dyn Gate {
        self.access.as_ref()
    }
}

/// Opens a pool on the database of a test pool, with its size and no parent, so the server and its
/// tasks do not share the 20 connections of `sqlx::test`. The pool closes, and the server stops,
/// when the test pool closes: sqlx can then drop the test database.
fn server_pool(test_pool: &PgPool) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(test_pool.options().get_max_connections())
        .connect_lazy_with((*test_pool.connect_options()).clone());
    let test_closed = test_pool.close_event();
    let closing = pool.clone();
    tokio::spawn(async move {
        test_closed.await;
        closing.close().await;
    });
    pool
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

/// Makes the gate of a test and writes the schema of the edition to its new SpiceDB store.
async fn schema_access(pool: &PgPool, edition: &Edition) -> Access {
    let access = test_access(pool.clone(), edition.authz_model().clone());
    access
        .spicedb()
        .write_schema(edition.authz_schema())
        .await
        .expect("schema applies");
    access
}
