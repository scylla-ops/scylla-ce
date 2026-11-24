//! The short tokens of the API tokens on the JSON gateway: a caller sends the secret of an API
//! token, the gateway exchanges it one time with `TokenService.ExchangeApiToken`, and keeps the
//! short token until a minute before its end. The check of each request still stops a revoked
//! API token at once.

use std::collections::HashMap;

use axum::http::HeaderValue;
use prost::Message;
use scylla_domain::identity::api_tokens::{APP_TOKEN_PREFIX, PERSONAL_TOKEN_PREFIX};
use scylla_proto::auth::v1::{ExchangeApiTokenRequest, ExchangeApiTokenResponse};
use time::{Duration, OffsetDateTime};
use tokio::sync::Mutex;
use tonic::Status;

use super::json::Gateway;
use crate::infra::secret;

/// The largest number of short tokens that the gateway keeps.
const MAX_TOKENS: usize = 10_000;

/// The time before its end when the gateway stops using a short token.
const MARGIN: Duration = Duration::minutes(1);

/// The short tokens, by the hash of the secret of their API token.
#[derive(Default)]
pub(super) struct TokenCache {
    /// The short tokens, with their end.
    tokens: Mutex<HashMap<Vec<u8>, (HeaderValue, OffsetDateTime)>>,
}

impl TokenCache {
    /// Returns `true` for the secret of an API token.
    pub(super) fn is_api_token(secret: &str) -> bool {
        secret.starts_with(APP_TOKEN_PREFIX) || secret.starts_with(PERSONAL_TOKEN_PREFIX)
    }

    /// Returns the authorization of the short token of an API token, and exchanges the secret
    /// when the gateway has no valid short token for it.
    pub(super) async fn short_token(
        &self,
        gateway: &Gateway,
        secret: &str,
    ) -> Result<HeaderValue, Status> {
        let key = secret::hash(secret);
        let now = OffsetDateTime::now_utc();
        if let Some((value, end)) = self.tokens.lock().await.get(&key) {
            if *end - MARGIN > now {
                return Ok(value.clone());
            }
        }
        let request = ExchangeApiTokenRequest {
            api_token: secret.to_owned(),
        };
        let path = "/scylla.auth.v1.TokenService/ExchangeApiToken";
        let bytes = gateway.unary(path, None, request.encode_to_vec()).await?;
        let token = ExchangeApiTokenResponse::decode(bytes)
            .ok()
            .and_then(|response| response.token)
            .ok_or_else(|| Status::internal("the exchange answered badly"))?;
        let end = token
            .expires_at
            .and_then(|at| OffsetDateTime::from_unix_timestamp(at.seconds).ok())
            .unwrap_or(now);
        let value = HeaderValue::try_from(format!("Bearer {}", token.value))
            .map_err(|_| Status::internal("the exchange gave a bad token"))?;
        let mut tokens = self.tokens.lock().await;
        if tokens.len() >= MAX_TOKENS {
            tokens.retain(|_, (_, end)| *end - MARGIN > now);
        }
        tokens.insert(key, (value.clone(), end));
        Ok(value)
    }
}
