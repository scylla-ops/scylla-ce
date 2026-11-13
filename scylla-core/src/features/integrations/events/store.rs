//! The SQL of the provider events: the deliveries of the webhooks and the keys of the polls in
//! `vcs_deliveries` (one time each, kept 7 days), the arrival of a webhook on its links and its
//! connections, the signing token of a GitLab webhook, and the projects of a repository. Each
//! function runs in the transaction of the caller.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

use crate::Error;

/// The reasons of a link in polling mode that a webhook ends: the webhooks did not arrive, or
/// GitLab disabled the webhook.
const ENDED_BY_A_WEBHOOK: [&str; 2] = ["WEBHOOKS_NOT_ARRIVING", "WEBHOOK_DISABLED"];

/// A delivery of a webhook, or the key of a poll.
pub(crate) struct Delivery<'a> {
    /// The provider.
    pub(crate) provider: VcsProvider,
    /// The id of the delivery, or the key of a poll.
    pub(crate) key: &'a str,
    /// The App of the instance that got it, or `None`.
    pub(crate) app: Option<VcsAppId>,
    /// A connection of the delivery, if Scylla knows one.
    pub(crate) connection: Option<VcsConnectionId>,
    /// A project of the delivery, if Scylla knows one.
    pub(crate) project: Option<ProjectId>,
    /// The event, such as `push`.
    pub(crate) event: &'a str,
    /// The action, such as `opened`.
    pub(crate) action: Option<&'a str>,
}

/// The GitLab webhook of a link: its connection, its project on GitLab and its sealed signing
/// token.
pub(crate) struct Hook {
    /// The connection of the link.
    pub(crate) connection: VcsConnectionId,
    /// The id of the GitLab project.
    pub(crate) external_id: String,
    /// The signing token, sealed with the id of the Scylla project.
    pub(crate) sealed: Vec<u8>,
}

/// Keeps a delivery one time, with the status 202. Returns `false` when the provider sent it
/// before.
pub(crate) async fn receive(tx: &mut Tx<'_>, delivery: &Delivery<'_>) -> Result<bool, Error> {
    let inserted = sqlx::query!(
        "INSERT INTO vcs_deliveries (provider, delivery_id, app_id, connection_id, project_id,
                                     event, action, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 202)
         ON CONFLICT (provider, delivery_id) DO NOTHING",
        delivery.provider.as_str(),
        delivery.key,
        delivery.app.map(VcsAppId::to_db),
        delivery.connection.map(VcsConnectionId::to_db),
        delivery.project.map(ProjectId::to_db),
        delivery.event,
        delivery.action,
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}

