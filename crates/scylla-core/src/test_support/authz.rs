use crate::application::PermissionAuthorizer;
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_extension::{Actions, Hooks};
use std::sync::{Arc, Mutex};

#[must_use]
pub fn actions(permissions: Arc<dyn PermissionService>) -> Actions {
    actions_with(permissions, Hooks::new())
}

#[must_use]
pub fn actions_with(permissions: Arc<dyn PermissionService>, hooks: Hooks) -> Actions {
    Actions::new(
        Arc::new(PermissionAuthorizer::new(permissions)),
        Arc::new(hooks),
    )
}

#[derive(Default)]
pub struct RecordingPermissionService {
    checks: Mutex<Vec<(CallerContext, Permission)>>,
}

impl RecordingPermissionService {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn checks(&self) -> Vec<(CallerContext, Permission)> {
        self.checks.lock().unwrap().clone()
    }

    #[must_use]
    pub fn permissions(&self) -> Vec<Permission> {
        self.checks
            .lock()
            .unwrap()
            .iter()
            .map(|(_, p)| p.clone())
            .collect()
    }
}

#[async_trait]
impl PermissionService for RecordingPermissionService {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.checks
            .lock()
            .unwrap()
            .push((caller.clone(), permission));
        Ok(())
    }
}

#[derive(Default)]
pub struct DenyingPermissionService;

impl DenyingPermissionService {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl PermissionService for DenyingPermissionService {
    async fn check(&self, _caller: &CallerContext, _permission: Permission) -> DomainResult<()> {
        Err(DomainError::forbidden("denied by DenyingPermissionService"))
    }
}
