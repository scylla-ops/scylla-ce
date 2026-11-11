//! The GitLab connections: the check of the token of a service account (its scopes, its expiry,
//! its account and its projects), then the token sealed with the id of its connection.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::connections::{
    AccountKind, ConnectionState, VcsConnection, VcsConnectionId,
};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Rejection;
use time::OffsetDateTime;

use super::calls::gitlab::{self, GitLabUser};
use super::checks;
use super::credentials;
use super::store::{self, Counted, Token};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Hidden, ProviderError};
use crate::Error;

/// The scope that a token needs.
const API_SCOPE: &str = "api";

/// What the check of a token found.
struct Checked {
    /// The user of the token.
    user: GitLabUser,
    /// The scopes of the token.
    scopes: Vec<String>,
    /// The expiry of the token.
    expires_at: Option<OffsetDateTime>,
    /// The number of projects where the user is Developer or more.
    repositories: u32,
    /// The warnings of the check.
    warnings: Vec<&'static str>,
}

/// Connects a workspace to GitLab with a token, and returns the connection with the warnings of
/// the check. A server with no key gives `SECRET_KEY_MISSING`.
pub(super) async fn connect(
    ctx: &IntegrationContext,
    workspace: WorkspaceId,
    caller: ActorId,
    token: &Hidden,
) -> Result<(Counted, Vec<&'static str>), Error> {
    // Check the token, then seal it with the id of the new connection.
    let sealer = credentials::key(ctx.sealer.as_deref())?;
    let checked = check(ctx, token).await?;
    let id = VcsConnectionId::generate();
    let sealed = sealer.seal_with(token.expose().as_bytes(), &id.to_bytes());

    // Keep the connection, and read it back.
    let external_id = checked.user.id.to_string();
    let values = values(&checked, &external_id, &sealed);
    let mut tx = ctx.pool.begin().await?;
    store::insert_token(&mut tx, id, workspace, caller, &values).await?;
    let counted = store::find(&mut tx, id).await?;
    tx.commit().await?;
    let counted = counted.ok_or(Error::NotFound("connection"))?;
    tracing::info!(
        event = "vcs.connected",
        connection = %id,
        %workspace,
        provider = VcsProvider::GitLab.as_str(),
        account = %checked.user.username,
        new = true,
    );
    Ok((counted, checked.warnings))
}

/// Replaces the token of a GitLab connection with a token of the same account, and makes the
/// connection active. Returns the connection with the warnings of the check.
pub(super) async fn replace(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
    token: &Hidden,
) -> Result<(Counted, Vec<&'static str>), Error> {
    // Refuse a GitHub connection, a server with no key, and a token of another account.
    if connection.provider != VcsProvider::GitLab {
        return Err(Error::invalid(
            "connection_id",
            "is not a GitLab connection",
        ));
    }
    let sealer = credentials::key(ctx.sealer.as_deref())?;
    let checked = check(ctx, token).await?;
    let external_id = checked.user.id.to_string();
    if external_id != connection.external_id {
        let message = "the token is of another account: connect it as a new connection";
        return Err(Error::Rejected(Rejection::new("ACCOUNT_CHANGED", message)));
    }

    // Seal the token with the id of the connection, and keep it.
    let id = connection.id;
    let sealed = sealer.seal_with(token.expose().as_bytes(), &id.to_bytes());
    let values = values(&checked, &external_id, &sealed);
    let mut tx = ctx.pool.begin().await?;
    store::replace_token(&mut tx, id, &values).await?;
    let counted = store::find(&mut tx, id).await?;
    tx.commit().await?;
    let counted = counted.ok_or(Error::NotFound("connection"))?;
    if connection.state != ConnectionState::Active {
        tracing::info!(
            event = "vcs.connection_state_changed",
            connection = %id,
            workspace = %connection.workspace_id,
            from = connection.state.as_str(),
            to = ConnectionState::Active.as_str(),
        );
    }
    Ok((counted, checked.warnings))
}

/// Checks a token on GitLab: an active token with the scope `api`, its user, and the projects
/// where the user is Developer or more. A refused token gives `TOKEN_INVALID`.
async fn check(ctx: &IntegrationContext, token: &Hidden) -> Result<Checked, Error> {
    // Read the token, and refuse a token that GitLab refuses or that has no scope api.
    let info = gitlab::token(&ctx.clients, token)
        .await
        .map_err(refused_token)?;
    if !info.active || info.revoked {
        return Err(token_invalid());
    }
    if !info.scopes.iter().any(|scope| scope == API_SCOPE) {
        let message = "the token has no scope api";
        return Err(Error::Rejected(Rejection::new(
            "TOKEN_SCOPE_MISSING",
            message,
        )));
    }

    // Read its user and its projects.
    let user = gitlab::user(&ctx.clients, token)
        .await
        .map_err(refused_token)?;
    let projects = gitlab::projects(&ctx.clients, token, "")
        .await
        .map_err(failed)?;

    // Find the warnings.
    let roles: Vec<_> = projects
        .iter()
        .filter_map(|project| checks::gitlab_role(project.level()))
        .collect();
    let expires_at = info.expires_at.as_deref().and_then(checks::expiry_of);
    let now = OffsetDateTime::now_utc();
    let warnings = checks::token_warnings(user.bot, expires_at, &roles, now);
    Ok(Checked {
        repositories: u32::try_from(roles.len()).unwrap_or(u32::MAX),
        user,
        scopes: info.scopes,
        expires_at,
        warnings,
    })
}

/// Returns the values of a checked token for its row.
fn values<'a>(checked: &'a Checked, external_id: &'a str, sealed: &'a [u8]) -> Token<'a> {
    Token {
        external_id,
        account: &checked.user.username,
        account_kind: match checked.user.bot {
            true => AccountKind::ServiceAccount,
            false => AccountKind::User,
        },
        avatar_url: checked.user.avatar_url.as_deref(),
        sealed,
        expires_at: checked.expires_at,
        scopes: &checked.scopes,
        repository_count: checked.repositories,
    }
}

/// Maps the failure of a call with the token: a refusal gives `TOKEN_INVALID`.
fn refused_token(error: ProviderError) -> Error {
    match error {
        ProviderError::Unauthorized | ProviderError::Forbidden | ProviderError::NotFound => {
            token_invalid()
        }
        error => failed(error),
    }
}

/// The refusal of a token that GitLab refuses, that expired, or that was revoked.
fn token_invalid() -> Error {
    let message = "GitLab refused the token";
    Error::Rejected(Rejection::new("TOKEN_INVALID", message))
}

/// The refusal of a failed call to GitLab.
fn failed(error: ProviderError) -> Error {
    credentials::provider_failed(VcsProvider::GitLab, error)
}
