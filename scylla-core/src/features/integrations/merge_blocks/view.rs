//! A merge rule as the routes give it: the rule of a pipeline or of the project, with the state
//! that the provider shows, the branch, and the page of the provider for a person.

use scylla_domain::integrations::connections::{AccountKind, VcsConnection};
use scylla_domain::integrations::merge_blocks::{MergeBlock, MergeBlockState};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use time::OffsetDateTime;

use super::rules::{self, Observed};
use super::store::Pipeline;
use crate::features::integrations::links::store::Linked;

/// A merge rule, as the routes give it.
#[derive(Clone, Debug)]
pub(super) struct Block {
    /// The project.
    pub(super) project: ProjectId,
    /// The pipeline, or `None` for the rule of the project on GitLab.
    pub(super) pipeline: Option<PipelineId>,
    /// The name of the report of the pipeline, or `None` for the rule of the project.
    pub(super) context: Option<String>,
    /// `true` when the rule must be on.
    pub(super) enabled: bool,
    /// The state on the provider.
    pub(super) state: MergeBlockState,
    /// The reason of the state, as a stable code.
    pub(super) reason: Option<String>,
    /// The branch of the rule: the default branch.
    pub(super) branch: String,
    /// The page of the provider where a person grants the permission or changes the rule.
    pub(super) action_url: String,
    /// The time of the last change, or `None` for a rule that never changed.
    pub(super) updated_at: Option<OffsetDateTime>,
}

/// The pages of the provider for the rules of a link.
pub(super) struct Pages {
    /// The page where a person grants the permission or the role.
    grant: String,
    /// The page of the rule.
    rule: String,
}

/// Returns the pages of the provider for the rules of a link: the installation or the members,
/// and the ruleset or the merge settings.
pub(super) fn pages(
    github_web_url: &str,
    linked: &Linked,
    connection: &VcsConnection,
    ruleset: Option<i64>,
) -> Pages {
    let repository = linked.link.web_url.trim_end_matches('/');
    match linked.provider {
        VcsProvider::GitHub => {
            let installation = &connection.external_id;
            let grant = match connection.account_kind {
                AccountKind::Organization => format!(
                    "{github_web_url}/organizations/{}/settings/installations/{installation}",
                    connection.account
                ),
                AccountKind::User | AccountKind::ServiceAccount => {
                    format!("{github_web_url}/settings/installations/{installation}")
                }
            };
            let rule = match ruleset {
                Some(id) => format!("{repository}/rules/{id}"),
                None => format!("{repository}/settings/rules"),
            };
            Pages { grant, rule }
        }
        VcsProvider::GitLab => Pages {
            grant: format!("{repository}/-/project_members"),
            rule: format!("{repository}/-/settings/merge_requests"),
        },
    }
}

/// Returns a rule as the routes give it: a pipeline with no rule is off, and a rule that is on
/// takes the state that the provider shows.
pub(super) fn view(
    linked: &Linked,
    pipeline: Option<Pipeline>,
    block: Option<MergeBlock>,
    found: &Observed,
    pages: &Pages,
) -> Block {
    // Take the state that the provider shows for a rule that is on.
    let current = pipeline
        .as_ref()
        .map(|pipeline| rules::context(&pipeline.name, pipeline.file_key.as_deref()));
    let (enabled, state, reason, updated_at) = match block {
        Some(block) => {
            let stored = block.context.as_deref();
            let state = rules::observed(
                block.state,
                block.enabled,
                stored,
                current.as_deref(),
                found,
            );
            (
                block.enabled,
                state,
                block.state_reason,
                Some(block.updated_at),
            )
        }
        None => (false, MergeBlockState::Off, None, None),
    };

    // Give the page of the provider for the state.
    let action_url = match state {
        MergeBlockState::NeedsPermission => pages.grant.clone(),
        _ => pages.rule.clone(),
    };
    Block {
        project: linked.link.project_id,
        pipeline: pipeline.map(|pipeline| pipeline.id),
        context: current,
        enabled,
        state,
        reason,
        branch: linked.link.default_branch.clone(),
        action_url,
        updated_at,
    }
}
