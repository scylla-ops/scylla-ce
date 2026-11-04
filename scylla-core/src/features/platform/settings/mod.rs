//! The settings feature: the settings of the instance, `SettingsService`, for its admins.

mod grpc;
mod service;
mod store;

pub(crate) use grpc::server;
pub(crate) use service::personal_tokens_enabled;
