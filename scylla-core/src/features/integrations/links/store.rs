//! The SQL of the repository links: the read of a link with its connection, the insert, the
//! change and the delete of a link. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::connections::{ConnectionState, DeliveryMode, VcsConnectionId};
use scylla_domain::integrations::links::{GitLabRole, PipelineSource, ReportMode, RepositoryLink};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// A link with the provider and the account of its connection. The state is the state of the
/// connection when the connection is not active. No field holds a secret.
#[derive(Clone, Debug)]
pub(crate) struct Linked {
    /// The link.
    pub(crate) link: RepositoryLink,
    /// The provider of the connection.
    pub(crate) provider: VcsProvider,
    /// The login of the account of the connection.
    pub(crate) account: String,
}

/// A new link.
pub(super) struct NewLink<'a> {
    /// The project.
    pub(super) project: ProjectId,
    /// The workspace of the project and of the connection.
    pub(super) workspace: WorkspaceId,
    /// The connection.
    pub(super) connection: VcsConnectionId,
    /// The repository, as the provider gives it.
    pub(super) repository: &'a Repository,
    /// How Scylla reports.
    pub(super) report_mode: ReportMode,
    /// How the provider tells Scylla about the new commits.
    pub(super) delivery_mode: DeliveryMode,
    /// The time between two polls, in seconds.
    pub(super) poll_interval_seconds: u32,
    /// The GitLab webhook and its signing token, sealed with the id of the project.
    pub(super) hook: Option<(&'a str, &'a [u8])>,
    /// The reason of the delivery mode, as a stable code, or `None`.
    pub(super) reason: Option<&'static str>,
    /// The actor that makes the link.
    pub(super) created_by: ActorId,
}

/// A repository, as the provider gives it.
#[derive(Clone, Debug)]
pub(super) struct Repository {
    /// The id on the provider.
    pub(super) external_id: String,
    /// The full name.
    pub(super) full_name: String,
    /// The page on the provider.
    pub(super) web_url: String,
    /// The default branch.
    pub(super) default_branch: String,
    /// `true` for a private or internal repository.
    pub(super) private: bool,
    /// The role of the service account on a GitLab project.
    pub(super) role: Option<GitLabRole>,
}

/// The changes of a link. `None` keeps a value.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Changes {
    /// `true` reports the results to the provider.
    pub(super) reports_enabled: Option<bool>,
    /// The report mode.
    pub(super) report_mode: Option<ReportMode>,
    /// The time between two polls, in seconds.
    pub(super) poll_interval_seconds: Option<u32>,
}

/// What a deleted link leaves: its repository, its webhook, and the counts of what stops.
pub(super) struct Unlinked {
    /// The connection.
    pub(super) connection: VcsConnectionId,
    /// The id of the repository on the provider.
    pub(super) external_id: String,
    /// The full name of the repository.
    pub(super) full_name: String,
    /// The id of the GitLab webhook that Scylla made, or `None`.
    pub(super) hook_id: Option<String>,
    /// The number of enabled Git triggers of the project.
    pub(super) git_triggers: i64,
    /// The number of merge rules of the project.
    pub(super) merge_blocks: i64,
}

