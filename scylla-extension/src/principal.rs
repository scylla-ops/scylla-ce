//! The actor of a request, and the tokens that prove it.

use std::any::Any;
use std::sync::Arc;

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::api_tokens::ApiTokenId;
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::users::SessionId;
use time::OffsetDateTime;

use crate::Operation;

/// The actor of a request, proven by a token, and the operation of the request.
#[derive(Clone)]
pub struct Principal {
    /// The actor.
    pub actor: Actor,
    /// The operation of the request.
    operation: Operation,
    /// The verified token, opaque for the plugins.
    token: Arc<dyn Any + Send + Sync>,
    /// What the token is bound to.
    binding: Binding,
    /// The end of the validity of the token.
    expires_at: OffsetDateTime,
}

impl Principal {
    /// Makes a principal. Only a gate makes one.
    pub fn new(
        actor: Actor,
        operation: Operation,
        token: Arc<dyn Any + Send + Sync>,
        binding: Binding,
        expires_at: OffsetDateTime,
    ) -> Self {
        Self {
            actor,
            operation,
            token,
            binding,
            expires_at,
        }
    }

    /// Returns the session of the token, if the actor signed in as a user.
    pub fn session(&self) -> Option<SessionId> {
        match self.binding {
            Binding::Session(session) => Some(session),
            _ => None,
        }
    }

    /// Returns the API token that gave the token, if the actor exchanged one.
    pub fn api_token(&self) -> Option<ApiTokenId> {
        match self.binding {
            Binding::ApiToken(token) => Some(token),
            _ => None,
        }
    }

    /// Returns the end of the validity of the token.
    pub fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }

    /// Returns the operation of the request.
    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    /// Returns the verified token, for the gate that made it.
    pub fn token(&self) -> &(dyn Any + Send + Sync) {
        self.token.as_ref()
    }
}

/// What a token is bound to. The token stops when its session closes, or its key or its API
/// token is revoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Nothing: a token of the command line.
    None,
    /// The session of a user that signed in.
    Session(SessionId),
    /// The key of an App that signed an assertion.
    Key(AppKeyId),
    /// The API token that the actor exchanged.
    ApiToken(ApiTokenId),
}

/// A short-lived token for an actor.
#[derive(Clone, Debug)]
pub struct IssuedToken {
    /// The token text.
    pub value: String,
    /// The end of its validity.
    pub expires_at: OffsetDateTime,
}
