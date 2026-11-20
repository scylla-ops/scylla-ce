//! The tests of the manifest flow of the GitHub App of an instance: the manifest and the page of
//! GitHub, the App with its secrets sealed, and the state of one use.

use std::time::{SystemTime, UNIX_EPOCH};

use scylla_core::infra::vcs::sign_github;
use scylla_domain::integrations::apps::VcsAppId;
use scylla_proto::integrations::v1::VcsProvider;
use scylla_testkit::{text, MANIFEST_CLIENT_SECRET, MANIFEST_WEBHOOK_SECRET, TEST_APP_PRIVATE_KEY};
use serde_json::json;
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, assert_reason, created, manifest_of, state_of, CODE};
use crate::assert_code;

/// A start gives the page of GitHub of the organization, or of the account of the person, with
/// a state of one hour, and the manifest: the URLs of this server, the webhook of the new row,
/// the permissions and the events of the App. A login that is not one of GitHub is refused.
#[sqlx::test(migrations = false)]
async fn start_git_hub_manifest_gives_the_manifest_and_the_page_of_github(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool, false).await;
    let admin = app.admin().await;
    let none = app.get_vcs_app(admin).await.unwrap();

    // when
    let for_org = app.start_github_manifest(admin, "acme").await.unwrap();
    let for_person = app.start_github_manifest(admin, " ").await.unwrap();
    let bad = app.start_github_manifest(admin, "-acme/x").await;

    // then
    assert!(none.is_none());
    let org_page = format!("{}/organizations/acme/settings/apps/new?state=", fake.url());
    let person_page = format!("{}/settings/apps/new?state=", fake.url());
    assert!(
        for_org.post_url.starts_with(&org_page),
        "{}",
        for_org.post_url
    );
    assert!(for_person.post_url.starts_with(&person_page));
    let state = state_of(&for_org.post_url);
    let (row, _) = state
        .split_once('.')
        .expect("the id of the row in the state");
    let manifest = manifest_of(&for_org.manifest);
    let hook = format!("http://{}/integrations/github/{row}/webhook", app.addr);
    assert_eq!(
        manifest["hook_attributes"],
        json!({ "url": hook, "active": true })
    );
    let permissions = json!({
        "checks": "write",
        "contents": "read",
        "pull_requests": "read",
        "metadata": "read",
        "administration": "write",
    });
    assert_eq!(manifest["default_permissions"], permissions);
    let events = json!(["push", "pull_request", "check_run", "check_suite"]);
    assert_eq!(manifest["default_events"], events);
    assert_eq!(manifest["request_oauth_on_install"], json!(true));
    let redirect = manifest["redirect_url"].as_str().unwrap();
    assert!(redirect.ends_with("/integrations/github/manifest"));
    let callback = manifest["callback_urls"][0].as_str().unwrap();
    assert!(callback.ends_with("/integrations/github/callback"));
    assert!(manifest["name"].as_str().unwrap().len() <= 34);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let life = for_org.expires_at.unwrap().seconds - i64::try_from(now).unwrap();
    assert!((3590..=3600).contains(&life), "{life}");
    assert_code(bad, Code::InvalidArgument);
}

/// A completion converts the code with no credentials, keeps the App in the row of the state
/// with its secrets sealed, and shows it. GitHub can then be connected, and a webhook of the App
/// is the last delivery of the App.
#[sqlx::test(migrations = false)]
async fn complete_git_hub_manifest_keeps_the_app_with_its_secrets_sealed(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool, false).await;
    let (admin, made) = created(&app, &fake).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let read = app.get_vcs_app(admin).await.unwrap().expect("the App");
    let providers = app.get_vcs_providers(owner, &workspace).await.unwrap();
    let id = text(&made.id).to_owned();
    let body = json!({ "action": "created" }).to_string();
    let response = reqwest::Client::new()
        .post(&made.webhook_url)
        .header(
            "x-hub-signature-256",
            sign_github(MANIFEST_WEBHOOK_SECRET.as_bytes(), body.as_bytes()),
        )
        .header("x-github-event", "check_run")
        .header("x-github-delivery", "d-1")
        .body(body)
        .send()
        .await
        .unwrap();
    let delivered = app.get_vcs_app(admin).await.unwrap().expect("the App");

    // then
    assert_eq!(made.provider, VcsProvider::Github as i32);
    let names = (
        made.app_id.as_str(),
        made.slug.as_str(),
        made.owner.as_str(),
    );
    assert_eq!(names, ("77", "scylla-acme", "acme"));
    assert_eq!(made.web_url, "https://github.com/apps/scylla-acme");
    let hook = format!("http://{}/integrations/github/{id}/webhook", app.addr);
    assert_eq!(made.webhook_url, hook);
    assert!(!made.managed_by_settings && made.created_at.is_some());
    assert_eq!(text(&made.created_by), admin.to_string());
    assert_eq!(read, made);
    let calls = fake.requests_to("POST", &format!("/app-manifests/{CODE}/conversions"));
    assert_eq!(calls.len(), 1);
    assert!(calls[0].header("authorization").is_none());
    let sealer = app.sealer.as_ref().unwrap();
    let data = VcsAppId::parse(&id).unwrap().to_bytes();
    let (client, key, webhook) = app.sealed_app_secrets().await;
    let open = |sealed: &[u8]| String::from_utf8(sealer.open_with(sealed, &data).unwrap()).unwrap();
    assert_eq!(open(&client), MANIFEST_CLIENT_SECRET);
    assert_eq!(open(&key), TEST_APP_PRIVATE_KEY);
    assert_eq!(open(&webhook), MANIFEST_WEBHOOK_SECRET);
    assert!(providers.iter().all(|provider| provider.available));
    assert_eq!(response.status().as_u16(), 202);
    assert_eq!(delivered.last_delivery_status, 202);
    assert!(delivered.last_delivery_at.is_some());
}

/// A state works one time, for the person that started the flow only, and a code that GitHub
/// does not know uses the state. After the App, a new start gives `APP_EXISTS`.
#[sqlx::test(migrations = false)]
async fn complete_git_hub_manifest_uses_the_state_one_time(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool, false).await;
    let (admin, other) = (app.admin().await, app.admin().await);
    let first = state_of(&app.start_github_manifest(admin, "").await.unwrap().post_url);
    let second = state_of(&app.start_github_manifest(admin, "").await.unwrap().post_url);
    fake.answer_manifest_code(CODE);

    // when
    let of_other = app.complete_github_manifest(other, CODE, &first).await;
    let unknown_code = app.complete_github_manifest(admin, "nope", &second).await;
    let reused = app.complete_github_manifest(admin, CODE, &second).await;
    let made = app.complete_github_manifest(admin, CODE, &first).await;
    let again = app.complete_github_manifest(admin, CODE, &first).await;
    let restarted = app.start_github_manifest(admin, "").await;
    let bad_code = app.complete_github_manifest(admin, "a/b", &first).await;

    // then
    assert_reason(of_other, "STATE_EXPIRED");
    assert_reason(unknown_code, "STATE_EXPIRED");
    assert_reason(reused, "STATE_EXPIRED");
    assert!(made.is_ok());
    assert_reason(again, "STATE_EXPIRED");
    assert_reason(restarted, "APP_EXISTS");
    assert_code(bad_code, Code::InvalidArgument);
}
