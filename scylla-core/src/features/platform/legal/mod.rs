//! The legal feature: the legal documents of the instance, its operator and its privacy contact,
//! `LegalService`. Anyone reads them, and the instance admins change them.

mod convert;
mod grpc;
mod service;
mod store;

pub(crate) use grpc::server;
pub(crate) use service::{read, version};
