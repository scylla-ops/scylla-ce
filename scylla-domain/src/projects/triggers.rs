//! The triggers of a pipeline: a cron schedule, a webhook or an event of the linked repository
//! that starts a job, with inputs that each node of the job gets in its environment.

use time::OffsetDateTime;

use crate::identity::actors::{ActorId, ActorKind};
use crate::projects::git_rules::{PullRequestRule, PushRule, TagRule};
use crate::projects::pipelines::{env_name, PipelineId};
use crate::projects::{Project, ProjectId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{DomainError, Id, Name, Page, PageRequest};

/// The actor kind of the runner of the triggers of a workspace: the author of their jobs.
pub const RUNNER_KIND: ActorKind = ActorKind::new("trigger_runner");

/// The id of a trigger.
pub type TriggerId = Id<Trigger>;

/// The display name of a trigger, unique in its pipeline.
pub type TriggerName = Name;

/// What starts the jobs of a trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// A schedule of 5 cron fields, in UTC.
    Cron(String),
    /// A signed request to the webhook route of the trigger.
    Webhook,
    /// A push to a branch of the linked repository.
    Push(PushRule),
    /// A pull request of the linked repository.
    PullRequest(PullRequestRule),
    /// A new tag of the linked repository.
    Tag(TagRule),
}

impl TriggerKind {
    /// Returns the name of the kind, as the database keeps it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Cron(_) => "cron",
            Self::Webhook => "webhook",
            Self::Push(_) => "push",
            Self::PullRequest(_) => "pull_request",
            Self::Tag(_) => "tag",
        }
    }

    /// Returns the schedule of a cron trigger, or `None` for another kind.
    pub fn schedule(&self) -> Option<&str> {
        match self {
            Self::Cron(schedule) => Some(schedule),
            _ => None,
        }
    }
}

/// The value of an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputValue {
    /// A text.
    Literal(String),
    /// A JSON pointer (RFC 6901) into the body of a webhook, for example `/after`.
    Pointer(String),
}

/// An input of a trigger: a variable that each node of the job gets. A variable of the node with
/// the same name wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// The name of the variable.
    pub name: String,
    /// The value.
    pub value: InputValue,
}

/// The checked inputs of a trigger.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inputs(Vec<Input>);

impl Inputs {
    /// The most inputs.
    pub const MAX: usize = 128;

    /// Checks the inputs: at most 128, unique names of variables, a pointer only for a webhook,
    /// and a pointer that is empty or starts with `/`. Returns the path and the rule of a refusal.
    pub fn parse(inputs: Vec<Input>, webhook: bool) -> Result<Self, (String, DomainError)> {
        if inputs.len() > Self::MAX {
            return Err(("inputs".to_owned(), DomainError::TooMany { max: Self::MAX }));
        }
        let mut names = std::collections::HashSet::new();
        for (index, input) in inputs.iter().enumerate() {
            let at = format!("inputs[{index}]");
            env_name(&input.name).map_err(|error| (format!("{at}.name"), error))?;
            if !names.insert(input.name.as_str()) {
                return Err((format!("{at}.name"), DomainError::Duplicate));
            }
            match &input.value {
                InputValue::Literal(text) if text.len() > 64 << 10 => {
                    let error = DomainError::TooLarge { max: 64 << 10 };
                    return Err((format!("{at}.literal"), error));
                }
                InputValue::Pointer(_) if !webhook => {
                    return Err((format!("{at}.json_pointer"), DomainError::NotAllowed));
                }
                InputValue::Pointer(pointer)
                    if !pointer.is_empty() && !pointer.starts_with('/') =>
                {
                    return Err((format!("{at}.json_pointer"), DomainError::NotAPointer));
                }
                _ => {}
            }
        }
        Ok(Self(inputs))
    }

    /// Returns the inputs.
    pub fn as_slice(&self) -> &[Input] {
        &self.0
    }
}

/// The result of the last fire of a trigger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FireResult {
    /// The fire made a job.
    Ok,
    /// The fire failed, with the reason.
    Error(String),
}

/// A trigger, as the store returns it. The secret of a webhook is not in it.
#[derive(Clone, Debug)]
pub struct Trigger {
    /// The id.
    pub id: TriggerId,
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The actor that created the trigger.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// What starts the jobs.
    pub kind: TriggerKind,
    /// The inputs.
    pub inputs: Vec<Input>,
    /// `false` stops the trigger: its schedule does not fire, and its webhook gives 404.
    pub enabled: bool,
    /// The next fire of a schedule.
    pub next_fire_at: Option<OffsetDateTime>,
    /// The last fire.
    pub last_fired_at: Option<OffsetDateTime>,
    /// The result of the last fire.
    pub last_result: Option<FireResult>,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The time of the last change.
    pub updated_at: OffsetDateTime,
    /// `true` for a trigger from the `on` table of a pipeline file. It is read only.
    pub from_file: bool,
}

/// The input of a trigger creation.
#[derive(Clone, Debug)]
pub struct NewTrigger {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The actor that creates the trigger.
    pub created_by: ActorId,
    /// The display name.
    pub name: TriggerName,
    /// What starts the jobs.
    pub kind: TriggerKind,
    /// The checked inputs.
    pub inputs: Inputs,
}

/// A change of a trigger. A field that is `None` does not change. The kind does not change.
#[derive(Clone, Debug, Default)]
pub struct TriggerChange {
    /// The new display name.
    pub name: Option<TriggerName>,
    /// The new schedule of a cron trigger.
    pub schedule: Option<String>,
    /// The new inputs.
    pub inputs: Option<Inputs>,
}

/// The input of a list of the triggers of a pipeline.
#[derive(Clone, Copy, Debug)]
pub struct ListTriggers {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The page to return, oldest first.
    pub page: PageRequest<TriggerId>,
}

/// One page of triggers.
pub type TriggerPage = Page<Trigger, TriggerId>;

crate::authz! {
    /// The routes of the triggers on a project.
    extend Project as TriggerAccess {
        /// The route `CreateTrigger`, on each pipeline of this project.
        permission create_trigger = edit;
        /// The route `GetTrigger`, on each trigger of this project.
        permission get_trigger = view;
        /// The route `UpdateTrigger`, on each trigger of this project.
        permission update_trigger = edit;
        /// The route `DeleteTrigger`, on each trigger of this project.
        permission delete_trigger = edit;
        /// The route `SetTriggerEnabled`, on each trigger of this project.
        permission set_trigger_enabled = edit;
        /// The route `FireTriggerNow`, on each trigger of this project.
        permission fire_trigger_now = edit;
        /// The route `ListPipelineTriggers`, on each pipeline of this project.
        permission list_pipeline_triggers = view;
    }

    /// The routes of the triggers on a workspace.
    extend Workspace as TriggerHolder {
        /// The route `CreateTrigger`, in each project of this workspace.
        permission create_trigger = work;
        /// The route `GetTrigger`, in each project of this workspace.
        permission get_trigger = read;
        /// The route `UpdateTrigger`, in each project of this workspace.
        permission update_trigger = work;
        /// The route `DeleteTrigger`, in each project of this workspace.
        permission delete_trigger = work;
        /// The route `SetTriggerEnabled`, in each project of this workspace.
        permission set_trigger_enabled = work;
        /// The route `FireTriggerNow`, in each project of this workspace.
        permission fire_trigger_now = work;
        /// The route `ListPipelineTriggers`, in each project of this workspace.
        permission list_pipeline_triggers = read;
    }
}
