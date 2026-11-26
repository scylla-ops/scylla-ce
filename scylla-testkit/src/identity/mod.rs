//! The `TestApp` helpers of the identity RPCs: the accounts, the profile with its avatar, the
//! instance admins, the identity providers, the Apps with their stream, their icon and their API
//! tokens, the personal tokens and the settings of the instance, the job stats of the agents, and
//! the state of the web app.

mod accounts;
mod admins;
mod agents;
mod app_tokens;
mod apps;
mod icons;
mod personal_tokens;
mod profile;
mod providers;
mod user_state;

pub use apps::{new_key, TestKey};
