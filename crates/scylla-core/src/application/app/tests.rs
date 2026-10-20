//! The app's actions through the engine, on stub ports.

use super::*;
use crate::application::PrincipalAccess;
use crate::domain::agent::Agent;
use crate::domain::app::{App, AppCredential, AppName, AppSecretLabel, TRIGGER_RUNNER_APP_NAME};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppCredentialId, AppId, OrganizationId};
use crate::domain::organization::OrganizationName;
use crate::domain::permission::Permission;
use crate::domain::role::{RoleDisplayName, RoleName};
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::stubs::{StubHash, StubRegistry, alice};
use async_trait::async_trait;
use scylla_auth::authz::{Grant, ORGANIZATION_AGENT_ROLE, PermissionService, Scope};
use scylla_extension::{Actions, Deleted};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct StubApps {
    rows: Mutex<HashMap<AppId, App>>,
    credentials: Mutex<Vec<AppCredential>>,
    access: Mutex<Vec<(AppId, PrincipalAccess)>>,
}

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, app: &App, credential: &AppCredential) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(app.id().clone(), app.clone());
        self.credentials.lock().unwrap().push(credential.clone());
        Ok(())
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("no agent in an app action")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("no grant in an app action")
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("App", id))
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("only a trigger action looks up the runner")
    }
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|a| a.organization_id() == organization_id)
            .cloned()
            .collect())
    }
    async fn set_active(&self, id: &AppId, active: bool) -> DomainResult<()> {
        let mut rows = self.rows.lock().unwrap();
        if let Some(a) = rows.get(id).cloned() {
            rows.insert(
                id.clone(),
                App::from_persistence(
                    a.id().clone(),
                    a.organization_id().clone(),
                    a.name().clone(),
                    a.kind(),
                    active,
                    a.created_at(),
                    a.updated_at(),
                ),
            );
        }
        Ok(())
    }
    async fn delete(&self, id: &AppId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        Ok(())
    }
    async fn list_access(&self, id: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        Ok(self
            .access
            .lock()
            .unwrap()
            .iter()
            .filter(|(app, _)| app == id)
            .map(|(_, access)| access.clone())
            .collect())
    }
}

#[derive(Default)]
struct StubCredentials {
    rows: Mutex<HashMap<AppCredentialId, AppCredential>>,
}

#[async_trait]
impl AppCredentialRepository for StubCredentials {
    async fn create(&self, credential: &AppCredential) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(credential.id().clone(), credential.clone());
        Ok(())
    }
    async fn find_by_id(&self, id: &AppCredentialId) -> DomainResult<AppCredential> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("AppCredential", id))
    }
    async fn list_by_app(&self, app_id: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.app_id() == app_id)
            .cloned()
            .collect())
    }
    async fn list_enabled_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        unreachable!("only the token exchange lists enabled secrets")
    }
    async fn set_enabled(&self, id: &AppCredentialId, enabled: bool) -> DomainResult<()> {
        let mut rows = self.rows.lock().unwrap();
        if let Some(c) = rows.get(id).cloned() {
            rows.insert(
                id.clone(),
                AppCredential::from_persistence(
                    c.id().clone(),
                    c.app_id().clone(),
                    c.label().clone(),
                    c.secret_hash().clone(),
                    enabled,
                    c.created_at(),
                    c.updated_at(),
                ),
            );
        }
        Ok(())
    }
    async fn set_label(&self, id: &AppCredentialId, label: &AppSecretLabel) -> DomainResult<()> {
        let mut rows = self.rows.lock().unwrap();
        let Some(c) = rows.get(id).cloned() else {
            return Ok(());
        };
        if rows
            .values()
            .any(|o| o.id() != id && o.app_id() == c.app_id() && o.label() == label)
        {
            return Err(DomainError::conflict(
                "App secret label already exists on this app",
            ));
        }
        rows.insert(
            id.clone(),
            AppCredential::from_persistence(
                c.id().clone(),
                c.app_id().clone(),
                label.clone(),
                c.secret_hash().clone(),
                c.is_enabled(),
                c.created_at(),
                c.updated_at(),
            ),
        );
        Ok(())
    }
    async fn delete(&self, id: &AppCredentialId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        Ok(())
    }
}

struct Lab {
    actions: Actions,
    uc: AppUseCases,
    apps: Arc<StubApps>,
    credentials: Arc<StubCredentials>,
    hash: Arc<StubHash>,
    registry: Arc<StubRegistry>,
}

