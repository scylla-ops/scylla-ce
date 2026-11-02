//! An edition, the plugins that make it, and the builder that collects them.

use std::sync::Arc;

use scylla_domain::authz::{Model, Schema};
use scylla_domain::ResourceKind;
use sqlx::migrate::Migrator;
use sqlx::PgPool;
use tonic::service::RoutesBuilder;

use crate::{AfterDelete, BeforeCreate, Gate, Hooks};

/// A function that adds the gRPC services of one plugin.
type AddServices = Box<dyn Fn(&mut RoutesBuilder, &ServiceContext<'_>) + Send + Sync>;

/// What a binary runs: its name, capabilities, migrations, permission schema, hooks and services.
pub struct Edition {
    /// The name that `GetCapabilities` reports.
    name: &'static str,
    /// The optional features, in registration order.
    capabilities: Vec<&'static str>,
    /// The migrators, in the order to apply them.
    migrators: Vec<&'static Migrator>,
    /// The permission model: the definitions of the plugins, with their extensions.
    authz: Schema,
    /// The hooks of each event and kind.
    hooks: Arc<Hooks>,
    /// The functions that add the gRPC services.
    services: Vec<AddServices>,
}

impl Edition {
    /// Starts an empty edition. Binaries use `scylla_core::edition`, which adds the core.
    pub fn builder(name: &'static str) -> EditionBuilder {
        EditionBuilder {
            edition: Edition {
                name,
                capabilities: Vec::new(),
                migrators: Vec::new(),
                authz: Schema::default(),
                hooks: Arc::new(Hooks::default()),
                services: Vec::new(),
            },
            hooks: Hooks::default(),
        }
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

    /// Returns the permission model, for the gate: it tells which definitions have a permission.
    pub fn authz_model(&self) -> &Schema {
        &self.authz
    }

    /// Returns the hooks.
    pub fn hooks(&self) -> Arc<Hooks> {
        Arc::clone(&self.hooks)
    }

    /// Adds the gRPC services of each plugin, with the pool and the gate.
    pub fn add_services(&self, routes: &mut RoutesBuilder, pool: &PgPool, gate: &Arc<dyn Gate>) {
        let ctx = ServiceContext {
            pool,
            gate,
            edition: self,
        };
        for add in &self.services {
            add(routes, &ctx);
        }
    }
}

/// What a plugin gets when it adds its gRPC services.
pub struct ServiceContext<'a> {
    /// The database pool.
    pool: &'a PgPool,
    /// The access gate of the server.
    gate: &'a Arc<dyn Gate>,
    /// The edition that runs.
    edition: &'a Edition,
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

    /// Returns the name of the edition.
    pub fn edition_name(&self) -> &'static str {
        self.edition.name()
    }

    /// Returns the optional features of the edition.
    pub fn capabilities(&self) -> &[&'static str] {
        self.edition.capabilities()
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
    /// The hooks, mutable until `build`.
    hooks: Hooks,
}

impl EditionBuilder {
    /// Adds an optional feature that `GetCapabilities` reports.
    pub fn capability(mut self, name: &'static str) -> Self {
        self.edition.capabilities.push(name);
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

    /// Adds a function that adds gRPC services at start. Service names must be unique.
    pub fn service<F>(mut self, add: F) -> Self
    where
        F: Fn(&mut RoutesBuilder, &ServiceContext<'_>) + Send + Sync + 'static,
    {
        self.edition.services.push(Box::new(add));
        self
    }

    /// Installs a plugin.
    pub fn plugin(self, plugin: impl Plugin) -> Self {
        plugin.install(self)
    }

    /// Makes the edition.
    pub fn build(mut self) -> Edition {
        self.edition.hooks = Arc::new(self.hooks);
        self.edition
    }
}
