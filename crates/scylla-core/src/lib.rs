/// `no_inline`: rustdoc would otherwise copy the whole model into this crate's docs.
#[doc(no_inline)]
pub use scylla_domain::domain;

pub mod application;
pub mod infrastructure;

pub mod bootstrap;
pub mod config;
pub mod error;
pub mod rest;
pub mod tls;

pub mod grpc;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_support;

pub use grpc::client::session_client;
pub use grpc::middleware::{caller_session, extract_auth_context};

pub use config::{BootstrapConfig, ControlPlaneConfig, CorsConfig, ServerConfig, UiConfig};
pub use error::{BootstrapError, ConfigError, StartupError};
