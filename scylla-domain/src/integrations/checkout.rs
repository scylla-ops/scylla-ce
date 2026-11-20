//! The checkout of a job: the GitLab deploy tokens that Scylla makes for it, and the time that
//! such a token lives.

use time::{Duration, OffsetDateTime};

use super::connections::VcsConnectionId;
use crate::projects::jobs::JobId;
use crate::projects::pipelines::Node;
use crate::Id;

/// The longest checkout of a job on the agent.
pub const CHECKOUT_LIMIT: Duration = Duration::hours(1);

/// The longest life of a token of a checkout.
pub const MAX_TOKEN_LIFE: Duration = Duration::days(30);

/// The id of the row of a GitLab deploy token of a checkout.
pub type CheckoutTokenId = Id<CheckoutToken>;

/// A GitLab deploy token of a checkout, as the store returns it. No field holds the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckoutToken {
    /// The id of the row.
    pub id: CheckoutTokenId,
    /// The job of the token, or `None` after the delete of the job.
    pub job_id: Option<JobId>,
    /// The connection that made the token.
    pub connection_id: VcsConnectionId,
    /// The id of the project on GitLab.
    pub project_external_id: String,
    /// The id of the token on GitLab.
    pub token_external_id: String,
    /// The time when GitLab stops the token.
    pub expires_at: OffsetDateTime,
}

/// Returns the life of a token of the checkout of a job: the checkout, then each attempt of each
/// node one after the other, at most `MAX_TOKEN_LIFE`.
pub fn token_life(nodes: &[Node]) -> Duration {
    let run = nodes.iter().fold(CHECKOUT_LIMIT, |total, node| {
        let attempts = i64::from(node.retries) + 1;
        total.saturating_add(Duration::seconds(
            i64::from(node.timeout_seconds) * attempts,
        ))
    });
    run.min(MAX_TOKEN_LIFE)
}
