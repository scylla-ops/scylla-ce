//! The tests of identity: accounts, a new link to verify an email, handles, the profile and its
//! avatar, the mails and their outbox, the deletion of an account, the instance admins and their
//! console, sessions, sign-in with an identity provider and the import of its photo, tokens, the
//! Apps, their labels, their icons, their creators, their keys, their API tokens with their limits
//! and their CI identities, the personal tokens and their switches, the streams of the agents, the
//! samples of their machines and their install scripts, the state of a user, the export of its
//! data, its acceptances of the terms with their proof after a deletion, and the hooks of a user.

pub(crate) mod accounts;
mod admins;
mod agent_metrics;
mod agent_protocol;
mod agent_samples;
mod agents;
mod app_creators;
mod app_icons;
mod app_labels;
mod app_tokens;
mod apps;
mod avatars;
mod browser;
mod ci_identities;
mod console;
mod data_export;
mod deletion;
mod disable;
mod glyph_avatars;
mod handles;
mod idp;
mod install;
mod keys;
mod mail_outbox;
pub(crate) mod mails;
mod oidc;
mod personal_tokens;
mod profile;
mod provider_pictures;
mod sessions;
mod sign_in;
mod sign_in_errors;
mod sign_in_link;
mod sign_in_link_refusals;
mod sign_in_mails;
mod terms;
mod terms_proof;
mod token_limits;
mod tokens;
mod user_hooks;
mod user_state;
mod user_state_limits;
mod verification;
