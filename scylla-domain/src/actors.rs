//! The actors: the entities that act and own resources, of any kind.

use std::borrow::Cow;

use crate::Id;

/// The id of an actor.
pub type ActorId = Id<Actor>;

/// The kind of an actor. The core defines none: each feature or edition defines its own.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActorKind(Cow<'static, str>);

impl ActorKind {
    /// Makes a kind from a constant name, in snake case.
    pub const fn new(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    /// Makes a kind from a name that is not a constant, for example from the database.
    pub fn from_name(name: String) -> Self {
        Self(Cow::Owned(name))
    }

    /// Returns the name of the kind.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An actor: its id and its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    /// The id of the actor.
    pub id: ActorId,
    /// The kind of the actor.
    pub kind: ActorKind,
}

crate::authz! {
    /// The permission model of the actors, for each kind. The features add the permissions of
    /// their routes on an owner with `extend Actor`.
    definition actor for Actor {
        /// The actor itself.
        relation identity: Actor;
        /// An actor that can act for this actor.
        relation delegate: Actor;
        /// The group of the routes that act for this actor: the actor itself and its delegates.
        permission act_for = identity + delegate;
    }
}
