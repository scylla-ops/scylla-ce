//! The logic of the merge blocks: the list of the rules of a project with the state on the
//! provider, and the change of one rule. The calls to the provider run between two
//! transactions, and the rule has the state `applying` during the call.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::connections::{ConnectionState, VcsConnection};
use scylla_domain::integrations::merge_blocks::{MergeBlock, MergeBlockState};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Rejection, Tx};

use super::apply::{self, Wanted};
use super::rules::{self, Observed, Refusal};
use super::store::{self, Change, Pipeline};
use super::view::{self, Block};
use crate::features::integrations::connections::store as connections;
use crate::features::integrations::links::store::{self as links, Linked};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// What a change finds before the call to the provider.
struct Found {
    /// The link of the project.
    linked: Linked,
    /// The connection of the link.
    connection: VcsConnection,
    /// The pipeline of the rule, or `None` for the rule of the project on GitLab.
    pipeline: Option<Pipeline>,
    /// The state of the rule before the change, or `None` for a new rule.
    previous: Option<MergeBlockState>,
}

/// The result of the call to the provider.
enum Outcome {
    /// No call: the provider has no rule to remove.
    Unchanged,
    /// The provider has the change, with the id of the GitHub ruleset.
    Applied(Option<i64>),
    /// Nothing changed on the provider.
    Refused(Refusal),
}

/// Returns the workspace of a project, or `None` for an unknown project.
pub(super) async fn workspace(
    ctx: &IntegrationContext,
    project: ProjectId,
) -> Result<Option<WorkspaceId>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let workspace = connections::project_workspace(&mut tx, project).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Returns the rules of a project: one for each pipeline on GitHub, one for the project on
/// GitLab. A rule that is on takes the state that the provider shows. A project with no link
/// has no rule.
pub(super) async fn list(
    ctx: &IntegrationContext,
    project: ProjectId,
) -> Result<Vec<Block>, Error> {
    // Read the link and the rules.
    let mut tx = ctx.pool.begin().await?;
    let Some(linked) = links::find(&mut tx, project).await? else {
        return Ok(Vec::new());
    };
    let connection = connection(&mut tx, &linked).await?;
    let ruleset = store::ruleset(&mut tx, project).await?;
    let rules: Vec<(Option<Pipeline>, Option<MergeBlock>)> = match linked.provider {
        VcsProvider::GitHub => store::pipeline_rules(&mut tx, project)
            .await?
            .into_iter()
            .map(|rule| (Some(rule.pipeline), rule.block))
            .collect(),
        VcsProvider::GitLab => vec![(None, store::find(&mut tx, project, None).await?)],
    };
    tx.commit().await?;

    // Read the rule on the provider when a rule is on.
    let on = rules.iter().any(|(_, block)| {
        block
            .as_ref()
            .is_some_and(|block| block.enabled && block.state == MergeBlockState::On)
    });
    let found = match on && linked.link.state == ConnectionState::Active {
        true => match apply::credentials(ctx, &linked, &connection).await {
            Ok(credentials) => apply::observe(ctx, &credentials, &linked, ruleset).await,
            Err(error) => {
                tracing::warn!(%project, %error, "the merge rules are not read on the provider");
                Observed::Unknown
            }
        },
        false => Observed::Unknown,
    };

    // Give each rule.
    let pages = view::pages(&ctx.settings.github_web_url, &linked, &connection, ruleset);
    Ok(rules
        .into_iter()
        .map(|(pipeline, block)| view::view(&linked, pipeline, block, &found, &pages))
        .collect())
}

