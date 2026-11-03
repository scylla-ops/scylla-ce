//! The key pair of the agent, and the exchange of a signed assertion for a token.

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use prost::Message;
use prost_types::Timestamp;
use scylla_proto::apps::v1::AppAssertion;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::ExchangeAppAssertionRequest;
use scylla_proto::common::v1::Id;
use tonic::transport::Channel;

use crate::agent::AgentError;
use crate::config::AgentConfig;

/// The life of an assertion, in seconds.
const ASSERTION_LIFE: i64 = 60;

/// Makes a new Ed25519 key pair.
pub(crate) fn new_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system gives random bytes");
    SigningKey::from_bytes(&bytes)
}

/// A token of the agent and its expiry, in unix seconds.
pub(crate) struct Token {
    /// The token.
    pub(crate) value: String,
    /// The expiry.
    pub(crate) expires_at: i64,
}

/// Signs a new assertion with the private key of the file, and exchanges it for a token.
pub(crate) async fn token(channel: Channel, config: &AgentConfig) -> Result<Token, AgentError> {
    // Read the private key.
    let bytes =
        hex::decode(&config.private_key).map_err(|error| AgentError::File(error.to_string()))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| AgentError::File("the private key must have 32 bytes".to_owned()))?;
    let key = SigningKey::from_bytes(&bytes);

    // Sign an assertion with a new nonce.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs() as i64)
        .unwrap_or_default();
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).expect("the system gives random bytes");
    let claims = AppAssertion {
        app_id: Some(Id {
            value: config.app_id.clone(),
        }),
        key_id: Some(Id {
            value: config.key_id.clone(),
        }),
        audience: "scylla".to_owned(),
        expires_at: Some(Timestamp {
            seconds: now + ASSERTION_LIFE,
            nanos: 0,
        }),
        nonce: hex::encode(nonce),
    };
    let assertion = claims.encode_to_vec();
    let request = ExchangeAppAssertionRequest {
        signature: key.sign(&assertion).to_bytes().to_vec(),
        assertion,
    };

    // Exchange it.
    let response = TokenServiceClient::new(channel)
        .exchange_app_assertion(request)
        .await
        .map_err(|status| AgentError::of_status(&status))?;
    let token = response
        .into_inner()
        .token
        .ok_or_else(|| AgentError::Connection("no token in the answer".to_owned()))?;
    Ok(Token {
        value: token.value,
        expires_at: token.expires_at.map(|at| at.seconds).unwrap_or(now),
    })
}
