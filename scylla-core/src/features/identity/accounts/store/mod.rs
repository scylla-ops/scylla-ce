//! The SQL of the accounts: the users, their emails and their passwords. Each function runs in
//! the transaction of the caller.

mod emails;
mod passwords;
mod users;

pub(crate) use emails::{
    account_of_email, email_in_use, insert_email, replace_email, user_of_email, verified_emails,
    verify_email, EmailAccount,
};
pub(crate) use passwords::{credentials, password_hash, record_failure, set_password, Lock};
pub(crate) use users::{
    avatar_image, delete_user, find_user, insert_user, list_users, lock_user, record_sign_in,
    set_avatar_image, set_status, update_profile,
};
