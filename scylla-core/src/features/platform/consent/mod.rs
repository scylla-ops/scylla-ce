//! The consent feature: the record of each choice of a browser on the trackers that need consent,
//! `ConsentService`. Anyone records a choice, and a record of a signed-in user names the user.

mod convert;
mod grpc;
mod rates;
mod service;
mod store;

pub(crate) use grpc::server;
