//! The test of the pages: a role that opens a page or an action can call each route of it.

use super::{Route, PAGES};
use crate::tenancy::catalog::{areas, complete, find, Level, VIEW_ALL_WORKSPACES};

/// Returns `true` if two routes are the same permission on the same definition.
fn same(one: &Route, other: &Route) -> bool {
    one.definition == other.definition && one.permission == other.permission
}

/// A role with one permission and what it needs can use each page and action that it opens. A
/// role of the instance with a permission of a workspace or of a project also finds each workspace.
#[test]
fn a_role_that_opens_a_page_calls_each_route_of_the_page() {
    let roles = [
        (Level::Platform, Level::Platform),
        (Level::Platform, Level::Workspace),
        (Level::Platform, Level::Project),
        (Level::Workspace, Level::Workspace),
        (Level::Workspace, Level::Project),
        (Level::Project, Level::Project),
    ];
    for (level, of) in roles {
        for permission in areas()
            .filter(|area| area.level == of && !area.machine)
            .flat_map(|area| area.permissions)
        {
            // given
            let held = complete(level, false, &[permission.id.to_owned()]).expect("valid");
            let routes: Vec<&Route> = held
                .iter()
                .filter_map(|id| find(id))
                .flat_map(|(_, cell)| cell.routes)
                .collect();
            let holds = |route: &Route| routes.iter().any(|held| same(held, route));

            // when
            let missing: Vec<String> = PAGES
                .iter()
                .filter(|page| holds(&page.opens))
                .flat_map(|page| {
                    page.needs
                        .iter()
                        .filter(|route| !holds(route))
                        .map(move |route| format!("{}: {}", page.name, route.permission))
                })
                .collect();
            let in_a_workspace = level == Level::Platform && of != Level::Platform;
            let finds_workspaces = !in_a_workspace || held.contains(&VIEW_ALL_WORKSPACES);

            // then
            assert!(
                missing.is_empty(),
                "a {} role with {} misses {missing:?}",
                level.as_str(),
                permission.id
            );
            assert!(
                finds_workspaces,
                "a platform role with {} does not find the workspaces",
                permission.id
            );
        }
    }
}
