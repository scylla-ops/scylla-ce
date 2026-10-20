use super::next::{Done, Next, Proceed};
use crate::action::Action;
use crate::domain::errors::{DomainError, DomainResult};
use crate::stage::{Stage, StageKind};
use async_trait::async_trait;

/// An erased veto before a stage, for a class of actions: quotas, plan limits, rate limits,
/// kill switches. Registered on a `StageKind`, so it also fires for commands that do not exist
/// yet. Must be idempotent and must not write.
#[async_trait]
pub trait Policy: Send + Sync {
    async fn enforce(&self, stage: StageKind, action: &dyn Action) -> DomainResult<()>;
}

/// A typed veto before one stage of one command, on the fields of the command or of the staged
/// value. It gets a shared reference: a gate decides, a `Wrap` changes.
#[async_trait]
pub trait Gate<S: Stage>: Send + Sync {
    async fn check(&self, input: &S::In) -> DomainResult<()>;
}

/// Erased control around a stage, for every command: timing, a tracing span, a failure metric.
/// It must return the `Done` it got from `next.run()`; it cannot build one, so it cannot skip the
/// stage. Runs outside every `Wrap`.
#[async_trait]
pub trait Around: Send + Sync {
    async fn around(&self, stage: StageKind, next: Proceed<'_>) -> DomainResult<Done>;
}

/// Typed control around one stage of one command: a cache, a dry run. It calls `next.run(input)`
/// exactly once, or not at all for a simulation, and then builds the output through the phase
/// API itself. It cannot build an `Authorized<C>`, so it cannot skip the access check.
#[async_trait]
pub trait Wrap<S: Stage>: Send + Sync {
    async fn wrap(&self, input: S::In, next: Next<'_, S>) -> DomainResult<S::Out>;
}

/// A typed side effect after one stage of one command succeeded: a notification, a cache
/// invalidation. It runs before `Actions::run` returns, outside any transaction, and cannot
/// fail the action; it logs and returns.
#[async_trait]
pub trait Listener<S: Stage>: Send + Sync {
    async fn listen(&self, output: &S::Out);
}

/// An erased record of every attempt, with its result: an audit journal, metrics, an outbox. On
/// success `action` is the output phase; on a veto it is the input phase; on a run error the input
/// was consumed and `action` is the envelope, so `downcast_ref` to a phase returns `None`.
#[async_trait]
pub trait Observer: Send + Sync {
    async fn observe(
        &self,
        stage: StageKind,
        action: &dyn Action,
        outcome: Result<(), &DomainError>,
    );
}
