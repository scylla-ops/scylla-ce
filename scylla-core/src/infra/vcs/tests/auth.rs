//! Unit tests of the auth of a GitHub App: the claims and the expiry of its JWT, and the cache
//! and the scope of its installation tokens.

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::json;
use time::{Duration, OffsetDateTime};

use super::key::{PRIVATE_KEY, PUBLIC_KEY};
use crate::infra::vcs::{
    AppClaims, GitHubAppSettings, Hidden, InstallationToken, TokenCache, TokenScope, VcsClients,
    VcsOptions, REFRESH_MARGIN,
};

/// Makes the settings of a test App.
fn settings() -> GitHubAppSettings {
    GitHubAppSettings {
        app_id: 42,
        slug: "scylla-test".to_owned(),
        client_id: "Iv1.test".to_owned(),
        client_secret: Hidden::new("client-secret"),
        private_key: Hidden::new(PRIVATE_KEY),
        webhook_secret: Hidden::new("webhook-secret"),
    }
}

/// Returns a token that expires at a time.
fn token_until(expires_at: OffsetDateTime) -> InstallationToken {
    InstallationToken {
        token: Hidden::new("ghs_test"),
        expires_at,
    }
}

/// The JWT is RS256, signed with the key of the App, with the client id as issuer, a time of
/// issue one minute before now and an expiry nine minutes after now.
#[test]
fn jwt_has_the_claims_and_the_expiry_of_github() {
    // given
    crate::infra::tls::install();
    let clients = VcsClients::new(&VcsOptions::default()).expect("clients");
    let app = clients.app(&settings()).expect("app");
    let now = OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time");

    // when
    let jwt = app.jwt(now).expect("jwt");

    // then
    let key = DecodingKey::from_rsa_pem(PUBLIC_KEY.as_bytes()).expect("public key");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.set_required_spec_claims(&["iat", "exp", "iss"]);
    let claims = jsonwebtoken::decode::<AppClaims>(jwt.expose(), &key, &validation)
        .expect("a valid signature")
        .claims;
    assert_eq!(
        claims,
        AppClaims {
            iat: 1_800_000_000 - 60,
            exp: 1_800_000_000 + 9 * 60,
            iss: "Iv1.test".to_owned(),
        }
    );
    assert!(claims.exp - claims.iat <= 10 * 60);
}

/// The settings of an App with a key that is not RSA make no App.
#[test]
fn app_refuses_a_key_that_is_not_rsa() {
    // given
    crate::infra::tls::install();
    let clients = VcsClients::new(&VcsOptions::default()).expect("clients");
    let mut settings = settings();
    settings.private_key = Hidden::new("-----BEGIN RSA PRIVATE KEY-----\nAAAA\n");

    // when
    let app = clients.app(&settings);

    // then
    assert!(app.is_err());
}

/// The cache gives a token again until 5 minutes before its expiry.
#[test]
fn token_cache_reuses_a_token_until_the_margin() {
    // given
    let cache = TokenCache::default();
    let now = OffsetDateTime::now_utc();
    let expires_at = now + Duration::hours(1);
    cache.put(7, TokenScope::default(), token_until(expires_at), now);

    // when
    let early = cache.get(7, &TokenScope::default(), now + Duration::minutes(54));
    let late = cache.get(7, &TokenScope::default(), expires_at - REFRESH_MARGIN);

    // then
    assert_eq!(early.expect("a cached token").token.expose(), "ghs_test");
    assert!(late.is_none());
}

/// A token serves only its installation and its scope: other repositories or permissions need
/// another token.
#[test]
fn token_cache_keeps_one_token_for_each_scope() {
    // given
    let cache = TokenCache::default();
    let now = OffsetDateTime::now_utc();
    let narrow = TokenScope::default()
        .repository(1)
        .permission("contents", "read");
    cache.put(
        7,
        narrow.clone(),
        token_until(now + Duration::hours(1)),
        now,
    );

    // when
    let same = cache.get(7, &narrow.clone().permission("contents", "read"), now);
    let wider = cache.get(7, &narrow.clone().repository(2), now);
    let all = cache.get(7, &TokenScope::default(), now);
    let other = cache.get(8, &narrow, now);

    // then
    assert!(same.is_some());
    assert!(wider.is_none());
    assert!(all.is_none());
    assert!(other.is_none());
}

/// After `forget`, an installation has no cached token.
#[test]
fn token_cache_forgets_an_installation() {
    // given
    let cache = TokenCache::default();
    let now = OffsetDateTime::now_utc();
    cache.put(
        7,
        TokenScope::default(),
        token_until(now + Duration::hours(1)),
        now,
    );
    cache.put(
        8,
        TokenScope::default(),
        token_until(now + Duration::hours(1)),
        now,
    );

    // when
    cache.forget(7);

    // then
    assert!(cache.get(7, &TokenScope::default(), now).is_none());
    assert!(cache.get(8, &TokenScope::default(), now).is_some());
}

/// The body of a token request narrows to the ids of the repositories and to the permissions,
/// and an empty scope asks for no narrowing.
#[test]
fn token_scope_body_narrows_the_token() {
    // given
    let scope = TokenScope::default()
        .repository(9)
        .repository(3)
        .permission("statuses", "write")
        .permission("contents", "read");

    // when
    let body = scope.body();

    // then
    assert_eq!(
        body,
        json!({
            "repository_ids": [3, 9],
            "permissions": { "contents": "read", "statuses": "write" },
        })
    );
    assert_eq!(TokenScope::default().body(), json!({}));
}

/// `Debug` of the settings of an App and of a token never shows a secret.
#[test]
fn debug_hides_the_secrets() {
    // given
    let settings = settings();
    let token = token_until(OffsetDateTime::now_utc());

    // when
    let shown = format!("{settings:?} {token:?}");

    // then
    assert!(!shown.contains("client-secret"));
    assert!(!shown.contains("webhook-secret"));
    assert!(!shown.contains("PRIVATE KEY"));
    assert!(!shown.contains("ghs_test"));
    assert!(shown.contains("scylla-test"));
}
