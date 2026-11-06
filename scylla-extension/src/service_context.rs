//! What a plugin gets when it adds its gRPC services.

use std::sync::Arc;

use scylla_domain::authz::Schema;
use sqlx::PgPool;

use crate::grants::Grants;
use crate::{Edition, Gate, Hooks, NotificationKinds, Notifier};

/// What a plugin gets when it adds its gRPC services.
pub struct ServiceContext<'a> {
    /// The database pool.
    pub(crate) pool: &'a PgPool,
    /// The access gate of the server.
    pub(crate) gate: &'a Arc<dyn Gate>,
    /// What records a fact for the notifications.
    pub(crate) notifier: &'a Arc<dyn Notifier>,
    /// The edition that runs.
    pub(crate) edition: &'a Edition,
}

impl ServiceContext<'_> {
    /// Returns a handle to the database pool.
    pub fn pool(&self) -> PgPool {
        self.pool.clone()
    }

    /// Returns the access gate of the server.
    pub fn gate(&self) -> Arc<dyn Gate> {
        Arc::clone(self.gate)
    }

    /// Returns the hooks of the edition.
    pub fn hooks(&self) -> Arc<Hooks> {
        self.edition.hooks()
    }

    /// Returns what records a fact for the notifications.
    pub fn notifier(&self) -> Arc<dyn Notifier> {
        Arc::clone(self.notifier)
    }

    /// Returns the kinds of notifications.
    pub fn notification_kinds(&self) -> Arc<NotificationKinds> {
        self.edition.notification_kinds()
    }

    /// Returns the roles that `AccessService` can give.
    pub fn grants(&self) -> Arc<Grants> {
        Arc::clone(&self.edition.grants)
    }

    /// Returns the permission model of the edition: each definition and its permissions.
    pub fn authz_model(&self) -> &Schema {
        self.edition.authz_model()
    }

    /// Returns the name of the edition.
    pub fn edition_name(&self) -> &'static str {
        self.edition.name()
    }

    /// Returns the optional features of the edition.
    pub fn capabilities(&self) -> &[&'static str] {
        self.edition.capabilities()
    }
}
