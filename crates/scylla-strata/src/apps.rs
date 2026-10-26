use scylla_core::application::agent::AgentDispatch;
use scylla_core::application::app::{
    CreateApp, CreateAppSecret, CreatedApp, CreatedAppSecret, DeleteApp, GetApp, IssueAppToken,
    ListAppAccess, ListAppSecrets, ListApps, RevokeAppSecret, SetAppActive, SetAppSecretEnabled,
    SetAppSecretLabel, mint_app_secret,
};
use scylla_core::application::auth::new_session;
use scylla_core::application::{
    AppCredentialRepository, AppRepository, AppTokenRepository, HashService, PrincipalAccess,
};
use scylla_domain::domain::app::{App, AppCredential, AppToken, default_secret_label};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::UserId;
use scylla_extension::Describe;
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

    #[usecase]
    async fn create_app(&self, cmd: CreateApp) -> DomainResult<CreatedApp> {
        self.require_access(cmd.access()).await?;
        let app = App::create(cmd.organization_id, cmd.name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let label = cmd
            .secret_label
            .unwrap_or_else(|| default_secret_label(cmd.name.as_str()));
        let credential = AppCredential::create(app.id().clone(), label, secret_hash);
        self.app_repo().create_app(&app, &credential).await?;
        Ok(CreatedApp { app, secret })
    }

    #[usecase]
    async fn get_app(&self, query: GetApp) -> DomainResult<App> {
        self.require_access(query.access()).await?;
        self.app_repo().find_by_id(&query.id).await
    }

    #[usecase]
    async fn list_apps(&self, query: ListApps) -> DomainResult<Vec<App>> {
        self.require_access(query.access()).await?;
        self.app_repo()
            .list_by_organization(&query.organization_id)
            .await
    }

    #[usecase]
    async fn set_app_active(&self, cmd: SetAppActive) -> DomainResult<App> {
        self.require_access(cmd.access()).await?;
        let app = self.app_repo().find_by_id(&cmd.id).await?;
        app.ensure_user_managed()?;
        let id = app.id();
        self.app_repo().set_active(id, cmd.is_active).await?;
        if !cmd.is_active {
            self.app_registry().disconnect(id);
        }
        self.app_repo().find_by_id(id).await
    }

    #[usecase]
    async fn delete_app(&self, cmd: DeleteApp) -> DomainResult<()> {
        self.require_access(cmd.access()).await?;
        let app = self.app_repo().find_by_id(&cmd.id).await?;
        app.ensure_user_managed()?;
        self.app_repo().delete(app.id()).await?;
        self.app_registry().disconnect(app.id());
        self.app_registry().wake(None);
        Ok(())
    }

    #[usecase]
    async fn create_app_secret(&self, cmd: CreateAppSecret) -> DomainResult<CreatedAppSecret> {
        self.require_access(cmd.access()).await?;
        self.app_repo()
            .find_by_id(&cmd.app_id)
            .await?
            .ensure_user_managed()?;
        let secret = mint_app_secret();
        let secret_hash = self.app_hash().hash_secret(&secret).await?;
        let credential = AppCredential::create(cmd.app_id, cmd.label, secret_hash);
        self.app_credentials().create(&credential).await?;
        Ok(CreatedAppSecret { credential, secret })
    }

    #[usecase]
    async fn list_app_secrets(&self, query: ListAppSecrets) -> DomainResult<Vec<AppCredential>> {
        self.require_access(query.access()).await?;
        self.app_credentials().list_by_app(&query.app_id).await
    }

    #[usecase]
    async fn revoke_app_secret(&self, cmd: RevokeAppSecret) -> DomainResult<()> {
        self.require_access(cmd.access()).await?;
        let credential = self.app_credentials().find_by_id(&cmd.id).await?;
        self.app_credentials().delete(credential.id()).await?;
        self.app_registry().disconnect(credential.app_id());
        Ok(())
    }

    #[usecase]
    async fn set_app_secret_enabled(
        &self,
        cmd: SetAppSecretEnabled,
    ) -> DomainResult<AppCredential> {
        self.require_access(cmd.access()).await?;
        let credential = self.app_credentials().find_by_id(&cmd.id).await?;
        self.app_credentials()
            .set_enabled(credential.id(), cmd.enabled)
            .await?;
        if !cmd.enabled {
            self.app_registry().disconnect(credential.app_id());
        }
        self.app_credentials().find_by_id(credential.id()).await
    }

    #[usecase]
    async fn set_app_secret_label(&self, cmd: SetAppSecretLabel) -> DomainResult<AppCredential> {
        self.require_access(cmd.access()).await?;
        let credential = self.app_credentials().find_by_id(&cmd.id).await?;
        self.app_credentials()
            .set_label(credential.id(), &cmd.label)
            .await?;
        self.app_credentials().find_by_id(credential.id()).await
    }

    #[usecase]
    async fn list_app_access(&self, query: ListAppAccess) -> DomainResult<Vec<PrincipalAccess>> {
        self.require_access(query.access()).await?;
        self.app_repo().list_access(&query.app_id).await
    }

    #[usecase]
    async fn issue_app_token(&self, cmd: IssueAppToken) -> DomainResult<AppToken> {
        self.require_access(cmd.access()).await?;
        let invalid = || DomainError::unauthorized("Invalid app credentials");
        let app = self
            .app_repo()
            .find_by_id(&cmd.app_id)
            .await
            .map_err(|_| invalid())?;
        if !app.is_active() {
            return Err(invalid());
        }
        let credentials = self
            .app_credentials()
            .list_enabled_by_app(&cmd.app_id)
            .await
            .map_err(|_| invalid())?;
        let mut matched: Option<&AppCredential> = None;
        for credential in &credentials {
            if self
                .app_hash()
                .verify_secret(&cmd.secret, credential.secret_hash())
                .await?
            {
                matched = Some(credential);
            }
        }
        let matched = matched.ok_or_else(invalid)?;
        let token = AppToken::create(
            cmd.app_id,
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
