//! The conversions of the commit of a job and of a Git ref to and from the proto messages.

use scylla_domain::integrations::sources::{GitRef, JobSource, SourceEvent};
use scylla_domain::integrations::VcsProvider;
use scylla_proto::integrations::v1 as integrations_pb;
use scylla_proto::pipelines::v1 as pb;

use crate::api::parse::{FromProto, Invalid, ToProto};

impl FromProto<pb::GitRef> for GitRef {
    fn from_proto(value: pb::GitRef) -> Result<Self, Invalid> {
        match value.kind {
            Some(pb::git_ref::Kind::Branch(name)) => {
                GitRef::branch(name).map_err(|error| Invalid::at("branch", error))
            }
            Some(pb::git_ref::Kind::Tag(name)) => {
                GitRef::tag(name).map_err(|error| Invalid::at("tag", error))
            }
            Some(pb::git_ref::Kind::Sha(sha)) => {
                GitRef::sha(&sha).map_err(|error| Invalid::at("sha", error))
            }
            None => Err(Invalid::at("kind", "is required")),
        }
    }
}

impl ToProto<pb::JobSource> for JobSource {
    fn to_proto(self) -> pb::JobSource {
        pb::JobSource {
            provider: provider(self.provider).into(),
            commit_url: self.commit_url(),
            from_fork: self.from_fork(),
            repository: self.repository,
            repository_url: self.url,
            sha: self.sha,
            r#ref: self.git_ref.unwrap_or_default(),
            branch: self.branch.unwrap_or_default(),
            tag: self.tag.unwrap_or_default(),
            pull_requests: self
                .pull_requests
                .into_iter()
                .map(|pull| pb::PullRequestRef {
                    number: pull.number,
                    title: pull.title,
                    url: pull.url,
                    source_branch: pull.source_branch,
                    target_branch: pull.target_branch,
                })
                .collect(),
            fork_repository: self.fork_repository.unwrap_or_default(),
            author_login: self.author_login.unwrap_or_default(),
            author_avatar_url: self.avatar.unwrap_or_default(),
            message_title: self.message_title,
            event: event(self.event).into(),
        }
    }
}

/// Returns the proto of a provider.
pub(crate) fn provider(provider: VcsProvider) -> integrations_pb::VcsProvider {
    match provider {
        VcsProvider::GitHub => integrations_pb::VcsProvider::Github,
        VcsProvider::GitLab => integrations_pb::VcsProvider::Gitlab,
    }
}

/// Returns the proto of an event.
pub(crate) fn event(event: SourceEvent) -> pb::SourceEvent {
    match event {
        SourceEvent::Push => pb::SourceEvent::Push,
        SourceEvent::PullRequest => pb::SourceEvent::PullRequest,
        SourceEvent::Tag => pb::SourceEvent::Tag,
        SourceEvent::Manual => pb::SourceEvent::Manual,
        SourceEvent::ProviderRerun => pb::SourceEvent::ProviderRerun,
        SourceEvent::Trigger => pb::SourceEvent::Trigger,
    }
}

/// Reads an event, or `None` for no event.
pub(crate) fn event_of(event: pb::SourceEvent) -> Option<SourceEvent> {
    match event {
        pb::SourceEvent::Unspecified => None,
        pb::SourceEvent::Push => Some(SourceEvent::Push),
        pb::SourceEvent::PullRequest => Some(SourceEvent::PullRequest),
        pb::SourceEvent::Tag => Some(SourceEvent::Tag),
        pb::SourceEvent::Manual => Some(SourceEvent::Manual),
        pb::SourceEvent::ProviderRerun => Some(SourceEvent::ProviderRerun),
        pb::SourceEvent::Trigger => Some(SourceEvent::Trigger),
    }
}
