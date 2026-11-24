//! The claims of a token: the actor, the expiry and the session that the first block signs, and
//! the checks of the token on an operation.

use std::collections::HashMap;
use std::time::Duration;

use biscuit_auth::builder::Term;
use biscuit_auth::{AuthorizerBuilder, AuthorizerLimits, Biscuit};
use scylla_domain::authz::Check;
use scylla_domain::identity::actors::{Actor, ActorId, ActorKind};
use scylla_domain::identity::api_tokens::ApiTokenId;
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::users::SessionId;
use scylla_extension::{AccessError, Binding, Operation};
use time::OffsetDateTime;

use super::TokenKeys;

/// The longest time of one Datalog run on a token. The default of Biscuit (1 ms) fails on a
/// busy machine.
const MAX_RUN_TIME: Duration = Duration::from_millis(100);

/// The claims of the first block of a token.
pub(super) struct Claims {
    /// The actor of the token.
    pub(super) actor: Actor,
    /// The end of the validity, in unix seconds.
    pub(super) expires: i64,
    /// The session of a user that signed in, the key of an App, or the API token.
    pub(super) binding: Binding,
}

impl Claims {
    /// Returns the session of a token of a user that signed in.
    pub(super) fn session(&self) -> Option<SessionId> {
        match self.binding {
            Binding::Session(session) => Some(session),
            _ => None,
        }
    }

    /// Returns the key of a token of an App.
    pub(super) fn key(&self) -> Option<AppKeyId> {
        match self.binding {
            Binding::Key(key) => Some(key),
            _ => None,
        }
    }

    /// Returns the API token of a token that an API token gave.
    pub(super) fn api_token(&self) -> Option<ApiTokenId> {
        match self.binding {
            Binding::ApiToken(token) => Some(token),
            _ => None,
        }
    }
}

/// Signs the first block of a token: the actor, the expiry, the session of a user that signed in,
/// the key of an App or the API token, and a check of the expiry.
pub(super) fn sign(
    keys: &TokenKeys,
    actor: &Actor,
    expires_at: OffsetDateTime,
    binding: Binding,
) -> Result<String, AccessError> {
    // Name the actor and the expiry, and the session if there is one.
    let mut params = HashMap::from([
        ("id".to_owned(), Term::Str(actor.id.to_string())),
        ("kind".to_owned(), Term::Str(actor.kind.as_str().to_owned())),
        (
            "exp".to_owned(),
            Term::Date(expires_at.unix_timestamp() as u64),
        ),
        (
            "until".to_owned(),
            Term::Integer(expires_at.unix_timestamp()),
        ),
    ]);
    let mut code =
        "actor({id}, {kind}); expires({until}); check if time($t), $t <= {exp};".to_owned();
    match binding {
        Binding::None => {}
        Binding::Session(session) => {
            params.insert("session".to_owned(), Term::Str(session.to_string()));
            code.push_str(" session({session});");
        }
        Binding::Key(key) => {
            params.insert("key".to_owned(), Term::Str(key.to_string()));
            code.push_str(" key({key});");
        }
        Binding::ApiToken(token) => {
            params.insert("api_token".to_owned(), Term::Str(token.to_string()));
            code.push_str(" api_token({api_token});");
        }
    }

    // Sign the block with the root key.
    Biscuit::builder()
        .code_with_params(&code, params, HashMap::new())
        .and_then(|builder| builder.root_key_id(keys.key_id()).build(keys.signer()))
        .and_then(|token| token.to_base64())
        .map_err(|error| AccessError::Unavailable(format!("token signing failed: {error}")))
}

/// Reads the actor, the expiry, and the session, the key or the API token from the first block
/// of a token.
/// Later blocks cannot add them.
pub(super) fn claims(token: &Biscuit) -> Result<Claims, AccessError> {
    // Query the facts of the first block.
    let mut authorizer = AuthorizerBuilder::new()
        .set_limits(limits())
        .build(token)
        .map_err(|_| unauthenticated("invalid token"))?;
    let actors: Vec<(String, String)> = authorizer
        .query("data($id, $kind) <- actor($id, $kind)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let expiries: Vec<(i64,)> = authorizer
        .query("data($exp) <- expires($exp)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let sessions: Vec<(String,)> = authorizer
        .query("data($id) <- session($id)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let app_keys: Vec<(String,)> = authorizer
        .query("data($id) <- key($id)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let api_tokens: Vec<(String,)> = authorizer
        .query("data($id) <- api_token($id)")
        .map_err(|_| unauthenticated("invalid token"))?;

    // Keep exactly one actor and one expiry, and at most one session, key or API token.
    let [(id, kind)] = actors.as_slice() else {
        return Err(unauthenticated("the token must name one actor"));
    };
    let id = ActorId::parse(id).map_err(|_| unauthenticated("invalid actor in the token"))?;
    let expires = expiries
        .first()
        .map(|(exp,)| *exp)
        .ok_or_else(|| unauthenticated("the token has no expiry"))?;
    let bound = (
        sessions.as_slice(),
        app_keys.as_slice(),
        api_tokens.as_slice(),
    );
    let binding = match bound {
        ([], [], []) => Binding::None,
        ([(session,)], [], []) => {
            Binding::Session(SessionId::parse(session).map_err(|_| invalid())?)
        }
        ([], [(key,)], []) => Binding::Key(AppKeyId::parse(key).map_err(|_| invalid())?),
        ([], [], [(token,)]) => Binding::ApiToken(ApiTokenId::parse(token).map_err(|_| invalid())?),
        _ => return Err(invalid()),
    };
    let actor = Actor {
        id,
        kind: ActorKind::from_name(kind.clone()),
    };
    Ok(Claims {
        actor,
        expires,
        binding,
    })
}

/// Returns the limits of one Datalog run on a token.
fn limits() -> AuthorizerLimits {
    AuthorizerLimits {
        max_time: MAX_RUN_TIME,
        ..AuthorizerLimits::default()
    }
}

/// Makes the error of a token with a bad session, key or API token.
fn invalid() -> AccessError {
    unauthenticated("invalid session or key in the token")
}

/// Makes an `Unauthenticated` error.
pub(super) fn unauthenticated(reason: &str) -> AccessError {
    AccessError::Unauthenticated(reason.to_owned())
}

/// Returns `true` if the checks of a token allow an operation on the object of a check.
pub(super) fn allows(token: &Biscuit, operation: &Operation, check: &Check) -> bool {
    let params = HashMap::from([
        ("op".to_owned(), Term::Str(operation.as_str().to_owned())),
        (
            "type".to_owned(),
            Term::Str(check.object().kind().to_owned()),
        ),
        (
            "object".to_owned(),
            Term::Str(check.object().id().to_owned()),
        ),
    ]);
    AuthorizerBuilder::new()
        .set_limits(limits())
        .time()
        .code_with_params(
            "operation({op}); resource({type}, {object}); allow if actor($id, $kind);",
            params,
            HashMap::new(),
        )
        .and_then(|builder| builder.build(token))
        .and_then(|mut authorizer| authorizer.authorize())
        .is_ok()
}
