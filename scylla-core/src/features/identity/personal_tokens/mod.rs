//! The personal tokens feature: the API tokens of the users, `PersonalTokenService`, and their
//! exchange for a short token of the user.

mod grpc;
mod service;

pub(crate) use grpc::server;
pub(crate) use service::exchange;
