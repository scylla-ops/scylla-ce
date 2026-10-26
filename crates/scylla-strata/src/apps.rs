use scylla_core::application::agent::AgentDispatch;
use scylla_core::application::app::{CreatedApp, CreatedAppSecret, mint_app_secret};
use scylla_core::application::auth::new_session;
use scylla_core::application::{
    AppCredentialRepository, AppRepository, AppTokenRepository, HashService, PrincipalAccess,
};
use scylla_domain::domain::app::{
    App, AppCredential, AppName, AppSecret, AppSecretLabel, AppToken, default_secret_label,
};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppCredentialId, AppId, OrganizationId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_extension::Access;
use sqlx::types::chrono::{DateTime, Utc};
use std::sync::Arc;

const DEFAULT_APP_TOKEN_DURATION_DAYS: i64 = 30;

#[strata::module(section = "apps")]
pub trait Apps: crate::Access {
    #[component]
    type AppRepo = Arc<dyn AppRepository>;

    #[component]
    type AppCredentials = Arc<dyn AppCredentialRepository>;

    #[component]
    type AppTokens = Arc<dyn AppTokenRepository>;

    #[component]
    type AppHash = Arc<dyn HashService>;

    #[component]
    type AppRegistry = Arc<dyn AgentDispatch>;

    #[usecase(write, access = Access::Requires(Permission::CreateApp(organization_id.clone())))]
    async fn create_app(
        &self,
        organization_id: OrganizationId,
        name: AppName,
        secret_label: Option<AppSecretLabel>,
    ) -> DomainResult<CreatedApp> {
        let app = App::create(organization_id, name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let label = secret_label.unwrap_or_else(|| default_secret_label(name.as_str()));
        let credential = AppCredential::create(app.id().clone(), label, secret_hash);
        self.app_repo().create_app(&app, &credential).await?;
        Ok(CreatedApp { app, secret })
    }

    #[usecase(read, access = Access::Requires(Permission::ReadApp(id.clone())))]
    async fn get_app(&self, id: AppId) -> DomainResult<App> {
        self.app_repo().find_by_id(&id).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListAppsByOrganization(organization_id.clone())))]
    async fn list_apps(&self, organization_id: OrganizationId) -> DomainResult<Vec<App>> {
        self.app_repo().list_by_organization(&organization_id).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteApp(id.clone())))]
    async fn set_app_active(&self, id: AppId, is_active: bool) -> DomainResult<App> {
        let app = self.app_repo().find_by_id(&id).await?;
        app.ensure_user_managed()?;
        let id = app.id();
        self.app_repo().set_active(id, is_active).await?;
        if !is_active {
            self.app_registry().disconnect(id);
        }
        self.app_repo().find_by_id(id).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteApp(id.clone())))]
    async fn delete_app(&self, id: AppId) -> DomainResult<()> {
        let app = self.app_repo().find_by_id(&id).await?;
        app.ensure_user_managed()?;
        self.app_repo().delete(app.id()).await?;
        self.app_registry().disconnect(app.id());
        self.app_registry().wake(None);
        Ok(())
    }

    #[usecase(write, access = Access::Requires(Permission::DeleteApp(app_id.clone())))]
    async fn create_app_secret(
        &self,
        app_id: AppId,
        label: AppSecretLabel,
    ) -> DomainResult<CreatedAppSecret> {
        self.app_repo()
            .find_by_id(&app_id)
            .await?
            .ensure_user_managed()?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let credential = AppCredential::create(app_id, label, secret_hash);
        self.app_credentials().create(&credential).await?;
        Ok(CreatedAppSecret { credential, secret })
    }

    #[usecase(read, access = Access::Requires(Permission::ReadApp(app_id.clone())))]
    async fn list_app_secrets(&self, app_id: AppId) -> DomainResult<Vec<AppCredential>> {
        self.app_credentials().list_by_app(&app_id).await
    }

    #[usecase(write, access = Access::Requires(Permission::ManageAppSecret(id.clone())))]
    async fn revoke_app_secret(&self, id: AppCredentialId) -> DomainResult<()> {
        let credential = self.app_credentials().find_by_id(&id).await?;
        self.app_credentials().delete(credential.id()).await?;
        self.app_registry().disconnect(credential.app_id());
        Ok(())
    }

    #[usecase(write, access = Access::Requires(Permission::ManageAppSecret(id.clone())))]
    async fn set_app_secret_enabled(
        &self,
        id: AppCredentialId,
        enabled: bool,
    ) -> DomainResult<AppCredential> {
        let credential = self.app_credentials().find_by_id(&id).await?;
        self.app_credentials()
            .set_enabled(credential.id(), enabled)
            .await?;
        if !enabled {
            self.app_registry().disconnect(credential.app_id());
        }
        self.app_credentials().find_by_id(credential.id()).await
    }

    #[usecase(write, access = Access::Requires(Permission::ManageAppSecret(id.clone())))]
    async fn set_app_secret_label(
        &self,
        id: AppCredentialId,
        label: AppSecretLabel,
    ) -> DomainResult<AppCredential> {
        let credential = self.app_credentials().find_by_id(&id).await?;
        self.app_credentials()
            .set_label(credential.id(), &label)
            .await?;
        self.app_credentials().find_by_id(credential.id()).await
    }

    #[usecase(read, access = Access::Requires(Permission::ReadApp(app_id.clone())))]
    async fn list_app_access(&self, app_id: AppId) -> DomainResult<Vec<PrincipalAccess>> {
        self.app_repo().list_access(&app_id).await
    }

    #[usecase(write, access = Access::Public)]
    async fn issue_app_token(&self, app_id: AppId, secret: AppSecret) -> DomainResult<AppToken> {
        let invalid = || DomainError::unauthorized("Invalid app credentials");
        let app = self
            .app_repo()
            .find_by_id(&app_id)
            .await
            .map_err(|_| invalid())?;
        if !app.is_active() {
            return Err(invalid());
        }
        let credentials = self
            .app_credentials()
            .list_enabled_by_app(&app_id)
            .await
            .map_err(|_| invalid())?;
        let mut matched: Option<&AppCredential> = None;
        for credential in &credentials {
            if self
                .app_hash()
                .verify_secret(&secret, credential.secret_hash())
                .await?
            {
                matched = Some(credential);
            }
        }
        let matched = matched.ok_or_else(invalid)?;
        let token = AppToken::create(
            app_id,
            matched.id().clone(),
            new_session(UserId::new("")).token().to_owned(),
            DateTime::from_timestamp(DEFAULT_APP_TOKEN_DURATION_DAYS * 86_400, 0)
                .unwrap_or_default()
                - DateTime::<Utc>::default(),
        );
        self.app_tokens().create(&token).await?;
        Ok(token)
    }
}
