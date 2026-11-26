//! The tests of the icon of an App: the glyph or the image that the admins give with
//! `SetAppIcon`, the reads that show it, and the delete of the image with the App.

use scylla_domain::identity::apps::AppHolder;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::icon_change::Kind as ChangeKind;
use scylla_proto::common::v1::{Icon, IconChange, IconGlyph};
use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use super::avatars::{get_image, image_count, png};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Makes a change to a glyph with a color.
fn glyph(name: &str, color: &str) -> Option<IconChange> {
    Some(IconChange {
        kind: Some(ChangeKind::Glyph(IconGlyph {
            name: name.to_owned(),
            color: color.to_owned(),
        })),
    })
}

/// Makes a change to an image of these bytes.
fn image(bytes: Vec<u8>) -> Option<IconChange> {
    Some(IconChange {
        kind: Some(ChangeKind::Image(bytes)),
    })
}

/// Gives the id of the image of an icon.
fn image_id(icon: Option<Icon>) -> String {
    match icon.and_then(|icon| icon.kind) {
        Some(Kind::ImageId(id)) => id.value,
        other => panic!("an image, not {other:?}"),
    }
}

/// An admin gives a glyph with a color to an agent. The reads and the members show it, and no
/// icon gives the default icon again.
#[sqlx::test(migrations = false)]
async fn set_app_icon_sets_a_glyph_with_a_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::ADMIN.of(workspace, admin)]).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let id = text(&created.app.clone().unwrap().id).to_owned();
    let scope = workspace.to_string();
    let runner = Role::on("workspace", &scope, "runner");
    app.grant_role(owner, runner, &id).await.unwrap();

    // when
    let set = app
        .set_app_icon(admin, &id, glyph("rocket", "#A1B2C3"))
        .await;
    let set = set.unwrap();
    let read = app.get_app(owner, &id).await.unwrap().app.unwrap();
    let listed = app.list_apps(owner, workspace, Some(true), 10, "").await;
    let members = app
        .members_of(owner, "workspace", &workspace.to_string())
        .await
        .unwrap();
    let reset = app.set_app_icon(owner, &id, None).await.unwrap();

    // then
    let expected = Some(Kind::Glyph(IconGlyph {
        name: "rocket".to_owned(),
        color: "#a1b2c3".to_owned(),
    }));
    assert_eq!(created.app.unwrap().icon, None);
    assert_eq!(set.icon.clone().and_then(|icon| icon.kind), expected);
    assert_eq!(read.icon, set.icon);
    assert_eq!(listed.unwrap().apps[0].icon, set.icon);
    let member = members.iter().find(|member| text(&member.actor_id) == id);
    assert_eq!(member.unwrap().icon, set.icon);
    assert_eq!(reset.icon, None);
}

/// An image becomes the icon, a new image replaces the old one, which goes, and the default
/// icon removes the last one.
#[sqlx::test(migrations = false)]
async fn set_app_icon_deletes_the_old_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let first = app.set_app_icon(owner, &key.app, image(png(8, 8))).await;
    let first = image_id(first.unwrap().icon);

    // when
    let second = app.set_app_icon(owner, &key.app, image(png(9, 9))).await;
    let second = image_id(second.unwrap().icon);
    let old = get_image(&app, &first).await.status();
    let reset = app
        .set_app_icon(owner, &key.app, Some(IconChange { kind: None }))
        .await;

    // then
    assert_eq!(old, 404);
    assert_eq!(get_image(&app, &second).await.status(), 404);
    assert_eq!(reset.unwrap().icon, None);
    assert_eq!(image_count(&app).await, 0);
}

/// An SVG, an empty file, a bad glyph name and a bad color give `INVALID_ARGUMENT` on their
/// field, and the App keeps its icon.
#[sqlx::test(migrations = false)]
async fn set_app_icon_refuses_a_bad_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#.to_vec();
    let cases = [
        (image(svg), "icon.image"),
        (image(Vec::new()), "icon.image"),
        (glyph("Rocket", ""), "icon.glyph.name"),
        (glyph("rocket", "#12345"), "icon.glyph.color"),
    ];

    // when
    let mut refusals = Vec::new();
    for (icon, field) in cases {
        refusals.push((app.set_app_icon(owner, &key.app, icon).await, field));
    }

    // then
    for (result, field) in refusals {
        assert_invalid_field(&result.unwrap_err(), field);
    }
    let read = app.get_app(owner, &key.app).await.unwrap().app.unwrap();
    assert_eq!(read.icon, None);
    assert_eq!(image_count(&app).await, 0);
}

/// Only the admins of the workspace set the icon: not a member, not the creator of the App with
/// no admin role, and not the App itself. An unknown App gets the same refusal.
#[sqlx::test(migrations = false)]
async fn set_app_icon_needs_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, creator) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::MEMBER.of(workspace, member),
        AppHolder::APP_CREATOR.of(workspace, creator),
    ])
    .await;
    let created = app.create_app(creator, workspace, true).await.unwrap();
    let own = text(&created.app.unwrap().id).to_owned();
    let (key, _) = app.app_in(owner, workspace, true).await;
    let rocket = || glyph("rocket", "");

    // when
    let by_member = app.set_app_icon(member, &key.app, rocket()).await;
    let by_creator = app.set_app_icon(creator, &own, rocket()).await;
    let by_itself = app.set_app_icon(key.actor(), &key.app, rocket()).await;
    let unknown = app.set_app_icon(owner, UNKNOWN_ID, rocket()).await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert_code(by_creator, Code::PermissionDenied);
    assert_code(by_itself, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// A delete of an App deletes the image of its icon.
#[sqlx::test(migrations = false)]
async fn delete_app_deletes_the_image_of_its_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let icon = app.set_app_icon(owner, &key.app, image(png(8, 8))).await;
    let image = image_id(icon.unwrap().icon);

    // when
    app.delete_app(owner, &key.app).await.unwrap();

    // then
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}
