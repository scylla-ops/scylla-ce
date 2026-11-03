//! The result of a call: the gRPC code of the answer, from its headers or its trailers, with
//! the details of a refusal, or the HTTP status of a browser route. A call that stops before its
//! end is `CANCELLED`.

use std::time::Instant;

use tonic::codegen::http::{HeaderMap, Response, StatusCode};
use tonic::{Code, Status};
use tonic_types::StatusExt;
use tracing::{Level, Span};

use super::body::Watched;
use crate::infra::logs::{self, Outcome, CALL};

/// A running call, which logs its result one time.
pub(super) struct Call {
    /// The span of the call.
    span: Span,
    /// When the call started.
    start: Instant,
    /// `true` for a gRPC call, `false` for a browser route.
    grpc: bool,
    /// `true` once the result is logged.
    done: bool,
}

impl Call {
    /// Starts a call.
    pub(super) fn new(span: Span, grpc: bool) -> Self {
        Self {
            span,
            start: Instant::now(),
            grpc,
            done: false,
        }
    }

    /// Logs the result of an answer that has it in its headers: a browser route, or a gRPC
    /// answer with no message. Else the body of the answer logs it at its trailers.
    pub(super) fn answer<R>(mut self, response: Response<R>) -> Response<Watched<R>> {
        let watch = match (self.grpc, Status::from_header_map(response.headers())) {
            (false, _) => {
                self.http(response.status());
                None
            }
            (true, Some(status)) => {
                self.status(&status);
                None
            }
            (true, None) => Some(self),
        };
        response.map(|body| Watched::new(body, watch))
    }

    /// Logs the result of the trailers of a gRPC answer.
    pub(super) fn trailers(mut self, trailers: &HeaderMap) {
        match Status::from_header_map(trailers) {
            Some(status) => self.status(&status),
            None => self.broken(),
        }
    }

    /// Logs an answer that ended with no gRPC status.
    pub(super) fn broken(mut self) {
        self.finish(Some(name(Code::Unknown)), None, Level::ERROR);
    }

    /// Logs a gRPC status, with the details of a refusal.
    fn status(&mut self, status: &Status) {
        logs::annotate(&self.span, |outcome| details(status, outcome));
        let level = match status.code() {
            Code::Internal | Code::Unknown | Code::DataLoss | Code::Unavailable => Level::ERROR,
            Code::PermissionDenied | Code::DeadlineExceeded | Code::Cancelled => Level::WARN,
            _ => Level::INFO,
        };
        self.finish(Some(name(status.code())), None, level);
    }

    /// Logs the HTTP status of a browser route.
    fn http(&mut self, status: StatusCode) {
        let level = match status.as_u16() {
            500.. => Level::ERROR,
            403 => Level::WARN,
            _ => Level::INFO,
        };
        self.finish(None, Some(status.as_u16()), level);
    }

    /// Logs the line of the call at a level, with its code or its status and its duration.
    fn finish(&mut self, code: Option<&'static str>, status: Option<u16>, level: Level) {
        self.done = true;
        let ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let span = &self.span;
        match level {
            Level::ERROR => tracing::error!(parent: span, event = CALL, code, status, ms),
            Level::WARN => tracing::warn!(parent: span, event = CALL, code, status, ms),
            _ => tracing::info!(parent: span, event = CALL, code, status, ms),
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        if !self.done {
            self.finish(Some(name(Code::Cancelled)), None, Level::WARN);
        }
    }
}

/// Puts the details of a gRPC status on the outcome: the `ErrorInfo` reason and metadata, the
/// field violations and the message.
fn details(status: &Status, outcome: &mut Outcome) {
    let details = status.get_error_details();
    if let Some(info) = details.error_info() {
        let mut metadata: Vec<_> = info.metadata.clone().into_iter().collect();
        metadata.sort();
        outcome.reason = Some(info.reason.clone());
        outcome.metadata = metadata;
    }
    if let Some(bad) = details.bad_request() {
        outcome.violations = bad
            .field_violations
            .iter()
            .map(|violation| (violation.field.clone(), violation.description.clone()))
            .collect();
    }
    if status.code() != Code::Ok && !status.message().is_empty() {
        outcome.message = Some(status.message().to_owned());
    }
}

/// Returns the name of a gRPC code, as the gRPC specification writes it.
fn name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}
