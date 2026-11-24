//! The SQL of the Apps: the `apps`, `app_keys`, `app_join_codes` and `ci_identities` tables,
//! and the assertion nonces. Each function runs in the transaction of the caller.

pub(crate) mod apps;
pub(crate) mod ci;
pub(crate) mod codes;
pub(crate) mod keys;
pub(crate) mod nonces;
