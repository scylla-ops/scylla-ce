//! The tests of the fragments of a package of an outside registry: their origin, the versions
//! that they take, and the routes that only an admin calls.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::library::v1::fragment_service_client::FragmentServiceClient;
use scylla_proto::library::v1::{AcceptOriginVersionRequest, Fragment, NewFragmentOrigin};
use scylla_testkit::{env_node, fragment_request, id, literal_parameter, text, Role, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code, assert_invalid_field};

/// Makes the origin of a linked fragment at a version of the package.
fn origin(version: u32) -> NewFragmentOrigin {
    NewFragmentOrigin {
        registry: "https://market.scylla.test".to_owned(),
        package_id: "01a2-acme-build".to_owned(),
        package_name: "@acme/build".to_owned(),
        version,
        linked: true,
    }
}

/// Calls `AcceptOriginVersion` as `actor` with a version of the package.
async fn accept(
    app: &TestApp,
    actor: ActorId,
    fragment: &str,
    version: u32,
) -> Result<Fragment, Status> {
    let request = AcceptOriginVersionRequest {
        fragment_id: Some(id(fragment)),
        version,
        nodes: vec![env_node("build", &[], &[("MODE", "debug")])],
        parameters: vec![literal_parameter("MODE", "debug")],
        changes: vec!["Debug mode".to_owned()],
    };
    let response = FragmentServiceClient::new(app.channel.clone())
        .accept_origin_version(app.authed(actor, request).await)
        .await?;
    Ok(response.into_inner().fragment.expect("fragment"))
}

/// A fragment of a package keeps its origin and the number of the version of the package, takes
/// the next versions of the package, and refuses a version made by hand.
#[sqlx::test(migrations = false)]
async fn a_linked_fragment_follows_its_package(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let mut request = fragment_request(&workspace, "shared-build");
    request.origin = Some(origin(4));
    let created = app.create_fragment_with(owner, request).await.unwrap();
    let fragment = text(&created.id).to_owned();

    // when
    let accepted = accept(&app, owner, &fragment, 6).await.unwrap();
    let older = accept(&app, owner, &fragment, 5).await;
    let nodes = vec![env_node("build", &[], &[("MODE", "x")])];
    let by_hand = app
        .publish_fragment_version(owner, &fragment, nodes, Vec::new(), &[])
        .await;

    // then
    let kept = created.origin.unwrap();
    assert_eq!(
        (kept.package_name.as_str(), kept.base_version, kept.linked),
        ("@acme/build", 4, true)
    );
    let numbers: Vec<_> = accepted
        .versions
        .iter()
        .map(|version| version.number)
        .collect();
    assert_eq!(numbers, [4, 6]);
    for (result, reason) in [
        (older, "FRAGMENT_VERSION_NOT_NEWER"),
        (by_hand, "FRAGMENT_LINKED"),
    ] {
        let status = result.expect_err("the call must fail");
        assert_eq!(status.get_details_error_info().unwrap().reason, reason);
    }
}

/// A bad origin is a request error with its path.
#[sqlx::test(migrations = false)]
async fn create_fragment_refuses_a_bad_origin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let mut no_web = origin(1);
    no_web.registry = "file:///etc".to_owned();
    let mut no_version = origin(0);
    no_version.package_id = "a".to_owned();
    let mut no_package = origin(1);
    no_package.package_id = " ".to_owned();

    // when
    for (field, bad) in [
        ("origin.registry", no_web),
        ("origin.version", no_version),
        ("origin.package_id", no_package),
    ] {
        let mut request = fragment_request(&workspace, "build");
        request.origin = Some(bad);
        let status = app.create_fragment_with(owner, request).await.unwrap_err();

        // then
        assert_invalid_field(&status, field);
    }
}

/// A member, and the author that is a member, cannot take a version of the package: only an
/// admin of the workspace does.
#[sqlx::test(migrations = false)]
async fn only_an_admin_takes_a_version_of_the_package(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (member, admin) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await.to_string();
    for (role, user) in [("member", member), ("admin", admin)] {
        let role = Role::on("workspace", &workspace, role);
        app.grant_role(owner, role, &user.to_string())
            .await
            .unwrap();
    }
    let mut request = fragment_request(&workspace, "shared-build");
    request.origin = Some(origin(1));
    let created = app.create_fragment_with(member, request).await.unwrap();
    let fragment = text(&created.id).to_owned();

    // when
    let by_member = accept(&app, member, &fragment, 2).await;
    let by_admin = accept(&app, admin, &fragment, 2).await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert!(by_admin.is_ok());
}
