//! The claims of a token: the actor, the expiry and the session that the first block signs, and
//! the checks of the token on an operation.

use std::collections::HashMap;

use biscuit_auth::builder::Term;
use biscuit_auth::{AuthorizerBuilder, Biscuit};
use scylla_domain::authz::Check;
use scylla_domain::identity::actors::{Actor, ActorId, ActorKind};
use scylla_domain::identity::users::SessionId;
use scylla_extension::{AccessError, Operation};
use time::OffsetDateTime;

use super::TokenKeys;

/// The claims of the first block of a token.
pub(super) struct Claims {
    /// The actor of the token.
    pub(super) actor: Actor,
    /// The end of the validity, in unix seconds.
    pub(super) expires: i64,
    /// The session of a user that signed in.
    pub(super) session: Option<SessionId>,
}

/// Signs the first block of a token: the actor, the expiry, the session of a user that signed in,
/// and a check of the expiry.
pub(super) fn sign(
    keys: &TokenKeys,
    actor: &Actor,
    expires_at: OffsetDateTime,
    session: Option<SessionId>,
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
    if let Some(session) = session {
        params.insert("session".to_owned(), Term::Str(session.to_string()));
        code.push_str(" session({session});");
    }

    // Sign the block with the root key.
    Biscuit::builder()
        .code_with_params(&code, params, HashMap::new())
        .and_then(|builder| builder.root_key_id(keys.key_id()).build(keys.signer()))
        .and_then(|token| token.to_base64())
        .map_err(|error| AccessError::Unavailable(format!("token signing failed: {error}")))
}

/// Reads the actor, the expiry and the session from the first block of a token. Later blocks
/// cannot add them.
pub(super) fn claims(token: &Biscuit) -> Result<Claims, AccessError> {
    // Query the facts of the first block.
    let mut authorizer = AuthorizerBuilder::new()
        .build(token)
        .map_err(|_| unauthenticated("invalid token"))?;
    let actors: Vec<(String, String)> = authorizer
        .query("data($id, $kind) <- actor($id, $kind)")
        .map_err(|_| unauthenticated("invalid token"))?;
    let expiries: Vec<(i64,)> = authorizer
        .query("data($exp) <- expires($exp)")
        .unwrap_or_default();
    let sessions: Vec<(String,)> = authorizer
        .query("data($id) <- session($id)")
        .unwrap_or_default();

    // Keep exactly one actor and one expiry, and at most one session.
    let [(id, kind)] = actors.as_slice() else {
        return Err(unauthenticated("the token must name one actor"));
    };
    let id = ActorId::parse(id).map_err(|_| unauthenticated("invalid actor in the token"))?;
    let expires = expiries
        .first()
        .map(|(exp,)| *exp)
        .ok_or_else(|| unauthenticated("the token has no expiry"))?;
    let session = match sessions.as_slice() {
        [] => None,
        [(session,)] => Some(SessionId::parse(session).map_err(|_| invalid())?),
        _ => return Err(invalid()),
    };
    let actor = Actor {
        id,
        kind: ActorKind::from_name(kind.clone()),
    };
    Ok(Claims {
        actor,
        expires,
        session,
    })
}

/// Makes the error of a token with a bad session.
fn invalid() -> AccessError {
    unauthenticated("invalid session in the token")
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