/// Turns a rule on or off, then changes the rule on the provider. A project with no link gives
/// `NO_REPOSITORY_LINK`, and a link that is not active gives `REPOSITORY_NOT_ACTIVE`. A refusal
/// of the provider changes nothing there, and the rule keeps the state and the reason.
pub(super) async fn set(
    ctx: &IntegrationContext,
    project: ProjectId,
    pipeline: Option<PipelineId>,
    enabled: bool,
    actor: ActorId,
) -> Result<Block, Error> {
    // Read the link, the pipeline and the rule, and open the credentials of a call.
    let found = find(ctx, project, pipeline).await?;
    let credentials = match rules::needs_call(found.previous, enabled) {
        true => Some(apply::credentials(ctx, &found.linked, &found.connection).await?),
        false => None,
    };

    // Record the change as applying, and read the rules that the provider must have.
    let context = found
        .pipeline
        .as_ref()
        .map(|pipeline| rules::context(&pipeline.name, pipeline.file_key.as_deref()));
    let change = Change {
        project,
        pipeline,
        context: context.as_deref(),
        enabled,
        actor,
    };
    let (ruleset, applied) = record(ctx, &found.linked, &change).await?;

    // Change the rule on the provider.
    let outcome = match &credentials {
        None => Outcome::Unchanged,
        Some(credentials) => {
            let contexts = rules::unique(applied.iter().map(|(_, context)| context.clone()));
            let wanted = Wanted {
                contexts: &contexts,
                enabled,
                ruleset,
            };
            match apply::apply(ctx, credentials, &found.linked, &wanted).await {
                Ok(id) => Outcome::Applied(id),
                Err(refusal) => Outcome::Refused(refusal),
            }
        }
    };

    // Write the result, and read the rule back.
    let mut tx = ctx.pool.begin().await?;
    match (&outcome, found.linked.provider) {
        (Outcome::Applied(id), VcsProvider::GitHub) => {
            store::applied(&mut tx, project, &applied, *id).await?;
        }
        (Outcome::Applied(_) | Outcome::Unchanged, _) => {
            let state = match enabled {
                true => MergeBlockState::On,
                false => MergeBlockState::Off,
            };
            store::set_state(&mut tx, project, pipeline, state, None).await?;
        }
        (Outcome::Refused(refusal), _) => {
            let reason = Some(refusal.reason);
            store::set_state(&mut tx, project, pipeline, refusal.state, reason).await?;
        }
    }
    let block = store::find(&mut tx, project, pipeline).await?;
    let ruleset = store::ruleset(&mut tx, project).await?;
    tx.commit().await?;
    let block = block.ok_or_else(no_link)?;
    tracing::info!(
        event = "merge_block.changed",
        %project,
        pipeline = pipeline.map(|id| id.to_string()).unwrap_or_default(),
        provider = found.linked.provider.as_str(),
        enabled,
        state = block.state.as_str(),
        reason = block.state_reason.as_deref().unwrap_or_default(),
    );

    // Give the rule.
    let web = &ctx.settings.github_web_url;
    let pages = view::pages(web, &found.linked, &found.connection, ruleset);
    let linked = &found.linked;
    let unknown = Observed::Unknown;
    Ok(view::view(
        linked,
        found.pipeline,
        Some(block),
        &unknown,
        &pages,
    ))
}

/// Reads what a change needs: the active link, its connection, the pipeline of the request for
/// the provider, and the state of the rule.
async fn find(
    ctx: &IntegrationContext,
    project: ProjectId,
    pipeline: Option<PipelineId>,
) -> Result<Found, Error> {
    // Read the link and its connection.
    let mut tx = ctx.pool.begin().await?;
    let linked = links::find(&mut tx, project).await?.ok_or_else(no_link)?;
    let connection = connection(&mut tx, &linked).await?;

    // Check the pipeline: one of the project on GitHub, none on GitLab.
    let target = match (linked.provider, pipeline) {
        (VcsProvider::GitHub, Some(id)) => {
            let found = store::pipeline(&mut tx, project, id).await?;
            Some(found.ok_or(Error::NotFound("pipeline"))?)
        }
        (VcsProvider::GitHub, None) => {
            return Err(Error::invalid("pipeline_id", "is required on GitHub"));
        }
        (VcsProvider::GitLab, Some(_)) => {
            let reason = "must not be set on GitLab: the rule is for the project";
            return Err(Error::invalid("pipeline_id", reason));
        }
        (VcsProvider::GitLab, None) => None,
    };
    let previous = store::find(&mut tx, project, pipeline).await?;
    tx.commit().await?;

    // Refuse a link that cannot serve.
    if linked.link.state != ConnectionState::Active {
        let message = "the repository of the project cannot be used now";
        let rejection = Rejection::new("REPOSITORY_NOT_ACTIVE", message)
            .detail("state", linked.link.state.as_str())
            .detail(
                "reason",
                linked.link.state_reason.as_deref().unwrap_or_default(),
            );
        return Err(Error::Rejected(rejection));
    }
    Ok(Found {
        linked,
        connection,
        pipeline: target,
        previous: previous.map(|block| block.state),
    })
}

/// Writes a change with the state `applying`, under the lock of the link. Returns the id of the
/// GitHub ruleset, and the pipelines whose rule must be on with the names of their reports.
async fn record(
    ctx: &IntegrationContext,
    linked: &Linked,
    change: &Change<'_>,
) -> Result<(Option<i64>, Vec<(PipelineId, String)>), Error> {
    let mut tx = ctx.pool.begin().await?;
    let ruleset = store::lock_link(&mut tx, change.project)
        .await?
        .ok_or_else(no_link)?;
    store::upsert(&mut tx, change).await?;
    let applied = match linked.provider {
        VcsProvider::GitHub => store::wanted(&mut tx, change.project)
            .await?
            .into_iter()
            .map(|pipeline| {
                let context = rules::context(&pipeline.name, pipeline.file_key.as_deref());
                (pipeline.id, context)
            })
            .collect(),
        VcsProvider::GitLab => Vec::new(),
    };
    tx.commit().await?;
    Ok((ruleset, applied))
}

/// Returns the connection of a link.
async fn connection(tx: &mut Tx<'_>, linked: &Linked) -> Result<VcsConnection, Error> {
    let found = connections::find(tx, linked.link.connection_id).await?;
    Ok(found.ok_or(Error::NotFound("connection"))?.connection)
}

/// The refusal of a project with no link.
fn no_link() -> Error {
    let message = "the project has no linked repository";
    Error::Rejected(Rejection::new("NO_REPOSITORY_LINK", message))
}
