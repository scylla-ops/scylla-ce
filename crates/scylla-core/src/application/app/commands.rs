//! The app's writes. Each command, its access and its payload types. The strata module of the slice
//! runs it. `DeleteApp` and `SetAppActive` read the App to refuse the trigger runner, then stage
//! its id; `CreateAppSecret` refuses it too, so the runner never holds a secret. Without a label,
//! `CreateApp` names its first secret with `default_secret_label`.

use crate::domain::app::{App, AppCredential, AppName, AppSecret, AppSecretLabel};
use crate::domain::ids::{AppCredentialId, AppId, OrganizationId};
use crate::domain::permission::Permission;
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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
