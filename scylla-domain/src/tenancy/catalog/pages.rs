//! The pages and the actions of the web app, with the routes that each one calls with no other
//! check. A test holds the catalog to them: a role that opens a page can use it.

use super::{areas, complete, find, Level, Route, VIEW_ALL_WORKSPACES};
use crate::identity::admins::UserAdminHolder;
use crate::identity::apps::AppHolder;
use crate::projects::pipelines::PipelineAccess;
use crate::projects::triggers::TriggerAccess;
use crate::projects::{Project, ProjectHolder};
use crate::tenancy::grants::{ProjectMembers, WorkspaceMembers};
use crate::tenancy::roles::{RoleAdmin, RoleHolder};
use crate::tenancy::workspaces::Workspace;

/// A page or an action of the web app.
struct Page {
    /// The name of the page or of the action.
    name: &'static str,
    /// The route that the web app checks before it shows the page or the action.
    opens: Route,
    /// The routes that the page or the action calls with no other check.
    needs: &'static [Route],
}

/// The routes that find a project and open its first page, the list of its pipelines.
const A_PROJECT: [Route; 3] = [
    Route::of(Project::LIST_PROJECTS),
    Route::of(Project::GET_PROJECT),
    Route::of(PipelineAccess::LIST_PROJECT_PIPELINES),
];

/// The pages and the actions, from the instance down to a project.
const PAGES: &[Page] = &[
    Page {
        name: "the new user of the users page",
        opens: Route::of(UserAdminHolder::CREATE_USER),
        needs: &[Route::of(UserAdminHolder::LIST_USERS)],
    },
    Page {
        name: "the roles of a user, on the users page",
        opens: Route::of(RoleAdmin::GRANT),
        needs: &[
            Route::of(UserAdminHolder::LIST_USERS),
            Route::of(RoleAdmin::LIST_ROLES),
        ],
    },
    Page {
        name: "the members of a workspace",
        opens: Route::of(Workspace::LIST_ACCESS),
        needs: &[
            Route::of(WorkspaceMembers::LIST_MEMBERS),
            Route::of(WorkspaceMembers::GET_MEMBER_ACCESS),
            Route::of(ProjectMembers::GET_MEMBER_ACCESS),
            Route::of(Project::LIST_ACCESS),
        ],
    },
    Page {
        name: "a new member or the removal of a member",
        opens: Route::of(Workspace::GRANT),
        needs: &[
            Route::of(Workspace::REVOKE),
            Route::of(Workspace::LIST_INVITATIONS),
            Route::of(Project::GRANT),
            Route::of(Project::REVOKE),
        ],
    },
    Page {
        name: "the delete of a role of the workspace",
        opens: Route::of(RoleHolder::DELETE_ROLE),
        needs: &[
            Route::of(RoleHolder::LIST_ROLES),
            Route::of(RoleHolder::LIST_ROLE_HOLDERS),
        ],
    },
    Page {
        name: "the new project of the projects page",
        opens: Route::of(ProjectHolder::CREATE_PROJECT),
        needs: &[Route::of(ProjectHolder::LIST_PROJECTS)],
    },
    Page {
        name: "the activity of the agents",
        opens: Route::of(AppHolder::LIST_AGENT_STATS),
        needs: &[Route::of(AppHolder::LIST_APPS)],
    },
    Page {
        name: "a new app or agent",
        opens: Route::of(AppHolder::CREATE_APP),
        needs: &[
            Route::of(AppHolder::LIST_APPS),
            Route::of(AppHolder::GET_APP),
        ],
    },
    Page {
        name: "the home of a project",
        opens: Route::of(Project::LIST_PROJECTS),
        needs: &A_PROJECT,
    },
    Page {
        name: "the members of a project",
        opens: Route::of(Project::LIST_ACCESS),
        needs: &A_PROJECT,
    },
    Page {
        name: "the new pipeline",
        opens: Route::of(PipelineAccess::CREATE_PIPELINE),
        needs: &[Route::of(PipelineAccess::GET_PIPELINE)],
    },
    Page {
        name: "the triggers of a pipeline",
        opens: Route::of(TriggerAccess::LIST_PIPELINE_TRIGGERS),
        needs: &A_PROJECT,
    },
    Page {
        name: "the jobs of a pipeline",
        opens: Route::of(PipelineAccess::LIST_PIPELINE_JOBS),
        needs: &[
            Route::of(PipelineAccess::GET_JOB),
            Route::of(PipelineAccess::LIST_PROJECT_PIPELINES),
        ],
    },
    Page {
        name: "the logs of a job",
        opens: Route::of(PipelineAccess::TAIL_JOB_LOGS),
        needs: &[Route::of(PipelineAccess::GET_JOB)],
    },
    Page {
        name: "the secrets of a project",
        opens: Route::of(Project::LIST_SECRETS),
        needs: &A_PROJECT,
    },
    Page {
        name: "a new secret",
        opens: Route::of(Project::CREATE_SECRET),
        needs: &[Route::of(Project::LIST_SECRETS)],
    },
];

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
