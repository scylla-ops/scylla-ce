//! The tests of identity: accounts, handles, the profile, the mails, the deletion of an
//! account, the instance admins and their console, sessions, sign-in with an identity provider,
//! tokens, the Apps, their creators, their keys and their API tokens with their limits, the
//! streams of the agents and their install scripts.

pub(crate) mod accounts;
mod admins;
mod agents;
mod app_creators;
mod app_tokens;
mod apps;
mod browser;
mod console;
mod deletion;
mod disable;
mod handles;
mod idp;
mod install;
mod keys;
mod mails;
mod oidc;
mod profile;
mod sessions;
mod sign_in;
mod token_limits;
mod tokens;
