//! The secrets feature: the `secrets` table of the projects and `SecretService`.

mod grpc;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use grpc::server;
pub(crate) use service::{missing, reveal};
