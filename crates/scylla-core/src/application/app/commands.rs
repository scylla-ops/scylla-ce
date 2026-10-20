//! The app's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` builds, what `Persist` writes. `DeleteApp` and
//! `SetAppActive` read the App to refuse the trigger runner, then stage its id; `CreateAppSecret`
//! refuses it too, so the runner never holds a secret. Without a label, `CreateApp` names its
//! first secret with `default_secret_label`.

use super::AppUseCases;
use super::mint_app_secret;
use crate::domain::app::{
    App, AppCredential, AppName, AppSecret, AppSecretLabel, default_secret_label,
};
use crate::domain::errors::DomainResult;
use crate::domain::ids::{AppCredentialId, AppId, OrganizationId};
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreateApp {
    pub organization_id: OrganizationId,
    pub name: AppName,
    pub secret_label: Option<AppSecretLabel>,
}

/// No `Debug`: `secret` is the plaintext the caller sees once.
pub struct NewApp {
    pub app: App,
    pub credential: AppCredential,
    pub secret: AppSecret,
}

pub struct CreatedApp {
    pub app: App,
    pub secret: AppSecret,
}

impl Describe for CreateApp {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreateApp(self.organization_id.clone()))
    }
}

impl Command for CreateApp {
    type Staged = Draft<NewApp>;
    type Committed = CreatedApp;
}

#[async_trait]
impl Run<Prepare<CreateApp>> for AppUseCases {
    async fn run(&self, input: Authorized<CreateApp>) -> DomainResult<Prepared<CreateApp>> {
        let cmd = input.command();
        let app = App::create(cmd.organization_id.clone(), cmd.name.clone())?;
        let secret = mint_app_secret();
        let secret_hash = self.hash_service.hash_secret(&secret).await?;
        let label = cmd
            .secret_label
            .clone()
            .unwrap_or_else(|| default_secret_label(cmd.name.as_str()));
        let credential = AppCredential::create(app.id().clone(), label, secret_hash);
        Ok(input.prepared(Draft::new(NewApp {
            app,
            credential,
            secret,
        })))
    }
}

#[async_trait]
impl Run<Persist<CreateApp>> for AppUseCases {
    async fn run(&self, input: Prepared<CreateApp>) -> DomainResult<Committed<CreateApp>> {
        input
            .commit(async |draft| {
                let NewApp {
                    app,
                    credential,
                    secret,
                } = draft.into_inner();
                self.app_repo.create_app(&app, &credential).await?;
                Ok(CreatedApp { app, secret })
            })
            .await
    }
}

#[derive(Debug)]
pub struct SetAppActive {
    pub id: AppId,
    pub is_active: bool,
}

impl Describe for SetAppActive {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteApp(self.id.clone()))
    }
}

impl Command for SetAppActive {
    type Staged = AppId;
    type Committed = App;
}

#[async_trait]
impl Run<Prepare<SetAppActive>> for AppUseCases {
    async fn run(&self, input: Authorized<SetAppActive>) -> DomainResult<Prepared<SetAppActive>> {
        let app = self.app_repo.find_by_id(&input.command().id).await?;
        app.ensure_user_managed()?;
        Ok(input.prepared(app.id().clone()))
    }
}

#[async_trait]
impl Run<Persist<SetAppActive>> for AppUseCases {
    async fn run(&self, input: Prepared<SetAppActive>) -> DomainResult<Committed<SetAppActive>> {
        let active = input.command().is_active;
        input
            .commit(async |id| {
                self.app_repo.set_active(&id, active).await?;
                // The per-action liveness check is the durable guarantee; this makes the effect instant.
                if !active {
                    self.registry.disconnect(&id);
                }
                self.app_repo.find_by_id(&id).await
            })
            .await
    }
}

#[derive(Debug)]
pub struct DeleteApp {
    pub id: AppId,
}

impl Describe for DeleteApp {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteApp(self.id.clone()))
    }
}

impl Command for DeleteApp {
    type Staged = AppId;
    type Committed = Deleted<AppId>;
}

#[async_trait]
impl Run<Prepare<DeleteApp>> for AppUseCases {
    async fn run(&self, input: Authorized<DeleteApp>) -> DomainResult<Prepared<DeleteApp>> {
        let app = self.app_repo.find_by_id(&input.command().id).await?;
        app.ensure_user_managed()?;
        Ok(input.prepared(app.id().clone()))
    }
}

#[async_trait]
impl Run<Persist<DeleteApp>> for AppUseCases {
    async fn run(&self, input: Prepared<DeleteApp>) -> DomainResult<Committed<DeleteApp>> {
        input
            .commit(async |id| {
                self.app_repo.delete(&id).await?;
                self.registry.disconnect(&id);
                self.registry.wake(None);
                Ok(Deleted::new(id))
            })
            .await
    }
}

#[derive(Debug)]
pub struct CreateAppSecret {
    pub app_id: AppId,
    pub label: AppSecretLabel,
}

/// No `Debug`: `secret` is the plaintext the caller sees once.
pub struct NewAppSecret {
    pub credential: AppCredential,
    pub secret: AppSecret,
}

