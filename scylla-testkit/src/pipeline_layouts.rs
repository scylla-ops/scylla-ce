//! The `TestApp` helpers of the places of the nodes of a pipeline.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{
    CreatePipelineRequest, GetPipelineRequest, Node, NodeList, Pipeline, PipelineLayout,
    SavePipelineLayoutRequest, UpdatePipelineRequest,
};
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreatePipeline` as `actor` in a project, with places.
    pub async fn create_pipeline_with_layout(
        &self,
        actor: ActorId,
        project: &str,
        nodes: Vec<Node>,
        layout: PipelineLayout,
    ) -> Result<Pipeline, Status> {
        let request = CreatePipelineRequest {
            project_id: Some(id(project)),
            name: "build".to_owned(),
            nodes,
            layout: Some(layout),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().create_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }

    /// Calls `GetPipeline` as `actor`, and returns the places of the answer.
    pub async fn pipeline_layout(
        &self,
        actor: ActorId,
        pipeline: &str,
    ) -> Result<Option<PipelineLayout>, Status> {
        let request = GetPipelineRequest {
            pipeline_id: Some(id(pipeline)),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().get_pipeline(request).await?;
        Ok(response.into_inner().layout)
    }

    /// Calls `UpdatePipeline` as `actor` with a version, new nodes and places.
    pub async fn update_pipeline_with_layout(
        &self,
        actor: ActorId,
        pipeline: &str,
        version: i64,
        nodes: Option<Vec<Node>>,
        layout: PipelineLayout,
    ) -> Result<Pipeline, Status> {
        let request = UpdatePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            version,
            name: None,
            nodes: nodes.map(|nodes| NodeList { nodes }),
            layout: Some(layout),
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().update_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }

    /// Calls `SavePipelineLayout` as `actor`.
    pub async fn save_pipeline_layout(
        &self,
        actor: ActorId,
        pipeline: &str,
        layout: Option<PipelineLayout>,
    ) -> Result<(), Status> {
        let request = SavePipelineLayoutRequest {
            pipeline_id: Some(id(pipeline)),
            layout,
        };
        let request = self.authed(actor, request).await;
        self.pipelines().save_pipeline_layout(request).await?;
        Ok(())
    }
}
