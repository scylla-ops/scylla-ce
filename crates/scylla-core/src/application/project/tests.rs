//! The project's actions through the engine, on stub ports.

use super::*;
use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::application::project::{CreateProject, DeleteProject, GetProject, UpdateProject};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::project::{Project, ProjectName};
use crate::test_support::authz::{
    DenyingPermissionService, RecordingPermissionService, actions_with,
};
use crate::test_support::jobs::JobBuilder;
use crate::test_support::pipelines::PipelineBuilder;
use crate::test_support::stubs::{
    NoUsers, StubJobs, StubRegistry, StubTails, alice, dispatcher, empty_page,
};
use async_trait::async_trait;
use scylla_auth::authz::{Grant, PermissionService, Visibility};
use scylla_extension::{Action, Actions, Hooks, Policy, StageKind};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct StubProjects {
    rows: Mutex<HashMap<ProjectId, Project>>,
    grants: Mutex<Vec<Grant>>,
    listed_for_user: Mutex<Vec<String>>,
    listed_by_organization: Mutex<Vec<Visibility>>,
}

#[async_trait]
impl ProjectRepository for StubProjects {
    async fn create(&self, project: &Project) -> DomainResult<Project> {
        self.rows
            .lock()
            .unwrap()
            .insert(project.id().clone(), project.clone());
        Ok(project.clone())
    }
    async fn provision_with_owner(&self, project: &Project, grant: &Grant) -> DomainResult<()> {
        self.create(project).await?;
        self.grants.lock().unwrap().push(grant.clone());
        Ok(())
    }
    async fn list_principals(
        &self,
        _: &ProjectId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        empty_page()
    }
    async fn list_for_user(
        &self,
        _: &UserId,
        permission: &str,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.listed_for_user
            .lock()
            .unwrap()
            .push(permission.to_string());
        empty_page()
    }
    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Project", id))
    }
    async fn update(&self, project: &Project) -> DomainResult<Project> {
        self.create(project).await
    }
    async fn delete(&self, project: &Project) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(project.id());
        Ok(())
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        empty_page()
    }
    async fn list_by_organization(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
        visible: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.listed_by_organization
            .lock()
            .unwrap()
            .push(visible.clone());
        empty_page()
    }
}

