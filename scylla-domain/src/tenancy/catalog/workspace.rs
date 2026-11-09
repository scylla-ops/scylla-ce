//! The areas of the catalog in a workspace.

use super::{Area, CatalogPermission, Level, Route};
use crate::identity::apps::{App, AppHolder};
use crate::projects::ProjectHolder;
use crate::tenancy::grants::WorkspaceMembers;
use crate::tenancy::roles::RoleHolder;
use crate::tenancy::workspaces::Workspace;

/// The areas of this level.
pub(super) const AREAS: &[Area] = &[
    Area {
        id: "workspace",
        level: Level::Workspace,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "workspace.view",
                action: "view",
                depends_on: &[],
                implicit: true,
                routes: &[
                    Route::of(Workspace::GET_WORKSPACE),
                    Route::of(Workspace::LIST_WORKSPACES),
                    Route::of(RoleHolder::LIST_ROLES),
                    Route::of(RoleHolder::GET_ROLE),
                ],
            },
            CatalogPermission {
                id: "workspace.edit",
                action: "edit",
                depends_on: &["workspace.view"],
                implicit: false,
                routes: &[Route::of(Workspace::UPDATE_WORKSPACE)],
            },
            CatalogPermission {
                id: "workspace.delete",
                action: "delete",
                depends_on: &["workspace.view"],
                implicit: false,
                routes: &[Route::of(Workspace::DELETE_WORKSPACE)],
            },
        ],
    },
    Area {
        id: "members",
        level: Level::Workspace,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "members.view",
                action: "view",
                depends_on: &[],
                implicit: false,
                routes: &[
                    Route::of(Workspace::LIST_ACCESS),
                    Route::of(WorkspaceMembers::LIST_MEMBERS),
                    Route::of(WorkspaceMembers::GET_MEMBER_ACCESS),
                    Route::of(RoleHolder::LIST_ROLE_HOLDERS),
                ],
            },
            CatalogPermission {
                id: "members.manage",
                action: "manage",
                depends_on: &["members.view"],
                implicit: false,
                routes: &[
                    Route::of(Workspace::GRANT),
                    Route::of(Workspace::REVOKE),
                    Route::of(Workspace::CREATE_INVITATION),
                    Route::of(Workspace::LIST_INVITATIONS),
                    Route::of(Workspace::REVOKE_INVITATION),
                    Route::of(Workspace::RESEND_INVITATION),
                ],
            },
        ],
    },
    Area {
        id: "workspace_roles",
        level: Level::Workspace,
        machine: false,
        permissions: &[CatalogPermission {
            id: "workspace_roles.manage",
            action: "manage",
            depends_on: &[],
            implicit: false,
            routes: &[
                Route::of(RoleHolder::CREATE_ROLE),
                Route::of(RoleHolder::UPDATE_ROLE),
                Route::of(RoleHolder::DELETE_ROLE),
            ],
        }],
    },
    Area {
        id: "apps",
        level: Level::Workspace,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "apps.view",
                action: "view",
                depends_on: &[],
                implicit: false,
                routes: &[
                    Route::of(AppHolder::GET_APP),
                    Route::of(AppHolder::LIST_APPS),
                    Route::of(AppHolder::LIST_APP_TOKENS),
                    Route::of(AppHolder::LIST_CI_IDENTITIES),
                    Route::of(App::GET_APP),
                    Route::of(App::LIST_APP_TOKENS),
                    Route::of(App::LIST_CI_IDENTITIES),
                ],
            },
            CatalogPermission {
                id: "apps.create",
                action: "create",
                depends_on: &[],
                implicit: false,
                routes: &[Route::of(AppHolder::CREATE_APP)],
            },
            CatalogPermission {
                id: "apps.manage",
                action: "manage",
                depends_on: &["apps.view"],
                implicit: false,
                routes: &[
                    Route::of(AppHolder::DELETE_APP),
                    Route::of(AppHolder::DISABLE_APP),
                    Route::of(AppHolder::ENABLE_APP),
                    Route::of(AppHolder::CREATE_JOIN_CODE),
                    Route::of(AppHolder::ADD_APP_KEY),
                    Route::of(AppHolder::REVOKE_APP_KEY),
                    Route::of(AppHolder::RENAME_APP_KEY),
                    Route::of(AppHolder::CREATE_APP_TOKEN),
                    Route::of(AppHolder::REVOKE_APP_TOKEN),
                    Route::of(AppHolder::ADD_CI_IDENTITY),
                    Route::of(AppHolder::REMOVE_CI_IDENTITY),
                    Route::of(App::DELETE_APP),
                    Route::of(App::DISABLE_APP),
                    Route::of(App::ENABLE_APP),
                    Route::of(App::CREATE_JOIN_CODE),
                    Route::of(App::ADD_APP_KEY),
                    Route::of(App::REVOKE_APP_KEY),
                    Route::of(App::RENAME_APP_KEY),
                    Route::of(App::CREATE_APP_TOKEN),
                    Route::of(App::REVOKE_APP_TOKEN),
                    Route::of(App::ADD_CI_IDENTITY),
                    Route::of(App::REMOVE_CI_IDENTITY),
                ],
            },
        ],
    },
    Area {
        id: "agents",
        level: Level::Workspace,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "agents.view",
                action: "view",
                depends_on: &[],
                implicit: false,
                routes: &[
                    Route::of(AppHolder::GET_AGENT_STATS),
                    Route::of(AppHolder::LIST_AGENT_STATS),
                    Route::of(App::GET_AGENT_STATS),
                ],
            },
            CatalogPermission {
                id: "agents.create",
                action: "create",
                depends_on: &[],
                implicit: false,
                routes: &[Route::of(AppHolder::CREATE_APP)],
            },
        ],
    },
    Area {
        id: "projects",
        level: Level::Workspace,
        machine: false,
        permissions: &[CatalogPermission {
            id: "projects.create",
            action: "create",
            depends_on: &[],
            implicit: false,
            routes: &[Route::of(ProjectHolder::CREATE_PROJECT)],
        }],
    },
];
