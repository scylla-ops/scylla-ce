//! The SQL of the activity feed: the rows that the changes record, the pages that a reader sees,
//! and the delete of the old rows.

use std::collections::BTreeMap;

use scylla_domain::activity::{
    Activity, ActivityId, NewActivity, Place, Subject, SubjectId, Visible,
};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The visible areas and projects of a reader, as the arrays of a query.
struct Arrays {
    /// The areas in which the reader sees each row of the workspace.
    areas: Vec<String>,
    /// The projects of the pairs in which the reader sees the rows.
    projects: Vec<Uuid>,
    /// The areas of the pairs, in the order of `projects`.
    project_areas: Vec<String>,
}

impl Arrays {
    /// Makes the arrays of what a reader sees.
    fn of(visible: &Visible) -> Self {
        Self {
            areas: visible
                .areas
                .iter()
                .map(|area| area.as_str().to_owned())
                .collect(),
            projects: visible.projects.iter().map(|(id, _)| id.to_db()).collect(),
            project_areas: visible
                .projects
                .iter()
                .map(|(_, area)| area.as_str().to_owned())
                .collect(),
        }
    }
}

/// Records a row. The workspace of a row of a project is the workspace of the project.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewActivity) -> Result<(), Error> {
    let (workspace, project) = match new.place {
        Place::Workspace(workspace) => (Some(workspace.to_db()), None),
        Place::Project(project) => (None, Some(project.to_db())),
    };
    let (subject_kind, subject_id) = new
        .subject
        .as_ref()
        .map(|subject| (subject.kind.clone(), subject.id.to_db()))
        .unzip();
    sqlx::query!(
        "INSERT INTO activity (id, workspace_id, project_id, area, kind, actor_id, subject_kind,
             subject_id, params)
         VALUES ($1, COALESCE($2, (SELECT workspace_id FROM projects WHERE id = $3)), $3, $4, $5,
             $6, $7, $8, $9)",
        ActivityId::generate().to_db(),
        workspace,
        project,
        new.kind.area().as_str(),
        new.kind.as_str(),
        new.actor.map(ActorId::to_db),
        subject_kind,
        subject_id,
        Json(&new.params) as _,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns one page of the rows of a workspace that a reader sees, newest first, with one extra
/// row.
pub(super) async fn list(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    visible: &Visible,
    page: PageRequest<ActivityId>,
) -> Result<Vec<Activity>, Error> {
    let arrays = Arrays::of(visible);
    let rows = sqlx::query!(
        r#"SELECT e.id, e.kind, e.area, e.actor_id, e.project_id, e.subject_kind, e.subject_id,
                  e.params AS "params: Json<BTreeMap<String, String>>", e.created_at,
                  COALESCE(u.display_name, a.name) AS actor_name, p.name AS "project_name?"
           FROM activity AS e
           LEFT JOIN users AS u ON u.id = e.actor_id
           LEFT JOIN apps AS a ON a.id = e.actor_id
           LEFT JOIN projects AS p ON p.id = e.project_id
           WHERE e.workspace_id = $1
             AND (e.area = ANY($2::text[])
                  OR (e.project_id, e.area) IN (SELECT * FROM unnest($3::uuid[], $4::text[])))
             AND ($5::uuid IS NULL OR (e.created_at, e.id) < (
                 SELECT c.created_at, c.id FROM activity AS c
                 WHERE c.id = $5 AND c.workspace_id = $1))
           ORDER BY e.created_at DESC, e.id DESC
           LIMIT $6"#,
        workspace.to_db(),
        &arrays.areas,
        &arrays.projects,
        &arrays.project_areas,
        page.after.map(ActivityId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Activity {
            id: ActivityId::from_db(row.id),
            kind: row.kind,
            area: row.area,
            actor_id: row.actor_id.map(ActorId::from_db),
            actor_name: row.actor_name,
            project_id: row.project_id.map(ProjectId::from_db),
            project_name: row.project_name,
            subject: row
                .subject_kind
                .zip(row.subject_id)
                .map(|(kind, id)| Subject {
                    kind,
                    id: SubjectId::from_db(id),
                }),
            params: row.params.0,
            at: row.created_at,
        })
        .collect())
}

/// Returns the number of the rows of a workspace that a reader sees at a time or later, up to a
/// cap.
pub(super) async fn count_since(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    visible: &Visible,
    since: OffsetDateTime,
    cap: i32,
) -> Result<i64, Error> {
    let arrays = Arrays::of(visible);
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM (
               SELECT 1 FROM activity AS e
               WHERE e.workspace_id = $1 AND e.created_at >= $5
                 AND (e.area = ANY($2::text[])
                      OR (e.project_id, e.area) IN (SELECT * FROM unnest($3::uuid[], $4::text[])))
               LIMIT $6) AS capped"#,
        workspace.to_db(),
        &arrays.areas,
        &arrays.projects,
        &arrays.project_areas,
        since,
        i64::from(cap),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(count)
}

/// Returns the name of a pipeline, or `None` if no pipeline has this id.
pub(super) async fn pipeline_name(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
) -> Result<Option<String>, Error> {
    let name = sqlx::query_scalar!("SELECT name FROM pipelines WHERE id = $1", pipeline.to_db(),)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(name)
}

/// Returns the display name of an actor: the name of a user or of an App, or `None`.
pub(super) async fn actor_name(tx: &mut Tx<'_>, actor: ActorId) -> Result<Option<String>, Error> {
    let name = sqlx::query_scalar!(
        "SELECT COALESCE((SELECT display_name FROM users WHERE id = $1),
                         (SELECT name FROM apps WHERE id = $1))",
        actor.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(name)
}

/// Returns the kind and the display name of an actor, or `None` if no actor has this id.
pub(super) async fn actor(
    tx: &mut Tx<'_>,
    actor: ActorId,
) -> Result<Option<(String, Option<String>)>, Error> {
    let row = sqlx::query!(
        r#"SELECT ac.kind, COALESCE(u.display_name, a.name) AS name
           FROM actors AS ac
           LEFT JOIN users AS u ON u.id = ac.id
           LEFT JOIN apps AS a ON a.id = ac.id
           WHERE ac.id = $1"#,
        actor.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| (row.kind, row.name)))
}

/// Deletes the rows recorded before a time. Returns their number.
pub(super) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, Error> {
    let deleted = sqlx::query!("DELETE FROM activity WHERE created_at < $1", before)
        .execute(&mut **tx)
        .await?;
    Ok(deleted.rows_affected())
}
