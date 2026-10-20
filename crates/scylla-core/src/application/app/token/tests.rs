//! The app token exchange through the engine, on stub ports.

use super::*;
use crate::application::PrincipalAccess;
use crate::domain::agent::Agent;
use crate::domain::app::AppToken;
use crate::domain::app::{
    App, AppCredential, AppKind, AppName, AppSecret, AppSecretHash, AppSecretLabel,
};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppCredentialId, AppId, OrganizationId};
use crate::domain::user::{Password, PasswordHash};
use crate::test_support::authz::{DenyingPermissionService, actions};
use async_trait::async_trait;
use scylla_auth::authz::Grant;
use std::sync::Mutex;

const SECRET: &str = "s3cr3t-s3cr3t-s3cr3t-s3cr3t-s3cr3t";
const OTHER: &str = "0th3r-0th3r-0th3r-0th3r-0th3r-0th3r";

struct OneApp(App);

#[async_trait]
impl AppRepository for OneApp {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("no app write in the token exchange")
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("no app write in the token exchange")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("no app write in the token exchange")
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        if id == self.0.id() {
            Ok(self.0.clone())
        } else {
            Err(DomainError::not_found("App", id))
        }
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("the token exchange reads one app")
    }
    async fn list_by_organization(&self, _: &OrganizationId) -> DomainResult<Vec<App>> {
        unreachable!("the token exchange reads one app")
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("no app write in the token exchange")
    }
    async fn delete(&self, _: &AppId) -> DomainResult<()> {
        unreachable!("no app write in the token exchange")
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        unreachable!("no app access in the token exchange")
    }
}

struct EnabledCredentials(Vec<AppCredential>);

#[async_trait]
impl AppCredentialRepository for EnabledCredentials {
    async fn create(&self, _: &AppCredential) -> DomainResult<()> {
        unreachable!("no secret write in the token exchange")
    }
    async fn find_by_id(&self, _: &AppCredentialId) -> DomainResult<AppCredential> {
        unreachable!("the token exchange lists the enabled secrets")
    }
    async fn list_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        unreachable!("the token exchange lists the enabled secrets")
    }
    async fn list_enabled_by_app(&self, _: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(self.0.clone())
    }
    async fn set_enabled(&self, _: &AppCredentialId, _: bool) -> DomainResult<()> {
        unreachable!("no secret write in the token exchange")
    }
    async fn set_label(&self, _: &AppCredentialId, _: &AppSecretLabel) -> DomainResult<()> {
        unreachable!("no secret write in the token exchange")
    }
    async fn delete(&self, _: &AppCredentialId) -> DomainResult<()> {
        unreachable!("no secret write in the token exchange")
    }
}

/// A hash is `$` and the secret, so a secret matches exactly one credential.
#[derive(Default)]
struct CheckingHash {
    verified: Mutex<usize>,
}

#[async_trait]
impl HashService for CheckingHash {
    async fn hash(&self, _: &Password) -> DomainResult<PasswordHash> {
        unreachable!("no password in the token exchange")
    }
    async fn verify(&self, _: &Password, _: &PasswordHash) -> DomainResult<bool> {
        unreachable!("no password in the token exchange")
    }
    async fn hash_secret(&self, _: &AppSecret) -> DomainResult<AppSecretHash> {
        unreachable!("no secret to hash in the token exchange")
    }
    async fn verify_secret(&self, secret: &AppSecret, hash: &AppSecretHash) -> DomainResult<bool> {
        *self.verified.lock().unwrap() += 1;
        Ok(hash.as_str() == format!("${}", secret.as_str()))
    }
}

#[derive(Default)]
struct StubTokens {
    rows: Mutex<Vec<AppToken>>,
}

#[async_trait]
impl AppTokenRepository for StubTokens {
    async fn create(&self, token: &AppToken) -> DomainResult<()> {
        self.rows.lock().unwrap().push(token.clone());
        Ok(())
    }
    async fn find_by_token(&self, _: &str) -> DomainResult<AppToken> {
        unreachable!("the token exchange never reads a token")
    }
}

struct Lab {
    uc: AppTokenUseCases,
    matching: AppCredentialId,
    hash: Arc<CheckingHash>,
    tokens: Arc<StubTokens>,
}

impl Lab {
    async fn issue(&self, app_id: &str, secret: &str) -> DomainResult<AppToken> {
        actions(Arc::new(DenyingPermissionService::new()))
            .run(
                &self.uc,
                &CallerContext::Anonymous,
                IssueAppToken {
                    app_id: AppId::new(app_id),
                    secret: AppSecret::new(secret).unwrap(),
                },
            )
            .await
    }
}

fn lab(active: bool) -> Lab {
    let app = App::from_persistence(
        AppId::new("app-1"),
        OrganizationId::new("acme"),
        AppName::new("ci").unwrap(),
        AppKind::Standard,
        active,
        chrono::Utc::now(),
        chrono::Utc::now(),
    );
    let credential = |secret: &str| {
        AppCredential::create(
            app.id().clone(),
            AppSecretLabel::new("default").unwrap(),
            AppSecretHash::new(format!("${secret}")).unwrap(),
        )
    };
    let credentials = vec![credential(SECRET), credential(OTHER)];
    let matching = credentials[0].id().clone();
    let hash = Arc::new(CheckingHash::default());
    let tokens = Arc::new(StubTokens::default());
    Lab {
        uc: AppTokenUseCases::new(
            Arc::new(OneApp(app)),
            tokens.clone(),
            Arc::new(EnabledCredentials(credentials)),
            hash.clone(),
        ),
        matching,
        hash,
        tokens,
    }
}

#[tokio::test]
async fn a_matching_secret_stores_a_token_for_that_secret_without_asking_a_permission() {
    let lab = lab(true);

    let token = lab.issue("app-1", SECRET).await.unwrap();

    assert_eq!(token.app_id(), &AppId::new("app-1"));
    assert_eq!(token.secret_id(), &lab.matching);
    assert_eq!(*lab.hash.verified.lock().unwrap(), 2);
    assert_eq!(lab.tokens.rows.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_unknown_app_an_inactive_app_and_a_wrong_secret_give_one_opaque_error() {
    let wrong = "wr0ng-wr0ng-wr0ng-wr0ng-wr0ng-wr0ng";
    for (lab, app_id, secret) in [
        (lab(true), "app-2", SECRET),
        (lab(false), "app-1", SECRET),
        (lab(true), "app-1", wrong),
    ] {
        let err = lab.issue(app_id, secret).await.unwrap_err();
        assert!(matches!(&err, DomainError::Unauthorized(m) if m == "Invalid app credentials"));
        assert!(lab.tokens.rows.lock().unwrap().is_empty());
    }
}
