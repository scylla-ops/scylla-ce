//! Unit tests of the settings of the Git providers: the URLs, all or none of the settings of the
//! GitHub App, its private key, and `Debug` with no secret.

use clap::{CommandFactory, FromArgMatches, Parser};

use super::key::PRIVATE_KEY;
use crate::infra::vcs::{VcsOptions, VcsSettings, GITHUB_API_URL, GITLAB_URL};
use crate::Error;

/// A command line with the settings of the Git providers only.
#[derive(Parser)]
struct Flags {
    /// The settings.
    #[command(flatten)]
    vcs: VcsSettings,
}

/// Reads the flags with no environment variable, so the environment of the test changes
/// nothing.
fn parse(flags: &[&str]) -> Result<Flags, clap::Error> {
    let command = Flags::command().mut_args(|arg| arg.env(None::<&'static str>));
    let matches =
        command.try_get_matches_from(std::iter::once("scylla").chain(flags.iter().copied()))?;
    Flags::from_arg_matches(&matches)
}

/// Reads the settings from flags.
fn settings(flags: &[&str]) -> VcsSettings {
    parse(flags).expect("valid flags").vcs
}

/// Returns the flags of a full GitHub App, with the key in a value.
fn app_flags(key: &str) -> Vec<String> {
    [
        "--github-app-id=12",
        "--github-app-slug=scylla-dev",
        "--github-app-client-id=Iv1.dev",
        "--github-app-client-secret=client-secret",
        "--github-app-webhook-secret=webhook-secret",
    ]
    .iter()
    .map(|flag| (*flag).to_owned())
    .chain([format!("--github-app-private-key={key}")])
    .collect()
}

/// Returns the name of the setting of a config error.
fn setting_of(result: Result<VcsOptions, Error>) -> &'static str {
    match result {
        Err(Error::Config { name, .. }) => name,
        other => panic!("a config error, not {other:?}"),
    }
}

/// With no flag, the server uses github.com and gitlab.com, with no App.
#[test]
fn resolve_uses_the_public_providers_by_default() {
    // when
    let options = settings(&[]).resolve().expect("options");

    // then
    assert_eq!(options.github_api_url, GITHUB_API_URL);
    assert_eq!(options.gitlab_url, GITLAB_URL);
    assert!(options.github_app.is_none());
}

/// All the settings of the App give the App, and a `\n` in the key is a new line.
#[test]
fn resolve_reads_a_full_app() {
    // given
    let flags = app_flags(&PRIVATE_KEY.replace('\n', "\\n"));
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();

    // when
    let options = settings(&flags).resolve().expect("options");

    // then
    let app = options.github_app.expect("an App");
    assert_eq!((app.app_id, app.slug.as_str()), (12, "scylla-dev"));
    assert_eq!(app.private_key.expose(), PRIVATE_KEY);
    assert!(app.key().is_ok());
}

/// The key can come from a file.
#[test]
fn resolve_reads_the_key_from_a_file() {
    // given
    let path = std::env::temp_dir().join(format!("scylla-app-{}.pem", uuid::Uuid::now_v7()));
    std::fs::write(&path, PRIVATE_KEY).expect("write key");
    let mut flags = app_flags("unused");
    flags.pop();
    flags.push(format!("--github-app-private-key-file={}", path.display()));
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();

    // when
    let options = settings(&flags).resolve();
    std::fs::remove_file(&path).expect("remove key");

    // then
    let app = options.expect("options").github_app.expect("an App");
    assert_eq!(app.private_key.expose(), PRIVATE_KEY);
}

/// A part of the settings of the App is refused, with the name of the first missing setting.
#[test]
fn resolve_refuses_a_part_of_the_app() {
    // given
    let only_id = settings(&["--github-app-id=12"]);
    let no_secret = {
        let flags = app_flags(PRIVATE_KEY);
        let flags: Vec<&str> = flags
            .iter()
            .map(String::as_str)
            .filter(|flag| !flag.starts_with("--github-app-webhook-secret"))
            .collect();
        settings(&flags)
    };

    // when
    let only_id = only_id.resolve();
    let no_secret = no_secret.resolve();

    // then
    assert_eq!(setting_of(only_id), "SCYLLA_GITHUB_APP_SLUG");
    assert_eq!(setting_of(no_secret), "SCYLLA_GITHUB_APP_WEBHOOK_SECRET");
}

/// A key that is not an RSA key, or a file that cannot be read, is refused.
#[test]
fn resolve_refuses_a_bad_key() {
    // given
    let flags = app_flags("not a key");
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
    let mut missing = app_flags("unused");
    missing.pop();
    missing.push("--github-app-private-key-file=/no/such/scylla/key.pem".to_owned());
    let missing: Vec<&str> = missing.iter().map(String::as_str).collect();

    // when
    let bad = settings(&flags).resolve();
    let unreadable = settings(&missing).resolve();

    // then
    assert_eq!(setting_of(bad), "SCYLLA_GITHUB_APP_PRIVATE_KEY");
    assert_eq!(setting_of(unreadable), "SCYLLA_GITHUB_APP_PRIVATE_KEY_FILE");
}

/// A URL that is not http or https is refused, and a `/` at its end goes away.
#[test]
fn resolve_checks_the_urls() {
    // given
    let bad = settings(&["--gitlab-url=gitlab.example"]);
    let slash = settings(&["--github-api-url=http://127.0.0.1:9000/api/"]);

    // when
    let bad = bad.resolve();
    let slash = slash.resolve().expect("options");

    // then
    assert_eq!(setting_of(bad), "SCYLLA_GITLAB_URL");
    assert_eq!(slash.github_api_url, "http://127.0.0.1:9000/api");
}

/// A key in a value and a key in a file at the same time are refused.
#[test]
fn flags_refuse_two_keys() {
    // given
    let flags = [
        "--github-app-private-key=a",
        "--github-app-private-key-file=b",
    ];

    // when
    let parsed = parse(&flags);

    // then
    assert!(parsed.is_err());
}

/// `Debug` of the settings and of the options never shows a secret.
#[test]
fn debug_hides_the_secrets_of_the_settings() {
    // given
    let flags = app_flags(&PRIVATE_KEY.replace('\n', "\\n"));
    let flags: Vec<&str> = flags.iter().map(String::as_str).collect();
    let settings = settings(&flags);
    let options = settings.resolve().expect("options");

    // when
    let shown = format!("{settings:?} {options:?}");

    // then
    assert!(!shown.contains("client-secret"));
    assert!(!shown.contains("webhook-secret"));
    assert!(!shown.contains("PRIVATE KEY"));
    assert!(shown.contains("scylla-dev"));
}
