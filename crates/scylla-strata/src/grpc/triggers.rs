use super::{authed, parse};
use crate::{Triggers, TriggersRpc};
use scylla_core::grpc::convert::wrap;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::trigger_to_proto;
use scylla_proto::trigger::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::trigger::trigger_service_client::TriggerServiceClient;
pub use super::service::trigger::trigger_service_server::{TriggerService, TriggerServiceServer};

strata_tonic::service! {
    impl<A: Triggers> TriggerService for Grpc<A> {
        rpc create_trigger(pb::CreateTriggerRequest) -> pb::CreateTriggerResponse = |call, req| {
            authed(&call).await?;
            let (pipeline_id, name, source, inputs) = parse(req)?;
            let created = TriggersRpc::create_trigger(&call.scoped(), pipeline_id, name, source, inputs).await.map_err(domain_error_to_status)?;
            let base = Triggers::trigger_links(call.app()).base();
            Ok(pb::CreateTriggerResponse { trigger: Some(trigger_to_proto(&created.trigger, base)), webhook_secret: created.webhook_secret })
        }
        rpc get_trigger(pb::GetTriggerRequest) -> pb::GetTriggerResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let trigger = TriggersRpc::get_trigger(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            let base = Triggers::trigger_links(call.app()).base();
            Ok(pb::GetTriggerResponse { trigger: Some(trigger_to_proto(&trigger, base)) })
        }
        rpc update_trigger(pb::UpdateTriggerRequest) -> pb::UpdateTriggerResponse = |call, req| {
            authed(&call).await?;
            let (id, name, source, inputs) = parse(req)?;
            let trigger = TriggersRpc::update_trigger(&call.scoped(), id, name, source, inputs).await.map_err(domain_error_to_status)?;
            let base = Triggers::trigger_links(call.app()).base();
            Ok(pb::UpdateTriggerResponse { trigger: Some(trigger_to_proto(&trigger, base)) })
        }
        rpc delete_trigger(pb::DeleteTriggerRequest) -> pb::DeleteTriggerResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            TriggersRpc::delete_trigger(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteTriggerResponse {})
        }
        rpc list_pipeline_triggers(pb::ListPipelineTriggersRequest) -> pb::ListPipelineTriggersResponse = |call, req| {
            authed(&call).await?;
            let pipeline_id = parse(req)?;
            let triggers = TriggersRpc::list_pipeline_triggers(&call.scoped(), pipeline_id).await.map_err(domain_error_to_status)?;
            let base = Triggers::trigger_links(call.app()).base();
            Ok(pb::ListPipelineTriggersResponse { triggers: triggers.iter().map(|t| trigger_to_proto(t, base)).collect() })
        }
        rpc set_trigger_enabled(pb::SetTriggerEnabledRequest) -> pb::SetTriggerEnabledResponse = |call, req| {
            authed(&call).await?;
            let (id, enabled) = parse(req)?;
            let trigger = TriggersRpc::set_trigger_enabled(&call.scoped(), id, enabled).await.map_err(domain_error_to_status)?;
            let base = Triggers::trigger_links(call.app()).base();
            Ok(pb::SetTriggerEnabledResponse { trigger: Some(trigger_to_proto(&trigger, base)) })
        }
        rpc fire_trigger_now(pb::FireTriggerNowRequest) -> pb::FireTriggerNowResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let job = TriggersRpc::fire_trigger_now(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::FireTriggerNowResponse { job_id: wrap(job.id().to_string()) })
        }
    }
}
