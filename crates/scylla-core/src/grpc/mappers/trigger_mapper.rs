//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::trigger::{
    CreateTrigger, DeleteTrigger, FireTriggerNow, GetTrigger, ListPipelineTriggers,
    SetTriggerEnabled, UpdateTrigger,
};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use crate::grpc::mappers::domain_error_to_status;
use scylla_domain::domain::pipeline::EnvKey;
use scylla_domain::domain::trigger::{
    CronSpec, FireObservation, Trigger, TriggerActivation, TriggerInput, TriggerInputSource,
    TriggerName, TriggerSource, WebhookSpec,
};
use scylla_proto::trigger::v1::{
    CreateTriggerRequest, CronSpec as ProtoCronSpec, DeleteTriggerRequest,
    FireObservation as ProtoFireObservation, FireTriggerNowRequest, GetTriggerRequest,
    ListPipelineTriggersRequest, SetTriggerEnabledRequest, Trigger as ProtoTrigger,
    TriggerInput as ProtoTriggerInput, UpdateTriggerRequest, WebhookSpec as ProtoWebhookSpec,
    create_trigger_request, fire_observation, trigger as proto_trigger, trigger_input,
    update_trigger_request,
};
use tonic::Status;

impl Parse for CreateTriggerRequest {
    type Into = CreateTrigger;

    fn parse(self) -> Result<CreateTrigger, Status> {
        Ok(CreateTrigger {
            pipeline_id: id(self.pipeline_id, "pipeline_id")?,
            name: valid(self.name, TriggerName::new)?,
            source: create_source_to_domain(self.source)?,
            inputs: proto_inputs_to_domain(self.inputs)?,
        })
    }
}

impl Parse for UpdateTriggerRequest {
    type Into = UpdateTrigger;

    fn parse(self) -> Result<UpdateTrigger, Status> {
        Ok(UpdateTrigger {
            id: id(self.trigger_id, "trigger_id")?,
            name: valid(
                self.name
                    .ok_or_else(|| Status::invalid_argument("missing name"))?,
                TriggerName::new,
            )?,
            source: update_source_to_domain(self.source)?,
            inputs: proto_inputs_to_domain(self.inputs)?,
        })
    }
}

parse!(GetTriggerRequest => GetTrigger { id: id(trigger_id) });
parse!(ListPipelineTriggersRequest => ListPipelineTriggers { pipeline_id: id(pipeline_id) });
parse!(SetTriggerEnabledRequest => SetTriggerEnabled {
    id: id(trigger_id),
    enabled: copy,
});
parse!(DeleteTriggerRequest => DeleteTrigger { id: id(trigger_id) });
parse!(FireTriggerNowRequest => FireTriggerNow { id: id(trigger_id) });

fn create_source_to_domain(
    source: Option<create_trigger_request::Source>,
) -> Result<TriggerSource, Status> {
    match source {
        Some(create_trigger_request::Source::Cron(c)) => cron_to_domain(c),
        Some(create_trigger_request::Source::Webhook(w)) => webhook_to_domain(w),
        None => Err(Status::invalid_argument(
            "trigger source is required (cron or webhook)",
        )),
    }
}

fn update_source_to_domain(
    source: Option<update_trigger_request::Source>,
) -> Result<TriggerSource, Status> {
    match source {
        Some(update_trigger_request::Source::Cron(c)) => cron_to_domain(c),
        Some(update_trigger_request::Source::Webhook(w)) => webhook_to_domain(w),
        None => Err(Status::invalid_argument(
            "trigger source is required (cron or webhook)",
        )),
    }
}

fn cron_to_domain(c: ProtoCronSpec) -> Result<TriggerSource, Status> {
    Ok(TriggerSource::Cron(valid(c.expression, CronSpec::new)?))
}

fn webhook_to_domain(w: ProtoWebhookSpec) -> Result<TriggerSource, Status> {
    let header = (!w.signature_header.trim().is_empty()).then_some(w.signature_header);
    Ok(TriggerSource::Webhook(
        WebhookSpec::new(header).map_err(domain_error_to_status)?,
    ))
}

fn proto_inputs_to_domain(inputs: Vec<ProtoTriggerInput>) -> Result<Vec<TriggerInput>, Status> {
    inputs
        .into_iter()
        .map(|input| {
            let key = valid(input.key.as_str(), EnvKey::new)?;
            match input.source {
                Some(trigger_input::Source::Literal(value)) => {
                    valid(value, |v| TriggerInput::literal(key, v))
                }
                Some(trigger_input::Source::JsonPointer(pointer)) => {
                    valid(pointer, |p| TriggerInput::json_pointer(key, p))
                }
                None => Err(Status::invalid_argument(format!(
                    "trigger input '{}' has no source (literal or json_pointer)",
                    input.key
                ))),
            }
        })
        .collect()
}

