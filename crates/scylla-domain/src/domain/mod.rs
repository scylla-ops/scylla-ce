// Adding a variant must fail the build wherever it is not handled.
#![deny(clippy::wildcard_enum_match_arm)]

pub mod clock;
pub mod errors;
pub mod ids;
pub mod text;

pub mod agent;
pub mod app;
pub mod caller;
pub mod job;
pub mod organization;
pub mod permission;
pub mod pipeline;
pub mod project;
pub mod role;
pub mod secret;
pub mod session;
pub mod trigger;
pub mod user;