impl Lab {
    async fn create(&self) -> DomainResult<CreatedApp> {
        self.actions.run(&self.uc, &alice(), create()).await
    }

    async fn create_secret(&self, app_id: &AppId) -> DomainResult<CreatedAppSecret> {
        self.create_secret_labelled(app_id, "ci").await
    }

    async fn create_secret_labelled(
        &self,
        app_id: &AppId,
        label: &str,
    ) -> DomainResult<CreatedAppSecret> {
        self.actions
            .run(
                &self.uc,
                &alice(),
                CreateAppSecret {
                    app_id: app_id.clone(),
                    label: AppSecretLabel::new(label).unwrap(),
                },
            )
            .await
    }

    async fn set_secret_label(
        &self,
        id: &AppCredentialId,
        label: &str,
    ) -> DomainResult<AppCredential> {
        self.actions
            .run(
                &self.uc,
                &alice(),
                SetAppSecretLabel {
                    id: id.clone(),
                    label: AppSecretLabel::new(label).unwrap(),
                },
            )
            .await
    }

    async fn revoke_secret(&self, id: &AppCredentialId) -> DomainResult<Deleted<AppCredential>> {
        self.actions
            .run(&self.uc, &alice(), RevokeAppSecret { id: id.clone() })
            .await
    }

    async fn set_secret_enabled(
        &self,
        id: &AppCredentialId,
        enabled: bool,
    ) -> DomainResult<AppCredential> {
        self.actions
            .run(
                &self.uc,
                &alice(),
                SetAppSecretEnabled {
                    id: id.clone(),
                    enabled,
                },
            )
            .await
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let apps = Arc::new(StubApps::default());
    let credentials = Arc::new(StubCredentials::default());
    let hash = Arc::new(StubHash::secrets());
    let registry = Arc::new(StubRegistry::default());
    Lab {
        actions: actions(permissions),
        uc: AppUseCases::new(
            apps.clone(),
            credentials.clone(),
            hash.clone(),
            registry.clone(),
        ),
        apps,
        credentials,
        hash,
        registry,
    }
}

fn org() -> OrganizationId {
    OrganizationId::new("acme")
}

fn create() -> CreateApp {
    CreateApp {
        organization_id: org(),
        name: AppName::new("ci-bot").unwrap(),
        secret_label: None,
    }
}

#[tokio::test]
async fn a_create_checks_the_permission_then_stores_the_app_with_its_default_secret() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());

    let created = lab.create().await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateApp(org())]
    );
    assert!(!created.secret.as_str().is_empty());
    assert!(lab.apps.rows.lock().unwrap().contains_key(created.app.id()));
    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].app_id(), created.app.id());
    assert_eq!(credentials[0].label().as_str(), "ci-bot");
}

#[tokio::test]
async fn a_create_with_a_secret_label_names_the_first_secret_with_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let labelled = CreateApp {
        secret_label: Some(AppSecretLabel::new("build-01.example.com").unwrap()),
        ..create()
    };
    lab.actions.run(&lab.uc, &alice(), labelled).await.unwrap();

    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials[0].label().as_str(), "build-01.example.com");
}

#[tokio::test]
async fn a_create_without_a_label_cuts_a_long_name_to_the_label_limit() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let long = CreateApp {
        name: AppName::new("b".repeat(100)).unwrap(),
        ..create()
    };
    lab.actions.run(&lab.uc, &alice(), long).await.unwrap();

    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials[0].label().as_str(), "b".repeat(64));
}

#[tokio::test]
async fn a_denied_create_never_hashes_or_persists() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab.create().await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reads_check_their_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let app = lab
        .actions
        .run(&lab.uc, &alice(), GetApp { id: id.clone() })
        .await
        .unwrap();
    let apps = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ListApps {
                organization_id: org(),
            },
        )
        .await
        .unwrap();
    let secrets = lab
        .actions
        .run(&lab.uc, &alice(), ListAppSecrets { app_id: id.clone() })
        .await
        .unwrap();

    assert_eq!(app.id(), &id);
    assert_eq!(apps.len(), 1);
    assert!(secrets.is_empty());
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::ReadApp(id.clone()),
            Permission::ListAppsByOrganization(org()),
            Permission::ReadApp(id),
        ]
    );
}

