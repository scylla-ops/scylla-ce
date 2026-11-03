//! The join of an App: the agent makes its key pair and sends the public key with the code.

use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::JoinAppRequest;

use super::{connect, keys};
use crate::agent::AgentError;
use crate::config::AgentConfig;

/// Joins the App of a join code, and returns the file to keep. The private key stays on this
/// machine.
pub async fn join(url: &str, code: &str) -> Result<AgentConfig, AgentError> {
    let key = keys::new_key();
    let request = JoinAppRequest {
        join_code: code.trim().to_owned(),
        public_key: key.verifying_key().to_bytes().to_vec(),
    };
    let channel = connect(url).await?;
    let response = TokenServiceClient::new(channel)
        .join_app(request)
        .await
        .map_err(|status| AgentError::of_status(&status))?
        .into_inner();
    let text = |id: Option<scylla_proto::common::v1::Id>| id.map(|id| id.value).unwrap_or_default();
    Ok(AgentConfig {
        url: url.trim_end_matches('/').to_owned(),
        app_id: text(response.app_id),
        key_id: text(response.key_id),
        private_key: hex::encode(key.to_bytes()),
    })
}
