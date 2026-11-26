//! The pages and the actions of the web app, with the routes that each one calls with no other
//! check. A test holds the catalog to them: a role that opens a page can use it.

use super::Route;
use crate::identity::admins::UserAdminHolder;
use crate::identity::apps::AppHolder;
use crate::integrations::apps::VcsAppHolder;
use crate::integrations::connections::VcsConnectionHolder;
use crate::integrations::links::{RepositoryAccess, RepositoryHolder};
use crate::platform::legal::LegalHolder;
use crate::platform::mail::MailSettingsHolder;
use crate::platform::SettingsHolder;
use crate::projects::pipelines::PipelineAccess;
use crate::projects::triggers::TriggerAccess;
use crate::projects::{Project, ProjectHolder};
use crate::tenancy::dashboards::DashboardHolder;
use crate::tenancy::grants::{ProjectMembers, WorkspaceMembers};
use crate::tenancy::library::FragmentHolder;
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
        name: "the legal and privacy page of the instance",
        opens: Route::of(SettingsHolder::UPDATE_INSTANCE_SETTINGS),
        needs: &[Route::of(LegalHolder::UPDATE_LEGAL_SETTINGS)],
    },
    Page {
        name: "the mail page of the instance",
        opens: Route::of(SettingsHolder::UPDATE_INSTANCE_SETTINGS),
        needs: &[
            Route::of(MailSettingsHolder::GET_MAIL_SETTINGS),
            Route::of(MailSettingsHolder::UPDATE_MAIL_SETTINGS),
            Route::of(MailSettingsHolder::RESET_MAIL_SETTINGS),
            Route::of(MailSettingsHolder::SEND_TEST_MAIL),
        ],
    },
    Page {
        name: "the Git providers of the instance",
        opens: Route::of(VcsAppHolder::GET_VCS_APP),
        needs: &[Route::of(VcsAppHolder::START_GIT_HUB_MANIFEST)],
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
        name: "the library of the workspace and the page of a fragment",
        opens: Route::of(FragmentHolder::LIST_FRAGMENTS),
        needs: &[
            Route::of(FragmentHolder::GET_FRAGMENT),
            Route::of(FragmentHolder::SET_FRAGMENT_FAVORITE),
        ],
    },
    Page {
        name: "the save of nodes of the editor to the library",
        opens: Route::of(FragmentHolder::CREATE_FRAGMENT),
        needs: &[Route::of(FragmentHolder::LIST_FRAGMENTS)],
    },
    Page {
        name: "the saved layouts of the dashboard",
        opens: Route::of(DashboardHolder::LIST_DASHBOARDS),
        needs: &[Route::of(DashboardHolder::GET_DASHBOARD)],
    },
    Page {
        name: "a new layout of the dashboard",
        opens: Route::of(DashboardHolder::CREATE_DASHBOARD),
        needs: &[
            Route::of(DashboardHolder::LIST_DASHBOARDS),
            Route::of(DashboardHolder::GET_DASHBOARD),
        ],
    },
    Page {
        name: "the change of a shared layout and the team default",
        opens: Route::of(DashboardHolder::UPDATE_DASHBOARD),
        needs: &[
            Route::of(DashboardHolder::LIST_DASHBOARDS),
            Route::of(DashboardHolder::GET_DASHBOARD),
            Route::of(DashboardHolder::DELETE_DASHBOARD),
            Route::of(DashboardHolder::SET_TEAM_DEFAULT_DASHBOARD),
        ],
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
        name: "the integrations of a workspace",
        opens: Route::of(VcsConnectionHolder::GET_VCS_PROVIDERS),
        needs: &[
            Route::of(VcsConnectionHolder::LIST_VCS_CONNECTIONS),
            Route::of(VcsConnectionHolder::GET_VCS_CONNECTION),
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
        name: "the export of a pipeline",
        opens: Route::of(PipelineAccess::GET_PIPELINE),
        needs: &[Route::of(PipelineAccess::EXPORT_PIPELINE)],
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
        name: "the reports of a job",
        opens: Route::of(PipelineAccess::GET_JOB),
        needs: &[Route::of(PipelineAccess::LIST_JOB_REPORTS)],
    },
    Page {
        name: "the approval of a job from a fork",
        opens: Route::of(PipelineAccess::APPROVE_JOB),
        needs: &[
            Route::of(PipelineAccess::REJECT_JOB),
            Route::of(PipelineAccess::GET_JOB),
        ],
    },
    Page {
        name: "the repository of a project",
        opens: Route::of(RepositoryAccess::GET_REPOSITORY_LINK),
        needs: &[
            Route::of(RepositoryAccess::LIST_REFS),
            Route::of(RepositoryAccess::LIST_MERGE_BLOCKS),
            Route::of(PipelineAccess::LIST_PIPELINE_FILES),
        ],
    },
    Page {
        name: "the link of a repository",
        opens: Route::of(RepositoryAccess::LINK_REPOSITORY),
        needs: &[
            Route::of(VcsConnectionHolder::LIST_VCS_CONNECTIONS),
            Route::of(RepositoryAccess::LIST_CONNECTION_REPOSITORIES),
        ],
    },
    Page {
        name: "a new project linked to a repository",
        opens: Route::of(RepositoryHolder::LINK_REPOSITORY),
        needs: &[
            Route::of(VcsConnectionHolder::LIST_VCS_CONNECTIONS),
            Route::of(RepositoryHolder::LIST_CONNECTION_REPOSITORIES),
        ],
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

mod tests;
