mod credential;
mod kind;
mod name;
mod secret;
mod secret_hash;
mod secret_label;
mod token;

pub use credential::*;
pub use kind::*;
pub use name::*;
pub use secret::*;
pub use secret_hash::*;
pub use secret_label::*;
pub use token::*;

use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId};
use chrono::{DateTime, Utc};

pub const TRIGGER_RUNNER_APP_NAME: &str = "trigger-runner";

#[derive(Debug, Clone)]
pub struct App {
    id: AppId,
    organization_id: OrganizationId,
    name: AppName,
    kind: AppKind,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl App {
    #[must_use]
    pub fn from_persistence(
        id: AppId,
        organization_id: OrganizationId,
        name: AppName,
        kind: AppKind,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            organization_id,
            name,
            kind,
            is_active,
            created_at,
            updated_at,
        }
    }

    pub fn create(organization_id: OrganizationId, name: AppName) -> DomainResult<Self> {
        if name.as_str() == TRIGGER_RUNNER_APP_NAME {
            return Err(DomainError::validation(format!(
                "App name '{TRIGGER_RUNNER_APP_NAME}' is reserved"
            )));
        }
        Ok(Self::new(organization_id, name, AppKind::Standard))
    }

    pub fn trigger_runner(organization_id: OrganizationId) -> DomainResult<Self> {
        let name = AppName::new(TRIGGER_RUNNER_APP_NAME)?;
        Ok(Self::new(organization_id, name, AppKind::TriggerRunner))
    }

    fn new(organization_id: OrganizationId, name: AppName, kind: AppKind) -> Self {
        let now = clock::now();
        Self {
            id: AppId::generate(),
            organization_id,
            name,
            kind,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn ensure_user_managed(&self) -> DomainResult<()> {
        if self.is_trigger_runner() {
            return Err(DomainError::business_rule(
                "The trigger-runner App is managed by the server",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &AppId {
        &self.id
    }

    #[must_use]
    pub fn organization_id(&self) -> &OrganizationId {
        &self.organization_id
    }

    #[must_use]
    pub fn name(&self) -> &AppName {
        &self.name
    }

    #[must_use]
    pub fn kind(&self) -> AppKind {
        self.kind
    }

    #[must_use]
    pub fn is_trigger_runner(&self) -> bool {
        self.kind == AppKind::TriggerRunner
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn org() -> OrganizationId {
        OrganizationId::new("acme")
    }

    #[test]
    fn a_user_app_cannot_take_the_runner_name() {
        let reserved = AppName::new(TRIGGER_RUNNER_APP_NAME).unwrap();
        assert!(matches!(
            App::create(org(), reserved),
            Err(DomainError::Validation(_))
        ));
    }

    #[test]
    fn only_the_runner_is_managed_by_the_server() {
        let runner = App::trigger_runner(org()).unwrap();
        let app = App::create(org(), AppName::new("ci").unwrap()).unwrap();

        assert_eq!(runner.kind(), AppKind::TriggerRunner);
        assert_eq!(runner.name().as_str(), TRIGGER_RUNNER_APP_NAME);
        assert!(matches!(
            runner.ensure_user_managed(),
            Err(DomainError::BusinessRule(_))
        ));
        assert_eq!(app.kind(), AppKind::Standard);
        assert!(app.ensure_user_managed().is_ok());
    }
}
