//! The `TestApp` helpers of the identity RPCs: the accounts, the profile, the instance admins,
//! the identity providers, and the Apps with their stream.

mod accounts;
mod admins;
mod apps;
mod profile;
mod providers;

pub use apps::{new_key, TestKey};