/// Keeps the end of the work of a delivery, with the error of a failure.
pub(crate) async fn processed(
    tx: &mut Tx<'_>,
    provider: VcsProvider,
    key: &str,
    error: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_deliveries SET processed_at = now(), error = $3
         WHERE provider = $1 AND delivery_id = $2",
        provider.as_str(),
        key,
        error,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the deliveries of more than 7 days. Returns their number.
pub(crate) async fn prune(tx: &mut Tx<'_>) -> Result<u64, Error> {
    let deleted =
        sqlx::query!("DELETE FROM vcs_deliveries WHERE received_at < now() - interval '7 days'")
            .execute(&mut **tx)
            .await?
            .rows_affected();
    Ok(deleted)
}

/// Notes that a webhook of a GitHub installation arrived: the time of the last event of its
/// connections and of the links of its repository. The connections and these links go back to
/// the webhook mode.
pub(crate) async fn github_arrived(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    repository: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_connections SET last_event_at = now(), delivery_mode = 'webhook'
         WHERE provider = 'github' AND external_id = $1 AND app_id IS NOT DISTINCT FROM $2",
        installation,
        app.map(VcsAppId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    let Some(repository) = repository else {
        return Ok(());
    };
    sqlx::query!(
        "UPDATE repository_links l
         SET last_event_at = now(),
             delivery_mode = CASE WHEN l.state_reason = ANY($4) OR l.state_reason IS NULL
                                  THEN 'webhook' ELSE l.delivery_mode END,
             state_reason = CASE WHEN l.state_reason = ANY($4) THEN NULL
                                 ELSE l.state_reason END,
             next_poll_at = CASE WHEN l.delivery_mode = 'polling'
                                  AND (l.state_reason = ANY($4) OR l.state_reason IS NULL)
                                 THEN now() + interval '15 minutes' ELSE l.next_poll_at END
         FROM vcs_connections c
         WHERE c.id = l.connection_id AND c.provider = 'github' AND c.external_id = $1
           AND c.app_id IS NOT DISTINCT FROM $2 AND l.external_id = $3 AND l.state = 'active'",
        installation,
        app.map(VcsAppId::to_db),
        repository,
        &ENDED_BY_A_WEBHOOK.map(str::to_owned),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Notes that the GitLab webhook of a project arrived: the time of the last event of the link
/// and of its connection. A link in polling mode with no reason or a reason that a webhook ends
/// goes back to the webhook mode.
pub(crate) async fn gitlab_arrived(tx: &mut Tx<'_>, project: ProjectId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE repository_links
         SET last_event_at = now(),
             delivery_mode = CASE WHEN state_reason = ANY($2) OR state_reason IS NULL
                                  THEN 'webhook' ELSE delivery_mode END,
             next_poll_at = CASE WHEN delivery_mode = 'polling'
                                  AND (state_reason = ANY($2) OR state_reason IS NULL)
                                 THEN now() + interval '15 minutes' ELSE next_poll_at END,
             state_reason = CASE WHEN state_reason = ANY($2) THEN NULL ELSE state_reason END
         WHERE project_id = $1 AND state = 'active'",
        project.to_db(),
        &ENDED_BY_A_WEBHOOK.map(str::to_owned),
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE vcs_connections c SET last_event_at = now()
         FROM repository_links l WHERE l.project_id = $1 AND c.id = l.connection_id",
        project.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the GitLab webhook of the link of a project, or `None` for a project with no link
/// or a link with no webhook.
pub(crate) async fn hook(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<Hook>, Error> {
    let row = sqlx::query!(
        r#"SELECT l.connection_id, l.external_id, l.sealed_hook_secret AS "sealed!"
           FROM repository_links l JOIN vcs_connections c ON c.id = l.connection_id
           WHERE l.project_id = $1 AND c.provider = 'gitlab'
             AND l.sealed_hook_secret IS NOT NULL"#,
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Hook {
        connection: VcsConnectionId::from_db(row.connection_id),
        external_id: row.external_id,
        sealed: row.sealed,
    }))
}

/// Returns the projects that link a repository of a GitHub installation.
pub(crate) async fn github_projects(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
    installation: &str,
    repository: &str,
) -> Result<Vec<ProjectId>, Error> {
    let rows = sqlx::query_scalar!(
        "SELECT l.project_id FROM repository_links l
         JOIN vcs_connections c ON c.id = l.connection_id
         WHERE c.provider = 'github' AND c.external_id = $1 AND c.app_id IS NOT DISTINCT FROM $2
           AND l.external_id = $3
         ORDER BY l.project_id",
        installation,
        app.map(VcsAppId::to_db),
        repository,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ProjectId::from_db).collect())
}

/// Returns the projects that link a repository of a provider, through each connection.
pub(crate) async fn projects_of(
    tx: &mut Tx<'_>,
    provider: VcsProvider,
    repository: &str,
) -> Result<Vec<ProjectId>, Error> {
    let rows = sqlx::query_scalar!(
        "SELECT l.project_id FROM repository_links l
         JOIN vcs_connections c ON c.id = l.connection_id
         WHERE c.provider = $1 AND l.external_id = $2
         ORDER BY l.project_id",
        provider.as_str(),
        repository,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ProjectId::from_db).collect())
}
