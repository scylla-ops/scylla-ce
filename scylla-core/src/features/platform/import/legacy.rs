//! The reads of the database of the old product. Its schema is not the schema of this base, so
//! the queries are checked at run time only.

use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::Error;

/// A user of the old product.
pub(super) struct OldUser {
    /// The ULID.
    pub(super) id: String,
    /// The unique user name.
    pub(super) username: String,
    /// The PHC hash of the password.
    pub(super) password_hash: String,
    /// `false` for a disabled user.
    pub(super) is_active: bool,
    /// The email, if the user had one.
    pub(super) email: Option<String>,
    /// The display name, if the user had one.
    pub(super) display_name: Option<String>,
}

/// A grant of the old product, with its role.
pub(super) struct OldGrant {
    /// `user` or `app`.
    pub(super) principal_kind: String,
    /// The ULID of the principal.
    pub(super) principal_id: String,
    /// The key of a built-in role, for example `organization-admin`.
    pub(super) role_key: Option<String>,
    /// `admin`, `member` or `agent`.
    pub(super) role_kind: String,
    /// `system`, `organization` or `project`.
    pub(super) scope_kind: String,
    /// The ULID of the scope.
    pub(super) scope_id: String,
}

/// An organization of the old product.
pub(super) struct OldOrganization {
    /// The ULID.
    pub(super) id: String,
    /// The name.
    pub(super) name: String,
}

/// A project of the old product.
pub(super) struct OldProject {
    /// The ULID.
    pub(super) id: String,
    /// The name.
    pub(super) name: String,
    /// The description.
    pub(super) description: Option<String>,
    /// The ULID of its organization.
    pub(super) organization_id: String,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
}

/// A secret of a project of the old product.
pub(super) struct OldSecret {
    /// The ULID.
    pub(super) id: String,
    /// The ULID of its project.
    pub(super) project_id: String,
    /// The name.
    pub(super) name: String,
    /// The value: a nonce of 24 bytes, then the ciphertext, with no associated data.
    pub(super) encrypted_value: Vec<u8>,
}

/// A pipeline of the old product.
pub(super) struct OldPipeline {
    /// The ULID.
    pub(super) id: String,
    /// The ULID of its project.
    pub(super) project_id: String,
    /// The name.
    pub(super) name: String,
    /// The nodes, in the JSON of the old product.
    pub(super) nodes: Value,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
}

/// Each row that the import reads.
pub(super) struct Legacy {
    /// The users, oldest first.
    pub(super) users: Vec<OldUser>,
    /// The grants, oldest first.
    pub(super) grants: Vec<OldGrant>,
    /// The organizations, oldest first.
    pub(super) organizations: Vec<OldOrganization>,
    /// The projects, oldest first.
    pub(super) projects: Vec<OldProject>,
    /// The secrets.
    pub(super) secrets: Vec<OldSecret>,
    /// The pipelines, oldest first.
    pub(super) pipelines: Vec<OldPipeline>,
}

/// Reads the rows of the old database.
pub(super) async fn read(pool: &PgPool) -> Result<Legacy, Error> {
    let users = sqlx::query_as::<_, (String, String, String, bool, Option<String>, Option<String>)>(
        "SELECT id, username, password_hash, is_active, email, display_name FROM users ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await?;
    let grants = sqlx::query_as::<_, (String, String, Option<String>, String, String, String)>(
        "SELECT g.principal_kind, g.principal_id, r.key, r.kind, g.scope_kind, g.scope_id
         FROM grants g JOIN roles r ON r.id = g.role_id ORDER BY g.created_at, g.id",
    )
    .fetch_all(pool)
    .await?;
    let organizations = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM organizations ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await?;
    let projects = sqlx::query_as::<_, (String, String, Option<String>, String, OffsetDateTime)>(
        "SELECT id, name, description, organization_id, created_at FROM projects ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await?;
    let secrets = sqlx::query_as::<_, (String, String, String, Vec<u8>)>(
        "SELECT id, project_id, name, encrypted_value FROM project_secrets ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    let pipelines = sqlx::query_as::<_, (String, String, String, Value, OffsetDateTime)>(
        "SELECT id, project_id, name, nodes, created_at FROM pipelines ORDER BY created_at, id",
    )
    .fetch_all(pool)
    .await?;
    Ok(Legacy {
        users: users
            .into_iter()
            .map(
                |(id, username, password_hash, is_active, email, display_name)| OldUser {
                    id,
                    username,
                    password_hash,
                    is_active,
                    email,
                    display_name,
                },
            )
            .collect(),
        grants: grants
            .into_iter()
            .map(
                |(principal_kind, principal_id, role_key, role_kind, scope_kind, scope_id)| {
                    OldGrant {
                        principal_kind,
                        principal_id,
                        role_key,
                        role_kind,
                        scope_kind,
                        scope_id,
                    }
                },
            )
            .collect(),
        organizations: organizations
            .into_iter()
            .map(|(id, name)| OldOrganization { id, name })
            .collect(),
        projects: projects
            .into_iter()
            .map(
                |(id, name, description, organization_id, created_at)| OldProject {
                    id,
                    name,
                    description,
                    organization_id,
                    created_at,
                },
            )
            .collect(),
        secrets: secrets
            .into_iter()
            .map(|(id, project_id, name, encrypted_value)| OldSecret {
                id,
                project_id,
                name,
                encrypted_value,
            })
            .collect(),
        pipelines: pipelines
            .into_iter()
            .map(|(id, project_id, name, nodes, created_at)| OldPipeline {
                id,
                project_id,
                name,
                nodes,
                created_at,
            })
            .collect(),
    })
}
