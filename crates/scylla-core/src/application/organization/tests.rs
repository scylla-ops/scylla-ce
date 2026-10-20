//! The organization's actions through the engine, on stub ports.

use super::*;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::application::{AppRepository, PrincipalAccess};
use crate::domain::agent::Agent;
use crate::domain::app::{App, AppCredential, AppName};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use crate::domain::organization::{Organization, OrganizationName};
use crate::domain::permission::Permission;
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::jobs::JobBuilder;
use crate::test_support::pipelines::PipelineBuilder;
use crate::test_support::stubs::{
    NoUsers, StubJobs, StubRegistry, StubTails, alice, dispatcher, empty_page,
};
use async_trait::async_trait;
use scylla_auth::authz::{Grant, PermissionService};
use scylla_extension::Actions;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct StubOrganizations {
    rows: Mutex<HashMap<OrganizationId, Organization>>,
    grants: Mutex<Vec<Grant>>,
}

#[async_trait]
impl OrganizationRepository for StubOrganizations {
    async fn create(&self, organization: &Organization) -> DomainResult<Organization> {
        self.rows
            .lock()
            .unwrap()
            .insert(organization.id().clone(), organization.clone());
        Ok(organization.clone())
    }
    async fn provision_with_owner(
        &self,
        organization: &Organization,
        grant: &Grant,
    ) -> DomainResult<()> {
        self.create(organization).await?;
        self.grants.lock().unwrap().push(grant.clone());
        Ok(())
    }
    async fn list_principals(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        empty_page()
    }
    async fn count_members(&self, _: &OrganizationId) -> DomainResult<u64> {
        Ok(0)
    }
    async fn has_member(&self, _: &OrganizationId, _: &UserId) -> DomainResult<bool> {
        Ok(false)
    }
    async fn list_for_user(
        &self,
        _: &UserId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        empty_page()
    }
    async fn find_by_id(&self, id: &OrganizationId) -> DomainResult<Organization> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Organization", id))
    }
    async fn update(&self, organization: &Organization) -> DomainResult<Organization> {
        self.create(organization).await
    }
    async fn delete(&self, organization: &Organization) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(organization.id());
        Ok(())
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        empty_page()
    }
}

/// The Apps of each organization; nothing else is read.
#[derive(Default)]
struct StubApps(Mutex<Vec<App>>);

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn find_by_id(&self, _: &AppId) -> DomainResult<App> {
        unreachable!("an organization action lists its apps")
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("an organization action lists its apps")
    }
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.organization_id() == organization_id)
            .cloned()
            .collect())
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn delete(&self, _: &AppId) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        unreachable!("an organization action reads no app access")
    }
}

struct Lab {
    actions: Actions,
    uc: OrganizationUseCases,
    organizations: Arc<StubOrganizations>,
    apps: Arc<StubApps>,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
}

impl Lab {
    async fn create(&self, name: &str) -> DomainResult<Organization> {
        self.actions.run(&self.uc, &alice(), create(name)).await
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let organizations = Arc::new(StubOrganizations::default());
    let apps = Arc::new(StubApps::default());
    let jobs = Arc::new(StubJobs::default());
    let registry = Arc::new(StubRegistry::default());
    Lab {
        actions: actions(permissions),
        uc: OrganizationUseCases::new(
            organizations.clone(),
            Arc::new(NoUsers),
            apps.clone(),
            dispatcher(
                registry.clone(),
                jobs.clone(),
                Arc::new(StubTails::default()),
            ),
        ),
        organizations,
        apps,
        jobs,
        registry,
    }
}

fn create(name: &str) -> CreateOrganization {
    CreateOrganization {
        name: OrganizationName::new(name).unwrap(),
        description: None,
    }
}

#[tokio::test]
async fn a_create_by_a_user_checks_the_permission_then_writes_the_owner_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());

    let organization = lab.create("acme").await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateOrganization]
    );
    assert!(
        lab.organizations
            .rows
            .lock()
            .unwrap()
            .contains_key(organization.id())
    );
    assert_eq!(lab.organizations.grants.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab.create("acme").await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.organizations.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_name_is_a_label_so_two_organizations_can_share_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let first = lab.create("acme").await.unwrap();

    let second = lab.create("acme").await.unwrap();

    assert_ne!(first.id(), second.id());
    assert_eq!(lab.organizations.rows.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_update_stages_the_change_and_persists_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create("old").await.unwrap();

    let updated = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            UpdateOrganization {
                id: created.id().clone(),
                name: Some(OrganizationName::new("new").unwrap()),
                description: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.name().as_str(), "new");
    assert_eq!(
        lab.organizations.rows.lock().unwrap()[created.id()]
            .name()
            .as_str(),
        "new"
    );
    assert_eq!(
        permissions.permissions()[1],
        Permission::UpdateOrganization(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_returns_the_tombstone() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create("gone").await.unwrap();

    let deleted = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteOrganization {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(deleted.last_state().id(), created.id());
    assert!(lab.organizations.rows.lock().unwrap().is_empty());
    assert_eq!(
        permissions.permissions()[1],
        Permission::DeleteOrganization(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_stops_the_live_jobs_of_the_organization_on_their_agents() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create("gone").await.unwrap();
    let agent = AppId::new("agent-1");
    lab.registry.connect(&agent);
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
    let running = lab.jobs.insert(
        &JobBuilder::new(&pipeline)
            .running(true)
            .agent(agent.clone())
            .build(),
    );

    lab.actions
        .run(
            &lab.uc,
            &alice(),
            DeleteOrganization {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(lab.registry.cancelled(), [(agent, running.id().clone())]);
}

#[tokio::test]
async fn a_delete_closes_the_agent_streams_of_the_organization_apps() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create("gone").await.unwrap();
    let other = lab.create("kept").await.unwrap();
    let app = |organization: &Organization, name: &str| {
        let app = App::create(organization.id().clone(), AppName::new(name).unwrap()).unwrap();
        lab.apps.0.lock().unwrap().push(app.clone());
        lab.registry.connect(app.id());
        app
    };
    let ours = app(&created, "runner");
    app(&other, "runner");

    lab.actions
        .run(
            &lab.uc,
            &alice(),
            DeleteOrganization {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(lab.registry.disconnected(), [ours.id().clone()]);
}

#[tokio::test]
async fn a_read_checks_its_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create("seen").await.unwrap();

    let read = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            GetOrganization {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(read.id(), created.id());
    assert_eq!(
        permissions.permissions()[1],
        Permission::ReadOrganization(created.id().clone())
    );
}
