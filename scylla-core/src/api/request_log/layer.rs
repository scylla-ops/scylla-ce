//! The layer that logs each call: a span with a new request id, the route and the client IP
//! address, the actor that the gate records, and one line with the result at the end of the
//! answer.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use scylla_domain::Id;
use tonic::codegen::http::{HeaderMap, HeaderValue, Request, Response};
use tower::{Layer, Service};
use tracing::field::Empty;
use tracing::Instrument;

use super::body::Watched;
use super::result::Call;
use crate::features::identity::sessions::Client;
use crate::infra::logs::CALL;

/// The header of the request id in each answer.
pub const REQUEST_ID: &str = "x-request-id";

/// A request to the server, as the type of its id.
pub enum ServerRequest {}

/// Logs each call, and gives its request id in the answer. Put it inside the gRPC-Web layer, so
/// it sees the gRPC status of each answer.
#[derive(Clone, Copy, Debug)]
pub struct RequestLog {
    /// Reads the client IP address from the last `X-Forwarded-For` address.
    trust_forwarded: bool,
}

impl RequestLog {
    /// Makes the layer. The client IP address follows the rule of the sessions.
    pub fn new(trust_forwarded: bool) -> Self {
        Self { trust_forwarded }
    }
}

impl<S> Layer<S> for RequestLog {
    type Service = Logged<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Logged {
            inner,
            trust_forwarded: self.trust_forwarded,
        }
    }
}

/// The service of `RequestLog`.
#[derive(Clone, Debug)]
pub struct Logged<S> {
    /// The service that gets the request.
    inner: S,
    /// Reads the client IP address from the last `X-Forwarded-For` address.
    trust_forwarded: bool,
}

/// The future of an answer of `Logged`.
type Answer<R, E> = Pin<Box<dyn Future<Output = Result<Response<Watched<R>>, E>> + Send>>;

impl<S, B, R> Service<Request<B>> for Logged<S>
where
    S: Service<Request<B>, Response = Response<R>>,
    S::Future: Send + 'static,
    R: Send + 'static,
{
    type Response = Response<Watched<R>>;
    type Error = S::Error;
    type Future = Answer<R, S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        // Open the span of the call with a new id, the route and the client IP address.
        let id = Id::<ServerRequest>::generate();
        let grpc = is_grpc(request.headers());
        let path = request.uri().path();
        let route = match grpc {
            true => path.trim_start_matches('/').to_owned(),
            false => format!("{} {path}", request.method()),
        };
        let client = Client::of_http(
            request.headers(),
            request.extensions(),
            self.trust_forwarded,
        );
        let span = tracing::error_span!(
            CALL,
            request_id = %id,
            rpc = %route,
            ip = client.ip.map(display),
            actor = Empty,
            actor_kind = Empty,
        );

        // Run the route in the span, then give the request id and log the result.
        let answer = span.in_scope(|| self.inner.call(request));
        let call = Call::new(span.clone(), grpc);
        Box::pin(async move {
            let mut response = answer.instrument(span).await?;
            if let Ok(value) = HeaderValue::from_str(&id.to_string()) {
                response.headers_mut().insert(REQUEST_ID, value);
            }
            Ok(call.answer(response))
        })
    }
}

/// Returns `true` for a gRPC request.
fn is_grpc(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/grpc"))
}
