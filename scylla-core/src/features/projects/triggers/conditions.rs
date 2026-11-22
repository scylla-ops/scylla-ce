//! The conversions between the proto messages and the domain types of the conditions of a
//! webhook trigger.

use scylla_domain::projects::triggers::{
    Condition, ConditionOperator, ConditionSource, WebhookRule,
};
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::webhook_condition;

use crate::Error;

/// Reads the conditions of a webhook. A condition with no source or no operator gives
/// `INVALID_ARGUMENT` on its field. The caller checks the rule.
pub(super) fn rule(webhook: pb::Webhook) -> Result<WebhookRule, Error> {
    let conditions = webhook
        .conditions
        .into_iter()
        .enumerate()
        .map(|(index, condition)| {
            let at = format!("webhook.conditions[{index}]");
            let source = match condition.source {
                Some(webhook_condition::Source::Header(name)) => ConditionSource::Header(name),
                Some(webhook_condition::Source::JsonPointer(pointer)) => {
                    ConditionSource::Pointer(pointer)
                }
                None => return Err(Error::invalid(format!("{at}.source"), "is required")),
            };
            let operator = match pb::ConditionOperator::try_from(condition.operator) {
                Ok(pb::ConditionOperator::Is) => ConditionOperator::Is,
                Ok(pb::ConditionOperator::Matches) => ConditionOperator::Matches,
                Ok(pb::ConditionOperator::OneOf) => ConditionOperator::OneOf,
                Ok(pb::ConditionOperator::Unspecified) | Err(_) => {
                    return Err(Error::invalid(format!("{at}.operator"), "is required"))
                }
            };
            Ok(Condition {
                source,
                operator,
                values: condition.values,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WebhookRule { conditions })
}

/// Returns the proto conditions of a webhook.
pub(super) fn to_proto(rule: WebhookRule) -> Vec<pb::WebhookCondition> {
    rule.conditions
        .into_iter()
        .map(|condition| {
            let source = match condition.source {
                ConditionSource::Header(name) => webhook_condition::Source::Header(name),
                ConditionSource::Pointer(pointer) => {
                    webhook_condition::Source::JsonPointer(pointer)
                }
            };
            let operator = match condition.operator {
                ConditionOperator::Is => pb::ConditionOperator::Is,
                ConditionOperator::Matches => pb::ConditionOperator::Matches,
                ConditionOperator::OneOf => pb::ConditionOperator::OneOf,
            };
            pb::WebhookCondition {
                source: Some(source),
                operator: operator.into(),
                values: condition.values,
            }
        })
        .collect()
}
