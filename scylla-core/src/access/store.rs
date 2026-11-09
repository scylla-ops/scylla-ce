//! The SQL of the gate: the token revocations, the owners of the workspaces, the instance admins
//! and the roles that are rows.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::ApiTokenId;
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::users::SessionId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Tx};

/// The facts of a token that can stop it.
pub(super) struct TokenFacts<'a> {
    /// The revocation ids of the token.
    pub(super) ids: &'a [Vec<u8>],
    /// The actor of the token.
    pub(super) actor: ActorId,
    /// `true` if the actor is a user: it must have a row in `users`.
    pub(super) user: bool,
    /// `true` if the actor is an App: it must have a row in `apps`.
    pub(super) app: bool,
    /// The session of the token.
    pub(super) session: Option<SessionId>,
    /// The App key of the token.
    pub(super) key: Option<AppKeyId>,
    /// The API token that gave the token.
    pub(super) api_token: Option<ApiTokenId>,
}

/// Returns `true` if a token cannot pass: one of its revocation ids is revoked, its session is
/// revoked or expired, its user is disabled or deleted, its App is deleted or disabled, its
/// key is revoked, its API token is revoked or expired, or it is a personal token and the
/// instance refuses them.
pub(super) async fn is_refused(
    tx: &mut Tx<'_>,
    facts: &TokenFacts<'_>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT (
               EXISTS (SELECT 1 FROM revoked_tokens WHERE revocation_id = ANY($1))
               OR EXISTS (SELECT 1 FROM users WHERE id = $2 AND status = 'disabled')
               OR ($3 AND NOT EXISTS (SELECT 1 FROM users WHERE id = $2))
               OR ($4::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM sessions
                   WHERE id = $4 AND revoked_at IS NULL AND expires_at > now()))
               OR ($5 AND NOT EXISTS (SELECT 1 FROM apps WHERE id = $2 AND disabled_at IS NULL))
               OR ($6::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM app_keys
                   WHERE id = $6 AND app_id = $2 AND revoked_at IS NULL))
               OR ($7::uuid IS NOT NULL AND NOT EXISTS (
                   SELECT 1 FROM api_tokens
                   WHERE id = $7 AND actor_id = $2 AND revoked_at IS NULL AND expires_at > now()))
               OR ($7::uuid IS NOT NULL AND $3
                   AND NOT (SELECT personal_tokens_enabled FROM instance_settings))
           ) AS "refused!""#,
        facts.ids,
        facts.actor.to_db(),
        facts.user,
        facts.session.map(SessionId::to_db),
        facts.app,
        facts.key.map(AppKeyId::to_db),
        facts.api_token.map(ApiTokenId::to_db),
    )
    .fetch_one(&mut **tx)
    .await
}

/// Revokes a revocation id. A token that has it never passes again.
pub async fn revoke(tx: &mut Tx<'_>, id: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO revoked_tokens (revocation_id) VALUES ($1) ON CONFLICT DO NOTHING",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns `true` if the actor is one of the owners of the workspace. An unknown workspace has
/// no owner.
pub(super) async fn is_owner(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    actor: ActorId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM workspace_owners WHERE workspace_id = $1 AND user_id = $2
           ) AS "owner!""#,
        workspace.to_db(),
        actor.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
}

/// Returns `true` if the actor is an admin of the instance.
pub(super) async fn is_admin(tx: &mut Tx<'_>, actor: ActorId) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM instance_admins WHERE user_id = $1) AS "admin!""#,
        actor.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
}

/// Returns the ids of the permissions that the roles that are rows give to an actor on the
/// instance, on a workspace and on a project, each with the kind of its scope.
pub(super) async fn held_in_rows(
    tx: &mut Tx<'_>,
    actor: ActorId,
    workspace: Option<WorkspaceId>,
    project: Option<ProjectId>,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT DISTINCT g.scope_kind, p.permission
         FROM role_grants g
         JOIN role_permissions p ON p.role_id = g.role_id
         WHERE g.actor_id = $1
           AND (g.scope_kind = 'platform'
                OR (g.scope_kind = 'workspace' AND g.scope_id = $2)
                OR (g.scope_kind = 'project' AND g.scope_id = $3))",
        actor.to_db(),
        workspace.map(WorkspaceId::to_db),
        project.map(ProjectId::to_db),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.scope_kind, row.permission))
        .collect())
}

/// Returns the ids of the objects of a kind where a role that is a row gives an actor one of the
/// permissions of the catalog.
pub(super) async fn objects_in_rows(
    tx: &mut Tx<'_>,
    actor: ActorId,
    kind: &str,
    permissions: &[&str],
) -> Result<Vec<String>, sqlx::Error> {
    let permissions: Vec<String> = permissions.iter().map(|id| (*id).to_owned()).collect();
    let rows = sqlx::query_scalar!(
        "SELECT DISTINCT g.scope_id
         FROM role_grants g
         JOIN role_permissions p ON p.role_id = g.role_id
         WHERE g.actor_id = $1 AND g.scope_kind = $2 AND p.permission = ANY($3)",
        actor.to_db(),
        kind,
        &permissions,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|id| ProjectId::from_db(id).to_string())
        .collect())
}

/// Logs a database failure of the gate, and hides its details from the client.
pub(super) fn failed(error: sqlx::Error) -> AccessError {
    tracing::error!(%error, "the access store failed");
    AccessError::Unavailable("access check unavailable".to_owned())
}

/// Returns `true` if a workspace refuses the personal tokens. An unknown workspace refuses none.
pub(super) async fn refuses_personal_tokens(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM workspaces WHERE id = $1 AND NOT personal_tokens_enabled
           ) AS "refused!""#,
        workspace.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
}
