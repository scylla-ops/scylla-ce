//! The conversions between the proto messages and the domain types of the triggers.

use scylla_domain::projects::git_rules::{PullRequestAction, PullRequestRule, PushRule, TagRule};
use scylla_domain::projects::triggers::{
    FireResult, Input, InputValue, Inputs, Trigger, TriggerKind,
};
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::{create_trigger_request, update_trigger_request};

use crate::api::parse::{self, ToProto};
use crate::features::projects::pipelines::files_convert;
use crate::Error;

/// Reads and checks the kind of a new trigger.
pub(super) fn kind(kind: Option<create_trigger_request::Kind>) -> Result<TriggerKind, Error> {
    let kind = match kind {
        Some(create_trigger_request::Kind::Cron(cron)) => TriggerKind::Cron(cron.schedule),
        Some(create_trigger_request::Kind::Webhook(_)) => TriggerKind::Webhook,
        Some(create_trigger_request::Kind::Push(push)) => TriggerKind::Push(push_rule(push)),
        Some(create_trigger_request::Kind::PullRequest(pull_request)) => {
            TriggerKind::PullRequest(pull_request_rule(pull_request)?)
        }
        Some(create_trigger_request::Kind::Tag(tag)) => TriggerKind::Tag(tag_rule(tag)),
        None => return Err(Error::invalid("kind", "is required")),
    };
    checked(kind)
}

/// Reads and checks the new rules of a Git trigger, as a kind with the new rule.
pub(super) fn rules(
    rules: Option<update_trigger_request::Rules>,
) -> Result<Option<TriggerKind>, Error> {
    let kind = match rules {
        Some(update_trigger_request::Rules::Push(push)) => TriggerKind::Push(push_rule(push)),
        Some(update_trigger_request::Rules::PullRequest(pull_request)) => {
            TriggerKind::PullRequest(pull_request_rule(pull_request)?)
        }
        Some(update_trigger_request::Rules::Tag(tag)) => TriggerKind::Tag(tag_rule(tag)),
        None => return Ok(None),
    };
    checked(kind).map(Some)
}

/// Checks the globs of a Git kind, with the path of the field of the request.
fn checked(kind: TriggerKind) -> Result<TriggerKind, Error> {
    kind.check_rules().map_err(|(path, error)| {
        let path = path.replacen("push.exclude", "push.exclude_branches", 1);
        Error::invalid(path, error.to_string())
    })?;
    Ok(kind)
}

/// Returns the rule of a push.
fn push_rule(push: pb::Push) -> PushRule {
    PushRule {
        branches: push.branches,
        exclude: push.exclude_branches,
        skip_ci: push.skip_ci,
    }
}

/// Returns the rule of a pull request. An unknown event gives `INVALID_ARGUMENT`.
fn pull_request_rule(pull_request: pb::PullRequest) -> Result<PullRequestRule, Error> {
    let events = pull_request
        .events
        .into_iter()
        .enumerate()
        .map(|(index, raw)| match pb::PullRequestEvent::try_from(raw) {
            Ok(pb::PullRequestEvent::Opened) => Ok(PullRequestAction::Opened),
            Ok(pb::PullRequestEvent::Synchronized) => Ok(PullRequestAction::NewCommits),
            Ok(pb::PullRequestEvent::Reopened) => Ok(PullRequestAction::Reopened),
            Ok(pb::PullRequestEvent::ReadyForReview) => Ok(PullRequestAction::ReadyForReview),
            Ok(pb::PullRequestEvent::Unspecified) | Err(_) => Err(Error::invalid(
                format!("pull_request.events[{index}]"),
                "is not an event of a pull request",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PullRequestRule {
        branches: pull_request.branches,
        events,
        drafts: pull_request.drafts,
    })
}

/// Returns the rule of a tag.
fn tag_rule(tag: pb::Tag) -> TagRule {
    TagRule {
        patterns: tag.patterns,
    }
}

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
            TriggerKind::Push(rule) => pb::trigger::Kind::Push(files_convert::push_of(rule)),
            TriggerKind::PullRequest(rule) => {
                pb::trigger::Kind::PullRequest(files_convert::pull_request_of(rule))
            }
            TriggerKind::Tag(rule) => pb::trigger::Kind::Tag(files_convert::tag_of(rule)),
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
            from_file: self.from_file,
        }
    }
}