#[tokio::test]
async fn a_deactivation_disconnects_the_app_and_returns_the_fresh_row() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let app = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            SetAppActive {
                id: id.clone(),
                is_active: false,
            },
        )
        .await
        .unwrap();

    assert!(!app.is_active());
    assert_eq!(lab.registry.disconnected(), vec![id.clone()]);
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn an_activation_keeps_the_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create().await.unwrap();

    lab.actions
        .run(
            &lab.uc,
            &alice(),
            SetAppActive {
                id: created.app.id().clone(),
                is_active: true,
            },
        )
        .await
        .unwrap();

    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_delete_closes_the_stream_and_wakes_the_dispatcher() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let deleted = lab
        .actions
        .run(&lab.uc, &alice(), DeleteApp { id: id.clone() })
        .await
        .unwrap();

    assert_eq!(deleted.last_state(), &id);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
    assert_eq!(lab.registry.disconnected(), vec![id.clone()]);
    assert_eq!(lab.registry.wakes(), [None]);
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn a_denied_delete_keeps_the_stream_and_the_row() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            DeleteApp {
                id: AppId::new("app-1"),
            },
        )
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_create_refuses_the_reserved_runner_name_before_hashing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let reserved = CreateApp {
        name: AppName::new(TRIGGER_RUNNER_APP_NAME).unwrap(),
        ..create()
    };
    let err = lab
        .actions
        .run(&lab.uc, &alice(), reserved)
        .await
        .err()
        .unwrap();

    assert!(matches!(err, DomainError::Validation(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_trigger_runner_cannot_be_disabled_deleted_or_given_a_secret() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let runner = App::trigger_runner(org()).unwrap();
    let id = runner.id().clone();
    lab.apps.rows.lock().unwrap().insert(id.clone(), runner);

    let disable = SetAppActive {
        id: id.clone(),
        is_active: false,
    };
    let disable = lab
        .actions
        .run(&lab.uc, &alice(), disable)
        .await
        .err()
        .unwrap();
    let delete = DeleteApp { id: id.clone() };
    let delete = lab
        .actions
        .run(&lab.uc, &alice(), delete)
        .await
        .err()
        .unwrap();
    let secret = lab.create_secret(&id).await.err().unwrap();

    assert!(matches!(disable, DomainError::BusinessRule(_)));
    assert!(matches!(delete, DomainError::BusinessRule(_)));
    assert!(matches!(secret, DomainError::BusinessRule(_)));
    assert!(lab.apps.rows.lock().unwrap()[&id].is_active());
    assert!(lab.credentials.rows.lock().unwrap().is_empty());
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn an_allowed_action_on_an_unknown_app_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let ghost = AppId::new("ghost");

    let delete = DeleteApp { id: ghost.clone() };
    let delete = lab
        .actions
        .run(&lab.uc, &alice(), delete)
        .await
        .err()
        .unwrap();
    let disable = SetAppActive {
        id: ghost,
        is_active: false,
    };
    let disable = lab
        .actions
        .run(&lab.uc, &alice(), disable)
        .await
        .err()
        .unwrap();

    assert!(matches!(delete, DomainError::NotFound(_)));
    assert!(matches!(disable, DomainError::NotFound(_)));
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_secret_create_checks_the_app_exists_then_stores_the_credential() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let secret = lab.create_secret(&id).await.unwrap();

    assert_eq!(secret.credential.app_id(), &id);
    assert_eq!(secret.credential.label().as_str(), "ci");
    assert!(
        lab.credentials
            .rows
            .lock()
            .unwrap()
            .contains_key(secret.credential.id())
    );
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn a_secret_for_a_missing_app_is_not_found_and_never_hashed() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab.create_secret(&AppId::new("ghost")).await.err().unwrap();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.credentials.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_revoke_checks_the_permission_on_the_secret_then_disconnects_its_app() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let secret = lab.create_secret(&id).await.unwrap();
    let secret_id = secret.credential.id().clone();

    let revoked = lab.revoke_secret(&secret_id).await.unwrap();

    assert_eq!(revoked.last_state().id(), &secret_id);
    assert!(lab.credentials.rows.lock().unwrap().is_empty());
    assert_eq!(lab.registry.disconnected(), vec![id]);
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn disabling_a_secret_disconnects_its_app_and_returns_the_fresh_row() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let secret = lab.create_secret(&id).await.unwrap();
    let secret_id = secret.credential.id().clone();

    let credential = lab.set_secret_enabled(&secret_id, false).await.unwrap();

    assert!(!credential.is_enabled());
    assert_eq!(lab.registry.disconnected(), vec![id]);
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn enabling_a_secret_keeps_the_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create().await.unwrap();
    let secret = lab.create_secret(created.app.id()).await.unwrap();

    let credential = lab
        .set_secret_enabled(secret.credential.id(), true)
        .await
        .unwrap();

    assert!(credential.is_enabled());
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_rename_checks_the_permission_on_the_secret_and_keeps_the_stream() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let secret = lab.create_secret(created.app.id()).await.unwrap();
    let secret_id = secret.credential.id().clone();

    let credential = lab
        .set_secret_label(&secret_id, "build-01.example.com")
        .await
        .unwrap();

    assert_eq!(credential.id(), &secret_id);
    assert_eq!(credential.label().as_str(), "build-01.example.com");
    assert!(credential.is_enabled());
    assert!(lab.registry.disconnected().is_empty());
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn a_rename_to_a_label_of_the_same_app_conflicts_and_keeps_the_label() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let first = lab.create_secret_labelled(&id, "first").await.unwrap();
    lab.create_secret_labelled(&id, "second").await.unwrap();

    let err = lab
        .set_secret_label(first.credential.id(), "second")
        .await
        .unwrap_err();
    let same = lab
        .set_secret_label(first.credential.id(), "first")
        .await
        .unwrap();

    assert!(matches!(err, DomainError::Conflict(_)));
    assert_eq!(same.label().as_str(), "first");
}

#[tokio::test]
async fn a_denied_secret_action_never_reads_or_writes() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));
    let credential = AppCredential::create(
        AppId::new("app-1"),
        AppSecretLabel::new("ci").unwrap(),
        lab.hash.hash_secret(&mint_app_secret()).await.unwrap(),
    );
    lab.credentials.create(&credential).await.unwrap();

    let revoke = lab.revoke_secret(credential.id()).await.unwrap_err();
    let disable = lab
        .set_secret_enabled(credential.id(), false)
        .await
        .unwrap_err();
    let rename = lab
        .set_secret_label(credential.id(), "renamed")
        .await
        .unwrap_err();

    assert!(matches!(revoke, DomainError::Forbidden(_)));
    assert!(matches!(disable, DomainError::Forbidden(_)));
    assert!(matches!(rename, DomainError::Forbidden(_)));
    assert!(lab.registry.disconnected().is_empty());
    let kept = lab.credentials.rows.lock().unwrap()[credential.id()].clone();
    assert!(
        kept.is_enabled(),
        "a denied action leaves the secret as it was"
    );
    assert_eq!(kept.label().as_str(), "ci");
}