/// Returns the link of a project with its connection, or `None` for a project with no link.
pub(crate) async fn find(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<Linked>, Error> {
    let row = sqlx::query!(
        "SELECT l.project_id, l.workspace_id, l.connection_id, l.external_id, l.full_name,
                l.web_url, l.default_branch, l.private, l.role, l.pipeline_source,
                l.reports_enabled, l.report_mode, l.delivery_mode, l.poll_interval_seconds,
                l.last_event_at, l.state, l.state_reason, l.created_by, l.created_at,
                c.provider, c.account, c.state AS connection_state,
                c.state_reason AS connection_reason
         FROM repository_links l JOIN vcs_connections c ON c.id = l.connection_id
         WHERE l.project_id = $1",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let unknown = |what: &str| Error::Internal(format!("unknown {what} of a repository link"));
    let state = ConnectionState::parse(&row.state).ok_or_else(|| unknown("state"))?;
    let connection = ConnectionState::parse(&row.connection_state);
    let connection = connection.ok_or_else(|| unknown("connection state"))?;
    let (state, state_reason) = match connection {
        ConnectionState::Active => (state, row.state_reason),
        _ => (connection, row.connection_reason),
    };
    let role = row.role.as_deref().map(GitLabRole::parse);
    let link = RepositoryLink {
        project_id: ProjectId::from_db(row.project_id),
        workspace_id: WorkspaceId::from_db(row.workspace_id),
        connection_id: VcsConnectionId::from_db(row.connection_id),
        external_id: row.external_id,
        full_name: row.full_name,
        web_url: row.web_url,
        default_branch: row.default_branch,
        private: row.private,
        role: role
            .map(|role| role.ok_or_else(|| unknown("role")))
            .transpose()?,
        pipeline_source: PipelineSource::parse(&row.pipeline_source)
            .ok_or_else(|| unknown("pipeline source"))?,
        reports_enabled: row.reports_enabled,
        report_mode: ReportMode::parse(&row.report_mode).ok_or_else(|| unknown("report mode"))?,
        delivery_mode: DeliveryMode::parse(&row.delivery_mode)
            .ok_or_else(|| unknown("delivery mode"))?,
        poll_interval_seconds: u32::try_from(row.poll_interval_seconds)
            .map_err(|_| unknown("poll interval"))?,
        last_event_at: row.last_event_at,
        state,
        state_reason,
        created_by: ActorId::from_db(row.created_by),
        created_at: row.created_at,
    };
    Ok(Some(Linked {
        link,
        provider: VcsProvider::parse(&row.provider).ok_or_else(|| unknown("provider"))?,
        account: row.account,
    }))
}

/// Returns `true` when a project has a link.
pub(super) async fn exists(tx: &mut Tx<'_>, project: ProjectId) -> Result<bool, Error> {
    let found = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM repository_links WHERE project_id = $1) AS "found!""#,
        project.to_db()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(found)
}

/// Inserts a link, in polling mode from now when the delivery mode is polling. Returns `false`
/// when the project has a link.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewLink<'_>) -> Result<bool, Error> {
    let repository = new.repository;
    let (hook_id, sealed) = new.hook.unzip();
    let inserted = sqlx::query!(
        "INSERT INTO repository_links (project_id, workspace_id, connection_id, external_id,
                                       full_name, web_url, default_branch, private, role,
                                       report_mode, delivery_mode, poll_interval_seconds,
                                       next_poll_at, hook_id, sealed_hook_secret, state_reason,
                                       created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 CASE WHEN $11 = 'polling' THEN now() END, $13, $14, $15, $16)
         ON CONFLICT (project_id) DO NOTHING",
        new.project.to_db(),
        new.workspace.to_db(),
        new.connection.to_db(),
        repository.external_id,
        repository.full_name,
        repository.web_url,
        repository.default_branch,
        repository.private,
        repository.role.map(GitLabRole::as_str),
        new.report_mode.as_str(),
        new.delivery_mode.as_str(),
        i32::try_from(new.poll_interval_seconds).unwrap_or(i32::MAX),
        hook_id,
        sealed,
        new.reason,
        new.created_by.to_db(),
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}

/// Changes the reports and the polling of a link. Returns `false` for a project with no link.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    project: ProjectId,
    changes: Changes,
) -> Result<bool, Error> {
    let updated = sqlx::query!(
        "UPDATE repository_links
         SET reports_enabled = COALESCE($2, reports_enabled),
             report_mode = COALESCE($3, report_mode),
             poll_interval_seconds = COALESCE($4, poll_interval_seconds),
             updated_at = now()
         WHERE project_id = $1",
        project.to_db(),
        changes.reports_enabled,
        changes.report_mode.map(ReportMode::as_str),
        changes
            .poll_interval_seconds
            .map(|seconds| i32::try_from(seconds).unwrap_or(i32::MAX)),
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(updated > 0)
}

/// Deletes the link of a project, with its merge rules and its poll state. Returns what the
/// link leaves, or `None` for a project with no link.
pub(super) async fn delete(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<Unlinked>, Error> {
    let row = sqlx::query!(
        r#"WITH counts AS (
               SELECT (SELECT count(*) FROM triggers
                       WHERE project_id = $1 AND enabled
                         AND kind IN ('push', 'pull_request', 'tag')) AS git_triggers,
                      (SELECT count(*) FROM merge_blocks WHERE project_id = $1) AS merge_blocks
           )
           DELETE FROM repository_links l USING counts
           WHERE l.project_id = $1
           RETURNING l.connection_id, l.external_id, l.full_name, l.hook_id,
                     counts.git_triggers AS "git_triggers!", counts.merge_blocks AS "merge_blocks!""#,
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Unlinked {
        connection: VcsConnectionId::from_db(row.connection_id),
        external_id: row.external_id,
        full_name: row.full_name,
        hook_id: row.hook_id,
        git_triggers: row.git_triggers,
        merge_blocks: row.merge_blocks,
    }))
}
