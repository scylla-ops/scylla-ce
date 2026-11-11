//! The RPCs of `PipelineService` for the pipeline files of the linked repository: validate,
//! export, read, list, and the source of the pipelines of a project. Read, list and the source
//! are not served yet.

use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_domain::projects::ProjectId;
use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::PipelinesGrpc;
use super::{files_convert, files_service};
use crate::api::parse;
use crate::Error;

impl PipelinesGrpc {
    /// Answers `ValidatePipelineFile`: reads a file that the client sends, with its errors and
    /// its warnings. Nothing changes.
    pub(super) async fn validate_file(
        &self,
        request: Request<pb::ValidatePipelineFileRequest>,
    ) -> Result<Response<pb::ValidatePipelineFileResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;

        // Check the route permission on the project, then read the file.
        let permission = PipelineAccess::VALIDATE_PIPELINE_FILE;
        self.authorize_in(&caller, permission, project).await?;
        let checked =
            files_service::validate(&self.pool, project, &request.path, &request.content).await?;
        Ok(Response::new(pb::ValidatePipelineFileResponse {
            file: Some(files_convert::checked_file(checked, request.content)),
        }))
    }

    /// Answers `ExportPipeline`: writes a pipeline as a file in a format.
    pub(super) async fn export(
        &self,
        request: Request<pb::ExportPipelineRequest>,
    ) -> Result<Response<pb::ExportPipelineResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let format = files_convert::format(request.format)?;

        // Check the route permission on the pipeline, then write its file.
        let permission = PipelineAccess::EXPORT_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;
        let file = files_service::export(&self.pool, id, format).await?;
        Ok(Response::new(pb::ExportPipelineResponse {
            content: file.content,
            path: file.path,
        }))
    }

    /// Answers `GetFilePipeline`: the server does not serve it.
    pub(super) async fn get_file(
        &self,
        request: Request<pb::GetFilePipelineRequest>,
    ) -> Result<Response<pb::GetFilePipelineResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("GetFilePipeline is not served"))
    }

    /// Answers `ListPipelineFiles`: the server does not serve it.
    pub(super) async fn list_files(
        &self,
        request: Request<pb::ListPipelineFilesRequest>,
    ) -> Result<Response<pb::ListPipelineFilesResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("ListPipelineFiles is not served"))
    }

    /// Answers `SetPipelineSource`: the server does not serve it.
    pub(super) async fn set_source(
        &self,
        request: Request<pb::SetPipelineSourceRequest>,
    ) -> Result<Response<pb::SetPipelineSourceResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("SetPipelineSource is not served"))
    }
}
