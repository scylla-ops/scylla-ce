//! The re-run of a check that a person asked for on GitHub (`check_run` and `check_suite` with the
//! action `rerequested`). It comes with the check runs: today the request starts no run.

use crate::features::integrations::links::store::Linked;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// A request of GitHub for a new run of the checks of a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code, reason = "the check runs read these fields")]
pub(crate) struct RerunRequest {
    /// The commit of the checks.
    pub(crate) sha: String,
    /// The `external_id` of the check run (the id of its job), or `None` for a whole suite.
    pub(crate) check: Option<String>,
    /// The login of the person that asked, if GitHub gives it.
    pub(crate) login: Option<String>,
}

/// Runs a request again for one active link. It starts no run yet.
pub(super) async fn handle(
    _ctx: &IntegrationContext,
    _linked: &Linked,
    _request: &RerunRequest,
) -> Result<(), Error> {
    Ok(())
}
