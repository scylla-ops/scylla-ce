//! Where one action's chain forks. `Authorize` is the same for every action; what follows is
//! decided by the trait the action implements: a `Command` goes through prepare and persist, a
//! `Query` through fetch.

use crate::action::{Authorized, Command, Describe, Query};
use crate::domain::errors::DomainResult;
use crate::hooks::Hooks;
use crate::stage::{Fetch, Persist, Prepare, Run};
use async_trait::async_trait;

mod sealed {
    pub trait Sealed {}
}

/// Sealed: a third path is a change to the pipeline, not to a use case.
pub trait Kind: sealed::Sealed + Send + Sync + 'static {
    const NAME: &'static str;
}

pub struct Write;
pub struct Read;

impl sealed::Sealed for Write {}
impl sealed::Sealed for Read {}

impl Kind for Write {
    const NAME: &'static str = "command";
}

impl Kind for Read {
    const NAME: &'static str = "query";
}

/// The rest of the chain for an action run by `R`: the stages after `Authorize`, the bound the
/// runner must satisfy, and what comes out. The marker `K` is a trait parameter and not a
/// declaration on the action: with one impl per marker, the compiler infers it from the trait
/// the action implements, which is what two blanket impls of a single trait could not do.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no path through `{R}`",
    label = "no `Run` impl on the runner for this action",
    note = "a `Command` needs `Run<Prepare<{Self}>>` and `Run<Persist<{Self}>>` on `{R}`, a `Query` needs `Run<Fetch<{Self}>>`"
)]
#[async_trait]
pub trait Path<K: Kind, R>: Describe + Sized {
    type Output;

    async fn run(
        hooks: &Hooks,
        runner: &R,
        authorized: Authorized<Self>,
    ) -> DomainResult<Self::Output>;
}

#[async_trait]
impl<C, R> Path<Write, R> for C
where
    C: Command,
    R: Run<Prepare<C>> + Run<Persist<C>>,
{
    type Output = C::Committed;

    async fn run(
        hooks: &Hooks,
        runner: &R,
        authorized: Authorized<C>,
    ) -> DomainResult<C::Committed> {
        let prepared = hooks.run::<Prepare<C>>(runner, authorized).await?;
        let committed = hooks.run::<Persist<C>>(runner, prepared).await?;
        Ok(committed.into_outcome())
    }
}

#[async_trait]
impl<Q, R> Path<Read, R> for Q
where
    Q: Query,
    R: Run<Fetch<Q>>,
{
    type Output = Q::Output;

    async fn run(hooks: &Hooks, runner: &R, authorized: Authorized<Q>) -> DomainResult<Q::Output> {
        let fetched = hooks.run::<Fetch<Q>>(runner, authorized).await?;
        Ok(fetched.into_output())
    }
}
