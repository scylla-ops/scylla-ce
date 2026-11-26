//! The tests of the custom icon of a workspace: `UpdateWorkspace` with an icon, and the delete of
//! its image.

use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::common::v1::icon::Kind;
use scylla_proto::common::v1::{IconChange, IconGlyph};
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use crate::app;
use crate::assert_invalid_field;
use crate::projects::icons::{get_image, glyph, image, image_count, image_id, png};

/// The largest upload, in bytes.
const MAX_UPLOAD: usize = 1_048_576;

/// A glyph with a color comes back with the hex color in lower case, the reads show it, and a
/// rename keeps it.
#[sqlx::test(migrations = false)]
async fn update_workspace_sets_a_glyph_with_a_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let created = app.create_workspace(user, "team").await.unwrap();
    let id = text(&created.id);

    // when
    let updated = app.set_workspace_icon(user, id, glyph("rocket", "#A1B2C3"));
    let updated = updated.await.unwrap();
    let renamed = app.update_workspace(user, id, Some("crew"), None).await;

    // then
    let expected = Some(Kind::Glyph(IconGlyph {
        name: "rocket".to_owned(),
        color: "#a1b2c3".to_owned(),
    }));
    assert_eq!(created.icon, None);
    assert_eq!(updated.icon.clone().and_then(|icon| icon.kind), expected);
    let renamed = renamed.unwrap();
    assert_eq!(renamed.icon, updated.icon);
    assert_eq!(app.get_workspace(user, id).await.unwrap(), renamed);
}

/// A new image replaces the old one, which goes, and the default icon removes the last one.
#[sqlx::test(migrations = false)]
async fn update_workspace_deletes_the_old_image(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.create_workspace(user, "team").await.unwrap();
    let id = text(&workspace.id);
    let first = app.set_workspace_icon(user, id, image(png(8, 8))).await;
    let first = image_id(first.unwrap().icon);

    // when
    let second = app.set_workspace_icon(user, id, image(png(300, 200))).await;
    let second = image_id(second.unwrap().icon);
    let new_image = get_image(&app, &second).await;
    let old_image = get_image(&app, &first).await.status();
    let reset = app.set_workspace_icon(user, id, IconChange { kind: None });

    // then
    assert_eq!(new_image.status(), 200);
    let body = new_image.bytes().await.unwrap();
    let square = image::load_from_memory(&body).unwrap();
    assert_eq!((square.width(), square.height()), (256, 256));
    assert_eq!(old_image, 404);
    assert_eq!(reset.await.unwrap().icon, None);
    assert_eq!(image_count(&app).await, 0);
}

/// A bad image, a bad glyph name and a bad color give `INVALID_ARGUMENT` on their field, and the
/// workspace keeps its icon.
#[sqlx::test(migrations = false)]
async fn update_workspace_refuses_a_bad_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.create_workspace(user, "team").await.unwrap();
    let id = text(&workspace.id);
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"></svg>"#.to_vec();
    let mut large = png(64, 64);
    large.resize(MAX_UPLOAD + 1, 0);
    let cases = [
        (image(large), "icon.image"),
        (image(svg), "icon.image"),
        (image(Vec::new()), "icon.image"),
        (glyph("Rocket", ""), "icon.glyph.name"),
        (glyph("rocket", "#12345"), "icon.glyph.color"),
    ];

    // when
    let mut refusals = Vec::new();
    for (icon, field) in cases {
        refusals.push((app.set_workspace_icon(user, id, icon).await, field));
    }

    // then
    for (result, field) in refusals {
        assert_invalid_field(&result.unwrap_err(), field);
    }
    assert_eq!(app.get_workspace(user, id).await.unwrap().icon, None);
    assert_eq!(image_count(&app).await, 0);
}

/// Only the owners and the admins of the workspace change its icon, and a refusal keeps no image.
#[sqlx::test(migrations = false)]
async fn update_workspace_icon_needs_a_role_that_manages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.create_workspace(owner, "team").await.unwrap();
    let id = WorkspaceId::parse(text(&workspace.id)).unwrap();
    let (admin, member, viewer, stranger) = (
        app.user().await,
        app.user().await,
        app.user().await,
        app.user().await,
    );
    app.grant(&[
        Workspace::ADMIN.of(id, admin),
        Workspace::MEMBER.of(id, member),
        Workspace::VIEWER.of(id, viewer),
    ])
    .await;
    let actors = [owner, admin, member, viewer, stranger];

    // when
    let mut codes = Vec::new();
    for actor in actors {
        let result = app.set_workspace_icon(actor, text(&workspace.id), image(png(8, 8)));
        codes.push(result.await.err().map(|status| status.code()));
    }

    // then
    let denied = Some(Code::PermissionDenied);
    assert_eq!(codes, [None, None, denied, denied, denied]);
    assert_eq!(image_count(&app).await, 1);
}

/// A delete of a workspace deletes the image of its icon.
#[sqlx::test(migrations = false)]
async fn delete_workspace_deletes_the_image_of_its_icon(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.create_workspace(user, "team").await.unwrap();
    let id = text(&workspace.id);
    let icon = app.set_workspace_icon(user, id, image(png(8, 8))).await;
    let image = image_id(icon.unwrap().icon);

    // when
    app.delete_workspace(user, id).await.unwrap();

    // then
    assert_eq!(get_image(&app, &image).await.status(), 404);
    assert_eq!(image_count(&app).await, 0);
}

/// The console of the users gives each workspace of a user with its icon, and the first owner of
/// each workspace that the user does not own.
#[sqlx::test(migrations = false)]
async fn list_users_gives_the_icon_and_the_owner_of_each_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let owner = app.user().await;
    let member = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let personal = app.workspace_of(member).await.to_string();
    app.set_workspace_icon(owner, &workspace, glyph("rocket", "#A1B2C3"))
        .await
        .unwrap();
    let viewer = scylla_testkit::Role::on("workspace", &workspace, "viewer");
    app.grant_role(owner, viewer, &member.to_string())
        .await
        .unwrap();

    // when
    let page = scylla_proto::common::v1::PageRequest::default();
    let users = app.list_users(admin, "", page).await.unwrap().users;

    // then
    let entry = users
        .iter()
        .find(|entry| text(&entry.user.as_ref().unwrap().id) == member.to_string())
        .unwrap();
    let details = &entry.workspace_details;
    assert_eq!(details.len(), 2);
    assert_eq!(entry.workspaces.len(), 2);
    let shared = details
        .iter()
        .find(|detail| text(&detail.workspace_id) == workspace)
        .unwrap();
    let own = details
        .iter()
        .find(|detail| text(&detail.workspace_id) == personal)
        .unwrap();
    let expected = Some(Kind::Glyph(IconGlyph {
        name: "rocket".to_owned(),
        color: "#a1b2c3".to_owned(),
    }));
    assert_eq!(shared.icon.clone().and_then(|icon| icon.kind), expected);
    assert!(!shared.owner);
    assert_eq!(shared.owner_name, "Test User");
    assert_eq!(own.icon, None);
    assert!(own.owner);
    assert_eq!(own.owner_name, "");
}
