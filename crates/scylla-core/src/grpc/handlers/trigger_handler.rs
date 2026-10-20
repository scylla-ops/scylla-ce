//! The adapter: each RPC is one `run` and its response.

use crate::application::{TriggerFireUseCases, TriggerUseCases};
use crate::domain::trigger::Trigger;
use crate::grpc::adapter::run;
use crate::grpc::convert::wrap;
use crate::grpc::mappers::trigger_mapper::trigger_to_proto;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::trigger::v1::{
    CreateTriggerRequest, CreateTriggerResponse, DeleteTriggerRequest, DeleteTriggerResponse,
    FireTriggerNowRequest, FireTriggerNowResponse, GetTriggerRequest, GetTriggerResponse,
    ListPipelineTriggersRequest, ListPipelineTriggersResponse, SetTriggerEnabledRequest,
    SetTriggerEnabledResponse, Trigger as ProtoTrigger, UpdateTriggerRequest,
    UpdateTriggerResponse, trigger_service_server::TriggerService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct TriggerHandler {
    actions: Arc<Actions>,
    triggers: Arc<TriggerUseCases>,
    fires: Arc<TriggerFireUseCases>,
    webhook_base_url: Option<String>,
}

impl TriggerHandler {
    fn view(&self, trigger: &Trigger) -> ProtoTrigger {
        trigger_to_proto(trigger, self.webhook_base_url.as_deref())
    }
}

#[async_trait::async_trait]
impl TriggerService for TriggerHandler {
    async fn create_trigger(
        &self,
        request: Request<CreateTriggerRequest>,
    ) -> Result<Response<CreateTriggerResponse>, Status> {
        let created = run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(CreateTriggerResponse {
            trigger: Some(self.view(&created.trigger)),
            webhook_secret: created.webhook_secret,
        }))
    }

    async fn get_trigger(
        &self,
        request: Request<GetTriggerRequest>,
    ) -> Result<Response<GetTriggerResponse>, Status> {
        let trigger = run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(GetTriggerResponse {
            trigger: Some(self.view(&trigger)),
        }))
    }

    async fn update_trigger(
        &self,
        request: Request<UpdateTriggerRequest>,
    ) -> Result<Response<UpdateTriggerResponse>, Status> {
        let trigger = run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(UpdateTriggerResponse {
            trigger: Some(self.view(&trigger)),
        }))
    }

    async fn delete_trigger(
        &self,
        request: Request<DeleteTriggerRequest>,
    ) -> Result<Response<DeleteTriggerResponse>, Status> {
        run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(DeleteTriggerResponse {}))
    }

    async fn list_pipeline_triggers(
        &self,
        request: Request<ListPipelineTriggersRequest>,
    ) -> Result<Response<ListPipelineTriggersResponse>, Status> {
        let triggers = run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(ListPipelineTriggersResponse {
            triggers: triggers.iter().map(|t| self.view(t)).collect(),
        }))
    }

    async fn set_trigger_enabled(
        &self,
        request: Request<SetTriggerEnabledRequest>,
    ) -> Result<Response<SetTriggerEnabledResponse>, Status> {
        let trigger = run(&self.actions, &*self.triggers, request).await?;
        Ok(Response::new(SetTriggerEnabledResponse {
            trigger: Some(self.view(&trigger)),
        }))
    }

    async fn fire_trigger_now(
        &self,
        request: Request<FireTriggerNowRequest>,
    ) -> Result<Response<FireTriggerNowResponse>, Status> {
        let job = run(&self.actions, &*self.fires, request).await?;
        Ok(Response::new(FireTriggerNowResponse {
            job_id: wrap(job.id().to_string()),
        }))
    }
}
