//! A row of the `pipelines` table, and the pipeline that it gives.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::pipelines::{FileOrigin, Pipeline, PipelineId};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde_json::Value;
use time::OffsetDateTime;

use super::super::json;
use crate::Error;

/// A row of the `pipelines` table.
pub(super) struct Row {
    /// The id.
    pub(super) id: uuid::Uuid,
    /// The project.
    pub(super) project_id: uuid::Uuid,
    /// The workspace.
    pub(super) workspace_id: uuid::Uuid,
    /// The author.
    pub(super) created_by: uuid::Uuid,
    /// The display name.
    pub(super) name: String,
    /// The nodes in JSON.
    pub(super) nodes: Value,
    /// The checkout settings in JSON, or `None` for the default.
    pub(super) checkout: Option<Value>,
    /// The version.
    pub(super) version: i64,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The time of the last change.
    pub(super) updated_at: OffsetDateTime,
    /// `editor` or `file`.
    pub(super) source: String,
    /// The key of a file pipeline.
    pub(super) file_key: Option<String>,
    /// The path of the file of a file pipeline.
    pub(super) file_path: Option<String>,
    /// The number of errors of the file.
    pub(super) file_errors: i32,
    /// The time when the pipeline stopped.
    pub(super) archived_at: Option<OffsetDateTime>,
    /// The time when the file left the default branch.
    pub(super) removed_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the pipeline of the row.
    pub(super) fn into_pipeline(self) -> Result<Pipeline, Error> {
        let source = PipelineSource::parse(&self.source).ok_or_else(|| {
            Error::Internal(format!("unknown source of a pipeline: {}", self.source))
        })?;
        let file = match (self.file_key, self.file_path) {
            (Some(key), Some(path)) => Some(FileOrigin {
                key,
                path,
                errors: u32::try_from(self.file_errors).unwrap_or_default(),
                removed_at: self.removed_at,
            }),
            _ => None,
        };
        Ok(Pipeline {
            id: PipelineId::from_db(self.id),
            project_id: ProjectId::from_db(self.project_id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            nodes: json::read_nodes(self.nodes)?,
            checkout: self
                .checkout
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::Internal(format!("bad checkout of a pipeline: {error}")))?
                .unwrap_or_default(),
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            source,
            file,
            archived_at: self.archived_at,
        })
    }
}
