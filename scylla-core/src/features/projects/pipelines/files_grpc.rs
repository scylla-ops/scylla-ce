//! The RPCs of `PipelineService` for the pipeline files of the linked repository: validate,
//! export, read, list, and the source of the pipelines of a project.

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

    /// Answers `GetFilePipeline`: the file of a file pipeline at a commit, with its text, its
    /// nodes and its errors.
    pub(super) async fn get_file(
        &self,
        request: Request<pb::GetFilePipelineRequest>,
    ) -> Result<Response<pb::GetFilePipelineResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let git_ref = request.r#ref.map(|value| parse::field(value, "ref"));
        let git_ref = git_ref.transpose()?;

        // Check the route permission on the pipeline, then read its file.
        let permission = PipelineAccess::GET_FILE_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;
        let found = files_service::get_file(&self.vcs, id, git_ref).await?;
        Ok(Response::new(pb::GetFilePipelineResponse {
            file: Some(files_convert::file_at(found, true)),
        }))
    }

    /// Answers `ListPipelineFiles`: the pipeline files of the linked repository at a commit, with
    /// no text.
    pub(super) async fn list_files(
        &self,
        request: Request<pb::ListPipelineFilesRequest>,
    ) -> Result<Response<pb::ListPipelineFilesResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let git_ref = request.r#ref.map(|value| parse::field(value, "ref"));
        let git_ref = git_ref.transpose()?;

        // Check the route permission on the project, then read the files.
        let permission = PipelineAccess::LIST_PIPELINE_FILES;
        self.authorize_in(&caller, permission, project).await?;
        let found = files_service::list_files(&self.vcs, project, git_ref).await?;
        Ok(Response::new(pb::ListPipelineFilesResponse {
            files: found
                .files
                .into_iter()
                .map(|file| files_convert::file_at(file, false))
                .collect(),
            errors: found
                .rejected
                .into_iter()
                .map(files_convert::rejected_of)
                .collect(),
            commit_sha: found.commit_sha,
        }))
    }

    /// Answers `SetPipelineSource`: the editor or the files of the linked repository.
    pub(super) async fn set_source(
        &self,
        request: Request<pb::SetPipelineSourceRequest>,
    ) -> Result<Response<pb::SetPipelineSourceResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let source = files_convert::source(request.source)?;

        // Check the route permission on the project, then move it.
        let permission = PipelineAccess::SET_PIPELINE_SOURCE;
        self.authorize_in(&caller, permission, project).await?;
        files_service::set_source(&self.vcs, caller.actor.id, project, source).await?;
        Ok(Response::new(pb::SetPipelineSourceResponse {}))
    }
}
