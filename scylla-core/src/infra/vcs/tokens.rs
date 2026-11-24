//! The installation tokens of a GitHub App: their scope (the repositories and the permissions),
//! and their cache in memory only. A token serves until 5 minutes before its expiry, for the
//! same installation and the same scope.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, PoisonError};

use serde_json::{json, Value};
use time::{Duration, OffsetDateTime};

use super::Hidden;

/// The time before its expiry when a token stops to serve.
pub const REFRESH_MARGIN: Duration = Duration::minutes(5);

/// The repositories and the permissions of an installation token. No repository gives each
/// repository of the installation, and no permission gives each permission of the App.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct TokenScope {
    /// The ids of the repositories.
    pub repository_ids: BTreeSet<u64>,
    /// The permissions, for example `contents` to `read`.
    pub permissions: BTreeMap<String, String>,
}

/// An installation token and its expiry.
#[derive(Clone, Debug)]
pub struct InstallationToken {
    /// The token.
    pub token: Hidden,
    /// The time when GitHub stops the token.
    pub expires_at: OffsetDateTime,
}

/// The installation tokens of each installation and scope.
#[derive(Debug, Default)]
pub struct TokenCache {
    /// The tokens, by installation id and scope.
    tokens: Mutex<HashMap<(u64, TokenScope), InstallationToken>>,
}

impl TokenScope {
    /// Narrows the scope to one more repository.
    pub fn repository(mut self, id: u64) -> Self {
        self.repository_ids.insert(id);
        self
    }

    /// Narrows the scope to one more permission, for example `contents` and `read`.
    pub fn permission(mut self, name: &str, access: &str) -> Self {
        self.permissions.insert(name.to_owned(), access.to_owned());
        self
    }

    /// Returns the body of `POST /app/installations/{id}/access_tokens`.
    pub fn body(&self) -> Value {
        let mut body = json!({});
        if !self.repository_ids.is_empty() {
            body["repository_ids"] = json!(self.repository_ids);
        }
        if !self.permissions.is_empty() {
            body["permissions"] = json!(self.permissions);
        }
        body
    }
}

impl InstallationToken {
    /// Returns `true` while the token serves at the time `now`.
    pub fn serves_at(&self, now: OffsetDateTime) -> bool {
        self.expires_at - REFRESH_MARGIN > now
    }
}

impl TokenCache {
    /// Returns a token of the installation and of the scope that serves at the time `now`.
    pub fn get(
        &self,
        installation: u64,
        scope: &TokenScope,
        now: OffsetDateTime,
    ) -> Option<InstallationToken> {
        let tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        tokens
            .get(&(installation, scope.clone()))
            .filter(|token| token.serves_at(now))
            .cloned()
    }

    /// Keeps a token, and drops the tokens that do not serve at the time `now`.
    pub fn put(
        &self,
        installation: u64,
        scope: TokenScope,
        token: InstallationToken,
        now: OffsetDateTime,
    ) {
        let mut tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        tokens.retain(|_, kept| kept.serves_at(now));
        tokens.insert((installation, scope), token);
    }

    /// Drops the tokens of an installation, after a refusal or a change of the installation.
    pub fn forget(&self, installation: u64) {
        let mut tokens = self.tokens.lock().unwrap_or_else(PoisonError::into_inner);
        tokens.retain(|(kept, _), _| *kept != installation);
    }
}
