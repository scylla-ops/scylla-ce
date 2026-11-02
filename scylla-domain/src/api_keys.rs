//! The API keys: secret keys that authenticate the clients of an actor.

use crate::Id;

/// The marker of an API key id.
pub enum ApiKey {}

/// The id of an API key.
pub type ApiKeyId = Id<ApiKey>;
