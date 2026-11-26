//! The tests of the avatar of a user: the image and the color of `UpdateProfile`, the reads that
//! show them, and the delete of the image with the account.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::common::v1::Avatar;
use scylla_testkit::{text, TestApp};
use serde_json::Value;
use sqlx::PgPool;

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_invalid_field};

/// The largest upload, in bytes.
const MAX_UPLOAD: usize = 1_048_576;

/// Writes a PNG of `width` by `height` pixels of one color.
pub(super) fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = RgbaImage::from_pixel(width, height, Rgba([40, 90, 200, 255]));
    let mut bytes = Cursor::new(Vec::new());
    picture.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Sends `GET /images/{id}` to the server of a test.
pub(super) async fn get_image(app: &TestApp, id: &str) -> reqwest::Response {
    reqwest::get(format!("http://{}/images/{id}", app.addr))
        .await
        .unwrap()
}

/// Counts the rows of the images.
pub(super) async fn image_count(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Makes a user with a password, signs it in, and returns its id and its access token.
async fn signed_in(app: &TestApp, email: &str) -> (ActorId, String) {
    let user = app.user_with(email, Some(PASSWORD)).await;
    let token = app.sign_in(email, PASSWORD).await.unwrap().token.unwrap();
    (user, token.value)
}

/// Gives the id of the image of an avatar.
fn image_id(avatar: Option<Avatar>) -> String {
    let image = avatar.and_then(|avatar| avatar.image_id);
    image.expect("an image").value
}

/// An image becomes a square PNG of 256 pixels that the reads show, and empty bytes remove it.
#[sqlx::test(migrations = false)]
async fn update_profile_sets_and_removes_the_avatar_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;
    let before = app.get_me(&token).await.unwrap();

    // when
    let set = app.set_avatar(&token, Some(png(300, 200)), None).await;
    let set = set.unwrap();
    let image = image_id(set.avatar.clone());
    let response = get_image(&app, &image).await;
    let me = app.get_me(&token).await.unwrap();
    let removed = app.set_avatar(&token, Some(Vec::new()), None).await;

    // then
    assert_eq!(before.avatar, None);
    assert_eq!(response.status(), 200);
    let body = response.bytes().await.unwrap();
    let square = image::load_from_memory_with_format(&body, ImageFormat::Png).unwrap();
    assert_eq!((square.width(), square.height()), (256, 256));
    assert_eq!(me.avatar, set.avatar);
    assert_eq!(removed.unwrap().avatar, None);
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}

/// The color of the initials is a hex color in lower case or the name of a color, it stays
/// with an image, and an empty text gives the default color.
#[sqlx::test(migrations = false)]
async fn update_profile_sets_the_color_of_the_initials(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;

    // when
    let hex = app.set_avatar(&token, None, Some("#3366CC")).await.unwrap();
    let named = app.set_avatar(&token, None, Some("chart-2")).await.unwrap();
    let image = app.set_avatar(&token, Some(png(8, 8)), None).await.unwrap();
    let reset = app.set_avatar(&token, None, Some("")).await.unwrap();
    let renamed = app.update_profile(&token, Some("Ava"), None).await.unwrap();

    // then
    let color = |avatar: Option<Avatar>| avatar.map(|avatar| avatar.color);
    assert_eq!(color(hex.avatar), Some("#3366cc".to_owned()));
    assert_eq!(color(named.avatar), Some("chart-2".to_owned()));
    assert_eq!(color(image.avatar.clone()), Some("chart-2".to_owned()));
    assert_eq!(color(reset.avatar.clone()), Some(String::new()));
    assert_eq!(image_id(reset.avatar), image_id(image.avatar.clone()));
    assert_eq!(renamed.avatar, reset_with_image(&image.avatar));
}

/// Gives the avatar with the same image and no color.
fn reset_with_image(avatar: &Option<Avatar>) -> Option<Avatar> {
    avatar.clone().map(|avatar| Avatar {
        color: String::new(),
        ..avatar
    })
}

/// A file that is too large, an SVG, a bad hex color and a bad color name give
/// `INVALID_ARGUMENT` on their field, and the avatar does not change.
#[sqlx::test(migrations = false)]
async fn update_profile_refuses_a_bad_avatar(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#.to_vec();
    let mut large = png(64, 64);
    large.resize(MAX_UPLOAD + 1, 0);

    // when
    let too_large = app.set_avatar(&token, Some(large), None).await;
    let not_an_image = app.set_avatar(&token, Some(svg), None).await;
    let bad_hex = app.set_avatar(&token, None, Some("#12345")).await;
    let bad_name = app.set_avatar(&token, None, Some("Red")).await;

    // then
    assert_invalid_field(&too_large.unwrap_err(), "avatar_image");
    assert_invalid_field(&not_an_image.unwrap_err(), "avatar_image");
    assert_invalid_field(&bad_hex.unwrap_err(), "avatar_color");
    assert_invalid_field(&bad_name.unwrap_err(), "avatar_color");
    assert_eq!(app.get_me(&token).await.unwrap().avatar, None);
    assert_eq!(image_count(&app).await, 0);
}

/// The members of a workspace show the avatar of each user, and the export of the account
/// gives it.
#[sqlx::test(migrations = false)]
async fn list_members_shows_the_avatars(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, owner_token) = signed_in(&app, "ava@example.com").await;
    let (member, member_token) = signed_in(&app, "bo@example.com").await;
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let photo = app.set_avatar(&owner_token, Some(png(8, 8)), None).await;
    let initials = app.set_avatar(&member_token, None, Some("chart-3")).await;

    // when
    let members = app
        .members_of(owner, "workspace", &workspace.to_string())
        .await
        .unwrap();
    let export = app.export_account_data(&member_token).await.unwrap();

    // then
    let avatar_of = |actor: ActorId| {
        let found = members
            .iter()
            .find(|m| text(&m.actor_id) == actor.to_string());
        found.and_then(|member| member.avatar.clone())
    };
    assert_eq!(avatar_of(owner), photo.unwrap().avatar);
    assert_eq!(avatar_of(member), initials.unwrap().avatar);
    let data: Value = serde_json::from_str(&export.json).unwrap();
    assert_eq!(data["account"]["avatar_color"], "chart-3");
    assert_eq!(data["account"]["avatar_image_id"], Value::Null);
}

/// The holders of a role show the avatar of each user.
#[sqlx::test(migrations = false)]
async fn list_role_holders_shows_the_avatars(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, _) = signed_in(&app, "ava@example.com").await;
    let (member, member_token) = signed_in(&app, "bo@example.com").await;
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let initials = app.set_avatar(&member_token, None, Some("#3366CC")).await;

    // when
    let holders = app
        .role_holders(owner, "workspace-member", Some(&workspace.to_string()))
        .await
        .unwrap();

    // then
    let holder = holders
        .holders
        .iter()
        .find(|holder| text(&holder.actor_id) == member.to_string());
    let avatar = holder.and_then(|holder| holder.avatar.clone());
    assert_eq!(avatar, initials.unwrap().avatar);
    assert_eq!(
        avatar.map(|avatar| avatar.color),
        Some("#3366cc".to_owned())
    );
}

/// A deletion of the account deletes the image of its avatar.
#[sqlx::test(migrations = false)]
async fn delete_account_deletes_the_avatar_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;
    let set = app.set_avatar(&token, Some(png(8, 8)), None).await;
    let image = image_id(set.unwrap().avatar);

    // when
    app.delete_account(&token, PASSWORD).await.unwrap();

    // then
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}
