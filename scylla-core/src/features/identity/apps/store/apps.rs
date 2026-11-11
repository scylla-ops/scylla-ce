//! The SQL of the `apps` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::{App, AppId, Host, NewApp};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The foreign key from an App to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "apps_workspace_id_fkey";

/// Inserts an App for an actor of kind `app`, and returns it. An unknown workspace gives
/// `NotFound("workspace")`.
pub(crate) async fn insert(tx: &mut Tx<'_>, id: AppId, new: &NewApp) -> Result<App, Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO apps (id, workspace_id, name, runner, created_by) VALUES ($1, $2, $3, $4, $5)
         RETURNING id, workspace_id, name, runner, created_by, created_at, last_seen_at, version,
                   os, arch, hostname, cpu_count, memory_bytes, capacity, container_runtime,
                   features, disabled_at",
        id.to_db(),
        new.workspace_id.to_db(),
        new.name.as_str(),
        new.runner,
        new.created_by.to_db(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(row.into_app())
}

/// Returns an App, or `None` if no App has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: AppId) -> Result<Option<App>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, name, runner, created_by, created_at, last_seen_at, version, os,
                arch, hostname, cpu_count, memory_bytes, capacity, container_runtime, features,
                disabled_at
         FROM apps WHERE id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_app))
}

/// Returns the Apps of a workspace after the page id, in id order, with one extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    runner: Option<bool>,
    page: PageRequest<AppId>,
) -> Result<Vec<App>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, name, runner, created_by, created_at, last_seen_at, version, os,
                arch, hostname, cpu_count, memory_bytes, capacity, container_runtime, features,
                disabled_at
         FROM apps
         WHERE workspace_id = $1
           AND ($2::boolean IS NULL OR runner = $2)
           AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id
         LIMIT $4",
        workspace.to_db(),
        runner,
        page.after.map(AppId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_app).collect())
}

/// Returns the workspace of an App, or `None` if no App has this id.
pub(crate) async fn workspace(
    tx: &mut Tx<'_>,
    id: AppId,
) -> Result<Option<WorkspaceId>, sqlx::Error> {
    let workspace = sqlx::query_scalar!("SELECT workspace_id FROM apps WHERE id = $1", id.to_db())
        .fetch_optional(&mut **tx)
        .await?;
    Ok(workspace.map(WorkspaceId::from_db))
}

/// Deletes an App with its keys and its join codes. Returns `false` if no App has this id.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: AppId) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM apps WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Disables an App, or enables it again, and returns it. A second disable keeps the first time.
/// Returns `None` if no App has this id.
pub(crate) async fn set_disabled(
    tx: &mut Tx<'_>,
    id: AppId,
    disabled: bool,
) -> Result<Option<App>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE apps
         SET disabled_at = CASE WHEN $2 THEN coalesce(disabled_at, now()) END
         WHERE id = $1
         RETURNING id, workspace_id, name, runner, created_by, created_at, last_seen_at, version,
                   os, arch, hostname, cpu_count, memory_bytes, capacity, container_runtime,
                   features, disabled_at",
        id.to_db(),
        disabled,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_app))
}

/// Stores the host of the hello of an agent, and its last seen time.
pub(crate) async fn set_host(tx: &mut Tx<'_>, id: AppId, host: &Host) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE apps SET last_seen_at = now(), version = $2, os = $3, arch = $4, hostname = $5,
                cpu_count = $6, memory_bytes = $7, capacity = $8, container_runtime = $9,
                features = $10
         WHERE id = $1",
        id.to_db(),
        host.version,
        host.os,
        host.arch,
        host.hostname,
        i32::try_from(host.cpu_count).unwrap_or(i32::MAX),
        i64::try_from(host.memory_bytes).unwrap_or(i64::MAX),
        host.capacity as i32,
        host.container_runtime,
        &host.features,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Sets the last seen time of an App to now.
pub(crate) async fn touch(tx: &mut Tx<'_>, id: AppId) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE apps SET last_seen_at = now() WHERE id = $1",
        id.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A row of the `apps` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The display name.
    name: String,
    /// `true` for an agent.
    runner: bool,
    /// The author.
    created_by: uuid::Uuid,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The last seen time.
    last_seen_at: Option<OffsetDateTime>,
    /// The version of the agent.
    version: Option<String>,
    /// The operating system.
    os: Option<String>,
    /// The CPU architecture.
    arch: Option<String>,
    /// The host name.
    hostname: Option<String>,
    /// The number of CPUs.
    cpu_count: Option<i32>,
    /// The memory in bytes.
    memory_bytes: Option<i64>,
    /// The capacity.
    capacity: i32,
    /// The container runtime.
    container_runtime: Option<String>,
    /// The features of the agent.
    features: Vec<String>,
    /// The time of the disable.
    disabled_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the App of the row. An App with no version has no host yet.
    fn into_app(self) -> App {
        let host = self.version.map(|version| Host {
            version,
            os: self.os.unwrap_or_default(),
            arch: self.arch.unwrap_or_default(),
            hostname: self.hostname.unwrap_or_default(),
            cpu_count: self.cpu_count.unwrap_or_default().max(0) as u32,
            memory_bytes: self.memory_bytes.unwrap_or_default().max(0) as u64,
            capacity: self.capacity.max(1) as u32,
            container_runtime: self.container_runtime,
            features: self.features,
        });
        App {
            id: AppId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            name: self.name,
            runner: self.runner,
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
            last_seen_at: self.last_seen_at,
            host,
            disabled_at: self.disabled_at,
        }
    }
}
