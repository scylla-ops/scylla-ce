//! The users: the persons, who sign in with an identity provider.

use crate::actors::ActorKind;

/// The actor kind of a user.
pub const KIND: ActorKind = ActorKind::new("user");
