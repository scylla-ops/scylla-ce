//! The areas of the catalog on the whole instance: only a role of the platform level holds them.

use super::{Area, CatalogPermission, Level, Route};
use crate::identity::admins::UserAdminHolder;
use crate::platform::SettingsHolder;
use crate::tenancy::roles::RoleAdmin;
use crate::tenancy::workspaces::WorkspaceCreation;

/// The areas of this level.
pub(super) const AREAS: &[Area] = &[
    Area {
        id: "users",
        level: Level::Platform,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "users.view",
                action: "view",
                depends_on: &[],
                implicit: false,
                routes: &[
                    Route::of(UserAdminHolder::LIST_USERS),
                    Route::of(UserAdminHolder::GET_USER),
                    Route::of(UserAdminHolder::LIST_USER_SESSIONS),
                ],
            },
            CatalogPermission {
                id: "users.create",
                action: "create",
                depends_on: &[],
                implicit: false,
                routes: &[Route::of(UserAdminHolder::CREATE_USER)],
            },
            CatalogPermission {
                id: "users.edit",
                action: "edit",
                depends_on: &["users.view"],
                implicit: false,
                routes: &[
                    Route::of(UserAdminHolder::UPDATE_USER),
                    Route::of(UserAdminHolder::SET_USER_STATUS),
                    Route::of(UserAdminHolder::SEND_PASSWORD_RESET),
                    Route::of(UserAdminHolder::REVOKE_USER_SESSIONS),
                ],
            },
            CatalogPermission {
                id: "users.delete",
                action: "delete",
                depends_on: &["users.view"],
                implicit: false,
                routes: &[Route::of(UserAdminHolder::DELETE_USER)],
            },
        ],
    },
    Area {
        id: "workspaces",
        level: Level::Platform,
        machine: false,
        permissions: &[
            CatalogPermission {
                id: "workspaces.view",
                action: "view",
                depends_on: &["workspace.view"],
                implicit: false,
                routes: &[],
            },
            CatalogPermission {
                id: "workspaces.create",
                action: "create",
                depends_on: &[],
                implicit: false,
                routes: &[Route::of(WorkspaceCreation::CREATE_WORKSPACE)],
            },
        ],
    },
    Area {
        id: "instance_roles",
        level: Level::Platform,
        machine: false,
        permissions: &[CatalogPermission {
            id: "instance_roles.manage",
            action: "manage",
            depends_on: &[],
            implicit: false,
            routes: &[
                Route::of(RoleAdmin::LIST_ROLES),
                Route::of(RoleAdmin::GET_ROLE),
                Route::of(RoleAdmin::CREATE_ROLE),
                Route::of(RoleAdmin::UPDATE_ROLE),
                Route::of(RoleAdmin::DELETE_ROLE),
            ],
        }],
    },
    Area {
        id: "instance_access",
        level: Level::Platform,
        machine: false,
        permissions: &[CatalogPermission {
            id: "instance_access.manage",
            action: "manage",
            depends_on: &[],
            implicit: false,
            routes: &[
                Route::of(RoleAdmin::LIST_ROLES),
                Route::of(RoleAdmin::GET_ROLE),
                Route::of(RoleAdmin::GRANT),
                Route::of(RoleAdmin::REVOKE),
            ],
        }],
    },
    Area {
        id: "instance_settings",
        level: Level::Platform,
        machine: false,
        permissions: &[CatalogPermission {
            id: "instance_settings.manage",
            action: "manage",
            depends_on: &[],
            implicit: false,
            routes: &[
                Route::of(SettingsHolder::GET_INSTANCE_SETTINGS),
                Route::of(SettingsHolder::UPDATE_INSTANCE_SETTINGS),
            ],
        }],
    },
];
