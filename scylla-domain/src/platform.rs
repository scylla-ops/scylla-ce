//! The platform: the instance itself, the object of the routes that act on the whole instance.

use crate::Id;

/// The id of the platform object.
pub type PlatformId = Id<Platform>;

/// The instance. It has one object only, with a fixed id. The instance admins pass each route
/// on it, from Postgres, and no role gives a route on it.
#[derive(Clone, Copy, Debug)]
pub struct Platform;

impl Platform {
    /// The id of the only platform object.
    pub const ID: PlatformId = Id::singleton();
}

crate::authz! {
    /// The instance. A feature adds its routes on the instance with `extend Platform`.
    definition platform for Platform {}
}
