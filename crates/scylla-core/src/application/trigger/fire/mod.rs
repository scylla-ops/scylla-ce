pub mod commands;

pub use commands::FireTriggerNow;

use crate::domain::errors::DomainResult;
use crate::domain::pipeline::{EnvKey, EnvValue};
use crate::domain::trigger::{Trigger, TriggerInputSource};

pub fn resolve_inputs(
    trigger: &Trigger,
    payload: Option<&serde_json::Value>,
) -> DomainResult<Vec<(EnvKey, EnvValue)>> {
    trigger
        .inputs()
        .iter()
        .filter_map(|input| {
            let value = match input.source() {
                TriggerInputSource::Literal(value) => Ok(value.clone()),
                TriggerInputSource::JsonPointer(pointer) => {
                    EnvValue::new(json_value_to_env(payload?.pointer(pointer.as_str())?))
                }
            };
            Some(value.map(|value| (input.key().clone(), value)))
        })
        .collect()
}

fn json_value_to_env(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::domain::ids::PipelineId;
    use crate::domain::pipeline::EnvKey;
    use crate::domain::trigger::{CronSpec, TriggerInput, TriggerName, TriggerSource, WebhookSpec};
    use serde_json::json;

    fn webhook_trigger(inputs: Vec<TriggerInput>) -> Trigger {
        Trigger::create(
            PipelineId::new("p"),
            TriggerName::new("on-push").unwrap(),
            TriggerSource::Webhook(WebhookSpec::new(None).unwrap()),
            inputs,
        )
        .unwrap()
    }

    fn key(k: &str) -> EnvKey {
        EnvKey::new(k).unwrap()
    }

    fn resolved(trigger: &Trigger, payload: Option<&serde_json::Value>) -> Vec<(String, String)> {
        resolve_inputs(trigger, payload)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn literal_inputs_resolve_without_payload() {
        let trigger = Trigger::create(
            PipelineId::new("p"),
            TriggerName::new("nightly").unwrap(),
            TriggerSource::Cron(CronSpec::new("0 9 * * *").unwrap()),
            vec![TriggerInput::literal(key("RUN_MODE"), "nightly").unwrap()],
        )
        .unwrap();
        let resolved = resolved(&trigger, None);
        assert_eq!(
            resolved,
            vec![("RUN_MODE".to_string(), "nightly".to_string())]
        );
    }

    #[test]
    fn json_pointer_inputs_resolve_from_payload() {
        let trigger = webhook_trigger(vec![
            TriggerInput::json_pointer(key("GIT_COMMIT"), "/after").unwrap(),
            TriggerInput::json_pointer(key("REPO"), "/repository/name").unwrap(),
        ]);
        let payload = json!({ "after": "abc123", "repository": { "name": "scylla" } });
        let resolved = resolved(&trigger, Some(&payload));
        assert!(resolved.contains(&("GIT_COMMIT".to_string(), "abc123".to_string())));
        assert!(resolved.contains(&("REPO".to_string(), "scylla".to_string())));
    }

    #[test]
    fn json_pointer_skipped_without_payload_or_when_absent() {
        let trigger = webhook_trigger(vec![
            TriggerInput::json_pointer(key("GIT_COMMIT"), "/after").unwrap(),
        ]);
        assert!(resolved(&trigger, None).is_empty());
        let payload = json!({ "other": 1 });
        assert!(resolved(&trigger, Some(&payload)).is_empty());
    }

    #[test]
    fn non_string_json_values_are_coerced() {
        let trigger = webhook_trigger(vec![
            TriggerInput::json_pointer(key("COUNT"), "/n").unwrap(),
            TriggerInput::json_pointer(key("FLAG"), "/ok").unwrap(),
        ]);
        let payload = json!({ "n": 42, "ok": true });
        let resolved = resolved(&trigger, Some(&payload));
        assert!(resolved.contains(&("COUNT".to_string(), "42".to_string())));
        assert!(resolved.contains(&("FLAG".to_string(), "true".to_string())));
    }

    #[test]
    fn a_payload_value_with_nul_is_a_validation_error() {
        let trigger = webhook_trigger(vec![TriggerInput::json_pointer(key("S"), "/s").unwrap()]);
        let payload = json!({ "s": "a\u{0}b" });
        let err = resolve_inputs(&trigger, Some(&payload)).unwrap_err();
        assert!(matches!(
            err,
            crate::domain::errors::DomainError::Validation(_)
        ));
    }
}
