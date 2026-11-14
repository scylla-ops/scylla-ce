//! The SQL of `vcs_poll_state`: the last head that Scylla saw on each ref of a linked
//! repository (`refs/heads/<branch>`, or the head of a pull request), and the cursor of each list
//! of a poll (`branches`, `pulls`). Each function runs in the transaction of the caller.

use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// A row of the poll state of a link: a head, or the cursor of a list.
pub(crate) struct Row {
    /// The ref of a head, or the name of a list.
    pub(crate) name: String,
    /// The last head of a ref.
    pub(crate) sha: Option<String>,
    /// The ETag of the last read of a list.
    pub(crate) etag: Option<String>,
    /// The time from which the next read of a list asks for the changes.
    pub(crate) updated_after: Option<OffsetDateTime>,
}

/// Returns the poll state of the link of a project.
pub(crate) async fn read(tx: &mut Tx<'_>, project: ProjectId) -> Result<Vec<Row>, Error> {
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT ref AS name, last_sha AS sha, etag, updated_after FROM vcs_poll_state
           WHERE project_id = $1"#,
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

/// Keeps the head of a ref of the link of a project. A project with no link keeps nothing.
pub(crate) async fn keep(
    tx: &mut Tx<'_>,
    project: ProjectId,
    git_ref: &str,
    sha: &str,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO vcs_poll_state (project_id, ref, last_sha)
         SELECT project_id, $2, $3 FROM repository_links WHERE project_id = $1
         ON CONFLICT (project_id, ref) DO UPDATE SET last_sha = $3, polled_at = now()",
        project.to_db(),
        git_ref,
        sha,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Forgets the head of a ref of the link of a project: a closed pull request.
pub(crate) async fn forget(
    tx: &mut Tx<'_>,
    project: ProjectId,
    git_ref: &str,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM vcs_poll_state WHERE project_id = $1 AND ref = $2",
        project.to_db(),
        git_ref,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Forgets the heads of the refs with a prefix that a full list does not have: the deleted
/// branches, or the pull requests that are not open.
pub(crate) async fn forget_others(
    tx: &mut Tx<'_>,
    project: ProjectId,
    prefix: &str,
    listed: &[String],
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM vcs_poll_state
         WHERE project_id = $1 AND starts_with(ref, $2) AND NOT (ref = ANY($3))",
        project.to_db(),
        prefix,
        listed,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Keeps the cursor of a list of the link of a project: its ETag, or the time of its next read.
pub(crate) async fn cursor(
    tx: &mut Tx<'_>,
    project: ProjectId,
    list: &str,
    etag: Option<&str>,
    updated_after: Option<OffsetDateTime>,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO vcs_poll_state (project_id, ref, etag, updated_after)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (project_id, ref) DO UPDATE
         SET etag = $3, updated_after = $4, polled_at = now()",
        project.to_db(),
        list,
        etag,
        updated_after,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
