//! The security headers of the web app: a content security policy that lets a page load only from
//! its own origin and from the hosts that the app needs, and a referrer policy.

use axum::extract::{Request, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine;
use sha2::{Digest, Sha256};
use url::Url;

use crate::infra::vcs::VcsOptions;

/// The referrer policy: another origin gets only the origin of the page, and nothing with HTTP.
const REFERRER: &str = "strict-origin-when-cross-origin";

/// The host of the avatars of github.com.
const GITHUB_AVATARS: &str = "https://avatars.githubusercontent.com";

/// The host of the Gravatar images that GitLab gives to an account with no avatar.
const GRAVATAR: &str = "https://secure.gravatar.com";

/// The security headers of the answers of the web app.
#[derive(Clone)]
pub(super) struct Headers {
    /// The value of `Content-Security-Policy`.
    policy: HeaderValue,
}

impl Headers {
    /// Makes the headers for the page `index` of the web app, the public URL of the API and the
    /// Git providers of the settings.
    pub(super) fn new(index: &[u8], api_url: &str, vcs: &VcsOptions) -> Self {
        let text = policy(&String::from_utf8_lossy(index), api_url, vcs);
        let policy = HeaderValue::from_str(&text)
            .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'"));
        Self { policy }
    }
}

/// Adds the security headers to an answer of the web app.
pub(super) async fn add(State(headers): State<Headers>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let map = response.headers_mut();
    map.insert(CONTENT_SECURITY_POLICY, headers.policy);
    map.insert(REFERRER_POLICY, HeaderValue::from_static(REFERRER));
    response
}

/// Makes the policy: the own origin, the inline scripts of the page, the API (its calls, and the
/// uploaded images of its route `/images/{id}`), and the hosts of the avatars, which the app
/// shows and which a capture of the page reads.
fn policy(index: &str, api_url: &str, vcs: &VcsOptions) -> String {
    let scripts = inline_scripts(index);
    let images = image_hosts(vcs);
    let api: Vec<String> = origin(api_url).into_iter().collect();
    let github: Vec<String> = origin(&vcs.github_web_url).into_iter().collect();
    [
        "default-src 'self'".to_owned(),
        format!("script-src 'self'{}", list(&scripts)),
        "style-src 'self' 'unsafe-inline'".to_owned(),
        format!("img-src 'self' data: blob:{}{}", list(&api), list(&images)),
        "font-src 'self' data:".to_owned(),
        format!("connect-src 'self'{}{}", list(&api), list(&images)),
        format!("form-action 'self'{}", list(&github)),
        "frame-ancestors 'self'".to_owned(),
        "base-uri 'self'".to_owned(),
        "object-src 'none'".to_owned(),
    ]
    .join("; ")
}

/// Returns the source `'sha256-<hash>'` of each inline script of a page: a `<script>` with no
/// `src`.
fn inline_scripts(page: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut rest = page;
    while let Some(start) = rest.find("<script") {
        // Read the open tag and the body of the script.
        let tag_and_body = &rest[start..];
        let Some(tag_end) = tag_and_body.find('>') else {
            break;
        };
        let body_and_rest = &tag_and_body[tag_end + 1..];
        let Some(body_end) = body_and_rest.find("</script>") else {
            break;
        };

        // Keep the hash of an inline script, then go to the end of the script.
        if !tag_and_body[..tag_end].contains("src=") {
            let hash = Sha256::digest(&body_and_rest.as_bytes()[..body_end]);
            let hash = base64::engine::general_purpose::STANDARD.encode(hash);
            sources.push(format!("'sha256-{hash}'"));
        }
        rest = &body_and_rest[body_end..];
    }
    sources
}

/// Returns the hosts of the avatars that the web app shows: GitHub, the GitLab of the settings,
/// and Gravatar.
fn image_hosts(vcs: &VcsOptions) -> Vec<String> {
    let mut hosts = github_avatars(&vcs.github_web_url);
    hosts.extend(origin(&vcs.gitlab_url));
    hosts.push(GRAVATAR.to_owned());
    hosts
}

/// Returns the hosts of the avatars of a GitHub: the avatar host of github.com, or the server of
/// a GitHub Enterprise and its subdomain `avatars`.
fn github_avatars(web_url: &str) -> Vec<String> {
    let Ok(url) = Url::parse(web_url) else {
        return vec![GITHUB_AVATARS.to_owned()];
    };
    match url.host_str() {
        Some("github.com") | None => vec![GITHUB_AVATARS.to_owned()],
        Some(host) => {
            let port = url
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default();
            let avatars = format!("{}://avatars.{host}{port}", url.scheme());
            vec![url.origin().ascii_serialization(), avatars]
        }
    }
}

/// Returns the origin of a URL: its scheme, its host and its port.
fn origin(url: &str) -> Option<String> {
    let origin = Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Writes each source after a space.
fn list(sources: &[String]) -> String {
    sources.iter().map(|source| format!(" {source}")).collect()
}

#[cfg(test)]
mod tests {
    //! Unit tests of the content security policy.

    use super::{policy, VcsOptions};

    /// The directive of a policy, with its sources.
    fn directive<'a>(policy: &'a str, name: &str) -> &'a str {
        policy
            .split("; ")
            .find(|directive| directive.starts_with(&format!("{name} ")))
            .unwrap_or_default()
    }

    /// An inline script runs by its hash, and a script file only from the own origin.
    #[test]
    fn policy_allows_the_inline_scripts_of_the_page_by_their_hash() {
        // given
        let page = "<html><head><script>theme()</script>\
                    <script type=\"module\" crossorigin src=\"/assets/app.js\"></script>";

        // when
        let policy = policy(page, "https://scylla.example.com", &VcsOptions::default());

        // then
        let hash = "'sha256-Rfr3GgHZFiXm+TmNr+li+0sZ4J9FGllo8jsXcJF5XZ8='";
        assert_eq!(
            directive(&policy, "script-src"),
            format!("script-src 'self' {hash}")
        );
        assert!(policy.starts_with("default-src 'self'; "));
        assert!(policy.contains("frame-ancestors 'self'"));
        assert!(policy.contains("object-src 'none'"));
    }

    /// The images come from the own origin, from the API and from the hosts of the avatars of
    /// github.com, gitlab.com and Gravatar. The calls go to the API and to these hosts.
    #[test]
    fn policy_allows_the_hosts_of_the_avatars_and_the_api() {
        // given
        let vcs = VcsOptions::default();

        // when
        let policy = policy("", "https://api.example.com/", &vcs);

        // then
        let hosts = "https://avatars.githubusercontent.com https://gitlab.com \
                     https://secure.gravatar.com";
        assert_eq!(
            directive(&policy, "img-src"),
            format!("img-src 'self' data: blob: https://api.example.com {hosts}")
        );
        assert_eq!(
            directive(&policy, "connect-src"),
            format!("connect-src 'self' https://api.example.com {hosts}")
        );
        assert_eq!(
            directive(&policy, "form-action"),
            "form-action 'self' https://github.com"
        );
        assert_eq!(directive(&policy, "script-src"), "script-src 'self'");
    }

    /// A GitHub Enterprise and a GitLab of the settings give the hosts of their avatars.
    #[test]
    fn policy_takes_the_git_providers_of_the_settings() {
        // given
        let vcs = VcsOptions {
            github_web_url: "https://github.corp.example".to_owned(),
            gitlab_url: "https://gitlab.corp.example:8443".to_owned(),
            ..VcsOptions::default()
        };

        // when
        let policy = policy("", "https://scylla.corp.example", &vcs);

        // then
        let images = directive(&policy, "img-src");
        assert!(images.contains(" https://github.corp.example "), "{images}");
        assert!(images.contains(" https://avatars.github.corp.example "));
        assert!(images.contains(" https://gitlab.corp.example:8443 "));
        assert!(!images.contains("githubusercontent"));
        assert_eq!(
            directive(&policy, "form-action"),
            "form-action 'self' https://github.corp.example"
        );
    }
}