#[tokio::test]
async fn an_allowed_action_on_an_unknown_secret_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let missing = AppCredentialId::new("missing");

    let revoke = lab.revoke_secret(&missing).await.unwrap_err();
    let disable = lab.set_secret_enabled(&missing, false).await.unwrap_err();
    let rename = lab.set_secret_label(&missing, "renamed").await.unwrap_err();

    assert!(matches!(revoke, DomainError::NotFound(_)));
    assert!(matches!(disable, DomainError::NotFound(_)));
    assert!(matches!(rename, DomainError::NotFound(_)));
    assert!(lab.registry.disconnected().is_empty());
}

fn agent_access(app_id: &AppId) -> PrincipalAccess {
    PrincipalAccess {
        grant_id: format!("{app_id}-acme"),
        scope: Scope::Organization(org()),
        organization_id: Some(org()),
        organization_name: Some(OrganizationName::new("Acme").unwrap()),
        project_name: None,
        role: RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        role_name: RoleDisplayName::new("Organization agent").unwrap(),
    }
}

#[tokio::test]
async fn the_access_list_needs_read_app_and_an_unknown_app_holds_nothing() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    lab.apps
        .access
        .lock()
        .unwrap()
        .push((id.clone(), agent_access(&id)));

    let access = lab
        .actions
        .run(&lab.uc, &alice(), ListAppAccess { app_id: id.clone() })
        .await
        .unwrap();
    let ghost = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ListAppAccess {
                app_id: AppId::new("ghost"),
            },
        )
        .await
        .unwrap();

    assert_eq!(access, vec![agent_access(&id)]);
    assert!(ghost.is_empty());
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::ReadApp(id),
            Permission::ReadApp(AppId::new("ghost")),
        ]
    );
}
