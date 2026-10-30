//! The secret's actions through the engine, on stub ports.

use super::*;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{ProjectId, SecretId};
use crate::domain::permission::Permission;
use crate::domain::secret::{Secret, SecretChange, SecretDescription, SecretName, SecretValue};
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
    async fn update(&self, change: &SecretChange) -> DomainResult<Secret> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .get(change.id())
            .ok_or_else(|| DomainError::not_found("Secret", change.id()))?;
        let updated = Secret::from_persistence(
            row.id().clone(),
            row.project_id().clone(),
            row.name().clone(),
            change
                .description()
                .cloned()
                .unwrap_or_else(|| row.description().clone()),
            change
                .encrypted_value()
                .unwrap_or(row.encrypted_value())
                .to_vec(),
            row.created_at(),
            change.updated_at(),
        );
        rows.insert(change.id().clone(), updated.clone());
        Ok(updated)
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
    fn encrypt(&self, plaintext: &str) -> DomainResult<Vec<u8>> {
        *self.encrypted.lock().unwrap() += 1;
        Ok(plaintext.as_bytes().to_vec())
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

    async fn update(
        &self,
        id: &SecretId,
        value: Option<&str>,
        description: Option<&str>,
    ) -> DomainResult<Secret> {
        let update = UpdateSecret {
            id: id.clone(),
            value: value.map(|v| SecretValue::new(v).unwrap()),
            description: description.map(|d| SecretDescription::new(d).unwrap()),
        };
        self.actions.run(&self.uc, &alice(), update).await
    }

    fn row(&self, id: &SecretId) -> Secret {
        self.secrets.rows.lock().unwrap()[id].clone()
    }
}

fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    lab_with(permissions, Arc::new(StubCipher::default()))
}

fn lab_with(permissions: Arc<dyn PermissionService>, cipher: Arc<StubCipher>) -> Lab {
    let secrets = Arc::new(StubSecrets::default());
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
    assert_eq!(secret.encrypted_value(), b"value");
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

#[tokio::test]
async fn an_update_checks_the_permission_on_the_secret_then_writes_a_new_ciphertext() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone());
    let created = lab.create().await.unwrap();

    let updated = lab
        .update(created.id(), Some("new-value"), None)
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions()[1],
        Permission::UpdateSecret(created.id().clone())
    );
    let row = lab.row(created.id());
    assert_eq!(row.encrypted_value(), b"new-value");
    assert_eq!(row.description().as_str(), "desc");
    assert_eq!(row.name(), created.name());
    assert_eq!(row.created_at(), created.created_at());
    assert!(row.updated_at() >= created.updated_at());
    assert_eq!(updated.id(), created.id());
}

#[tokio::test]
async fn an_update_of_the_description_alone_keeps_the_value_and_encrypts_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create().await.unwrap();

    lab.update(created.id(), None, Some("rotated monthly"))
        .await
        .unwrap();

    let row = lab.row(created.id());
    assert_eq!(row.description().as_str(), "rotated monthly");
    assert_eq!(row.encrypted_value(), created.encrypted_value());
    assert_eq!(*lab.cipher.encrypted.lock().unwrap(), 1);
}

#[tokio::test]
async fn an_update_with_neither_field_is_invalid_and_writes_nothing() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));
    let created = lab.create().await.unwrap();

    let err = lab.update(created.id(), None, None).await.unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert_eq!(lab.row(created.id()), created);
}

struct NoMasterKey;

impl SecretCipher for NoMasterKey {
    fn encrypt(&self, _: &str) -> DomainResult<Vec<u8>> {
        Err(DomainError::business_rule("no secret master key"))
    }
    fn decrypt(&self, _: &[u8]) -> DomainResult<String> {
        unreachable!("no secret action decrypts")
    }
}

#[tokio::test]
async fn a_new_value_without_a_master_key_fails_its_precondition() {
    let secrets = Arc::new(StubSecrets::default());
    let uc = SecretUseCases::new(secrets.clone(), Arc::new(NoMasterKey));
    let actions = actions(Arc::new(RecordingPermissionService::new()));
    let secret = Secret::create(
        project(),
        SecretName::new("DB_PASSWORD").unwrap(),
        SecretDescription::new("").unwrap(),
        vec![0xAA],
    );
    secrets.create(&secret).await.unwrap();
    let update = |value: Option<&str>| UpdateSecret {
        id: secret.id().clone(),
        value: value.map(|v| SecretValue::new(v).unwrap()),
        description: Some(SecretDescription::new("kept").unwrap()),
    };

    let err = actions
        .run(&uc, &alice(), update(Some("new")))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert_eq!(secrets.rows.lock().unwrap()[secret.id()], secret);

    let described = actions.run(&uc, &alice(), update(None)).await.unwrap();
    assert_eq!(described.description().as_str(), "kept");
}

#[tokio::test]
async fn a_denied_update_never_reads_encrypts_or_writes() {
    let lab = lab_with(
        Arc::new(DenyingPermissionService::new()),
        Arc::new(StubCipher::default()),
    );
    let secret = Secret::create(
        project(),
        SecretName::new("DB_PASSWORD").unwrap(),
        SecretDescription::new("").unwrap(),
        vec![0xAA],
    );
    lab.secrets.create(&secret).await.unwrap();

    let err = lab
        .update(secret.id(), Some("new"), None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(*lab.cipher.encrypted.lock().unwrap(), 0);
    assert_eq!(lab.row(secret.id()), secret);
}

#[tokio::test]
async fn an_allowed_update_of_an_unknown_secret_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()));

    let err = lab
        .update(&SecretId::new("missing"), Some("v"), None)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
}
