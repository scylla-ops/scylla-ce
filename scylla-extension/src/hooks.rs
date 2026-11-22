//! The hooks that run in the transaction of a core write. One trait for each event.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::ResourceKind;

use crate::{CreateContext, DeleteContext, ExportPart, HookError, Tx, UserContext};

/// Runs before the core inserts a resource. It can refuse. It never commits.
#[async_trait]
pub trait BeforeCreate: Send + Sync + 'static {
    /// Accepts the creation, or refuses it with `HookError::Rejected`.
    async fn before_create(&self, tx: &mut Tx<'_>, ctx: &CreateContext) -> Result<(), HookError>;
}

/// Runs before the core deletes a resource. It can refuse. It never commits.
#[async_trait]
pub trait BeforeDelete: Send + Sync + 'static {
    /// Accepts the deletion, or refuses it with `HookError::Rejected`.
    async fn before_delete(&self, tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError>;
}

/// Runs after the core deletes a resource, before the commit. It never commits.
#[async_trait]
pub trait AfterDelete: Send + Sync + 'static {
    /// Follows the deletion. An error rolls it back.
    async fn after_delete(&self, tx: &mut Tx<'_>, ctx: &DeleteContext) -> Result<(), HookError>;
}

/// Runs after the core deletes a user, before the commit. It never commits.
#[async_trait]
pub trait AfterDeleteUser: Send + Sync + 'static {
    /// Removes or changes the rows of the user in the tables of the edition. An error rolls the
    /// deletion back.
    async fn after_delete_user(&self, tx: &mut Tx<'_>, ctx: &UserContext) -> Result<(), HookError>;
}

/// Adds the data of an edition to the export of the data of a user. It only reads: the
/// transaction is read only.
#[async_trait]
pub trait ExportUser: Send + Sync + 'static {
    /// Returns the data of the user in the tables of the edition. An error stops the export.
    async fn export_user(
        &self,
        tx: &mut Tx<'_>,
        ctx: &UserContext,
    ) -> Result<ExportPart, HookError>;
}

/// The hooks of an edition, by event and by resource kind. The hooks of a user have no kind.
#[derive(Default)]
pub struct Hooks {
    /// The `BeforeCreate` hooks of each kind, in registration order.
    before_create: HashMap<ResourceKind, Vec<Arc<dyn BeforeCreate>>>,
    /// The `BeforeDelete` hooks of each kind, in registration order.
    before_delete: HashMap<ResourceKind, Vec<Arc<dyn BeforeDelete>>>,
    /// The `AfterDelete` hooks of each kind, in registration order.
    after_delete: HashMap<ResourceKind, Vec<Arc<dyn AfterDelete>>>,
    /// The `AfterDeleteUser` hooks, in registration order.
    after_delete_user: Vec<Arc<dyn AfterDeleteUser>>,
    /// The `ExportUser` hooks, in registration order.
    export_user: Vec<Arc<dyn ExportUser>>,
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

    /// Runs the `BeforeDelete` hooks of `ctx.kind` in order. The first error stops the run.
    pub async fn before_delete(
        &self,
        tx: &mut Tx<'_>,
        ctx: &DeleteContext,
    ) -> Result<(), HookError> {
        for hook in self.before_delete.get(&ctx.kind).into_iter().flatten() {
            hook.before_delete(tx, ctx).await?;
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

    /// Runs the `AfterDeleteUser` hooks in order. The first error stops the run.
    pub async fn after_delete_user(
        &self,
        tx: &mut Tx<'_>,
        ctx: &UserContext,
    ) -> Result<(), HookError> {
        for hook in &self.after_delete_user {
            hook.after_delete_user(tx, ctx).await?;
        }
        Ok(())
    }

    /// Runs the `ExportUser` hooks in order, and returns their parts. The first error stops the
    /// run.
    pub async fn export_user(
        &self,
        tx: &mut Tx<'_>,
        ctx: &UserContext,
    ) -> Result<Vec<ExportPart>, HookError> {
        let mut parts = Vec::with_capacity(self.export_user.len());
        for hook in &self.export_user {
            parts.push(hook.export_user(tx, ctx).await?);
        }
        Ok(parts)
    }

    /// Adds a `BeforeCreate` hook for one kind, after the existing ones.
    pub(crate) fn add_before_create(&mut self, kind: ResourceKind, hook: Arc<dyn BeforeCreate>) {
        self.before_create.entry(kind).or_default().push(hook);
    }

    /// Adds a `BeforeDelete` hook for one kind, after the existing ones.
    pub(crate) fn add_before_delete(&mut self, kind: ResourceKind, hook: Arc<dyn BeforeDelete>) {
        self.before_delete.entry(kind).or_default().push(hook);
    }

    /// Adds an `AfterDelete` hook for one kind, after the existing ones.
    pub(crate) fn add_after_delete(&mut self, kind: ResourceKind, hook: Arc<dyn AfterDelete>) {
        self.after_delete.entry(kind).or_default().push(hook);
    }

    /// Adds an `AfterDeleteUser` hook, after the existing ones.
    pub(crate) fn add_after_delete_user(&mut self, hook: Arc<dyn AfterDeleteUser>) {
        self.after_delete_user.push(hook);
    }

    /// Adds an `ExportUser` hook, after the existing ones.
    pub(crate) fn add_export_user(&mut self, hook: Arc<dyn ExportUser>) {
        self.export_user.push(hook);
    }
}
