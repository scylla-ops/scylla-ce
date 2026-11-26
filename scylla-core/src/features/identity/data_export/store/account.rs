//! The reads of the account of a user for its export: the user, its emails, its identities, its
//! sessions, its personal tokens, the state of the web app and its acceptances of the terms, as
//! JSON.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::ApiTokenId;
use scylla_domain::identity::users::SessionId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use serde_json::{json, Value};

use super::time;
use crate::Error;

/// Returns the user with its password date, its admin flag and its avatar, or `None` for an
/// unknown user.
pub(crate) async fn user(tx: &mut Tx<'_>, user: ActorId) -> Result<Option<Value>, Error> {
    let row = sqlx::query!(
        r#"SELECT u.display_name, u.handle, u.status, u.created_at, u.last_sign_in_at,
                  u.avatar_image_id, u.avatar_color,
                  p.updated_at AS "password_changed_at?",
                  EXISTS (SELECT 1 FROM instance_admins a WHERE a.user_id = u.id)
                      AS "instance_admin!"
           FROM users u
           LEFT JOIN passwords p ON p.user_id = u.id
           WHERE u.id = $1"#,
        user.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        json!({
            "id": user.to_string(),
            "display_name": row.display_name,
            "handle": row.handle,
            "status": row.status,
            "created_at": time(row.created_at),
            "last_sign_in_at": row.last_sign_in_at.map(time),
            "password_changed_at": row.password_changed_at.map(time),
            "instance_admin": row.instance_admin,
            "avatar_image_id": row.avatar_image_id.map(|id| id.to_string()),
            "avatar_color": row.avatar_color,
        })
    }))
}

/// Returns the emails of a user, oldest first.
pub(crate) async fn emails(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT email, is_primary, verified_at, created_at
         FROM user_emails
         WHERE user_id = $1
         ORDER BY created_at, email",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "email": row.email,
                "primary": row.is_primary,
                "verified_at": row.verified_at.map(time),
                "created_at": time(row.created_at),
            })
        })
        .collect())
}

/// Returns the identities of a user at the identity providers, oldest first.
pub(crate) async fn identities(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT p.slug, p.display_name, e.subject, e.email, e.linked_at
         FROM external_identities e
         JOIN identity_providers p ON p.id = e.provider_id
         WHERE e.user_id = $1
         ORDER BY e.linked_at, p.slug",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "provider": row.slug,
                "provider_name": row.display_name,
                "subject": row.subject,
                "email": row.email,
                "linked_at": time(row.linked_at),
            })
        })
        .collect())
}

/// Returns each session of a user, open or closed, newest first.
pub(crate) async fn sessions(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT id, methods, user_agent, host(ip_address) AS ip, created_at, last_used_at,
                expires_at, revoked_at
         FROM sessions
         WHERE user_id = $1
         ORDER BY id DESC",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "id": SessionId::from_db(row.id).to_string(),
                "methods": row.methods,
                "user_agent": row.user_agent,
                "ip_address": row.ip,
                "created_at": time(row.created_at),
                "last_used_at": time(row.last_used_at),
                "expires_at": time(row.expires_at),
                "revoked_at": row.revoked_at.map(time),
            })
        })
        .collect())
}

/// Returns the personal tokens of a user, with no secret and no part of a secret, oldest first.
pub(crate) async fn personal_tokens(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT id, name, preset, actions, project_ids, created_at, expires_at, last_used_at,
                revoked_at
         FROM api_tokens
         WHERE actor_id = $1
         ORDER BY created_at, id",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let projects: Vec<String> = row
                .project_ids
                .into_iter()
                .map(|id| ProjectId::from_db(id).to_string())
                .collect();
            json!({
                "id": ApiTokenId::from_db(row.id).to_string(),
                "name": row.name,
                "preset": row.preset,
                "actions": row.actions,
                "project_ids": projects,
                "created_at": time(row.created_at),
                "expires_at": time(row.expires_at),
                "last_used_at": row.last_used_at.map(time),
                "revoked_at": row.revoked_at.map(time),
            })
        })
        .collect())
}

/// Returns the state of the web app for a user, by key.
pub(crate) async fn interface_state(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT key, value, updated_at FROM user_state WHERE user_id = $1 ORDER BY key",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "key": row.key,
                "value": row.value,
                "updated_at": time(row.updated_at),
            })
        })
        .collect())
}

/// Returns the acceptances of the terms by a user, oldest first.
pub(crate) async fn terms_acceptances(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT kind, version, method, accepted_at
         FROM terms_acceptances
         WHERE user_id = $1
         ORDER BY accepted_at, kind, version",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "document": row.kind,
                "version": row.version,
                "method": row.method,
                "accepted_at": time(row.accepted_at),
            })
        })
        .collect())
}
