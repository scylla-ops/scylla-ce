//! The tokens feature: `TokenService` exchanges a proof of identity for a short-lived token.

mod grpc;

pub(crate) use grpc::server;
