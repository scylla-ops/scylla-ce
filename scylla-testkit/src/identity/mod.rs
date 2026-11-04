//! The `TestApp` helpers of the identity RPCs: the accounts, the profile, the instance admins,
//! the identity providers, the Apps with their stream and their API tokens, and the job stats of
//! the agents.

mod accounts;
mod admins;
mod agents;
mod app_tokens;
mod apps;
mod profile;
mod providers;

pub use apps::{new_key, TestKey};
