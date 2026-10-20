//! The one engine. A use case implements `Run<Prepare<C>>` and `Run<Persist<C>>` for its
//! commands, `Run<Fetch<Q>>` for its queries, and an adapter calls `run` with it for both. The
//! use case has no hook code, no access code and no method of its own.

use crate::action::Requested;
use crate::authz::{Access, AuthorizeStage, Authorizer};
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::hooks::Hooks;
use crate::path::{Kind, Path};
use crate::stage::Authorize;
use std::sync::Arc;
use tracing::{Instrument, field, info_span};

pub struct Actions {
    authorize: AuthorizeStage,
    hooks: Arc<Hooks>,
}

impl Actions {
    #[must_use]
    pub fn new(authorizer: Arc<dyn Authorizer>, hooks: Arc<Hooks>) -> Self {
        Self {
            authorize: AuthorizeStage::new(authorizer),
            hooks,
        }
    }

    /// Authorize, then the action's path, each stage through the hooks. The runner is one object
    /// for every stage of the action; the turbofish on `Hooks::run` selects which impl a stage
    /// uses. Every action runs in one span; use cases carry no `#[instrument]` of their own.
    pub async fn run<A, K, R>(
        &self,
        runner: &R,
        caller: &CallerContext,
        action: A,
    ) -> DomainResult<A::Output>
    where
        A: Path<K, R>,
        K: Kind,
    {
        let requested = Requested::new(caller.clone(), action);
        let span = info_span!(
            "action",
            kind = K::NAME,
            action = %requested.id(),
            caller = %requested.caller(),
            access = %requested.access(),
            resource = resources(requested.access()).map(field::display),
        );
        async {
            let authorized = self
                .hooks
                .run::<Authorize<A>>(&self.authorize, requested)
                .await?;
            A::run(&self.hooks, runner, authorized).await
        }
        .instrument(span)
        .await
    }
}

fn resources(access: &Access) -> Option<String> {
    let mut resources: Vec<_> = access
        .permissions()
        .iter()
        .map(|p| p.resource().to_string())
        .collect();
    resources.dedup();
    (!resources.is_empty()).then(|| resources.join("+"))
}
