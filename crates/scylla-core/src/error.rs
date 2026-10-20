use scylla_domain::domain::errors::DomainError;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TOML config: {0}")]
    ParseToml(#[from] toml::de::Error),
}

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("invalid bootstrap username: {0}")]
    InvalidUsername(#[source] DomainError),
    #[error("invalid bootstrap password: {0}")]
    InvalidPassword(#[source] DomainError),
    #[error("invalid bootstrap email: {0}")]
    InvalidEmail(#[source] DomainError),
    #[error("failed to create bootstrap user: {0}")]
    CreateUser(#[source] DomainError),
    #[error("failed to find bootstrap user: {0}")]
    FindUser(#[source] DomainError),
    #[error("the bootstrap email belongs to another account")]
    EmailOfAnotherAccount,
    #[error("the bootstrap username belongs to an account with another email")]
    UsernameWithAnotherEmail,
    #[error("failed to set the bootstrap email: {0}")]
    UpdateEmail(#[source] DomainError),
    #[error("failed to grant admin permissions: {0}")]
    GrantPermission(#[source] DomainError),
}

#[derive(Debug, Error)]
pub enum StartupError {
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    #[error("database initialization: {0}")]
    Database(#[from] DomainError),
    #[error("permission service init: {0}")]
    Permission(String),
    #[error("bootstrap: {0}")]
    Bootstrap(#[from] BootstrapError),
    #[error("gRPC reflection: {0}")]
    Reflection(String),
    #[error("serve: {0}")]
    Serve(#[from] tonic::transport::Error),
    #[error("tls: {0}")]
    Tls(String),
}