/// Answers each permission key from its map, `All` for a key it does not hold.
#[derive(Default)]
struct StubVisibility(HashMap<&'static str, Visibility>);

#[async_trait]
impl VisibilityResolver for StubVisibility {
    async fn visible_scopes(&self, _: &CallerContext, key: &str) -> DomainResult<Visibility> {
        Ok(self.0.get(key).cloned().unwrap_or(Visibility::All))
    }
}

struct Veto;

#[async_trait]
impl Policy for Veto {
    async fn enforce(&self, _: StageKind, action: &dyn Action) -> DomainResult<()> {
        Err(DomainError::quota_exceeded(format!(
            "vetoed {}",
            action.access()
        )))
    }
}

struct Lab {
    actions: Actions,
    uc: ProjectUseCases,
    projects: Arc<StubProjects>,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
}

impl Lab {
    async fn create(&self, name: &str) -> DomainResult<Project> {
        self.actions.run(&self.uc, &alice(), create(name)).await
    }
}

fn lab(permissions: Arc<dyn PermissionService>, hooks: Hooks) -> Lab {
    lab_seeing(permissions, hooks, StubVisibility::default())
}

fn lab_seeing(
    permissions: Arc<dyn PermissionService>,
    hooks: Hooks,
    visibility: StubVisibility,
) -> Lab {
    let projects = Arc::new(StubProjects::default());
    let jobs = Arc::new(StubJobs::default());
    let registry = Arc::new(StubRegistry::default());
    Lab {
        actions: actions_with(permissions, hooks),
        uc: ProjectUseCases::new(
            projects.clone(),
            Arc::new(NoUsers),
            Arc::new(visibility),
            dispatcher(
                registry.clone(),
                jobs.clone(),
                Arc::new(StubTails::default()),
            ),
        ),
        projects,
        jobs,
        registry,
    }
}

fn create(name: &str) -> CreateProject {
    CreateProject {
        organization_id: OrganizationId::new("acme"),
        name: ProjectName::new(name).unwrap(),
        description: None,
    }
}

#[tokio::test]
async fn a_create_by_a_user_checks_the_permission_then_writes_the_owner_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), Hooks::new());

    let project = lab.create("rocket").await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateProject(OrganizationId::new("acme"))]
    );
    assert!(lab.projects.rows.lock().unwrap().contains_key(project.id()));
    assert_eq!(lab.projects.grants.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new()), Hooks::new());

    let err = lab.create("rocket").await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.projects.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_policy_in_the_hooks_vetoes_after_the_permission_check() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let mut hooks = Hooks::new();
    hooks.policy(StageKind::Prepare, Arc::new(Veto));
    let lab = lab(permissions.clone(), hooks);

    let err = lab.create("rocket").await.unwrap_err();

    assert!(matches!(&err, DomainError::QuotaExceeded(m) if m == "vetoed createProject"));
    assert_eq!(permissions.permissions().len(), 1);
    assert!(lab.projects.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_update_stages_the_change_and_persists_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), Hooks::new());
    let created = lab.create("old").await.unwrap();

    let updated = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            UpdateProject {
                id: created.id().clone(),
                name: Some(ProjectName::new("new").unwrap()),
                description: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.name().as_str(), "new");
    assert_eq!(
        lab.projects.rows.lock().unwrap()[created.id()]
            .name()
            .as_str(),
        "new"
    );
    assert_eq!(
        permissions.permissions()[1],
        Permission::UpdateProject(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_returns_the_tombstone() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), Hooks::new());
    let created = lab.create("gone").await.unwrap();

    let deleted = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteProject {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(deleted.last_state().id(), created.id());
    assert!(lab.projects.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_delete_stops_the_live_jobs_of_the_project_on_their_agents() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), Hooks::new());
    let created = lab.create("gone").await.unwrap();
    let agent = AppId::new("agent-1");
    lab.registry.connect(&agent);
    let pipeline = PipelineBuilder::for_project_id(created.id().clone()).build();
    let placed = lab
        .jobs
        .insert(&JobBuilder::new(&pipeline).agent(agent.clone()).build());

    lab.actions
        .run(
            &lab.uc,
            &alice(),
            DeleteProject {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap();

    assert_eq!(lab.registry.cancelled(), [(agent, placed.id().clone())]);
}

#[tokio::test]
async fn a_read_checks_its_permission_and_takes_the_same_hooks() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let mut hooks = Hooks::new();
    hooks.policy(StageKind::Fetch, Arc::new(Veto));
    let lab = lab(permissions.clone(), hooks);
    let created = lab.create("seen").await.unwrap();

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            GetProject {
                id: created.id().clone(),
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(&err, DomainError::QuotaExceeded(m) if m == "vetoed readProject"));
    assert_eq!(
        permissions.permissions()[1],
        Permission::ReadProject(created.id().clone())
    );
}

#[tokio::test]
async fn the_projects_of_a_user_are_the_ones_its_grants_let_it_read() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), Hooks::new());

    lab.actions
        .run(
            &lab.uc,
            &alice(),
            ListUserProjects {
                user_id: UserId::new("alice"),
                pagination: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        *lab.projects.listed_for_user.lock().unwrap(),
        vec!["readProject".to_string()]
    );
}

fn list_acme() -> ListOrganizationProjects {
    ListOrganizationProjects {
        organization_id: OrganizationId::new("acme"),
        pagination: None,
    }
}

#[tokio::test]
async fn an_organization_listing_asks_the_access_model_once_and_reads_the_rest_from_the_grants() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let own = Visibility::Scoped {
        orgs: Vec::new(),
        projects: vec![ProjectId::new("p1")],
    };
    let lab = lab_seeing(
        permissions.clone(),
        Hooks::new(),
        StubVisibility(HashMap::from([
            ("listProjectsByOrganization", Visibility::none()),
            ("readProject", own.clone()),
        ])),
    );

    lab.actions
        .run(&lab.uc, &alice(), list_acme())
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadOrganization(OrganizationId::new("acme"))]
    );
    assert_eq!(
        *lab.projects.listed_by_organization.lock().unwrap(),
        vec![own]
    );
}

#[tokio::test]
async fn listing_the_projects_of_an_organization_shows_them_all() {
    let wide = Visibility::Scoped {
        orgs: vec![OrganizationId::new("acme")],
        projects: Vec::new(),
    };
    let lab = lab_seeing(
        Arc::new(RecordingPermissionService::new()),
        Hooks::new(),
        StubVisibility(HashMap::from([
            ("listProjectsByOrganization", wide.clone()),
            ("readProject", Visibility::none()),
        ])),
    );

    lab.actions
        .run(&lab.uc, &alice(), list_acme())
        .await
        .unwrap();

    assert_eq!(
        *lab.projects.listed_by_organization.lock().unwrap(),
        vec![wide]
    );
}
