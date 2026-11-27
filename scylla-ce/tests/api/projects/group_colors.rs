//! The tests of the color of the folder of a group of projects: `CreateProjectGroup` and
//! `UpdateProjectGroup` with a color.

use scylla_testkit::text;
use sqlx::PgPool;

use super::cast::{passed, Cast};
use crate::{app, assert_invalid_field};

/// A group keeps a color of the theme as it is, and a hex color in lower case. The list shows it.
#[sqlx::test(migrations = false)]
async fn create_project_group_keeps_its_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let plain = app.create_project_group(owner, workspace, "Plain").await;
    let theme = app.create_colored_group(owner, workspace, "Theme", "chart-2");
    let theme = theme.await;
    let free = app.create_colored_group(owner, workspace, "Free", "#A1B2C3");
    let free = free.await;
    let groups = app.list_project_groups(owner, workspace).await.unwrap();

    // then
    assert_eq!(plain.unwrap().color, "");
    assert_eq!(theme.unwrap().color, "chart-2");
    assert_eq!(free.unwrap().color, "#a1b2c3");
    let colors: Vec<_> = groups.iter().map(|group| group.color.as_str()).collect();
    assert_eq!(colors, ["", "chart-2", "#a1b2c3"]);
}

/// A new color replaces the old one, an empty color gives the default color again, and a change
/// of the name keeps the color.
#[sqlx::test(migrations = false)]
async fn update_project_group_changes_and_resets_the_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let group = app.create_colored_group(owner, workspace, "G", "chart-3");
    let group = text(&group.await.unwrap().id).to_owned();

    // when
    let colored = app.set_group_color(owner, &group, "#00FF00").await.unwrap();
    let renamed = app.update_project_group(owner, &group, Some("H"), None);
    let renamed = renamed.await.unwrap();
    let reset = app.set_group_color(owner, &group, "").await.unwrap();

    // then
    assert_eq!(colored.color, "#00ff00");
    assert_eq!(
        (renamed.name.as_str(), renamed.color.as_str()),
        ("H", "#00ff00")
    );
    assert_eq!(reset.color, "");
    let groups = app.list_project_groups(owner, workspace).await.unwrap();
    assert_eq!(groups, [reset]);
}

/// A color that is not a hex color or a color name gives `INVALID_ARGUMENT` on the color, and
/// the group keeps its color.
#[sqlx::test(migrations = false)]
async fn the_routes_of_the_groups_refuse_a_bad_color(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let group = app.create_colored_group(owner, workspace, "G", "chart-4");
    let group = text(&group.await.unwrap().id).to_owned();
    let bad = ["#12345", "#gggggg", "Blue", "1blue", "bl ue"];

    // when
    let mut refusals = Vec::new();
    for color in bad {
        refusals.push(app.create_colored_group(owner, workspace, "X", color).await);
        refusals.push(app.set_group_color(owner, &group, color).await);
    }

    // then
    for refusal in refusals {
        assert_invalid_field(&refusal.unwrap_err(), "color");
    }
    let groups = app.list_project_groups(owner, workspace).await.unwrap();
    let colors: Vec<_> = groups.iter().map(|group| group.color.as_str()).collect();
    assert_eq!(colors, ["chart-4"]);
}

/// Only the roles that change the groups change the color of a group.
#[sqlx::test(migrations = false)]
async fn update_project_group_color_needs_a_role_that_works(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let workspace = app.workspace_of(cast.owner()).await;
    let group = app.create_project_group(cast.owner(), workspace, "G").await;
    let group = text(&group.unwrap().id).to_owned();

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        results.push((role, app.set_group_color(actor, &group, "chart-5").await));
    }

    // then
    assert_eq!(passed(results), ["owner", "workspace admin", "member"]);
}
