//! The plugin of the API keys feature.

use std::sync::Arc;

use scylla_extension::{EditionBuilder, Plugin};

use super::auth::KeyAuthenticator;

/// Adds the API key authenticator. The table comes from the core migrations.
pub(crate) struct ApiKeys;

impl Plugin for ApiKeys {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authenticator(Arc::new(KeyAuthenticator))
    }
}
