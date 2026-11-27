//! The tests of the custom icon of a project (`UpdateProject` with an icon), and of the route
//! `GET /images/{id}` that gives an uploaded image.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};
use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::icon_change::Kind as ChangeKind;
use scylla_proto::common::v1::{IconChange, IconGlyph};
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;

use super::cast::{passed, Cast};
use crate::{app, assert_invalid_field, UNKNOWN_ID};

/// The largest upload, in bytes.
const MAX_UPLOAD: usize = 1_048_576;

/// Makes a change to a glyph with a color.
pub(crate) fn glyph(name: &str, color: &str) -> IconChange {
    IconChange {
        kind: Some(ChangeKind::Glyph(IconGlyph {
            name: name.to_owned(),
            color: color.to_owned(),
        })),
    }
}

/// Makes a change to an image of these bytes.
pub(crate) fn image(bytes: Vec<u8>) -> IconChange {
    IconChange {
        kind: Some(ChangeKind::Image(bytes)),
    }
}

/// Writes a PNG of `width` by `height` pixels of one color.
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let picture = RgbaImage::from_pixel(width, height, Rgba([200, 40, 90, 255]));
    let mut bytes = Cursor::new(Vec::new());
    picture.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Gives the id of the image of an icon.
pub(crate) fn image_id(icon: Option<scylla_proto::common::v1::Icon>) -> String {
    match icon.and_then(|icon| icon.kind) {
        Some(Kind::ImageId(id)) => id.value,
        other => panic!("an image, not {other:?}"),
    }
}

/// Sends `GET /images/{id}` to the server of a test.
pub(crate) async fn get_image(app: &TestApp, id: &str) -> reqwest::Response {
    reqwest::get(format!("http://{}/images/{id}", app.addr))
        .await
        .unwrap()
}

/// Counts the rows of the images.
pub(crate) async fn image_count(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// A glyph with a color comes back with the hex color in lower case, and the reads show it.
#[sqlx::test(migrations = false)]
async fn update_project_sets_a_glyph_with_a_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let created = app.create_project(user, "alpha").await.unwrap();
    let id = text(&created.id);

    // when
    let updated = app
        .set_project_icon(user, id, glyph("rocket", "#A1B2C3"))
        .await
        .unwrap();

    // then
    let expected = Some(Kind::Glyph(IconGlyph {
        name: "rocket".to_owned(),
        color: "#a1b2c3".to_owned(),
    }));
    assert_eq!(updated.icon.clone().and_then(|icon| icon.kind), expected);
    assert_eq!(updated.name, "alpha");
    assert_eq!(created.icon, None);
    assert_eq!(app.get_project(user, id).await.unwrap(), updated);
    let listed = app.list_projects(user, 10, "").await.unwrap().projects;
    assert_eq!(listed, [updated]);
}

/// An image becomes a square PNG of 256 pixels, which the route gives with safe headers.
#[sqlx::test(migrations = false)]
async fn update_project_makes_a_square_png_of_an_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();

    // when
    let updated = app
        .set_project_icon(user, text(&project.id), image(png(300, 200)))
        .await
        .unwrap();
    let response = get_image(&app, &image_id(updated.icon)).await;

    // then
    assert_eq!(response.status(), 200);
    let header = |name: &str| response.headers()[name].to_str().unwrap().to_owned();
    assert_eq!(header("content-type"), "image/png");
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(
        header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    assert!(header("cache-control").contains("immutable"));
    let body = response.bytes().await.unwrap();
    let square = image::load_from_memory_with_format(&body, ImageFormat::Png).unwrap();
    assert_eq!((square.width(), square.height()), (256, 256));
}

/// A new image replaces the old one, which goes, and the default icon removes the last one.
#[sqlx::test(migrations = false)]
async fn update_project_deletes_the_old_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);
    let first = app.set_project_icon(user, id, image(png(8, 8))).await;
    let first = image_id(first.unwrap().icon);

    // when
    let second = app.set_project_icon(user, id, image(png(9, 9))).await;
    let second = image_id(second.unwrap().icon);
    let old = get_image(&app, &first).await.status();
    let reset = app
        .set_project_icon(user, id, IconChange { kind: None })
        .await
        .unwrap();

    // then
    assert_eq!(old, 404);
    assert_eq!(reset.icon, None);
    assert_eq!(get_image(&app, &second).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}

/// A file that is too large, an SVG, an empty file, a bad glyph name and a bad color give
/// `INVALID_ARGUMENT` on their field, and the project keeps its icon.
#[sqlx::test(migrations = false)]
async fn update_project_refuses_a_bad_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#.to_vec();
    let mut large = png(64, 64);
    large.resize(MAX_UPLOAD + 1, 0);
    let wide = png(4097, 1);
    let cases = [
        (image(large), "icon.image"),
        (image(svg), "icon.image"),
        (image(Vec::new()), "icon.image"),
        (image(wide), "icon.image"),
        (glyph("Rocket", ""), "icon.glyph.name"),
        (glyph("rocket", "#12345"), "icon.glyph.color"),
    ];

    // when
    let mut refusals = Vec::new();
    for (icon, field) in cases {
        refusals.push((app.set_project_icon(user, id, icon).await, field));
    }

    // then
    for (result, field) in refusals {
        assert_invalid_field(&result.unwrap_err(), field);
    }
    assert_eq!(app.get_project(user, id).await.unwrap().icon, None);
    assert_eq!(image_count(&app).await, 0);
}

/// Only the roles that edit the project change its icon, and a refusal keeps no image.
#[sqlx::test(migrations = false)]
async fn update_project_icon_needs_a_role_that_edits(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        let result = app
            .set_project_icon(actor, &cast.project, image(png(8, 8)))
            .await;
        results.push((role, result));
    }

    // then
    let editors = [
        "owner",
        "workspace admin",
        "member",
        "project admin",
        "editor",
    ];
    assert_eq!(passed(results), editors);
    assert_eq!(image_count(&app).await, 1);
}

/// A delete of a project deletes the image of its icon.
#[sqlx::test(migrations = false)]
async fn delete_project_deletes_the_image_of_its_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);
    let icon = app.set_project_icon(user, id, image(png(8, 8))).await;
    let image = image_id(icon.unwrap().icon);

    // when
    app.delete_project(user, id).await.unwrap();

    // then
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}

/// An unknown id and a text that is not an id give `404`.
#[sqlx::test(migrations = false)]
async fn image_route_gives_404_for_an_unknown_image(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let unknown = get_image(&app, UNKNOWN_ID).await.status();
    let not_an_id = get_image(&app, "nope").await.status();

    // then
    assert_eq!(unknown, 404);
    assert_eq!(not_an_id, 404);
}