pub struct CreatedAppSecret {
    pub credential: AppCredential,
    pub secret: AppSecret,
}

impl Describe for CreateAppSecret {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeleteApp(self.app_id.clone()))
    }
}

impl Command for CreateAppSecret {
    type Staged = Draft<NewAppSecret>;
    type Committed = CreatedAppSecret;
}

#[async_trait]
impl Run<Prepare<CreateAppSecret>> for AppUseCases {
    async fn run(
        &self,
        input: Authorized<CreateAppSecret>,
    ) -> DomainResult<Prepared<CreateAppSecret>> {
        let cmd = input.command();
        self.app_repo
            .find_by_id(&cmd.app_id)
            .await?
            .ensure_user_managed()?;
        let secret = mint_app_secret();
        let secret_hash = self.hash_service.hash_secret(&secret).await?;
        let credential = AppCredential::create(cmd.app_id.clone(), cmd.label.clone(), secret_hash);
        Ok(input.prepared(Draft::new(NewAppSecret { credential, secret })))
    }
}

#[async_trait]
impl Run<Persist<CreateAppSecret>> for AppUseCases {
    async fn run(
        &self,
        input: Prepared<CreateAppSecret>,
    ) -> DomainResult<Committed<CreateAppSecret>> {
        input
            .commit(async |draft| {
                let NewAppSecret { credential, secret } = draft.into_inner();
                self.credential_repo.create(&credential).await?;
                Ok(CreatedAppSecret { credential, secret })
            })
            .await
    }
}

#[derive(Debug)]
pub struct RevokeAppSecret {
    pub id: AppCredentialId,
}

impl Describe for RevokeAppSecret {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageAppSecret(self.id.clone()))
    }
}

impl Command for RevokeAppSecret {
    type Staged = AppCredential;
    type Committed = Deleted<AppCredential>;
}

#[async_trait]
impl Run<Prepare<RevokeAppSecret>> for AppUseCases {
    async fn run(
        &self,
        input: Authorized<RevokeAppSecret>,
    ) -> DomainResult<Prepared<RevokeAppSecret>> {
        let credential = self.credential_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(credential))
    }
}

#[async_trait]
impl Run<Persist<RevokeAppSecret>> for AppUseCases {
    async fn run(
        &self,
        input: Prepared<RevokeAppSecret>,
    ) -> DomainResult<Committed<RevokeAppSecret>> {
        input
            .commit(async |credential| {
                self.credential_repo.delete(credential.id()).await?;
                // The stream was authenticated once at open; a reconnect with another enabled secret re-registers.
                self.registry.disconnect(credential.app_id());
                Ok(Deleted::new(credential))
            })
            .await
    }
}

#[derive(Debug)]
pub struct SetAppSecretEnabled {
    pub id: AppCredentialId,
    pub enabled: bool,
}

impl Describe for SetAppSecretEnabled {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageAppSecret(self.id.clone()))
    }
}

impl Command for SetAppSecretEnabled {
    type Staged = AppCredential;
    type Committed = AppCredential;
}

#[async_trait]
impl Run<Prepare<SetAppSecretEnabled>> for AppUseCases {
    async fn run(
        &self,
        input: Authorized<SetAppSecretEnabled>,
    ) -> DomainResult<Prepared<SetAppSecretEnabled>> {
        let credential = self.credential_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(credential))
    }
}

#[async_trait]
impl Run<Persist<SetAppSecretEnabled>> for AppUseCases {
    async fn run(
        &self,
        input: Prepared<SetAppSecretEnabled>,
    ) -> DomainResult<Committed<SetAppSecretEnabled>> {
        let enabled = input.command().enabled;
        input
            .commit(async |credential| {
                self.credential_repo
                    .set_enabled(credential.id(), enabled)
                    .await?;
                if !enabled {
                    self.registry.disconnect(credential.app_id());
                }
                self.credential_repo.find_by_id(credential.id()).await
            })
            .await
    }
}

#[derive(Debug)]
pub struct SetAppSecretLabel {
    pub id: AppCredentialId,
    pub label: AppSecretLabel,
}

impl Describe for SetAppSecretLabel {
    fn access(&self) -> Access {
        Access::Requires(Permission::ManageAppSecret(self.id.clone()))
    }
}

impl Command for SetAppSecretLabel {
    type Staged = AppCredential;
    type Committed = AppCredential;
}

#[async_trait]
impl Run<Prepare<SetAppSecretLabel>> for AppUseCases {
    async fn run(
        &self,
        input: Authorized<SetAppSecretLabel>,
    ) -> DomainResult<Prepared<SetAppSecretLabel>> {
        let credential = self.credential_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(credential))
    }
}

#[async_trait]
impl Run<Persist<SetAppSecretLabel>> for AppUseCases {
    async fn run(
        &self,
        input: Prepared<SetAppSecretLabel>,
    ) -> DomainResult<Committed<SetAppSecretLabel>> {
        let label = input.command().label.clone();
        input
            .commit(async |credential| {
                self.credential_repo
                    .set_label(credential.id(), &label)
                    .await?;
                self.credential_repo.find_by_id(credential.id()).await
            })
            .await
    }
}
