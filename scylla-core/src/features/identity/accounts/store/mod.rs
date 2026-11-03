//! The SQL of the accounts: the users, their emails and their passwords. Each function runs in
//! the transaction of the caller.

mod emails;
mod passwords;
mod users;

pub(crate) use emails::{
    email_in_use, insert_email, replace_email, user_of_email, verified_emails, verify_email,
};
pub(crate) use passwords::{credentials, password_hash, record_failure, set_password};
pub(crate) use users::{
    delete_user, find_user, insert_user, list_users, lock_user, record_sign_in, set_status,
    update_profile,
};
