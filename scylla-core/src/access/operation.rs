//! The layer that adds the gRPC method of each request as its `Operation`.

use std::task::{Context, Poll};

use scylla_extension::Operation;
use tonic::codegen::http::Request;
use tower::{Layer, Service};

/// Adds the `Operation` of each request, from its path. `Gate::caller` reads it.
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

impl<S, B> Service<Request<B>> for WithOperation<S>
where
    S: Service<Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<B>) -> Self::Future {
        let operation = Operation::from_path(request.uri().path());
        request.extensions_mut().insert(operation);
        self.inner.call(request)
    }
}
