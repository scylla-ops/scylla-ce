//! The `TestApp` helpers of `TriggerService`.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::trigger_service_client::TriggerServiceClient;
use scylla_proto::pipelines::v1::{
    create_trigger_request, trigger_input, CreateTriggerRequest, CreateTriggerResponse, Cron,
    DeleteTriggerRequest, FireTriggerNowRequest, GetTriggerRequest, Job,
    ListPipelineTriggersRequest, ListPipelineTriggersResponse, SetTriggerEnabledRequest, Trigger,
    TriggerInput, TriggerInputList, UpdateTriggerRequest, Webhook,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

/// Makes an input with a literal.
pub fn literal_input(name: &str, text: &str) -> TriggerInput {
    TriggerInput {
        name: name.to_owned(),
        value: Some(trigger_input::Value::Literal(text.to_owned())),
    }
}

/// Makes an input with a JSON pointer.
pub fn pointer_input(name: &str, pointer: &str) -> TriggerInput {
    TriggerInput {
        name: name.to_owned(),
        value: Some(trigger_input::Value::JsonPointer(pointer.to_owned())),
    }
}

impl TestApp {
    /// Returns a client of `TriggerService`.
    fn triggers(&self) -> TriggerServiceClient<Channel> {
        TriggerServiceClient::new(self.channel.clone())
    }

    /// Calls `CreateTrigger` as `actor` for a cron trigger, or a webhook trigger if `schedule`
    /// is `None`.
    pub async fn create_trigger(
        &self,
        actor: ActorId,
        pipeline: &str,
        name: &str,
        schedule: Option<&str>,
        inputs: Vec<TriggerInput>,
    ) -> Result<CreateTriggerResponse, Status> {
        let kind = match schedule {
            Some(schedule) => create_trigger_request::Kind::Cron(Cron {
                schedule: schedule.to_owned(),
            }),
            None => create_trigger_request::Kind::Webhook(Webhook {
                path: String::new(),
            }),
        };
        let request = CreateTriggerRequest {
            pipeline_id: Some(id(pipeline)),
            name: name.to_owned(),
            kind: Some(kind),
            inputs,
        };
        let request = self.authed(actor, request).await;
        Ok(self.triggers().create_trigger(request).await?.into_inner())
    }

    /// Calls `GetTrigger` as `actor`.
    pub async fn get_trigger(&self, actor: ActorId, trigger: &str) -> Result<Trigger, Status> {
        let request = GetTriggerRequest {
            trigger_id: Some(id(trigger)),
        };
        let request = self.authed(actor, request).await;
        let response = self.triggers().get_trigger(request).await?.into_inner();
        Ok(response.trigger.expect("trigger in response"))
    }

    /// Calls `UpdateTrigger` as `actor` with a new schedule and new inputs.
    pub async fn update_trigger(
        &self,
        actor: ActorId,
        trigger: &str,
        schedule: Option<&str>,
        inputs: Option<Vec<TriggerInput>>,
    ) -> Result<Trigger, Status> {
        let request = UpdateTriggerRequest {
            trigger_id: Some(id(trigger)),
            name: None,
            schedule: schedule.map(str::to_owned),
            inputs: inputs.map(|inputs| TriggerInputList { inputs }),
        };
        let request = self.authed(actor, request).await;
        let response = self.triggers().update_trigger(request).await?.into_inner();
        Ok(response.trigger.expect("trigger in response"))
    }

    /// Calls `DeleteTrigger` as `actor`.
    pub async fn delete_trigger(&self, actor: ActorId, trigger: &str) -> Result<(), Status> {
        let request = DeleteTriggerRequest {
            trigger_id: Some(id(trigger)),
        };
        let request = self.authed(actor, request).await;
        self.triggers().delete_trigger(request).await?;
        Ok(())
    }

    /// Calls `ListPipelineTriggers` as `actor` for the first page.
    pub async fn list_pipeline_triggers(
        &self,
        actor: ActorId,
        pipeline: &str,
    ) -> Result<ListPipelineTriggersResponse, Status> {
        let request = ListPipelineTriggersRequest {
            pipeline_id: Some(id(pipeline)),
            page: Some(page(0, "")),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .triggers()
            .list_pipeline_triggers(request)
            .await?
            .into_inner())
    }

    /// Calls `SetTriggerEnabled` as `actor`.
    pub async fn set_trigger_enabled(
        &self,
        actor: ActorId,
        trigger: &str,
        enabled: bool,
    ) -> Result<Trigger, Status> {
        let request = SetTriggerEnabledRequest {
            trigger_id: Some(id(trigger)),
            enabled,
        };
        let request = self.authed(actor, request).await;
        let response = self
            .triggers()
            .set_trigger_enabled(request)
            .await?
            .into_inner();
        Ok(response.trigger.expect("trigger in response"))
    }

    /// Calls `FireTriggerNow` as `actor`, and returns the new job.
    pub async fn fire_trigger_now(&self, actor: ActorId, trigger: &str) -> Result<Job, Status> {
        let request = FireTriggerNowRequest {
            trigger_id: Some(id(trigger)),
        };
        let request = self.authed(actor, request).await;
        let response = self
            .triggers()
            .fire_trigger_now(request)
            .await?
            .into_inner();
        Ok(response.job.expect("job in response"))
    }
}
