//! The tests of the agents: registration, and the exchange of a signed assertion for a token.

use std::time::{SystemTime, UNIX_EPOCH};

use biscuit_auth::KeyPair;
use ed25519_dalek::{Signer, SigningKey};
use prost::Message;
use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::agents::v1::agent_service_client::AgentServiceClient;
use scylla_proto::agents::v1::RegisterAgentRequest;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{AgentAssertion, ExchangeAgentAssertionRequest};
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_testkit::{id, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::{app, assert_code};

/// An agent of the tests: its ids and its private key.
struct TestAgent {
    /// The id of the agent.
    id: String,
    /// The id of its key.
    key_id: String,
    /// Its private key.
    key: SigningKey,
}

/// Calls `RegisterAgent` as `actor` in a workspace, with the public key of a new key pair.
async fn try_register(
    app: &TestApp,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<TestAgent, Status> {
    let private = KeyPair::new().private().to_bytes();
    let key = SigningKey::from_bytes(&private[..].try_into().unwrap());
    let request = RegisterAgentRequest {
        public_key: key.verifying_key().to_bytes().to_vec(),
        workspace_id: Some(id(workspace.to_string())),
    };
    let response = AgentServiceClient::new(app.channel.clone())
        .register_agent(app.authed(actor, request).await)
        .await?
        .into_inner();
    Ok(TestAgent {
        id: text(&response.agent_id).to_owned(),
        key_id: text(&response.key_id).to_owned(),
        key,
    })
}

/// Registers an agent in the personal workspace of a user.
async fn register(app: &TestApp, user: ActorId) -> TestAgent {
    let workspace = app.workspace_of(user).await;
    try_register(app, user, workspace).await.unwrap()
}

/// Makes a signed assertion that expires after `seconds`.
fn assertion(agent: &TestAgent, seconds: i64, nonce: &str) -> ExchangeAgentAssertionRequest {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let expires_at = now + seconds;
    let claims = AgentAssertion {
        agent_id: Some(id(agent.id.clone())),
        key_id: Some(id(agent.key_id.clone())),
        audience: "scylla".to_owned(),
        expires_at: Some(Timestamp {
            seconds: expires_at,
            nanos: 0,
        }),
        nonce: nonce.to_owned(),
    };
    let bytes = claims.encode_to_vec();
    ExchangeAgentAssertionRequest {
        signature: agent.key.sign(&bytes).to_bytes().to_vec(),
        assertion: bytes,
    }
}

/// Exchanges an assertion for a token.
async fn exchange(app: &TestApp, request: ExchangeAgentAssertionRequest) -> Result<String, Status> {
    let response = TokenServiceClient::new(app.channel.clone())
        .exchange_agent_assertion(request)
        .await?;
    Ok(response.into_inner().token.unwrap().value)
}

/// A new nonce for each assertion: a new id is unique.
fn nonce() -> String {
    ActorId::generate().to_string()
}

/// Registers an agent in a workspace as `actor`, and returns the id of the agent and a token of
/// the agent.
pub(crate) async fn agent_in(
    app: &TestApp,
    actor: ActorId,
    workspace: WorkspaceId,
) -> (ActorId, String) {
    let agent = try_register(app, actor, workspace).await.unwrap();
    let token = exchange(app, assertion(&agent, 60, &nonce()))
        .await
        .unwrap();
    (ActorId::parse(&agent.id).unwrap(), token)
}

/// An agent signs an assertion and gets a token that acts as the agent.
#[sqlx::test(migrations = false)]
async fn exchange_agent_assertion_gives_a_token(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let agent = register(&app, user).await;

    // when
    let token = exchange(&app, assertion(&agent, 60, &nonce()))
        .await
        .unwrap();

    // then
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    let listed = app
        .projects
        .clone()
        .list_projects(with_token(&token, list))
        .await;
    assert!(listed.is_ok());
}

/// The owner and the admins of a workspace register its agents. A member and another user
/// cannot.
#[sqlx::test(migrations = false)]
async fn register_agent_needs_an_admin_of_the_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (admin, member, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    let roles = [
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
    ];
    app.grant(&roles).await;

    // when
    let by_owner = try_register(&app, owner, workspace).await;
    let by_admin = try_register(&app, admin, workspace).await;
    let by_member = try_register(&app, member, workspace).await;
    let by_stranger = try_register(&app, stranger, workspace).await;

    // then
    assert!(by_owner.is_ok() && by_admin.is_ok());
    assert_eq!(
        by_member.err().map(|status| status.code()),
        Some(Code::PermissionDenied)
    );
    assert_eq!(
        by_stranger.err().map(|status| status.code()),
        Some(Code::PermissionDenied)
    );
}

/// A replayed, expired, too long, or wrongly signed assertion is refused.
#[sqlx::test(migrations = false)]
async fn exchange_agent_assertion_refuses_a_bad_assertion(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let agent = register(&app, user).await;
    let other = register(&app, user).await;
    let replayed = assertion(&agent, 60, &nonce());
    exchange(&app, replayed.clone()).await.unwrap();
    let mut forged = assertion(&agent, 60, &nonce());
    forged.signature = assertion(&other, 60, &nonce()).signature;

    // when
    let replay = exchange(&app, replayed).await;
    let expired = exchange(&app, assertion(&agent, -1, &nonce())).await;
    let too_long = exchange(&app, assertion(&agent, 3600, &nonce())).await;
    let short_nonce = exchange(&app, assertion(&agent, 60, "short")).await;
    let wrong_key = exchange(&app, forged).await;

    // then
    assert_code(replay, Code::Unauthenticated);
    assert_code(expired, Code::Unauthenticated);
    assert_code(too_long, Code::Unauthenticated);
    assert_code(short_nonce, Code::Unauthenticated);
    assert_code(wrong_key, Code::Unauthenticated);
}

/// A key that is not an Ed25519 public key is refused.
#[sqlx::test(migrations = false)]
async fn register_agent_checks_the_public_key(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let request = RegisterAgentRequest {
        public_key: vec![1, 2, 3],
        workspace_id: Some(id(app.workspace_of(user).await.to_string())),
    };

    // when
    let result = AgentServiceClient::new(app.channel.clone())
        .register_agent(app.authed(user, request).await)
        .await;

    // then
    assert_code(result, Code::InvalidArgument);
}
