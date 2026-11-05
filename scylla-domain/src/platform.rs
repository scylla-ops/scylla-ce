//! The platform: the instance itself, the object of the routes that act on the whole instance,
//! and its settings.

use crate::Id;

/// The id of the platform object.
pub type PlatformId = Id<Platform>;

/// The instance. It has one object only, with a fixed id. The instance admins pass each route
/// on it, from Postgres, and a role of the platform level gives a route on it.
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

/// The settings of the instance, which its admins change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceSettings {
    /// `true` when the users can create and use personal tokens.
    pub personal_tokens_enabled: bool,
}

crate::authz! {
    /// The routes on the settings of the instance.
    extend Platform as SettingsHolder {
        /// The route `GetInstanceSettings`.
        permission get_instance_settings = nil;
        /// The route `UpdateInstanceSettings`.
        permission update_instance_settings = nil;
    }
}
