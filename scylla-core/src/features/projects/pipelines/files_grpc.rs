//! The RPCs of `PipelineService` for the pipeline files of the linked repository: validate,
//! export, read, list, and the source of the pipelines of a project.

use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::PipelinesGrpc;
use crate::Error;

impl PipelinesGrpc {
    /// Answers `ValidatePipelineFile`: the server does not serve it.
    pub(super) async fn validate_file(
        &self,
        request: Request<pb::ValidatePipelineFileRequest>,
    ) -> Result<Response<pb::ValidatePipelineFileResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("ValidatePipelineFile is not served"))
    }

    /// Answers `ExportPipeline`: the server does not serve it.
    pub(super) async fn export(
        &self,
        request: Request<pb::ExportPipelineRequest>,
    ) -> Result<Response<pb::ExportPipelineResponse>, Status> {
        self.gate.caller(&request).await.map_err(Error::from)?;
        Err(Status::unimplemented("ExportPipeline is not served"))
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
