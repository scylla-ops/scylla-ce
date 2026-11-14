//! Unit tests of the catalog.

use super::{
    areas, complete, covering, find, giving, held, needs, permissions_of, Level, PermissionError,
};

/// A role holds at most 44, 35 or 21 permissions, and the machine area is apart.
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
    assert_eq!(counts, [44, 35, 21]);
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

/// A role of a level can hold each dependency of a permission, and a person can hold each
/// dependency of a permission for people.
#[test]
fn each_dependency_fits_the_level_and_the_holders_of_its_permission() {
    for area in areas() {
        for permission in area.permissions {
            for dependency in permission.depends_on {
                let (other, _) = find(dependency).expect("in the catalog");
                assert!(
                    area.level.covers(other.level),
                    "{dependency} is above the level of {}",
                    permission.id
                );
                assert!(
                    area.machine || !other.machine,
                    "{dependency} is for apps only, and {} is not",
                    permission.id
                );
            }
        }
    }
}

/// The server adds what a permission depends on, what these depend on, and the implicit
/// permissions of the level.
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
        [
            "workspace.view",
            "project.view",
            "pipelines.view",
            "pipelines.edit"
        ]
    );
    assert_eq!(
        on_project,
        ["project.view", "pipelines.view", "pipelines.edit"]
    );
}

/// The edit of the repository, the approval of a job and the manage of the connections bring
/// what they need, in the order of the catalog.
#[test]
fn complete_adds_what_the_git_integration_needs() {
    // given
    let repository = ["repository.edit".to_owned()];
    let approve = ["jobs.approve".to_owned()];
    let connections = ["integrations.manage".to_owned()];

    // when
    let on_project = complete(Level::Project, false, &repository).expect("valid");
    let approver = complete(Level::Project, false, &approve).expect("valid");
    let on_workspace = complete(Level::Workspace, false, &connections).expect("valid");

    // then
    assert_eq!(
        on_project,
        [
            "project.view",
            "pipelines.view",
            "repository.view",
            "repository.edit"
        ]
    );
    assert_eq!(
        approver,
        [
            "project.view",
            "pipelines.view",
            "jobs.view",
            "jobs.approve"
        ]
    );
    assert_eq!(
        on_workspace,
        ["workspace.view", "integrations.view", "integrations.manage"]
    );
}

/// The view of a project and the view of its pipelines need each other, and `needs` stops.
#[test]
fn needs_follows_the_dependencies_through_a_cycle() {
    // given
    let id = "job_logs.view";

    // when
    let found = needs(id);

    // then
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        [
            "job_logs.view",
            "jobs.view",
            "pipelines.view",
            "project.view"
        ]
    );
}

/// A role of the platform with a permission of a workspace or of a project sees each workspace. A
/// role of the platform with permissions of the instance only does not.
#[test]
fn complete_shows_each_workspace_to_a_role_of_the_platform_that_acts_in_one() {
    // given
    let in_a_project = ["pipelines.view".to_owned()];
    let in_a_workspace = ["apps.view".to_owned()];
    let on_the_instance = ["users.view".to_owned()];

    // when
    let project = complete(Level::Platform, false, &in_a_project).expect("valid");
    let workspace = complete(Level::Platform, false, &in_a_workspace).expect("valid");
    let instance = complete(Level::Platform, false, &on_the_instance).expect("valid");

    // then
    assert_eq!(
        project,
        [
            "workspaces.view",
            "workspace.view",
            "project.view",
            "pipelines.view"
        ]
    );
    assert_eq!(
        workspace,
        ["workspaces.view", "workspace.view", "apps.view"]
    );
    assert_eq!(instance, ["users.view"]);
}

/// A stored role holds what its permissions need now, also when it was saved before a new
/// dependency. An id that the catalog does not know gives nothing.
#[test]
fn held_adds_what_stored_permissions_need() {
    // given
    let stored = ["secrets.view", "workspaces.create"];

    // when
    let found = held(Level::Project, stored);

    // then
    assert_eq!(found, ["project.view", "pipelines.view", "secrets.view"]);
}

/// A route of the view of a project comes from each permission that needs this view, and not from
/// a permission that does not need it.
#[test]
fn giving_finds_each_permission_that_needs_an_id() {
    // given
    let ids = ["project.view"];

    // when
    let found = giving(Level::Project, &ids);

    // then
    assert!(found.contains(&"project.view"));
    assert!(found.contains(&"secrets.view"));
    assert!(found.contains(&"job_logs.view"));
    assert!(!found.contains(&"job_execution.run"));
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
