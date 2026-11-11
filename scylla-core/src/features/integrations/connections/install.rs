//! The install of the GitHub App in a workspace: a state of one use for 10 minutes, the install
//! page of GitHub, then the check of the redirect and the connection of the installation.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::connections::{AccountKind, VcsConnectionId};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Rejection;
use time::{Duration, OffsetDateTime};
use url::Url;

use super::calls::github;
use super::checks;
use super::credentials::{self, AppSource};
use super::store::{self, states, Counted, Installation};
use crate::features::integrations::IntegrationContext;
use crate::infra::secret;
use crate::infra::vcs::ProviderError;
use crate::Error;

/// The life of a state of an install.
const STATE_LIFE: Duration = Duration::minutes(10);

/// The values of the redirect of GitHub after an install.
pub(super) struct Redirect {
    /// The id of the installation, or `None` for a cancelled install.
    pub(super) installation: Option<u64>,
    /// The code of the authorization of the user.
    pub(super) code: String,
    /// What GitHub did: `install`, `update` or `request`.
    pub(super) setup_action: String,
}

/// Starts an install: keeps the hash of a new state, and returns the install page with the
/// state and the end of its validity. An instance with no App gives `APP_MISSING`.
pub(super) async fn start(
    ctx: &IntegrationContext,
    workspace: WorkspaceId,
    caller: ActorId,
) -> Result<(String, OffsetDateTime), Error> {
    // Find the App.
    let source = credentials::github_app(ctx).await?;
    let source = source.ok_or_else(credentials::app_missing)?;

    // Keep the hash of a new state.
    let state = secret::token();
    let expires_at = OffsetDateTime::now_utc() + STATE_LIFE;
    let mut tx = ctx.pool.begin().await?;
    states::insert(
        &mut tx,
        &secret::hash(&state),
        workspace,
        caller,
        expires_at,
    )
    .await?;
    tx.commit().await?;

    // Make the install page of the App.
    let slug = &source.app.settings().slug;
    let page = format!(
        "{}/apps/{slug}/installations/new",
        ctx.settings.github_web_url
    );
    let mut url = Url::parse(&page).map_err(|_| Error::Internal(format!("{page} is not a URL")))?;
    url.query_pairs_mut().append_pair("state", &state);
    Ok((url.into(), expires_at))
}

/// Returns the workspace of a state that is open for the caller, with no change. Else
/// `STATE_EXPIRED`.
pub(super) async fn workspace_of(
    ctx: &IntegrationContext,
    state: &str,
    caller: ActorId,
) -> Result<WorkspaceId, Error> {
    let mut tx = ctx.pool.begin().await?;
    let workspace = states::open(&mut tx, &secret::hash(state), caller).await?;
    tx.commit().await?;
    workspace.ok_or_else(state_expired)
}

/// Completes an install: uses the state, checks that the user sees the installation, reads it,
/// and makes or refreshes its connection.
pub(super) async fn complete(
    ctx: &IntegrationContext,
    state: &str,
    caller: ActorId,
    redirect: Redirect,
) -> Result<Counted, Error> {
    // Use the state one time.
    let mut tx = ctx.pool.begin().await?;
    let workspace = states::take(&mut tx, &secret::hash(state), caller).await?;
    tx.commit().await?;
    let workspace = workspace.ok_or_else(state_expired)?;

    // Refuse an install that waits for an owner, or that was cancelled.
    if redirect.setup_action == "request" {
        let message = "an owner of the organization must approve the request on GitHub";
        return Err(Error::Rejected(Rejection::new(
            "INSTALL_PENDING_APPROVAL",
            message,
        )));
    }
    let Some(installation) = redirect.installation else {
        let message = "the install was cancelled";
        return Err(Error::Rejected(Rejection::new(
            "INSTALL_CANCELLED",
            message,
        )));
    };

    // Check that the user sees the installation on GitHub.
    let source = credentials::github_app(ctx).await?;
    let source = source.ok_or_else(credentials::app_missing)?;
    let user = source
        .app
        .exchange_code(&redirect.code)
        .await
        .map_err(|error| match error {
            ProviderError::Invalid(_) => state_expired(),
            error => failed(error),
        })?;
    let sees = github::user_sees(&ctx.clients, &user, installation).await;
    if !sees.map_err(failed)? {
        return Err(not_visible());
    }

    // Read the installation, and keep its connection.
    let values = read(ctx, &source, installation).await?;
    keep(ctx, workspace, caller, &source, installation, values).await
}

/// What GitHub tells about an installation.
struct Read {
    /// The installation.
    found: github::Installation,
    /// The number of its repositories.
    repositories: u32,
}

/// Reads an installation and the number of its repositories. A suspended installation gives no
/// token, so it has no count.
async fn read(
    ctx: &IntegrationContext,
    source: &AppSource,
    installation: u64,
) -> Result<Read, Error> {
    let found = github::installation(&ctx.clients, &source.app, installation)
        .await
        .map_err(|error| match error {
            ProviderError::NotFound => not_visible(),
            error => failed(error),
        })?;
    let repositories = match found.suspended_at {
        Some(_) => 0,
        None => github::repository_count(&ctx.clients, &source.app, installation)
            .await
            .map_err(failed)?,
    };
    Ok(Read {
        found,
        repositories,
    })
}

/// Makes or refreshes the connection of an installation in a workspace, and returns it.
async fn keep(
    ctx: &IntegrationContext,
    workspace: WorkspaceId,
    caller: ActorId,
    source: &AppSource,
    installation: u64,
    read: Read,
) -> Result<Counted, Error> {
    // Find the state of the installation.
    let found = &read.found;
    let missing = checks::missing_permissions(&found.permissions);
    let (state, reason) = checks::installation_state(found.suspended_at.is_some(), &missing);
    let permissions = serde_json::to_value(&found.permissions)
        .map_err(|error| Error::Internal(format!("the permissions of GitHub: {error}")))?;
    let external_id = installation.to_string();
    let values = Installation {
        workspace,
        app: source.row,
        external_id: &external_id,
        account: &found.account.login,
        account_kind: match found.account.kind.as_str() {
            "Organization" => AccountKind::Organization,
            _ => AccountKind::User,
        },
        avatar_url: found.account.avatar_url.as_deref(),
        permissions: &permissions,
        all_repositories: found.repository_selection == "all",
        repository_count: read.repositories,
        state,
        state_reason: reason,
        created_by: caller,
    };

    // Write the connection, and read it back.
    let mut tx = ctx.pool.begin().await?;
    let (id, new) =
        store::upsert_installation(&mut tx, VcsConnectionId::generate(), &values).await?;
    let counted = store::find(&mut tx, id).await?;
    tx.commit().await?;
    let counted = counted.ok_or(Error::NotFound("connection"))?;
    tracing::info!(
        event = "vcs.connected",
        connection = %id,
        %workspace,
        provider = VcsProvider::GitHub.as_str(),
        account = %found.account.login,
        new,
    );
    Ok(counted)
}

/// The refusal of a state that is unknown, used, old or of another user.
fn state_expired() -> Error {
    let message = "the link expired: start the install again";
    Error::Rejected(Rejection::new("STATE_EXPIRED", message))
}

/// The refusal of an installation that the user does not see on GitHub.
fn not_visible() -> Error {
    let message = "the user cannot see this installation on GitHub";
    Error::Rejected(Rejection::new("INSTALLATION_NOT_VISIBLE", message))
}

/// The refusal of a failed call to GitHub.
fn failed(error: ProviderError) -> Error {
    credentials::provider_failed(VcsProvider::GitHub, error)
}
