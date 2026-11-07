//! The state of the web app for one user: the `user_state` table and `UserStateService`.

mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::UserState;
