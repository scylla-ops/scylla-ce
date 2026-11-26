//! The tests of the glyph avatars: the SVG that the server draws for `UpdateProfile` and
//! `SetAppIcon`, its options in the reads, the route that gives it, and the refusals.

use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::icon_change::Kind as ChangeKind;
use scylla_proto::common::v1::{GlyphAvatar, GlyphFill, GlyphFlip, GlyphPaint, IconChange};
use scylla_testkit::text;
use sqlx::PgPool;

use super::avatars::signed_in;
use crate::projects::icons::{get_image, image_count, png};
use crate::{app, assert_invalid_field};

/// Makes the options of a glyph avatar with a linear gradient on the glyph.
fn glyph_avatar(shape: &str) -> GlyphAvatar {
    GlyphAvatar {
        shape: shape.to_owned(),
        glyph: Some(GlyphPaint {
            colors: vec!["#5D9972".to_owned(), "#205090".to_owned()],
            fill: GlyphFill::Linear.into(),
            angle: 45,
        }),
        background: None,
        flip: GlyphFlip::Horizontal.into(),
        rotate: -15,
        scale: 0,
    }
}

/// Gives the options that the server keeps for `glyph_avatar`: the colors in lower case, a white
/// background and the default size.
fn kept(shape: &str) -> GlyphAvatar {
    let mut avatar = glyph_avatar(shape);
    if let Some(glyph) = avatar.glyph.as_mut() {
        glyph.colors = vec!["#5d9972".to_owned(), "#205090".to_owned()];
    }
    avatar.background = Some(GlyphPaint {
        colors: vec!["#ffffff".to_owned()],
        fill: GlyphFill::Unspecified.into(),
        angle: 0,
    });
    avatar.scale = 100;
    avatar
}

/// Makes a change of an icon to a glyph avatar.
fn icon(avatar: GlyphAvatar) -> IconChange {
    IconChange {
        kind: Some(ChangeKind::GlyphAvatar(avatar)),
    }
}

/// A glyph avatar becomes an SVG that the route gives with the strict headers, the reads give
/// its options, and an upload replaces it and its options.
#[sqlx::test(migrations = false)]
async fn update_profile_draws_a_glyph_avatar_as_an_svg(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;

    // when
    let set = app.set_avatar_glyph(&token, glyph_avatar("scylla"), None);
    let set = set.await.unwrap().avatar.unwrap();
    let image = text(&set.image_id).to_owned();
    let response = get_image(&app, &image).await;
    let me = app.get_me(&token).await.unwrap();
    let replaced = app.set_avatar(&token, Some(png(8, 8)), None).await.unwrap();

    // then
    assert_eq!(set.glyph_avatar, Some(kept("scylla")));
    assert_eq!(response.status(), 200);
    let header = |name: &str| response.headers()[name].to_str().unwrap().to_owned();
    assert_eq!(header("content-type"), "image/svg+xml");
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(
        header("content-security-policy"),
        "default-src 'none'; sandbox"
    );
    let body = response.text().await.unwrap();
    assert!(body.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
    assert!(body.contains("shape-scylla"));
    assert_eq!(me.avatar, Some(set));
    let replaced = replaced.avatar.unwrap();
    assert_eq!(replaced.glyph_avatar, None);
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 1);
}

/// Bad options and a glyph avatar with an image give `INVALID_ARGUMENT` on their field, and the
/// avatar does not change.
#[sqlx::test(migrations = false)]
async fn update_profile_refuses_a_bad_glyph_avatar(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (_, token) = signed_in(&app, "ava@example.com").await;
    let mut solid_two = glyph_avatar("variant01");
    solid_two.glyph.as_mut().unwrap().fill = GlyphFill::Unspecified.into();
    let mut bad_angle = glyph_avatar("variant01");
    bad_angle.background = Some(GlyphPaint {
        colors: vec!["#ffffff".to_owned(), "#000000".to_owned()],
        fill: GlyphFill::Linear.into(),
        angle: 400,
    });
    let mut too_large = glyph_avatar("variant01");
    too_large.scale = 151;
    let mut no_glyph = glyph_avatar("variant01");
    no_glyph.glyph = None;
    let cases = [
        (glyph_avatar("variant36"), None, "avatar_glyph.shape"),
        (solid_two, None, "avatar_glyph.glyph.colors"),
        (no_glyph, None, "avatar_glyph.glyph"),
        (bad_angle, None, "avatar_glyph.background.angle"),
        (too_large, None, "avatar_glyph.scale"),
        (glyph_avatar("variant01"), Some(png(8, 8)), "avatar_glyph"),
    ];

    // when
    let mut refusals = Vec::new();
    for (glyph, image, field) in cases {
        refusals.push((app.set_avatar_glyph(&token, glyph, image).await, field));
    }

    // then
    for (result, field) in refusals {
        assert_invalid_field(&result.unwrap_err(), field);
    }
    assert_eq!(app.get_me(&token).await.unwrap().avatar, None);
    assert_eq!(image_count(&app).await, 0);
}

/// A glyph avatar becomes the icon of an App with its options, the reads show them, and a bad
/// shape gives `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn set_app_icon_draws_a_glyph_avatar(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let set = app
        .set_app_icon(owner, &key.app, Some(icon(glyph_avatar("variant07"))))
        .await
        .unwrap();
    let read = app.get_app(owner, &key.app).await.unwrap().app.unwrap();
    let bad = app
        .set_app_icon(owner, &key.app, Some(icon(glyph_avatar("Scylla"))))
        .await;

    // then
    let icon = set.icon.clone().unwrap();
    assert_eq!(icon.glyph_avatar, Some(kept("variant07")));
    let Some(Kind::ImageId(image)) = icon.kind else {
        panic!("an image");
    };
    let response = get_image(&app, &image.value).await;
    assert_eq!(response.headers()["content-type"], "image/svg+xml");
    assert_eq!(read.icon, set.icon);
    assert_invalid_field(&bad.unwrap_err(), "icon.glyph_avatar.shape");
    assert_eq!(image_count(&app).await, 1);
}

/// A project and a workspace refuse a glyph avatar: only a person or an App takes one.
#[sqlx::test(migrations = false)]
async fn a_project_or_a_workspace_refuses_a_glyph_avatar(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let workspace = app.create_workspace(user, "team").await.unwrap();

    // when
    let on_project = app
        .set_project_icon(user, text(&project.id), icon(glyph_avatar("scylla")))
        .await;
    let on_workspace = app
        .set_workspace_icon(user, text(&workspace.id), icon(glyph_avatar("scylla")))
        .await;

    // then
    assert_invalid_field(&on_project.unwrap_err(), "icon.glyph_avatar");
    assert_invalid_field(&on_workspace.unwrap_err(), "icon.glyph_avatar");
    assert_eq!(image_count(&app).await, 0);
}
