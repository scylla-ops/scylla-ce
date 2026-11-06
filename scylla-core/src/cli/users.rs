//! The commands of the operator on the users: a new user, and the instance admins.

use clap::Subcommand;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::NewUser;

use super::accounts::stdin_line;
use super::commands::id;
use crate::api::parse;
use crate::features::identity::{accounts, admins};
use crate::infra::db;
use crate::{Config, Error};

/// The commands on the users.
#[derive(Subcommand, Clone, Debug)]
pub enum UserCommand {
    /// Creates an active user with a verified email and its personal workspace, and prints its
    /// id.
    Create {
        /// The email.
        #[arg(long)]
        email: String,
        /// The display name.
        #[arg(long)]
        name: String,
        /// The handle, a short unique name for the sign-in.
        #[arg(long)]
        handle: Option<String>,
        /// Reads a password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
        /// Makes the user an admin of the instance.
        #[arg(long)]
        admin: bool,
    },
    /// Manages the admins of the instance.
    #[command(subcommand)]
    Admin(AdminCommand),
}

/// The commands on the instance admins.
#[derive(Subcommand, Clone, Debug)]
pub enum AdminCommand {
    /// Makes a user an admin of the instance.
    Grant {
        /// The id of the user.
        #[arg(value_parser = id::<Actor>)]
        user: ActorId,
    },
    /// Takes the admin right from a user. The last active admin stays.
    Revoke {
        /// The id of the user.
        #[arg(value_parser = id::<Actor>)]
        user: ActorId,
    },
}

/// Runs a user command.
pub(super) async fn user(config: &Config, command: UserCommand) -> Result<(), Error> {
    let pool = db::connect(config).await?;
    match command {
        UserCommand::Create {
            email,
            name,
            handle,
            password_stdin,
            admin,
        } => {
            // Read the values, then create the user and print its id.
            let new = NewUser {
                email: parse::field(email, "email")?,
                email_verified: true,
                display_name: parse::field(name, "name")?,
                handle: handle
                    .map(|handle| parse::field(handle, "handle"))
                    .transpose()?,
                password: match password_stdin {
                    true => Some(parse::field(stdin_line()?, "password")?),
                    false => None,
                },
            };
            let user = match admin {
                true => accounts::create_admin(&pool, new).await?,
                false => accounts::create_user(&pool, new).await?,
            };
            println!("{}", user.id);
        }
        UserCommand::Admin(AdminCommand::Grant { user }) => {
            admins::grant(&pool, user, None).await?
        }
        UserCommand::Admin(AdminCommand::Revoke { user }) => {
            admins::revoke(&pool, user, None).await?
        }
    }
    pool.close().await;
    Ok(())
}
