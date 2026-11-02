//! The tests of the agents: registration, and the exchange of a signed assertion for a token.

use std::time::{SystemTime, UNIX_EPOCH};

use biscuit_auth::KeyPair;
use ed25519_dalek::{Signer, SigningKey};
use prost::Message;
use prost_types::Timestamp;
use scylla_domain::actors::ActorId;
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

/// Registers an agent for a user, with a new key pair.
async fn register(app: &TestApp, user: ActorId) -> TestAgent {
    let private = KeyPair::new().private().to_bytes();
    let key = SigningKey::from_bytes(&private[..].try_into().unwrap());
    let request = RegisterAgentRequest {
        public_key: key.verifying_key().to_bytes().to_vec(),
    };
    let response = AgentServiceClient::new(app.channel.clone())
        .register_agent(app.authed(user, request).await)
        .await
        .unwrap()
        .into_inner();
    TestAgent {
        id: text(&response.agent_id).to_owned(),
        key_id: text(&response.key_id).to_owned(),
        key,
    }
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
        owner_id: None,
        page: None,
    };
    let listed = app
        .projects
        .clone()
        .list_projects(with_token(&token, list))
        .await;
    assert!(listed.is_ok());
}

/// The user that registered an agent can act for it, but the agent cannot act for the user.
#[sqlx::test(migrations = false)]
async fn an_agent_does_not_act_for_its_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let agent = register(&app, user).await;
    let token = exchange(&app, assertion(&agent, 60, &nonce()))
        .await
        .unwrap();
    let request = |owner: String| ListProjectsRequest {
        owner_id: Some(id(owner)),
        page: None,
    };
    let mut client = app.projects.clone();

    // when
    let user_for_agent = client
        .list_projects(app.authed(user, request(agent.id.clone())).await)
        .await;
    let agent_for_user = client
        .list_projects(with_token(&token, request(user.to_string())))
        .await;

    // then
    assert!(user_for_agent.is_ok());
    assert_code(agent_for_user, Code::PermissionDenied);
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
    };

    // when
    let result = AgentServiceClient::new(app.channel.clone())
        .register_agent(app.authed(user, request).await)
        .await;

    // then
    assert_code(result, Code::InvalidArgument);
}
