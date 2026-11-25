//! The editor pipelines next to the file pipelines of a project of files: the key of a pipeline
//! in the name of its report, and the opt-in of an admin for the Git events of an editor pipeline.

use time::OffsetDateTime;

use super::Pipeline;
use crate::identity::actors::ActorId;
use crate::integrations::reports::pipeline_key;

/// The opt-in of a workspace admin: the push, pull request and tag triggers of an editor pipeline
/// start runs in a project of files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitEventsOptIn {
    /// The time of the opt-in.
    pub at: OffsetDateTime,
    /// The actor of the opt-in.
    pub by: ActorId,
}

impl Pipeline {
    /// Returns the key of the pipeline in the name of its report: the key of its file, else the
    /// key of its name. The file pipelines and the editor pipelines of a project share the keys.
    pub fn key(&self) -> String {
        match &self.file {
            Some(file) => file.key.clone(),
            None => pipeline_key(&self.name),
        }
    }
}
