//! The SQL of the `job_logs` table: one row for each batch of lines of a node.

use scylla_domain::projects::jobs::{JobId, LogBatch, LogBatchId, LogLine, LogStream};
use scylla_extension::Tx;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::Error;

/// A line in JSON. The time is in unix microseconds.
#[derive(Serialize, Deserialize)]
struct LineJson {
    /// `true` for the standard error.
    err: bool,
    /// The text.
    text: String,
    /// The time.
    at: i64,
}

/// Inserts a batch of lines of a node of a job.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    job: JobId,
    node: &str,
    lines: &[LogLine],
) -> Result<(), Error> {
    let lines: Vec<LineJson> = lines
        .iter()
        .map(|line| LineJson {
            err: line.stream == LogStream::Stderr,
            text: line.text.clone(),
            at: (line.at.unix_timestamp_nanos() / 1_000) as i64,
        })
        .collect();
    let lines = serde_json::to_value(lines).map_err(|error| Error::Internal(error.to_string()))?;
    sqlx::query!(
        "INSERT INTO job_logs (id, job_id, node_id, lines) VALUES ($1, $2, $3, $4)",
        LogBatchId::generate().to_db(),
        job.to_db(),
        node,
        lines,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns at most `limit` batches of a job after a cursor, in order, of one node if it is set.
pub(crate) async fn after(
    tx: &mut Tx<'_>,
    job: JobId,
    node: Option<&str>,
    cursor: Option<LogBatchId>,
    limit: i64,
) -> Result<Vec<LogBatch>, Error> {
    let rows = sqlx::query!(
        "SELECT id, node_id, lines FROM job_logs
         WHERE job_id = $1 AND ($2::text IS NULL OR node_id = $2) AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id LIMIT $4",
        job.to_db(),
        node,
        cursor.map(LogBatchId::to_db),
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let lines: Vec<LineJson> = serde_json::from_value(row.lines)
                .map_err(|error| Error::Internal(format!("bad log data: {error}")))?;
            let lines = lines
                .into_iter()
                .map(|line| LogLine {
                    stream: if line.err {
                        LogStream::Stderr
                    } else {
                        LogStream::Stdout
                    },
                    text: line.text,
                    at: OffsetDateTime::from_unix_timestamp_nanos(i128::from(line.at) * 1_000)
                        .unwrap_or(OffsetDateTime::UNIX_EPOCH),
                })
                .collect();
            Ok(LogBatch {
                id: LogBatchId::from_db(row.id),
                node: row.node_id,
                lines,
            })
        })
        .collect()
}

/// Deletes the batches older than `before`, and returns their count.
pub(crate) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM job_logs WHERE created_at < $1", before)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}
