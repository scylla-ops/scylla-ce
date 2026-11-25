//! The `TestApp` helpers of the labels that the agent of a job of a pipeline must have.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{
    AgentSettings, CreatePipelineRequest, Node, Pipeline, UpdatePipelineRequest,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreatePipeline` as `actor` in a project, with the labels that the agent of a job
    /// must have.
    pub async fn create_pipeline_with_labels(
        &self,
        actor: ActorId,
        project: &str,
        nodes: Vec<Node>,
        labels: &[&str],
    ) -> Result<Pipeline, Status> {
        let request = CreatePipelineRequest {
            project_id: Some(id(project)),
            name: "build".to_owned(),
            nodes,
            agent: Some(AgentSettings {
                labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            }),
            ..CreatePipelineRequest::default()
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().create_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }

    /// Calls `UpdatePipeline` as `actor` with the labels that the agent of a job must have.
    pub async fn set_pipeline_labels(
        &self,
        actor: ActorId,
        pipeline: &str,
        version: i64,
        labels: &[&str],
    ) -> Result<Pipeline, Status> {
        let request = UpdatePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            version,
            agent: Some(AgentSettings {
                labels: labels.iter().map(|label| (*label).to_owned()).collect(),
            }),
            ..UpdatePipelineRequest::default()
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().update_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }
}
