use crate::application::BootstrapUseCases;
use crate::config::BootstrapConfig;
use crate::error::BootstrapError;
use scylla_auth::authz::SYSTEM_ADMIN_ROLE;
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::{Email, Password, Username};

pub async fn bootstrap_admin(
    bootstrap_uc: &BootstrapUseCases,
    cfg: &BootstrapConfig,
) -> Result<(), BootstrapError> {
    if cfg.username == "admin" && cfg.password == "admin123" {
        tracing::warn!(
            "Bootstrapping the admin account with the DEFAULT credentials (admin/admin123). \
             Change `bootstrap.password` before exposing this instance — these are public."
        );
    }

    let username = Username::new(&cfg.username).map_err(BootstrapError::InvalidUsername)?;
    let password = Password::new(&cfg.password).map_err(BootstrapError::InvalidPassword)?;
    let email = Email::new(&cfg.email).map_err(BootstrapError::InvalidEmail)?;
    let role = RoleName::new(SYSTEM_ADMIN_ROLE).map_err(BootstrapError::GrantPermission)?;

    bootstrap_uc
        .bootstrap_admin(username, email, password, role)
        .await
}
