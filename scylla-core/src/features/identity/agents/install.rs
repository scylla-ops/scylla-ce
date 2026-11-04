//! The install scripts of the agent: `GET /install.sh` for Linux and macOS, and
//! `GET /install.ps1` for Windows. Each script has the URL of this server, its version, and the
//! folder of the downloads.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

/// The script for Linux and macOS, with its marks.
const SH: &str = include_str!("install.sh");

/// The script for Windows, with its marks.
const PS1: &str = include_str!("install.ps1");

/// The two scripts, with the values of this server.
struct Scripts {
    /// The script for Linux and macOS.
    sh: String,
    /// The script for Windows.
    ps1: String,
}

/// Makes the routes of the two scripts.
pub(crate) fn router(api_url: &str, downloads: &str) -> Router {
    let scripts = Scripts {
        sh: fill(SH, api_url, downloads, sh_quote),
        ps1: fill(PS1, api_url, downloads, ps1_quote),
    };
    Router::new()
        .route("/install.sh", get(sh))
        .route("/install.ps1", get(ps1))
        .with_state(Arc::new(scripts))
}

/// Serves the script for Linux and macOS.
async fn sh(State(scripts): State<Arc<Scripts>>) -> impl IntoResponse {
    script("text/x-shellscript; charset=utf-8", scripts.sh.clone())
}

/// Serves the script for Windows.
async fn ps1(State(scripts): State<Arc<Scripts>>) -> impl IntoResponse {
    script("text/plain; charset=utf-8", scripts.ps1.clone())
}

/// Makes the response of a script, which a cache must not keep.
fn script(content_type: &'static str, body: String) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-store")],
        body,
    )
}

/// Puts the values of this server in the marks of a script, each in a quoted text of its
/// language.
fn fill(template: &str, api_url: &str, downloads: &str, quote: fn(&str) -> String) -> String {
    template
        .replace("'@SCYLLA_URL@'", &quote(api_url))
        .replace("'@SCYLLA_VERSION@'", &quote(env!("CARGO_PKG_VERSION")))
        .replace("'@DOWNLOADS_URL@'", &quote(downloads))
}

/// Quotes a text for `sh`: in single quotes, where a single quote is `'\''`.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Quotes a text for PowerShell: in single quotes, where a single quote is doubled.
fn ps1_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_in_a_value_stays_in_its_text() {
        let template = "URL='@SCYLLA_URL@'";

        let sh = fill(template, "http://a'b", "d", sh_quote);
        let ps1 = fill(template, "http://a'b", "d", ps1_quote);

        assert_eq!(sh, r"URL='http://a'\''b'");
        assert_eq!(ps1, "URL='http://a''b'");
    }
}
