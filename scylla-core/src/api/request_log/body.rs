//! The body of an answer that logs the result of its call at its gRPC trailers.

use std::pin::Pin;
use std::task::{Context, Poll};

use http_body::{Body, Frame, SizeHint};

use super::result::Call;

/// A body that logs the result of its call when its trailers come. A body that stops before
/// them logs a cancelled call.
pub struct Watched<B> {
    /// The body of the route.
    inner: B,
    /// The call that waits for its trailers, if its result is not logged yet.
    call: Option<Call>,
}

impl<B> Watched<B> {
    /// Watches a body for the trailers of a call.
    pub(super) fn new(inner: B, call: Option<Call>) -> Self {
        Self { inner, call }
    }
}

impl<B> Body for Watched<B>
where
    B: Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = &mut *self;
        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(trailers) = frame.trailers_ref() {
                    if let Some(call) = this.call.take() {
                        call.trailers(trailers);
                    }
                }
            }
            Poll::Ready(Some(Err(_)) | None) => {
                if let Some(call) = this.call.take() {
                    call.broken();
                }
            }
            Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
