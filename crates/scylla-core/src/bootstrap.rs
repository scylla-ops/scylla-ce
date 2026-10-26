use crate::config::BootstrapConfig;
use crate::error::BootstrapError;
use scylla_auth::authz::SYSTEM_ADMIN_ROLE;
use scylla_domain::domain::role::RoleName;
use scylla_domain::domain::user::{Email, Password, Username};

/// The admin account of a bootstrap config, with the role that the bootstrap grants it.
pub struct AdminAccount {
    pub username: Username,
    pub email: Email,
    pub password: Password,
    pub role: RoleName,
}

/// Reads the admin account of `cfg`. The default credentials log a warning.
pub fn admin_account(cfg: &BootstrapConfig) -> Result<AdminAccount, BootstrapError> {
    if cfg.username == "admin" && cfg.password == "admin123" {
        tracing::warn!(
            "Bootstrapping the admin account with the DEFAULT credentials (admin/admin123). \
             Change `bootstrap.password` before exposing this instance — these are public."
        );
    }

    Ok(AdminAccount {
        username: Username::new(&cfg.username).map_err(BootstrapError::InvalidUsername)?,
        password: Password::new(&cfg.password).map_err(BootstrapError::InvalidPassword)?,
        email: Email::new(&cfg.email).map_err(BootstrapError::InvalidEmail)?,
        role: RoleName::new(SYSTEM_ADMIN_ROLE).map_err(BootstrapError::GrantPermission)?,
    })
}
