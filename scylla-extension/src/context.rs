//! What the core tells a hook about the write. Use `new()`: the structs are non-exhaustive.

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::ResourceKind;

/// A creation that a `BeforeCreate` hook can refuse.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CreateContext {
    /// The kind of the new resource.
    pub kind: ResourceKind,
    /// The actor that creates the resource.
    pub actor: Actor,
    /// The actor that will own the resource.
    pub owner_id: ActorId,
}

impl CreateContext {
    /// Makes the context of a creation.
    pub fn new(kind: ResourceKind, actor: Actor, owner_id: ActorId) -> Self {
        Self {
            kind,
            actor,
            owner_id,
        }
    }
}

/// A deletion that an `AfterDelete` hook follows.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DeleteContext {
    /// The kind of the deleted resource.
    pub kind: ResourceKind,
    /// The actor that deletes the resource.
    pub actor: Actor,
    /// The actor that owned the resource.
    pub owner_id: ActorId,
}

impl DeleteContext {
    /// Makes the context of a deletion.
    pub fn new(kind: ResourceKind, actor: Actor, owner_id: ActorId) -> Self {
        Self {
            kind,
            actor,
            owner_id,
        }
    }
}
