//! Unit tests of the catalog.

use super::{areas, complete, covering, find, permissions_of, Level, PermissionError};

/// A role holds at most 38, 30 or 18 permissions, and the machine area is apart.
#[test]
fn the_catalog_has_the_counts_of_the_model() {
    // given
    let human = |level: Level| {
        areas()
            .filter(|area| !area.machine && level.covers(area.level))
            .map(|area| area.permissions.len())
            .sum::<usize>()
    };

    // when
    let counts = [Level::Platform, Level::Workspace, Level::Project].map(human);

    // then
    assert_eq!(counts, [38, 30, 18]);
}

/// Each dependency names a permission of the catalog, and each id has its area in front.
#[test]
fn each_id_fits_its_area_and_each_dependency_exists() {
    for area in areas() {
        for permission in area.permissions {
            assert_eq!(
                permission.id,
                format!("{}.{}", area.id, permission.action),
                "the id of a permission"
            );
            for dependency in permission.depends_on {
                assert!(
                    find(dependency).is_some(),
                    "{dependency} is not in the catalog"
                );
            }
        }
    }
}

/// A route on a project gives the same permission as the route on its workspace.
#[test]
fn covering_finds_a_route_on_each_definition() {
    assert_eq!(covering("project", "get_project"), ["project.view"]);
    assert_eq!(covering("workspace", "get_project"), ["project.view"]);
    assert!(covering("platform", "get_project").is_empty());
}

/// The server adds what a permission depends on and the implicit permissions of the level.
#[test]
fn complete_adds_the_dependencies_and_the_implicit_permissions() {
    // given
    let ids = ["pipelines.edit".to_owned()];

    // when
    let on_workspace = complete(Level::Workspace, false, &ids).expect("valid");
    let on_project = complete(Level::Project, false, &ids).expect("valid");

    // then
    assert_eq!(
        on_workspace,
        ["workspace.view", "pipelines.view", "pipelines.edit"]
    );
    assert_eq!(on_project, ["pipelines.view", "pipelines.edit"]);
}

/// A permission above the level, not in the catalog, or for apps only is refused.
#[test]
fn complete_refuses_a_permission_that_does_not_fit() {
    // given
    let above = ["users.view".to_owned()];
    let unknown = ["pipelines.fly".to_owned()];
    let machine = ["job_execution.run".to_owned()];

    // when
    let results = [
        complete(Level::Workspace, false, &above),
        complete(Level::Project, false, &unknown),
        complete(Level::Workspace, false, &machine),
    ];

    // then
    assert_eq!(results[0], Err(PermissionError::AboveLevel("users.view")));
    assert_eq!(results[1], Err(PermissionError::Unknown("pipelines.fly")));
    assert_eq!(
        results[2],
        Err(PermissionError::Holder("job_execution.run"))
    );
}

/// A role for apps holds the area of the machines on a workspace and on a project.
#[test]
fn complete_gives_the_area_of_the_machines_to_a_role_for_apps() {
    // given
    let ids = ["job_execution.run".to_owned()];

    // when
    let on_workspace = complete(Level::Workspace, true, &ids).expect("valid");
    let on_project = complete(Level::Project, true, &ids).expect("valid");

    // then
    assert_eq!(
        on_workspace,
        ["workspace.view", "job_execution.edit", "job_execution.run"]
    );
    assert_eq!(on_project, ["job_execution.edit", "job_execution.run"]);
}

/// A role of the project level sees only the areas of a project.
#[test]
fn permissions_of_a_level_stop_at_that_level() {
    assert!(permissions_of(Level::Project).all(|permission| !permission.id.starts_with("apps")));
    assert!(permissions_of(Level::Workspace).any(|permission| permission.id == "apps.view"));
}
