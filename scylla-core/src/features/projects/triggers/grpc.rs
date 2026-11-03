//! The gRPC handler of `TriggerService`: authenticate, parse, authorize on the project, call the
//! service, answer.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{
    ListTriggers, NewTrigger, TriggerAccess, TriggerChange, TriggerId, TriggerKind,
};
use scylla_domain::projects::Project;
use scylla_extension::{Gate, Hooks, Principal};
use scylla_proto::common::v1::Id;
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::trigger_service_server::{TriggerService, TriggerServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{convert, fire, service};
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::Registry;
use crate::features::projects::pipelines::access;
use crate::infra::secret::Sealer;
use crate::Error;

/// Makes the service.
pub(crate) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
    registry: Arc<Registry>,
    sealer: Option<Arc<Sealer>>,
) -> TriggerServiceServer<TriggersGrpc> {
    TriggerServiceServer::new(TriggersGrpc {
        pool,
        hooks,
        gate,
        registry,
        sealer,
    })
}

/// The handler.
pub(crate) struct TriggersGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The open streams of the agents.
    registry: Arc<Registry>,
    /// The key of the secrets of the webhooks.
    sealer: Option<Arc<Sealer>>,
}

#[tonic::async_trait]
impl TriggerService for TriggersGrpc {
    async fn create_trigger(
        &self,
        request: Request<pb::CreateTriggerRequest>,
    ) -> Result<Response<pb::CreateTriggerResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let pipeline: PipelineId = parse::required(request.pipeline_id, "pipeline_id")?;
        let kind = match request.kind {
            Some(pb::create_trigger_request::Kind::Cron(cron)) => TriggerKind::Cron(cron.schedule),
            Some(pb::create_trigger_request::Kind::Webhook(_)) => TriggerKind::Webhook,
            None => return Err(Error::invalid("kind", "is required").into()),
        };
        let webhook = kind == TriggerKind::Webhook;
        let new = NewTrigger {
            pipeline_id: pipeline,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
            kind,
            inputs: convert::inputs(request.inputs, webhook)?,
        };

        // Check the route permission on the pipeline, then create the trigger.
        let place = service::pipeline_place(&self.pool, pipeline).await?;
        let object = format!("pipeline:{pipeline}");
        let gate = self.gate.as_ref();
        access::authorize(gate, &caller, TriggerAccess::CREATE_TRIGGER, object, place).await?;
        let (trigger, secret) = service::create(&self.pool, self.sealer.as_deref(), new).await?;
        Ok(Response::new(pb::CreateTriggerResponse {
            trigger: Some(trigger.to_proto()),
            webhook_secret: secret.unwrap_or_default(),
        }))
    }

    async fn get_trigger(
        &self,
        request: Request<pb::GetTriggerRequest>,
    ) -> Result<Response<pb::GetTriggerResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let permission = TriggerAccess::GET_TRIGGER;
        let id = self
            .trigger_of(&caller, request.into_inner().trigger_id, permission)
            .await?;
        let trigger = service::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound("trigger"))?;
        Ok(Response::new(pb::GetTriggerResponse {
            trigger: Some(trigger.to_proto()),
        }))
    }

    async fn update_trigger(
        &self,
        request: Request<pb::UpdateTriggerRequest>,
    ) -> Result<Response<pb::UpdateTriggerResponse>, Status> {
        // Authenticate the caller, check the route permission, then read the change.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let permission = TriggerAccess::UPDATE_TRIGGER;
        let id = self
            .trigger_of(&caller, request.trigger_id, permission)
            .await?;
        let current = service::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound("trigger"))?;
        let webhook = current.kind == TriggerKind::Webhook;
        let change = TriggerChange {
            name: request
                .name
                .map(|name| parse::field(name, "name"))
                .transpose()?,
            schedule: request.schedule,
            inputs: request
                .inputs
                .map(|list| convert::inputs(list.inputs, webhook))
                .transpose()?,
        };

        // Change the trigger.
        let trigger = service::update(&self.pool, id, change).await?;
        Ok(Response::new(pb::UpdateTriggerResponse {
            trigger: Some(trigger.to_proto()),
        }))
    }

    async fn delete_trigger(
        &self,
        request: Request<pb::DeleteTriggerRequest>,
    ) -> Result<Response<pb::DeleteTriggerResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let permission = TriggerAccess::DELETE_TRIGGER;
        let id = self
            .trigger_of(&caller, request.into_inner().trigger_id, permission)
            .await?;
        service::delete(&self.pool, id).await?;
        Ok(Response::new(pb::DeleteTriggerResponse {}))
    }

    async fn list_pipeline_triggers(
        &self,
        request: Request<pb::ListPipelineTriggersRequest>,
    ) -> Result<Response<pb::ListPipelineTriggersResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the pipeline.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let pipeline: PipelineId = parse::required(request.pipeline_id, "pipeline_id")?;
        let page = parse::optional(request.page, "page")?;
        let place = service::pipeline_place(&self.pool, pipeline).await?;
        let object = format!("pipeline:{pipeline}");
        let permission = TriggerAccess::LIST_PIPELINE_TRIGGERS;
        access::authorize(self.gate.as_ref(), &caller, permission, object, place).await?;

        // Read one page.
        let query = ListTriggers {
            pipeline_id: pipeline,
            page,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(Response::new(pb::ListPipelineTriggersResponse {
            triggers: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn set_trigger_enabled(
        &self,
        request: Request<pb::SetTriggerEnabledRequest>,
    ) -> Result<Response<pb::SetTriggerEnabledResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let permission = TriggerAccess::SET_TRIGGER_ENABLED;
        let id = self
            .trigger_of(&caller, request.trigger_id, permission)
            .await?;
        let trigger = service::set_enabled(&self.pool, id, request.enabled).await?;
        Ok(Response::new(pb::SetTriggerEnabledResponse {
            trigger: Some(trigger.to_proto()),
        }))
    }

    async fn fire_trigger_now(
        &self,
        request: Request<pb::FireTriggerNowRequest>,
    ) -> Result<Response<pb::FireTriggerNowResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let permission = TriggerAccess::FIRE_TRIGGER_NOW;
        let id = self
            .trigger_of(&caller, request.into_inner().trigger_id, permission)
            .await?;
        let job = match fire::by_id(&self.pool, &self.hooks, &self.registry, id).await {
            Ok(job) => job,
            Err(fire::FireError::Failed(error)) => return Err(error.into()),
            Err(fire::FireError::Payload(reason)) => {
                return Err(Error::invalid("body", reason).into())
            }
        };
        Ok(Response::new(pb::FireTriggerNowResponse {
            job: Some(job.to_proto()),
        }))
    }
}

impl TriggersGrpc {
    /// Reads the trigger id of a request and checks a route permission on its project. An
    /// unknown trigger gets the answer of a refusal.
    async fn trigger_of(
        &self,
        caller: &Principal,
        id: Option<Id>,
        permission: Permission<Project>,
    ) -> Result<TriggerId, Error> {
        let id: TriggerId = parse::required(id, "trigger_id")?;
        let place = service::place(&self.pool, id).await?;
        let object = format!("trigger:{id}");
        access::authorize(self.gate.as_ref(), caller, permission, object, place).await?;
        Ok(id)
    }
}
