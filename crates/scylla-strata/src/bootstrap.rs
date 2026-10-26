use crate::access::CallerOf;
use crate::{Grants, GrantsRpc, Slice, Users, UsersRpc};
use scylla_auth::authz::{Principal, Scope};
use scylla_core::application::grant::CreateGrant;
use scylla_core::application::user::{
    CreateUser, GetUserByEmail, GetUserByUsername, UpdateUserEmail,
};
use scylla_core::bootstrap::{AdminAccount, admin_account};
use scylla_core::config::BootstrapConfig;
use scylla_core::error::BootstrapError;
use scylla_domain::domain::caller::ServiceIdentity;
use scylla_domain::domain::errors::DomainError;
use scylla_domain::domain::user::{Email, User, Username};
use strata::{Contextual, Scoped};

/// The slices of the bootstrap writes. The bootstrap runs on strata only when each one is switched.
pub const BOOTSTRAP_SLICES: &[Slice] = &[Slice::Users, Slice::Grants];

/// The CE bootstrap on the Users and Grants modules, as the `bootstrap` service. The admin is a
/// new account, the account with the configured email, or the account with the configured
/// username when it has no email. No other account gets the System grant. A second run writes
/// nothing new.
pub async fn bootstrap_admin<E>(app: &E, cfg: &BootstrapConfig) -> Result<(), BootstrapError>
where
    E: Users + Grants,
    <E as Contextual>::Ctx: CallerOf,
{
    let AdminAccount {
        username,
        email,
        password,
        role,
    } = admin_account(cfg)?;
    let ctx = crate::workers::service::<<E as Contextual>::Ctx>(ServiceIdentity::bootstrap());
    let view = app.scoped(&ctx);

    let create = CreateUser {
        username: username.clone(),
        email: email.clone(),
        password,
        display_name: None,
    };
    let user = match UsersRpc::create_user(&view, create).await {
        Ok(user) => {
            tracing::info!(user_id = %user.id(), username = %username, "bootstrap user created");
            user
        }
        Err(DomainError::Conflict(_)) => {
            tracing::debug!(username = %username, "bootstrap user already exists");
            existing_admin(&view, &username, email).await?
        }
        Err(e) => return Err(BootstrapError::CreateUser(e)),
    };

    let grant = CreateGrant {
        principal: Principal::User(user.id().clone()),
        role: role.clone(),
        scope: Scope::System,
    };
    GrantsRpc::create_grant(&view, grant)
        .await
        .map_err(BootstrapError::GrantPermission)?;
    tracing::info!(user_id = %user.id(), role = %role, "bootstrap system grant ensured");
    Ok(())
}

async fn existing_admin<E: Users>(
    view: &Scoped<'_, E>,
    username: &Username,
    email: Email,
) -> Result<User, BootstrapError> {
    let by_email = GetUserByEmail {
        email: email.clone(),
    };
    match UsersRpc::get_user_by_email(view, by_email).await {
        Ok(user) if user.username() == username => return Ok(user),
        Ok(_) => return Err(BootstrapError::EmailOfAnotherAccount),
        Err(e) if e.is_not_found() => {}
        Err(e) => return Err(BootstrapError::FindUser(e)),
    }

    let by_username = GetUserByUsername {
        username: username.clone(),
    };
    let user = UsersRpc::get_user_by_username(view, by_username)
        .await
        .map_err(BootstrapError::FindUser)?;
    if user.email().is_some() {
        return Err(BootstrapError::UsernameWithAnotherEmail);
    }
    let update = UpdateUserEmail {
        id: user.id().clone(),
        email,
    };
    let user = UsersRpc::update_user_email(view, update)
        .await
        .map_err(BootstrapError::UpdateEmail)?;
    tracing::info!(user_id = %user.id(), "bootstrap email set on the existing user");
    Ok(user)
}
