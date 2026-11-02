//! The hooks that run in the transaction of a core write. One trait for each event.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::ResourceKind;

use crate::{CreateContext, DeleteContext, HookError, Tx};

/// Runs before the core inserts a resource. It can refuse. It never commits.
#[async_trait]
pub trait BeforeCreate: Send + Sync + 'static {
    /// Accepts the creation, or refuses it with `HookError::Rejected`.
    async fn before_create(&self, tx: &mut Tx<'_>, ctx: &CreateContext) -> Result<(), HookError>;
}

/// Runs after the core deletes a resource, before the commit. It never commits.
#[async_trait]
pub trait AfterDelete: Send + Sync + 'static {
    /// Follows the deletion. An error rolls it back.
    async fn after_delete(&self, tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError>;
}

/// The hooks of an edition, by event and by resource kind.
#[derive(Default)]
pub struct Hooks {
    /// The `BeforeCreate` hooks of each kind, in registration order.
    before_create: HashMap<ResourceKind, Vec<Arc<dyn BeforeCreate>>>,
    /// The `AfterDelete` hooks of each kind, in registration order.
    after_delete: HashMap<ResourceKind, Vec<Arc<dyn AfterDelete>>>,
}

impl Hooks {
    /// Runs the `BeforeCreate` hooks of `ctx.kind` in order. The first error stops the run.
    pub async fn before_create(
        &self,
        tx: &mut Tx<'_>,
        ctx: &CreateContext,
    ) -> Result<(), HookError> {
        for hook in self.before_create.get(&ctx.kind).into_iter().flatten() {
            hook.before_create(tx, ctx).await?;
        }
        Ok(())
    }

    /// Runs the `AfterDelete` hooks of `ctx.kind` in order. The first error stops the run.
    pub async fn after_delete(
        &self,
        tx: &mut Tx<'_>,
        ctx: &DeleteContext,
    ) -> Result<(), HookError> {
        for hook in self.after_delete.get(&ctx.kind).into_iter().flatten() {
            hook.after_delete(tx, ctx).await?;
        }
        Ok(())
    }

    /// Adds a `BeforeCreate` hook for one kind, after the existing ones.
    pub(crate) fn add_before_create(&mut self, kind: ResourceKind, hook: Arc<dyn BeforeCreate>) {
        self.before_create.entry(kind).or_default().push(hook);
    }

    /// Adds an `AfterDelete` hook for one kind, after the existing ones.
    pub(crate) fn add_after_delete(&mut self, kind: ResourceKind, hook: Arc<dyn AfterDelete>) {
        self.after_delete.entry(kind).or_default().push(hook);
    }
}