pub fn trigger_to_proto(t: &Trigger, webhook_base_url: Option<&str>) -> ProtoTrigger {
    ProtoTrigger {
        trigger_id: wrap(t.id().to_string()),
        pipeline_id: wrap(t.pipeline_id().to_string()),
        name: t.name().to_string(),
        source: Some(source_to_proto(t, webhook_base_url)),
        inputs: t.inputs().iter().map(input_to_proto).collect(),
        activation: Some(activation_to_proto(t.activation())),
        last_observation: t.last_observation().map(observation_to_proto),
        created_at: ts(t.created_at()),
        updated_at: ts(t.updated_at()),
    }
}

fn activation_to_proto(activation: &TriggerActivation) -> proto_trigger::Activation {
    match activation {
        TriggerActivation::Disabled => {
            proto_trigger::Activation::Disabled(proto_trigger::Disabled {})
        }
        TriggerActivation::Enabled { next_fire_at } => {
            proto_trigger::Activation::Enabled(proto_trigger::Enabled {
                next_fire_at: next_fire_at.and_then(ts),
            })
        }
    }
}

fn observation_to_proto(observation: &FireObservation) -> ProtoFireObservation {
    let result = if observation.status == "ok" {
        fire_observation::Result::Succeeded(fire_observation::Succeeded {})
    } else {
        fire_observation::Result::Failed(fire_observation::Failed {
            error: observation.status.clone(),
        })
    };
    ProtoFireObservation {
        fired_at: ts(observation.fired_at),
        result: Some(result),
    }
}

fn source_to_proto(t: &Trigger, webhook_base_url: Option<&str>) -> proto_trigger::Source {
    match t.source() {
        TriggerSource::Cron(c) => proto_trigger::Source::Cron(proto_trigger::Cron {
            expression: c.expression().to_string(),
        }),
        TriggerSource::Webhook(w) => proto_trigger::Source::Webhook(proto_trigger::Webhook {
            signature_header: w.signature_header().unwrap_or_default().to_string(),
            url: webhook_base_url
                .map(|base| format!("{}/webhooks/{}", base.trim_end_matches('/'), t.id()))
                .unwrap_or_default(),
        }),
    }
}

fn input_to_proto(input: &TriggerInput) -> ProtoTriggerInput {
    let source = match input.source() {
        TriggerInputSource::Literal(value) => trigger_input::Source::Literal(value.to_string()),
        TriggerInputSource::JsonPointer(pointer) => {
            trigger_input::Source::JsonPointer(pointer.to_string())
        }
    };
    ProtoTriggerInput {
        key: input.key().to_string(),
        source: Some(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    fn cron(expression: &str) -> Option<create_trigger_request::Source> {
        Some(create_trigger_request::Source::Cron(ProtoCronSpec {
            expression: expression.into(),
        }))
    }

    #[test]
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateTriggerRequest {
            pipeline_id: wrap("pl-1"),
            name: " nightly ".into(),
            source: cron("0 3 * * *"),
            inputs: vec![ProtoTriggerInput {
                key: "MODE".into(),
                source: Some(trigger_input::Source::Literal("nightly".into())),
            }],
        }
        .parse()
        .unwrap();

        assert_eq!(command.pipeline_id.as_str(), "pl-1");
        assert_eq!(command.name.as_str(), "nightly");
        assert!(
            matches!(command.source, TriggerSource::Cron(ref c) if c.expression() == "0 3 * * *")
        );
        assert_eq!(command.inputs.len(), 1);
        assert_eq!(command.inputs[0].key().to_string(), "MODE");
    }

    #[test]
    fn a_blank_signature_header_means_the_default_one() {
        let command = CreateTriggerRequest {
            pipeline_id: wrap("pl-1"),
            name: "hook".into(),
            source: Some(create_trigger_request::Source::Webhook(ProtoWebhookSpec {
                signature_header: "  ".into(),
            })),
            inputs: Vec::new(),
        }
        .parse()
        .unwrap();

        let TriggerSource::Webhook(spec) = command.source else {
            panic!("expected a webhook source");
        };
        assert_eq!(spec, WebhookSpec::new(None).unwrap());
    }

    #[test]
    fn a_missing_pipeline_id_is_an_invalid_argument() {
        let err = ListPipelineTriggersRequest {
            pipeline_id: None::<common::PipelineId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing pipeline_id");
    }

    #[test]
    fn a_create_without_a_source_is_an_invalid_argument() {
        let err = CreateTriggerRequest {
            pipeline_id: wrap("pl-1"),
            name: "nightly".into(),
            source: None,
            inputs: Vec::new(),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(
            err.message(),
            "trigger source is required (cron or webhook)"
        );
    }

    #[test]
    fn an_input_without_a_source_is_an_invalid_argument() {
        let err = CreateTriggerRequest {
            pipeline_id: wrap("pl-1"),
            name: "nightly".into(),
            source: cron("0 3 * * *"),
            inputs: vec![ProtoTriggerInput {
                key: "MODE".into(),
                source: None,
            }],
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(
            err.message(),
            "trigger input 'MODE' has no source (literal or json_pointer)"
        );
    }

    #[test]
    fn an_invalid_name_is_an_invalid_argument() {
        let err = CreateTriggerRequest {
            pipeline_id: wrap("pl-1"),
            name: "   ".into(),
            source: cron("0 3 * * *"),
            inputs: Vec::new(),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
