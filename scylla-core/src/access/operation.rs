//! The layer that adds the `Operation` of each request, and refuses the success of a route that
//! did not check its own permission.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use scylla_extension::Operation;
use tonic::codegen::http::{Request, Response};
use tonic::Status;
use tower::{Layer, Service};

/// The header that ends a gRPC call with a status and no message.
const GRPC_STATUS: &str = "grpc-status";

/// Adds the `Operation` of each request, from its path. `Gate::caller` reads it. After the
/// route, it refuses the success of a request that verified a token but passed no check of its
/// route permission.
#[derive(Clone, Copy, Debug, Default)]
pub struct OperationLayer;

impl<S> Layer<S> for OperationLayer {
    type Service = WithOperation<S>;

    fn layer(&self, inner: S) -> Self::Service {
        WithOperation { inner }
    }
}

/// The service of `OperationLayer`.
#[derive(Clone, Debug)]
pub struct WithOperation<S> {
    /// The service that gets the request.
    inner: S,
}

/// The future of a response of `WithOperation`.
type Answer<R, E> = Pin<Box<dyn Future<Output = Result<Response<R>, E>> + Send>>;

impl<S, B, R> Service<Request<B>> for WithOperation<S>
where
    S: Service<Request<B>, Response = Response<R>>,
    S::Future: Send + 'static,
    R: Default,
{
    type Response = Response<R>;
    type Error = S::Error;
    type Future = Answer<R, S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        // Add the operation, then run the route.
        let operation = Operation::from_path(request.uri().path());
        request.extensions_mut().insert(operation.clone());
        let answer = self.inner.call(request);

        // Refuse a success of a route that verified a token but passed no check of its own.
        Box::pin(async move {
            let response = answer.await?;
            let failed = response
                .headers()
                .get(GRPC_STATUS)
                .is_some_and(|code| code.as_bytes() != b"0");
            if operation.is_unchecked() && !failed {
                tracing::error!(
                    operation = operation.as_str(),
                    "the route did not check its permission"
                );
                let status = Status::permission_denied("the route did not check its permission");
                return Ok(status.into_http());
            }
            Ok(response)
        })
    }
}
