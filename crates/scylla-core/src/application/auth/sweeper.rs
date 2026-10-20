use super::{AuthUseCases, PurgeExpiredSessions};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use scylla_extension::Actions;
use std::sync::Arc;
use tracing::{info, instrument, warn};

/// The driver of `PurgeExpiredSessions`. A request with an expired token is refused before the
/// pass, so the pass only frees storage.
pub struct SessionSweeper {
    actions: Arc<Actions>,
    auth: Arc<AuthUseCases>,
}

impl SessionSweeper {
    #[must_use]
    pub fn new(actions: Arc<Actions>, auth: Arc<AuthUseCases>) -> Self {
        Self { actions, auth }
    }

    #[instrument(skip_all)]
    pub async fn sweep(&self) -> u64 {
        let caller = CallerContext::Service(ServiceIdentity::session_sweeper());
        match self
            .actions
            .run(&*self.auth, &caller, PurgeExpiredSessions)
            .await
        {
            Ok(0) => 0,
            Ok(n) => {
                info!(purged = n, "purged expired sessions");
                n
            }
            Err(e) => {
                warn!(error = %e, "session sweeper: purge pass failed");
                0
            }
        }
    }
}
