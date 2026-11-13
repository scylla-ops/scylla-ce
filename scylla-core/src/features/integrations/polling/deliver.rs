//! A new head that a poll found goes to the path of a webhook: a Git event of the repository
//! with the key `poll:<repo>:<ref>:<sha>`, kept one time, then run for each link of the
//! repository.

use scylla_domain::integrations::connections::VcsConnection;
use scylla_domain::integrations::events::{EventRepository, GitChange, GitEvent};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::git_rules::PullRequestAction;

use super::listed::{Head, HeadKind, Seen};
use crate::features::integrations::events::{self, Delivery, Scope};
use crate::features::integrations::links::store::Linked;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Gives a new head to the path of a webhook, as a Git event of the repository.
pub(super) async fn deliver(
    ctx: &IntegrationContext,
    linked: &Linked,
    connection: &VcsConnection,
    seen: &Seen,
    head: Head,
) -> Result<(), Error> {
    // Make the event: a push, or a pull request opened or with new commits.
    let (provider, link) = (linked.provider, &linked.link);
    let git_ref = head.git_ref(provider);
    let action = match seen.heads.contains_key(&git_ref) {
        true => PullRequestAction::NewCommits,
        false => PullRequestAction::Opened,
    };
    let (change, event, name) = match head.kind {
        HeadKind::Branch(branch) => (GitChange::Push { branch }, "push", None),
        HeadKind::Pull { pull, draft, fork } => {
            let change = GitChange::PullRequest {
                pull,
                action,
                draft,
                fork_repository: fork,
            };
            let name = match action {
                PullRequestAction::Opened => "opened",
                _ => "new_commits",
            };
            (change, "pull_request", Some(name))
        }
    };
    let key = GitEvent::poll_key(&link.full_name, &git_ref, &head.commit.sha);
    let git_event = GitEvent {
        provider,
        key: key.clone(),
        repository: EventRepository {
            external_id: link.external_id.clone(),
            full_name: link.full_name.clone(),
            web_url: link.web_url.clone(),
        },
        change,
        commit: head.commit,
        polled: true,
    };

    // Keep its key one time, and run it for each link of the repository.
    let delivery = Delivery {
        provider,
        key: &key,
        app: connection.app_id,
        connection: Some(connection.id),
        project: Some(link.project_id),
        event,
        action: name,
    };
    let scope = match provider {
        VcsProvider::GitHub => Scope::GitHub {
            app: connection.app_id,
            installation: connection.external_id.clone(),
            repository: Some(link.external_id.clone()),
        },
        VcsProvider::GitLab => Scope::GitLab {
            repository: link.external_id.clone(),
        },
    };
    events::polled(ctx, &delivery, &scope, git_event).await?;
    Ok(())
}
