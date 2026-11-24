//! The tests of the calls to a fake provider: the pages of a list, the ETag of a read, and the
//! errors of the provider.

use scylla_core::infra::vcs::{Answer, Auth, Hidden, ProviderError, VcsClients};
use scylla_testkit::FakeProvider;
use serde_json::{json, Value};

/// Makes the clients of a server that calls the fake.
fn clients(fake: &FakeProvider) -> VcsClients {
    scylla_core::infra::tls::install();
    VcsClients::new(&fake.options(false)).expect("clients")
}

/// A list follows the link of each next page: a GitLab keyset list with the token of a service
/// account.
#[tokio::test]
async fn list_follows_the_next_pages() {
    // given
    let fake = FakeProvider::start().await;
    let next = format!(
        "<{}/api/v4/projects?id_after=2&pagination=keyset>; rel=\"next\"",
        fake.url()
    );
    let first = json!([{ "id": 1 }, { "id": 2 }]);
    fake.respond("GET", "/api/v4/projects", 200, first, &[("link", &next)]);
    let last = "/api/v4/projects?id_after=2&pagination=keyset";
    fake.respond("GET", last, 200, json!([{ "id": 3 }]), &[]);
    let clients = clients(&fake);
    let token = Hidden::new("glpat-test");

    // when
    let projects: Vec<Value> = clients
        .gitlab
        .get("/projects")
        .auth(Auth::PrivateToken(&token))
        .keyset("id")
        .list(None)
        .await
        .expect("projects");

    // then
    let ids: Vec<i64> = projects
        .iter()
        .filter_map(|project| project["id"].as_i64())
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    let requests = fake.requests_to("GET", "/api/v4/projects");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].query, "pagination=keyset&order_by=id&sort=asc");
    assert_eq!(requests[1].query, "id_after=2&pagination=keyset");
    assert!(requests
        .iter()
        .all(|request| request.header("private-token") == Some("glpat-test")));
}

/// A list reads the array of a field of each page, as `/installation/repositories` of GitHub.
#[tokio::test]
async fn list_reads_the_array_of_a_field() {
    // given
    let fake = FakeProvider::start().await;
    let page = json!({ "total_count": 1, "repositories": [{ "full_name": "octo/app" }] });
    fake.respond("GET", "/installation/repositories", 200, page, &[]);
    let clients = clients(&fake);

    // when
    let repositories: Vec<Value> = clients
        .github
        .get("/installation/repositories")
        .list(Some("repositories"))
        .await
        .expect("repositories");

    // then
    assert_eq!(repositories, [json!({ "full_name": "octo/app" })]);
}

/// A list never follows a link to another host, so a token never leaves the provider.
#[tokio::test]
async fn list_refuses_a_link_to_another_host() {
    // given
    let fake = FakeProvider::start().await;
    let link = [(
        "link",
        "<https://evil.example/projects?page=2>; rel=\"next\"",
    )];
    fake.respond("GET", "/api/v4/projects", 200, json!([{ "id": 1 }]), &link);
    let clients = clients(&fake);
    let token = Hidden::new("glpat-test");

    // when
    let result = clients
        .gitlab
        .get("/projects")
        .auth(Auth::PrivateToken(&token))
        .list::<Value>(None)
        .await;

    // then
    assert!(matches!(result, Err(ProviderError::Invalid(_))));
}

/// A read with the ETag of the last read gets `NotModified` when nothing changed.
#[tokio::test]
async fn send_gives_not_modified_for_the_same_etag() {
    // given
    let fake = FakeProvider::start().await;
    let path = "/repos/octo/app/branches";
    let etag = "W/\"abc\"";
    fake.respond(
        "GET",
        path,
        200,
        json!([{ "name": "main" }]),
        &[("etag", etag)],
    );
    fake.respond("GET", path, 304, Value::Null, &[]);
    let clients = clients(&fake);

    // when
    let first = clients
        .github
        .get(path)
        .send::<Value>()
        .await
        .expect("branches");
    let Answer::Fresh { etag: seen, .. } = first else {
        panic!("a first answer with a body");
    };
    let second = clients
        .github
        .get(path)
        .if_none_match(seen.as_deref())
        .send::<Value>()
        .await
        .expect("an answer");

    // then
    assert_eq!(seen.as_deref(), Some(etag));
    assert!(matches!(second, Answer::NotModified));
    assert_eq!(
        fake.requests_to("GET", path)[1].header("if-none-match"),
        Some(etag)
    );
}

/// The errors of the provider: a limit until its reset, an unknown object, refused credentials.
#[tokio::test]
async fn send_maps_the_errors_of_the_provider() {
    // given
    let fake = FakeProvider::start().await;
    let limit = [
        ("x-ratelimit-remaining", "0"),
        ("x-ratelimit-reset", "4102444800"),
    ];
    let message = json!({ "message": "API rate limit exceeded" });
    fake.respond("GET", "/repos/octo/app", 403, message, &limit);
    let not_found = json!({ "message": "Not Found" });
    fake.respond("GET", "/repos/octo/gone", 404, not_found, &[]);
    let bad = json!({ "message": "Bad credentials" });
    fake.respond("GET", "/user", 401, bad, &[]);
    let clients = clients(&fake);

    // when
    let limited = clients.github.get("/repos/octo/app").read::<Value>().await;
    let gone = clients.github.get("/repos/octo/gone").read::<Value>().await;
    let refused = clients.github.get("/user").read::<Value>().await;

    // then
    match limited {
        Err(ProviderError::RateLimited(until)) => assert_eq!(until.unix_timestamp(), 4102444800),
        other => panic!("a limit, not {other:?}"),
    }
    assert_eq!(gone.expect_err("an error"), ProviderError::NotFound);
    assert_eq!(refused.expect_err("an error"), ProviderError::Unauthorized);
}
