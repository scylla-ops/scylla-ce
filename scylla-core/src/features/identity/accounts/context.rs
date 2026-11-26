//! The settings of the accounts, and what the account routes share.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::authz::Schema;
use scylla_extension::{Gate, Grants, Hooks};
use sqlx::PgPool;

use crate::infra::mail::{self, InstanceMailer};
use crate::infra::secret::Sealer;
use crate::{Config, SignUpMode};

/// The settings of the accounts and of the sessions.
#[derive(Clone, Debug)]
pub struct AccountSettings {
    /// The public URL of this server, with no trailing slash.
    pub api_url: String,
    /// The public URL of the web app, with no trailing slash.
    pub app_url: String,
    /// Who can create an account.
    pub sign_up_mode: SignUpMode,
    /// The lifetime of a session.
    pub session_ttl: Duration,
    /// The web origins of the app: a cookie refresh needs one, and a sign-in returns to one.
    pub web_origins: Vec<String>,
    /// `true` if the IP address of a sign-in comes from the last `X-Forwarded-For` address.
    pub trust_forwarded: bool,
    /// The email address that the mails of the default mailer come from. The sign-in page shows
    /// it when the mail settings of the instance have no relay.
    pub mail_sender: String,
}

impl Default for AccountSettings {
    /// The settings of a local server: the web app on `http://127.0.0.1:3000`, the sign-up of an
    /// invitation only, and sessions of 30 days.
    fn default() -> Self {
        let local = "http://127.0.0.1:3000".to_owned();
        Self {
            api_url: local.clone(),
            app_url: local,
            sign_up_mode: SignUpMode::InviteOnly,
            session_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            web_origins: Vec::new(),
            trust_forwarded: false,
            mail_sender: "no-reply@localhost".to_owned(),
        }
    }
}

impl AccountSettings {
    /// Reads the settings of the accounts.
    pub fn from_config(config: &Config) -> Self {
        Self {
            api_url: config.api_url.trim_end_matches('/').to_owned(),
            app_url: config.app_url.trim_end_matches('/').to_owned(),
            sign_up_mode: config.sign_up_mode,
            session_ttl: config.session_ttl,
            web_origins: config.cors_origins.clone(),
            trust_forwarded: config.trust_forwarded_headers,
            mail_sender: mail::address_of(&config.mail_from),
        }
    }

    /// Returns `true` if the cookies need HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.api_url.starts_with("https://")
    }

    /// Makes a link of the web app with a one-time token in the fragment, so the token never
    /// goes to a server log or a `Referer` header: `<app>/<page>#token=<token>`.
    pub fn link(&self, page: &str, token: &str) -> String {
        format!("{}/{page}#token={token}", self.app_url)
    }

    /// Makes a link to a page of the web app, with no token: `<app>/<page>`.
    pub fn page(&self, page: &str) -> String {
        format!("{}/{page}", self.app_url)
    }

    /// Returns the address of the web app with no scheme, which names the instance in a mail:
    /// `scylla.example.com`.
    pub fn instance(&self) -> &str {
        self.app_url
            .split_once("://")
            .map_or(self.app_url.as_str(), |(_, rest)| rest)
    }

    /// Returns `true` if a request from this origin comes from the web app: the origin of
    /// `app_url`, which serves the app when the server holds it, or one of `web_origins`.
    pub fn is_web_origin(&self, origin: &str) -> bool {
        origin_of(&self.app_url) == origin
            || self.web_origins.iter().any(|allowed| allowed == origin)
    }

    /// Returns the root of the web app of an address: the origin of `web_origins` that has it,
    /// or `app_url`. It never returns text of the address.
    pub fn web_root_of(&self, url: &str) -> &str {
        self.web_origins
            .iter()
            .find(|origin| url == *origin || url.starts_with(&format!("{origin}/")))
            .map_or(&self.app_url, String::as_str)
    }

    /// Returns `true` if a browser can go back to this address after a sign-in: the web app,
    /// or one of its origins.
    pub fn can_return_to(&self, url: &str) -> bool {
        let allowed = std::iter::once(&self.app_url).chain(&self.web_origins);
        allowed
            .into_iter()
            .any(|origin| url == origin || url.starts_with(&format!("{origin}/")))
    }
}

/// Returns the origin of a URL: its scheme, its host and its port, with no path.
fn origin_of(url: &str) -> &str {
    let start = url.find("://").map_or(0, |index| index + 3);
    url[start..]
        .find('/')
        .map_or(url, |end| &url[..start + end])
}

/// What the account routes share.
#[derive(Clone)]
pub struct AccountContext {
    /// The permission model: what the roles of the schema give.
    pub schema: Schema,
    /// The database pool.
    pub pool: PgPool,
    /// The access gate, which signs the tokens and writes the roles.
    pub gate: Arc<dyn Gate>,
    /// The mailer of the instance: the relay of the mail settings, else the default mailer.
    pub mailer: Arc<InstanceMailer>,
    /// The settings.
    pub settings: Arc<AccountSettings>,
    /// The key of the secrets, if the settings give one.
    pub sealer: Option<Arc<Sealer>>,
    /// The roles that an invitation can give.
    pub grants: Arc<Grants>,
    /// The hooks of the edition, for the invitations and the memberships.
    pub hooks: Arc<Hooks>,
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    #[test]
    fn the_origin_of_a_url_has_no_path() {
        assert_eq!(
            origin_of("https://scylla.example/app/"),
            "https://scylla.example"
        );
        assert_eq!(origin_of("http://127.0.0.1:3000"), "http://127.0.0.1:3000");
        assert_eq!(origin_of("http://127.0.0.1:3000/"), "http://127.0.0.1:3000");
    }
}
