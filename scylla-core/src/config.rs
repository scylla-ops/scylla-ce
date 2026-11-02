//! The settings of the server, from command line flags or environment variables.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args;

/// The core settings. Each flag also reads the environment variable in its doc.
#[derive(Args, Clone)]
#[group(id = "core")]
pub struct Config {
    /// The Postgres URL.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    pub database_url: String,

    /// The address of the gRPC server.
    #[arg(long, env = "SCYLLA_BIND_ADDR", default_value = "127.0.0.1:3000")]
    pub bind_addr: SocketAddr,

    /// The maximum number of database connections.
    #[arg(long, env = "SCYLLA_DB_MAX_CONNECTIONS", default_value_t = 10)]
    pub db_max_connections: u32,

    /// The time to wait for a free connection, in milliseconds.
    #[arg(long, env = "SCYLLA_DB_ACQUIRE_TIMEOUT_MS", default_value = "5000", value_parser = millis)]
    pub db_acquire_timeout: Duration,

    /// The time after which Postgres stops a statement, in milliseconds.
    #[arg(long, env = "SCYLLA_DB_STATEMENT_TIMEOUT_MS", default_value = "10000", value_parser = millis)]
    pub db_statement_timeout: Duration,

    /// The time after which the server stops a request, in milliseconds.
    #[arg(long, env = "SCYLLA_REQUEST_TIMEOUT_MS", default_value = "15000", value_parser = millis)]
    pub request_timeout: Duration,

    /// Trusts the `x-scylla-actor` header as the actor of a request. Not secure: development only.
    #[arg(long, env = "SCYLLA_TRUST_ACTOR_HEADER", default_value_t = false, action = clap::ArgAction::Set)]
    pub trust_actor_header: bool,

    /// The PEM certificate chain of the server. With `--tls-key`, the server uses TLS.
    #[arg(long, env = "SCYLLA_TLS_CERT", requires = "tls_key")]
    pub tls_cert: Option<PathBuf>,

    /// The PEM private key of the server.
    #[arg(long, env = "SCYLLA_TLS_KEY", requires = "tls_cert")]
    pub tls_key: Option<PathBuf>,
}

impl AsRef<Config> for Config {
    fn as_ref(&self) -> &Config {
        self
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("database_url", &redact(&self.database_url))
            .field("bind_addr", &self.bind_addr)
            .field("db_max_connections", &self.db_max_connections)
            .field("db_acquire_timeout", &self.db_acquire_timeout)
            .field("db_statement_timeout", &self.db_statement_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("trust_actor_header", &self.trust_actor_header)
            .field("tls_cert", &self.tls_cert)
            .field("tls_key", &self.tls_key)
            .finish()
    }
}

/// Reads a duration in milliseconds.
fn millis(text: &str) -> Result<Duration, String> {
    text.parse()
        .map(Duration::from_millis)
        .map_err(|_| format!("{text:?} is not a number of milliseconds"))
}

/// Hides the password of a URL, for the logs.
fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let Some((user_info, host)) = rest.split_once('@') else {
        return url.to_owned();
    };
    match user_info.split_once(':') {
        Some((user, _)) => format!("{scheme}://{user}:***@{host}"),
        None => url.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the config helpers.

    use super::redact;

    /// The log never shows the password of the database URL.
    #[test]
    fn redact_hides_the_password() {
        // given
        let url = "postgres://scylla:secret@localhost:55432/scylla";

        // when
        let shown = redact(url);

        // then
        assert_eq!(shown, "postgres://scylla:***@localhost:55432/scylla");
    }

    /// A URL with no password stays the same.
    #[test]
    fn redact_keeps_a_url_without_password() {
        // given
        let url = "postgres://localhost/scylla";

        // when
        let shown = redact(url);

        // then
        assert_eq!(shown, url);
    }
}
