//! The `TestApp` helpers of the check and the export of the pipeline files.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{
    ExportPipelineRequest, ExportPipelineResponse, PipelineFile, PipelineFileFormat,
    ValidatePipelineFileRequest,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `ValidatePipelineFile` as `actor` with the path and the text of a file of a project,
    /// and returns the file as the server reads it.
    pub async fn validate_pipeline_file(
        &self,
        actor: ActorId,
        project: &str,
        path: &str,
        content: &str,
    ) -> Result<PipelineFile, Status> {
        let request = ValidatePipelineFileRequest {
            project_id: Some(id(project)),
            path: path.to_owned(),
            content: content.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().validate_pipeline_file(request).await?;
        Ok(response.into_inner().file.expect("file in response"))
    }

    /// Calls `ExportPipeline` as `actor` in a format, and returns the whole answer.
    pub async fn export_pipeline(
        &self,
        actor: ActorId,
        pipeline: &str,
        format: PipelineFileFormat,
    ) -> Result<ExportPipelineResponse, Status> {
        let request = ExportPipelineRequest {
            pipeline_id: Some(id(pipeline)),
            format: format.into(),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .pipelines()
            .export_pipeline(request)
            .await?
            .into_inner())
    }
}
