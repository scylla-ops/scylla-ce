//! The tests of the photo of an identity provider: its import as the avatar of a new account, the
//! checks of its address, and `ImportAvatar`.

use image::ImageFormat;
use scylla_core::api::ServeOptions;
use scylla_core::SignUpMode;
use scylla_proto::common::v1::Avatar;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::identity::avatars::{get_image, png};
use crate::identity::browser::Browser;
use crate::identity::idp::{person, Idp, CLIENT_ID};

/// The photo of the fake provider.
const PHOTO: &str = "{url}/avatars/u/7";

/// The subject of the person of the fake provider at its GitHub.
const GITHUB_SUBJECT: &str = "5007";

/// Starts the Community edition with an open sign-up. `local` lets a photo come from the fake
/// provider, on a loopback address with HTTP.
async fn app_with(pool: PgPool, local: bool) -> TestApp {
    let mut options = ServeOptions::default();
    options.accounts.sign_up_mode = SignUpMode::Open;
    options.accounts.local_pictures = local;
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Starts a fake provider that gives a photo, as an OIDC provider `idp` and a GitHub `github`.
async fn idp_of(app: &TestApp, picture: &str) -> Idp {
    let idp = Idp::start().await;
    idp.give_picture(Some(picture));
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    app.add_github_provider("github", idp.url(), CLIENT_ID, "secret")
        .await;
    idp
}

/// Signs up or signs in with a provider in a new browser, and returns an access token.
async fn sign_in(app: &TestApp, slug: &str) -> String {
    let mut browser = Browser::new();
    let step = browser.sign_in(app, slug, "").await;
    assert_eq!(step.status, 303, "{}", step.location);
    let refresh = &browser.cookies["scylla_session"];
    app.refresh(refresh).await.unwrap().token.unwrap().value
}

/// Gives the id of the image of an avatar, if it has one.
fn image_of(avatar: Option<Avatar>) -> Option<String> {
    avatar.and_then(|avatar| avatar.image_id).map(|id| id.value)
}

/// A sign-up with OIDC and with GitHub makes the photo of the provider the avatar of the new
/// account: a square PNG of 256 pixels on the route of the images.
#[sqlx::test(migrations = false)]
async fn a_sign_up_imports_the_photo_of_the_provider(pool: PgPool) {
    // given
    let app = app_with(pool, true).await;
    let idp = idp_of(&app, PHOTO).await;

    for (slug, name) in [("idp", "alice"), ("github", "bob")] {
        idp.sign_in_as(person(name, &format!("{name}@idp.example"), true));

        // when
        let token = sign_in(&app, slug).await;

        // then
        let me = app.get_me(&token).await.unwrap();
        let image = image_of(me.avatar).expect(slug);
        let response = get_image(&app, &image).await;
        assert_eq!(response.status(), 200, "{slug}");
        let body = response.bytes().await.unwrap();
        let square = image::load_from_memory_with_format(&body, ImageFormat::Png).unwrap();
        assert_eq!((square.width(), square.height()), (256, 256), "{slug}");
        let identities = app.list_identities(&token).await.unwrap().identities;
        assert!(identities[0].has_picture, "{slug}");
    }
}

/// A photo that the server cannot download does not stop the sign-up: the new account keeps the
/// initials. A GitHub photo must come from an avatar host of its GitHub.
#[sqlx::test(migrations = false)]
async fn a_photo_that_does_not_pass_keeps_the_initials(pool: PgPool) {
    // given
    let app = app_with(pool, true).await;
    let idp = idp_of(&app, PHOTO).await;
    let cases = [
        ("idp", "carol", "{url}/missing.png"),
        ("github", "dave", "http://localhost:1/avatars/u/7"),
    ];

    for (slug, name, picture) in cases {
        idp.sign_in_as(person(name, &format!("{name}@idp.example"), true));
        idp.give_picture(Some(picture));

        // when
        let token = sign_in(&app, slug).await;

        // then
        let me = app.get_me(&token).await.unwrap();
        assert_eq!(image_of(me.avatar), None, "{slug} {picture}");
    }
}

/// By default, a photo on a loopback address with HTTP is refused, and the new account keeps the
/// initials.
#[sqlx::test(migrations = false)]
async fn a_photo_on_a_private_address_is_refused(pool: PgPool) {
    // given
    let app = app_with(pool, false).await;
    let _idp = idp_of(&app, PHOTO).await;

    // when
    let token = sign_in(&app, "idp").await;

    // then
    let me = app.get_me(&token).await.unwrap();
    assert_eq!(image_of(me.avatar), None);
    let identities = app.list_identities(&token).await.unwrap().identities;
    assert!(identities[0].has_picture);
}

/// A later sign-in never replaces a photo that the user set. `ImportAvatar` replaces it on
/// request, and the old image goes.
#[sqlx::test(migrations = false)]
async fn import_avatar_replaces_the_photo_on_request(pool: PgPool) {
    // given
    let app = app_with(pool, true).await;
    let _idp = idp_of(&app, PHOTO).await;
    let token = sign_in(&app, "github").await;
    let own = app.set_avatar(&token, Some(png(8, 8)), None).await.unwrap();
    let own = image_of(own.avatar).unwrap();

    // when
    let token = sign_in(&app, "github").await;
    let kept = image_of(app.get_me(&token).await.unwrap().avatar);
    let imported = app.import_avatar(&token, "github", GITHUB_SUBJECT).await;

    // then
    assert_eq!(kept.as_deref(), Some(own.as_str()));
    let imported = image_of(imported.unwrap().avatar).unwrap();
    assert_ne!(imported, own);
    assert_eq!(get_image(&app, &imported).await.status(), 200);
    assert_eq!(get_image(&app, &own).await.status(), 404);
}

/// An unknown identity gives `NOT_FOUND`, an identity with no photo `NO_PICTURE`, and a photo
/// that the server cannot download `PICTURE_UNAVAILABLE`. The avatar stays.
#[sqlx::test(migrations = false)]
async fn import_avatar_refuses_an_identity_with_no_photo(pool: PgPool) {
    // given
    let app = app_with(pool, true).await;
    let idp = idp_of(&app, "{url}/missing.png").await;
    let token = sign_in(&app, "idp").await;
    let unavailable = app.import_avatar(&token, "idp", "alice").await;
    idp.give_picture(None);
    let token = sign_in(&app, "idp").await;

    // when
    let unknown = app.import_avatar(&token, "idp", "bob").await;
    let other = app.import_avatar(&token, "github", "alice").await;
    let no_picture = app.import_avatar(&token, "idp", "alice").await;

    // then
    assert_refused(unavailable, "PICTURE_UNAVAILABLE");
    assert_eq!(unknown.unwrap_err().code(), Code::NotFound);
    assert_eq!(other.unwrap_err().code(), Code::NotFound);
    assert_refused(no_picture, "NO_PICTURE");
    let identities = app.list_identities(&token).await.unwrap().identities;
    assert!(!identities[0].has_picture);
    assert_eq!(image_of(app.get_me(&token).await.unwrap().avatar), None);
}
