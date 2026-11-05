//! The import of the users: their emails, their password hashes, their handles, their personal
//! workspaces, their state, and the instance admins.

use scylla_domain::identity::users::{Email, Handle, NewUser};
use scylla_domain::Name;

use super::legacy::Legacy;
use super::service::Run;
use crate::features::identity::{accounts, admins};
use crate::features::tenancy::roles;
use crate::Error;

/// The key of the role of an instance admin in the old product.
const SYSTEM_ADMIN: &str = "system-admin";

/// The key of the role of the old product that creates organizations.
const ORGANIZATION_CREATOR: &str = "organization-creator";

/// Imports each user with an email, and the instance admins.
pub(super) async fn import(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    for old in &legacy.users {
        // Refuse a user with no valid email: an account of this base signs in with an email.
        let Some(email) = old
            .email
            .as_ref()
            .and_then(|email| Email::parse(email.trim().to_lowercase()).ok())
        else {
            run.report
                .skipped
                .push(format!("user {}: no valid email", old.username));
            continue;
        };

        // Keep the name, and the user name as a handle when it fits and is free.
        let name = old
            .display_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| old.username.clone());
        let display_name = Name::parse(name)
            .or_else(|_| Name::parse("User".to_owned()))
            .expect("a valid name");
        let handle = Handle::parse(old.username.to_lowercase())
            .ok()
            .filter(|handle| !run.handles.contains(handle.as_str()));
        if let Some(handle) = &handle {
            run.handles.insert(handle.as_str().to_owned());
        }
        let new = NewUser {
            email,
            email_verified: true,
            display_name,
            handle,
            password: None,
        };

        // Insert the user with its hash, and keep its state.
        let mut tx = run.pool.begin().await?;
        let hash = Some(old.password_hash.as_str()).filter(|hash| hash.starts_with("$argon2"));
        let actor = match accounts::insert_user(&mut tx, &new, hash).await {
            Ok(actor) => actor,
            Err(error) => {
                run.report
                    .skipped
                    .push(format!("user {}: {error}", old.username));
                continue;
            }
        };
        if !old.is_active {
            sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
                .bind(actor.id.to_db())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        run.users.insert(old.id.clone(), actor.id);
        run.report.users += 1;
    }

    // Make the instance admins and the workspace creators.
    for grant in &legacy.grants {
        let Some(user) = run.users.get(&grant.principal_id).copied() else {
            continue;
        };
        if grant.principal_kind != "user" {
            continue;
        }
        let mut tx = run.pool.begin().await?;
        match grant.role_key.as_deref() {
            Some(SYSTEM_ADMIN) => {
                admins::grant_in(&mut tx, user).await?;
                run.report.admins += 1;
            }
            Some(ORGANIZATION_CREATOR) => roles::make_workspace_creator(&mut tx, user).await?,
            _ => {}
        }
        tx.commit().await?;
    }
    Ok(())
}
