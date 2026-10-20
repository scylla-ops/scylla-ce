//! `grpc::adapter::run_public` for an HTTP route. No interceptor guards a route, so the caller
//! is `Anonymous` and only a `Public` action passes the authorize stage. The route builds the
//! command from the path, the headers and the body, and maps the `DomainError` to its status.

use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use scylla_extension::{Actions, Kind, Path};

pub async fn run_public<A, K, R>(
    actions: &Actions,
    runner: &R,
    action: A,
) -> DomainResult<A::Output>
where
    A: Path<K, R>,
    K: Kind,
{
    actions.run(runner, &CallerContext::Anonymous, action).await
}
