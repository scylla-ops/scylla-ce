//! What the parts of the Git integration share: the pool, the gate, the hooks, the open streams
//! of the agents, the key of the secrets, the settings and the clients of the providers, the
//! public URLs, and the wake of the sender of the reports.

use std::sync::Arc;

use scylla_extension::{Gate, Hooks};
use sqlx::PgPool;
use tokio::sync::Notify;

use crate::features::identity::accounts::{AccountContext, AccountSettings};
use crate::features::identity::agents::Registry;
use crate::infra::secret::Sealer;
use crate::infra::vcs::{VcsClients, VcsOptions};
use crate::Error;

/// What the parts of the Git integration share.
#[derive(Clone)]
#[allow(dead_code, reason = "the parts of the integration read these fields")]
pub(crate) struct IntegrationContext {
    /// The database pool.
    pub(crate) pool: PgPool,
    /// The access gate.
    pub(crate) gate: Arc<dyn Gate>,
    /// The hooks of the edition.
    pub(crate) hooks: Arc<Hooks>,
    /// The open streams of the agents.
    pub(crate) registry: Arc<Registry>,
    /// The key of the secrets: the tokens and the webhook secrets of the providers.
    pub(crate) sealer: Option<Arc<Sealer>>,
    /// The checked settings of the Git providers.
    pub(crate) settings: Arc<VcsOptions>,
    /// The clients of the Git providers.
    pub(crate) clients: VcsClients,
    /// The public URLs of the API and of the web app: the webhook URLs, the links of the
    /// reports and the callbacks.
    pub(crate) urls: Arc<AccountSettings>,
    /// Wakes the sender of the reports after a commit.
    pub(crate) wake: Arc<Notify>,
}

impl IntegrationContext {
    /// Makes the context from what the account routes share, the open streams of the agents and
    /// the settings of the providers.
    pub(crate) fn new(
        accounts: &AccountContext,
        registry: Arc<Registry>,
        settings: VcsOptions,
    ) -> Result<Self, Error> {
        Ok(Self {
            pool: accounts.pool.clone(),
            gate: Arc::clone(&accounts.gate),
            hooks: Arc::clone(&accounts.hooks),
            registry,
            sealer: accounts.sealer.clone(),
            clients: VcsClients::new(&settings)?,
            settings: Arc::new(settings),
            urls: Arc::clone(&accounts.settings),
            wake: Arc::default(),
        })
    }
}
