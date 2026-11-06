//! An edition, the plugins that make it, and the builder that collects them.

use std::sync::Arc;

use scylla_domain::authz::{Model, Relation, Resource, Schema};
use scylla_domain::identity::actors::Actor;
use scylla_domain::ResourceKind;
use sqlx::migrate::Migrator;
use sqlx::PgPool;
use tonic::service::RoutesBuilder;

use crate::grants::{AccessRoutes, FindWorkspace, Grants};
use crate::{
    AfterDelete, BeforeCreate, Gate, Hooks, NotificationKind, NotificationKinds, Notifier,
    ServiceContext,
};

/// A function that adds the gRPC services of one plugin.
type AddServices = Box<dyn Fn(&mut RoutesBuilder, &ServiceContext<'_>) + Send + Sync>;

/// Returns one file of the built web app that a binary holds.
pub type WebFiles = fn(&str) -> Option<rust_embed::EmbeddedFile>;

/// What a binary runs: its name, capabilities, migrations, permission schema, hooks and services.
pub struct Edition {
    /// The files of the built web app that the binary holds.
    web_files: Option<WebFiles>,
    /// The name that `GetCapabilities` reports.
    name: &'static str,
    /// The optional features, in registration order.
    capabilities: Vec<&'static str>,
    /// The migrators, in the order to apply them.
    migrators: Vec<&'static Migrator>,
    /// The permission model: the definitions of the plugins, with their extensions.
    authz: Schema,
    /// The roles that `AccessService` can give.
    pub(crate) grants: Arc<Grants>,
    /// The hooks of each event and kind.
    hooks: Arc<Hooks>,
    /// The kinds of notifications.
    notification_kinds: Arc<NotificationKinds>,
    /// The functions that add the gRPC services.
    services: Vec<AddServices>,
    /// The encoded descriptor sets of the protos of the edition, for the JSON gateway.
    descriptors: Vec<&'static [u8]>,
}

impl Edition {
    /// Starts an empty edition. Binaries use `scylla_core::edition`, which adds the core.
    pub fn builder(name: &'static str) -> EditionBuilder {
        EditionBuilder {
            edition: Edition {
                web_files: None,
                name,
                capabilities: Vec::new(),
                migrators: Vec::new(),
                authz: Schema::default(),
                grants: Arc::default(),
                hooks: Arc::new(Hooks::default()),
                notification_kinds: Arc::default(),
                services: Vec::new(),
                descriptors: Vec::new(),
            },
            grants: Grants::default(),
            hooks: Hooks::default(),
            notification_kinds: NotificationKinds::default(),
        }
    }

    /// Returns the encoded descriptor sets of the protos that the edition adds to the core.
    pub fn descriptor_sets(&self) -> &[&'static [u8]] {
        &self.descriptors
    }

    /// Returns the files of the built web app that the binary holds.
    pub fn web_files(&self) -> Option<WebFiles> {
        self.web_files
    }

    /// Returns the name of the edition.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the optional features of the edition.
    pub fn capabilities(&self) -> &[&'static str] {
        &self.capabilities
    }

    /// Returns the migrators, in the order to apply them.
    pub fn migrators(&self) -> &[&'static Migrator] {
        &self.migrators
    }

    /// Returns the SpiceDB schema of the permission model.
    pub fn authz_schema(&self) -> String {
        self.authz.to_string()
    }

    /// Returns the roles that `AccessService` and the invitations can give.
    pub fn grants(&self) -> Arc<Grants> {
        Arc::clone(&self.grants)
    }

    /// Returns the permission model, for the gate: it tells which definitions have a permission.
    pub fn authz_model(&self) -> &Schema {
        &self.authz
    }

    /// Returns the hooks.
    pub fn hooks(&self) -> Arc<Hooks> {
        Arc::clone(&self.hooks)
    }

    /// Returns the kinds of notifications.
    pub fn notification_kinds(&self) -> Arc<NotificationKinds> {
        Arc::clone(&self.notification_kinds)
    }

    /// Adds the gRPC services of each plugin, with the pool, the gate and the notifier.
    pub fn add_services(
        &self,
        routes: &mut RoutesBuilder,
        pool: &PgPool,
        gate: &Arc<dyn Gate>,
        notifier: &Arc<dyn Notifier>,
    ) {
        let ctx = ServiceContext {
            pool,
            gate,
            notifier,
            edition: self,
        };
        for add in &self.services {
            add(routes, &ctx);
        }
    }
}

/// A feature that installs its parts into an edition.
pub trait Plugin {
    /// Registers the parts of the feature and gives the builder back.
    fn install(self, edition: EditionBuilder) -> EditionBuilder;
}

/// Collects the parts of an edition. `build` makes the `Edition`.
pub struct EditionBuilder {
    /// The edition under construction.
    edition: Edition,
    /// The roles that `AccessService` can give, mutable until `build`.
    grants: Grants,
    /// The hooks, mutable until `build`.
    hooks: Hooks,
    /// The kinds of notifications, mutable until `build`.
    notification_kinds: NotificationKinds,
}

impl EditionBuilder {
    /// Adds an optional feature that `GetCapabilities` reports.
    pub fn capability(mut self, name: &'static str) -> Self {
        self.edition.capabilities.push(name);
        self
    }

    /// Adds an encoded descriptor set of the protos of the edition, so the JSON gateway calls
    /// their methods.
    pub fn descriptors(mut self, set: &'static [u8]) -> Self {
        self.edition.descriptors.push(set);
        self
    }

    /// Adds a migrator. Each migrator has its own ledger table in its `sqlx.toml`.
    pub fn migrations(mut self, migrator: &'static Migrator) -> Self {
        self.edition.migrators.push(migrator);
        self
    }

    /// Adds a piece of the permission model, from `scylla_domain::authz!`: `.authz::<Project>()`.
    /// `migrate` writes the full schema.
    ///
    /// # Panics
    ///
    /// If the piece does not fit: a definition declared twice, an extension before its
    /// definition, or a name declared twice in a definition. Each start of the edition finds it.
    pub fn authz<M: Model>(mut self) -> Self {
        if let Err(error) = self.edition.authz.add(M::part()) {
            panic!("permission model: {error}");
        }
        self
    }

    /// Lets `AccessService` give, take back and list the roles on the objects of `R`. `routes`
    /// are the route permissions of `R`. `workspace` finds the workspace of an object in
    /// Postgres, the object itself for a workspace, and tells that an object does not exist.
    ///
    /// # Panics
    ///
    /// If `R` has its roles already. Each start of the edition finds it.
    pub fn grants<R: Resource>(
        mut self,
        routes: AccessRoutes<R>,
        workspace: FindWorkspace<R>,
    ) -> Self {
        if !self.grants.add_kind(routes, workspace) {
            panic!("grants: `{}` has its roles already", R::NAME);
        }
        self
    }

    /// Lets `AccessService` give a role: `.role(Project::VIEWER)`.
    ///
    /// # Panics
    ///
    /// If the kind of the role has no `grants` before, or has the role already.
    pub fn role<R: Resource>(mut self, role: Relation<R, Actor>) -> Self {
        if !self.grants.add_role(role) {
            panic!("grants: `{}#{}` cannot be added", R::NAME, role.name());
        }
        self
    }

    /// Adds a `BeforeCreate` hook for one resource kind.
    pub fn before_create(mut self, kind: ResourceKind, hook: Arc<dyn BeforeCreate>) -> Self {
        self.hooks.add_before_create(kind, hook);
        self
    }

    /// Adds an `AfterDelete` hook for one resource kind.
    pub fn after_delete(mut self, kind: ResourceKind, hook: Arc<dyn AfterDelete>) -> Self {
        self.hooks.add_after_delete(kind, hook);
        self
    }

    /// Adds a kind of notification.
    ///
    /// # Panics
    ///
    /// If the key is not `<subject>.<action>`, or has a kind already. Each start of the edition
    /// finds it.
    pub fn notification_kind(mut self, kind: NotificationKind) -> Self {
        let key = kind.key;
        if !self.notification_kinds.add(kind) {
            panic!("notifications: `{key}` cannot be added");
        }
        self
    }

    /// Adds a function that adds gRPC services at start. Service names must be unique.
    pub fn service<F>(mut self, add: F) -> Self
    where
        F: Fn(&mut RoutesBuilder, &ServiceContext<'_>) + Send + Sync + 'static,
    {
        self.edition.services.push(Box::new(add));
        self
    }

    /// Sets the files of the built web app that the binary holds.
    pub fn web_files(mut self, files: WebFiles) -> Self {
        self.edition.web_files = Some(files);
        self
    }

    /// Installs a plugin.
    pub fn plugin(self, plugin: impl Plugin) -> Self {
        plugin.install(self)
    }

    /// Makes the edition.
    pub fn build(mut self) -> Edition {
        self.edition.grants = Arc::new(self.grants);
        self.edition.hooks = Arc::new(self.hooks);
        self.edition.notification_kinds = Arc::new(self.notification_kinds);
        self.edition
    }
}
