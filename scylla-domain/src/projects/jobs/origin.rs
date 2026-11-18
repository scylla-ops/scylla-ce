//! What started a job: a person, a client of the API, a new run, a trigger or a Git event.

use crate::integrations::named;
use crate::projects::git_rules::TriggerEvent;

named! {
    /// What started a job. The server sets it at the creation, and it does not change.
    pub enum JobOrigin {
        /// A person in the web app, with `RunPipeline` or `FireTriggerNow`.
        Manual = "manual",
        /// A client with no session of a person, with `RunPipeline` or `FireTriggerNow`.
        Api = "api",
        /// A new run of a job with `RerunJob`.
        Rerun = "rerun",
        /// A cron trigger at its time.
        Cron = "cron",
        /// A webhook trigger that got a call.
        Webhook = "webhook",
        /// A push to a branch of the linked repository.
        Push = "push",
        /// A pull request or a merge request of the linked repository.
        PullRequest = "pull_request",
        /// A push of a tag to the linked repository.
        Tag = "tag",
        /// A person asked for a new run on the provider.
        ProviderRerun = "provider_rerun",
    }
}

impl JobOrigin {
    /// Returns the origin of a job that an actor starts: a person with a session, or a client of
    /// the API.
    pub fn of_caller(session: bool) -> Self {
        match session {
            true => Self::Manual,
            false => Self::Api,
        }
    }

    /// Returns the origin of a job of a Git event.
    pub fn of_git(event: &TriggerEvent<'_>) -> Self {
        match event {
            TriggerEvent::Push { .. } => Self::Push,
            TriggerEvent::PullRequest { .. } => Self::PullRequest,
            TriggerEvent::Tag { .. } => Self::Tag,
        }
    }
}
