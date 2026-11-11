//! The conversions between the proto messages and the domain types of the triggers.

use scylla_domain::projects::triggers::{
    FireResult, Input, InputValue, Inputs, Trigger, TriggerKind,
};
use scylla_proto::pipelines::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

/// The reason of a kind of trigger that this server does not run.
pub(super) const KIND_NOT_SUPPORTED: &str = "TRIGGER_KIND_NOT_SUPPORTED";

/// Reads and checks the inputs of a request.
pub(super) fn inputs(inputs: Vec<pb::TriggerInput>, webhook: bool) -> Result<Inputs, Error> {
    let inputs = inputs
        .into_iter()
        .enumerate()
        .map(|(index, input)| {
            let value = match input.value {
                Some(pb::trigger_input::Value::Literal(text)) => InputValue::Literal(text),
                Some(pb::trigger_input::Value::JsonPointer(pointer)) => {
                    InputValue::Pointer(pointer)
                }
                None => {
                    return Err(Error::invalid(
                        format!("inputs[{index}].value"),
                        "is required",
                    ))
                }
            };
            Ok(Input {
                name: input.name,
                value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Inputs::parse(inputs, webhook).map_err(|(path, error)| Error::invalid(path, error.to_string()))
}

impl ToProto<pb::Trigger> for Trigger {
    fn to_proto(self) -> pb::Trigger {
        let kind = match self.kind {
            TriggerKind::Cron(schedule) => pb::trigger::Kind::Cron(pb::Cron { schedule }),
            TriggerKind::Webhook => pb::trigger::Kind::Webhook(pb::Webhook {
                path: format!("/webhooks/{}", self.id),
            }),
        };
        let inputs = self
            .inputs
            .into_iter()
            .map(|input| pb::TriggerInput {
                name: input.name,
                value: Some(match input.value {
                    InputValue::Literal(text) => pb::trigger_input::Value::Literal(text),
                    InputValue::Pointer(pointer) => pb::trigger_input::Value::JsonPointer(pointer),
                }),
            })
            .collect();
        let (status, error) = match self.last_result {
            None => (pb::FireStatus::Unspecified, String::new()),
            Some(FireResult::Ok) => (pb::FireStatus::Ok, String::new()),
            Some(FireResult::Error(reason)) => (pb::FireStatus::Error, reason),
        };
        pb::Trigger {
            id: Some(self.id.to_proto()),
            pipeline_id: Some(self.pipeline_id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            kind: Some(kind),
            inputs,
            enabled: self.enabled,
            next_fire_at: self.next_fire_at.map(parse::timestamp),
            last_fired_at: self.last_fired_at.map(parse::timestamp),
            last_status: status.into(),
            last_error: error,
            created_at: Some(parse::timestamp(self.created_at)),
            updated_at: Some(parse::timestamp(self.updated_at)),
            from_file: false,
        }
    }
}
