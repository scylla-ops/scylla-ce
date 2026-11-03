//! The settings of the server, from command line flags or environment variables.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args;

use crate::infra::logs::LogFormat;

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

    /// The Ed25519 private key in hex that signs the tokens. Make one with `token keygen`.
    #[arg(long, env = "SCYLLA_TOKEN_PRIVATE_KEY", hide_env_values = true)]
    pub token_private_key: Option<String>,

    /// The id of the signing key. Change it at each key rotation.
    #[arg(long, env = "SCYLLA_TOKEN_KEY_ID", default_value_t = 1)]
    pub token_key_id: u32,

    /// Older public keys that still verify tokens, as `<id>:<hex>`, separated by commas.
    #[arg(long, env = "SCYLLA_TOKEN_PUBLIC_KEYS", value_delimiter = ',')]
    pub token_public_keys: Vec<String>,

    /// The lifetime of a token, in seconds.
    #[arg(long, env = "SCYLLA_TOKEN_TTL_SECS", default_value = "600", value_parser = seconds)]
    pub token_ttl: Duration,

    /// The gRPC endpoint of SpiceDB.
    #[arg(
        long,
        env = "SCYLLA_SPICEDB_ENDPOINT",
        default_value = "http://127.0.0.1:55051"
    )]
    pub spicedb_endpoint: String,

    /// The preshared key of SpiceDB.
    #[arg(long, env = "SCYLLA_SPICEDB_KEY", hide_env_values = true)]
    pub spicedb_key: Option<String>,

    /// The public URL of this server, for the redirect URIs of the identity providers.
    #[arg(long, env = "SCYLLA_API_URL", default_value = "http://127.0.0.1:3000")]
    pub api_url: String,

    /// The public URL of the web app, for the links in the mails and after a sign-in.
    #[arg(long, env = "SCYLLA_APP_URL", default_value = "http://127.0.0.1:3000")]
    pub app_url: String,

    /// Who can create an account.
    #[arg(long, env = "SCYLLA_SIGN_UP_MODE", value_enum, default_value_t = SignUpMode::InviteOnly)]
    pub sign_up_mode: SignUpMode,

    /// The lifetime of a session, in days.
    #[arg(long, env = "SCYLLA_SESSION_TTL_DAYS", default_value = "30", value_parser = days)]
    pub session_ttl: Duration,

    /// The time that the server keeps the logs of the jobs, in days.
    #[arg(long, env = "SCYLLA_LOG_RETENTION_DAYS", default_value = "30", value_parser = days)]
    pub log_retention: Duration,

    /// The SMTP server of the mails: `smtp://host:port`, or `smtps://user:password@host` for
    /// TLS. Without it, the server sends no mail and logs each one with no link.
    #[arg(long, env = "SCYLLA_SMTP_URL", hide_env_values = true)]
    pub smtp_url: Option<String>,

    /// The sender of the mails.
    #[arg(
        long,
        env = "SCYLLA_MAIL_FROM",
        default_value = "Scylla <no-reply@localhost>"
    )]
    pub mail_from: String,

    /// The key in hex that encrypts the client secrets of the identity providers and the secrets
    /// of the projects. With no key, a new secret of a project is refused. Make one with
    /// `secret keygen`.
    #[arg(long, env = "SCYLLA_SECRET_KEY", hide_env_values = true)]
    pub secret_key: Option<String>,

    /// The web origins that can call the API with gRPC-Web, separated by commas.
    #[arg(long, env = "SCYLLA_CORS_ORIGINS", value_delimiter = ',')]
    pub cors_origins: Vec<String>,

    /// Reads the client IP address of a sign-in from the last `X-Forwarded-For` address, which
    /// the proxy adds, in place of the peer address. Set it only behind one proxy that adds it.
    #[arg(long, env = "SCYLLA_TRUST_FORWARDED_HEADERS")]
    pub trust_forwarded_headers: bool,

    /// The PEM certificate chain of the server. With `--tls-key`, the server uses TLS.
    #[arg(long, env = "SCYLLA_TLS_CERT", requires = "tls_key")]
    pub tls_cert: Option<PathBuf>,

    /// The PEM private key of the server.
    #[arg(long, env = "SCYLLA_TLS_KEY", requires = "tls_cert")]
    pub tls_key: Option<PathBuf>,

    /// The format of the logs on stderr: `pretty` for a person, `json` for a machine, or `auto`:
    /// pretty in a terminal, else JSON. `RUST_LOG` sets the filter.
    #[arg(long, env = "SCYLLA_LOG_FORMAT", value_enum, default_value_t = LogFormat::Auto)]
    pub log_format: LogFormat,
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
            .field(
                "token_private_key",
                &self.token_private_key.as_ref().map(|_| "***"),
            )
            .field("token_key_id", &self.token_key_id)
            .field("token_public_keys", &self.token_public_keys)
            .field("token_ttl", &self.token_ttl)
            .field("spicedb_endpoint", &self.spicedb_endpoint)
            .field("spicedb_key", &self.spicedb_key.as_ref().map(|_| "***"))
            .field("api_url", &self.api_url)
            .field("app_url", &self.app_url)
            .field("sign_up_mode", &self.sign_up_mode)
            .field("session_ttl", &self.session_ttl)
            .field("log_retention", &self.log_retention)
            .field("smtp_url", &self.smtp_url.as_deref().map(redact))
            .field("mail_from", &self.mail_from)
            .field("secret_key", &self.secret_key.as_ref().map(|_| "***"))
            .field("cors_origins", &self.cors_origins)
            .field("trust_forwarded_headers", &self.trust_forwarded_headers)
            .field("tls_cert", &self.tls_cert)
            .field("tls_key", &self.tls_key)
            .field("log_format", &self.log_format)
            .finish()
    }
}

impl Config {
    /// Returns a required secret, or an error that names its setting.
    pub fn secret<'a>(
        value: &'a Option<String>,
        name: &'static str,
    ) -> Result<&'a str, crate::Error> {
        value.as_deref().ok_or(crate::Error::Config {
            name,
            reason: "is required".to_owned(),
        })
    }
}

/// Who can create an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum SignUpMode {
    /// Each person can sign up.
    Open,
    /// Only a person with an invitation can sign up.
    InviteOnly,
    /// No person can sign up: the operator creates the users with `user create`.
    Closed,
}

impl SignUpMode {
    /// Returns the name of the mode.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InviteOnly => "invite_only",
            Self::Closed => "closed",
        }
    }
}

/// Reads a duration in days.
fn days(text: &str) -> Result<Duration, String> {
    text.parse::<u64>()
        .map(|days| Duration::from_secs(days * 24 * 60 * 60))
        .map_err(|_| format!("{text:?} is not a number of days"))
}

/// Reads a duration in seconds.
fn seconds(text: &str) -> Result<Duration, String> {
    text.parse()
        .map(Duration::from_secs)
        .map_err(|_| format!("{text:?} is not a number of seconds"))
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
