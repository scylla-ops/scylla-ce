//! The secret's actions through the engine, on stub ports.

use super::*;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::permission::Permission;
use crate::domain::secret::{Secret, SecretDescription, SecretName, SecretValue};
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::stubs::alice;
use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_extension::{Actions, Deleted};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct StubSecrets {
    rows: Mutex<HashMap<SecretId, Secret>>,
}

#[async_trait]
impl SecretRepository for StubSecrets {
    async fn create(&self, secret: &Secret) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(secret.id().clone(), secret.clone());
        Ok(())
    }
    async fn find_by_id(&self, id: &SecretId) -> DomainResult<Secret> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Secret", id))
    }
    async fn list_by_project(&self, project_id: &ProjectId) -> DomainResult<Vec<Secret>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|s| s.project_id() == project_id)
            .cloned()
            .collect())
    }
    async fn delete(&self, id: &SecretId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        Ok(())
    }
}

#[derive(Default)]
struct StubCipher {
    encrypted: Mutex<usize>,
}

impl SecretCipher for StubCipher {
    fn encrypt(&self, _: &str) -> DomainResult<Vec<u8>> {
        *self.encrypted.lock().unwrap() += 1;
        Ok(vec![0xAA, 0xBB])
    }
    fn decrypt(&self, _: &[u8]) -> DomainResult<String> {
        unreachable!("no secret action decrypts")
    }
}

struct Lab {
    actions: Actions,
    uc: SecretUseCases,
    secrets: Arc<StubSecrets>,
    cipher: Arc<StubCipher>,
}

impl Lab {
    async fn create(&self) -> DomainResult<Secret> {
        self.actions.run(&self.uc, &alice(), create()).await
    }

    async fn delete(&self, id: &SecretId) -> DomainResult<Deleted<Secret>> {
        self.actions
            .run(&self.uc, &alice(), DeleteSecret { id: id.clone() })
            .await
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let secrets = Arc::new(StubSecrets::default());
    let cipher = Arc::new(StubCipher::default());
    Lab {
        actions: actions(permissions),
        uc: SecretUseCases::new(secrets.clone(), cipher.clone()),
        secrets,
        cipher,
    }
}

fn project() -> ProjectId {
    ProjectId::new("proj-1")
}

fn create() -> CreateSecret {
    CreateSecret {
        project_id: project(),
        name: SecretName::new("DB_PASSWORD").unwrap(),
        description: SecretDescription::new("desc").unwrap(),
        value: SecretValue::new("value").unwrap(),
    }
}

#[tokio::test]
async fn a_create_checks_the_permission_on_the_project_then_stores_the_ciphertext() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());

    let secret = lab.create().await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateSecret(project())]
    );
    assert_eq!(secret.encrypted_value(), &[0xAA, 0xBB]);
    assert!(lab.secrets.rows.lock().unwrap().contains_key(secret.id()));
}

#[tokio::test]
async fn a_denied_create_never_encrypts_or_persists() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));

    let err = lab.create().await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(*lab.cipher.encrypted.lock().unwrap(), 0);
    assert!(lab.secrets.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_list_checks_its_permission_and_reads_the_project_secrets() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();

    let secrets = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            ListSecrets {
                project_id: project(),
            },
        )
        .await
        .unwrap();

    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].id(), created.id());
    assert_eq!(
        permissions.permissions()[1],
        Permission::ListSecrets(project())
    );
}

#[tokio::test]
async fn a_delete_checks_the_permission_on_the_secret_then_removes_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();

    let deleted = lab.delete(created.id()).await.unwrap();

    assert_eq!(deleted.last_state().id(), created.id());
    assert_eq!(
        permissions.permissions()[1],
        Permission::DeleteSecret(created.id().clone())
    );
    assert!(lab.secrets.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_denied_delete_never_reads_or_removes() {
    let lab = lab(Arc::new(DenyingPermissionService::new()));
    let secret = Secret::create(
        project(),
        SecretName::new("DB_PASSWORD").unwrap(),
        SecretDescription::new("").unwrap(),
        vec![0xAA],
    );
    lab.secrets.create(&secret).await.unwrap();

    let err = lab.delete(secret.id()).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.secrets.rows.lock().unwrap().contains_key(secret.id()));
}

#[tokio::test]
async fn an_allowed_delete_of_an_unknown_secret_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab.delete(&SecretId::new("missing")).await.unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn an_unknown_secret_ref_names_the_secret_its_node_and_its_env_key() {
    use crate::domain::pipeline::{EnvKey, EnvVar, NodeId, PipelineNode, Step};
    let resolver = DispatchSecretResolver::new(
        Arc::new(StubSecrets::default()),
        Arc::new(StubCipher::default()),
    );
    let node = PipelineNode::new(
        NodeId::new("n1").unwrap(),
        Vec::new(),
        Step::exec("echo".into(), Vec::new()).unwrap(),
        None,
        vec![EnvVar::secret(
            EnvKey::new("X").unwrap(),
            SecretName::new("MISSING").unwrap(),
        )],
    );

    let err = resolver.resolve(&project(), &[node]).await.unwrap_err();

    let DomainError::NotFound(message) = err else {
        panic!("expected NotFound, got {err:?}");
    };
    assert_eq!(
        message,
        "secret 'MISSING' referenced by node 'n1' env 'X' is not in this project"
    );
}
