//! The join of an App: the agent makes its key pair and sends the public key with the code, and
//! a first start joins when it has a code and no file.

use std::path::Path;

use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::JoinAppRequest;

use super::{connect, keys};
use crate::agent::AgentError;
use crate::config::AgentConfig;

/// Joins the App of a join code, and returns the file to keep. The private key stays on this
/// machine, and the key gets the name of the machine.
pub async fn join(url: &str, code: &str) -> Result<AgentConfig, AgentError> {
    let key = keys::new_key();
    let request = JoinAppRequest {
        join_code: code.trim().to_owned(),
        public_key: key.verifying_key().to_bytes().to_vec(),
        label: crate::stream::hostname().unwrap_or_default(),
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

/// Reads the file of the agent at `path`. When it does not exist and `join_with` has the URL of the
/// server and a join code, joins first and writes the file. Returns `true` with the file of a
/// new join.
pub async fn load_or_join(
    path: &Path,
    join_with: Option<(&str, &str)>,
) -> Result<(AgentConfig, bool), AgentError> {
    match join_with {
        Some((url, code)) if !path.exists() => {
            let config = join(url, code).await?;
            config.save(path)?;
            Ok((config, true))
        }
        _ => Ok((AgentConfig::load(path)?, false)),
    }
}
