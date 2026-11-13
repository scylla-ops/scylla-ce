//! The re-run of a check that a person asked for on GitHub (`check_run` and `check_suite` with the
//! action `rerequested`): a new run of the job of each check run, as the runner of the triggers
//! of the workspace.

use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::jobs::{JobId, RerunOrigin};
use scylla_domain::projects::triggers::RUNNER_KIND;

use crate::features::integrations::links::store::Linked;
use crate::features::integrations::{reports, IntegrationContext};
use crate::features::projects::jobs;
use crate::features::projects::triggers::fire;
use crate::Error;

/// A request of GitHub for a new run of the checks of a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RerunRequest {
    /// The commit of the checks.
    pub(crate) sha: String,
    /// The `external_id` of the check run (the id of its job), or `None` for a whole suite.
    pub(crate) check: Option<String>,
    /// The login of the person that asked, if GitHub gives it.
    pub(crate) login: Option<String>,
}

/// Runs again, for one active link, the job of the check run of a request, or the newest job of
/// each check run of the commit for a suite. A job that did not end, of another project or of
/// another commit does not run again.
pub(super) async fn handle(
    ctx: &IntegrationContext,
    linked: &Linked,
    request: &RerunRequest,
) -> Result<(), Error> {
    // Find the jobs of the request in the project of the link.
    let check = match request.check.as_deref().map(JobId::parse) {
        Some(Ok(id)) => Some(id),
        Some(Err(_)) => return Ok(()),
        None => None,
    };
    let project = linked.link.project_id;
    let found = reports::reruns(&ctx.pool, project, &request.sha, check).await?;
    if found.is_empty() {
        return Ok(());
    }

    // Run each job again as the runner of the triggers of the workspace.
    let actor = Actor {
        id: fire::runner(&ctx.pool, linked.link.workspace_id).await?,
        kind: RUNNER_KIND,
    };
    let origin = RerunOrigin::Provider;
    for old in found {
        let by = actor.clone();
        let job = jobs::rerun(&ctx.pool, &ctx.hooks, &ctx.registry, by, old, origin).await?;
        let login = request.login.as_deref().unwrap_or_default();
        tracing::info!(job = %job.id, rerun_of = %old, login, "GitHub asked for a new run");
    }
    Ok(())
}
