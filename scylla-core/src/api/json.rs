//! The HTTP/JSON gateway: `POST /api/<package>.<Service>/<Method>` with a JSON body calls the
//! unary gRPC method on the same server, with the proto3 JSON mapping. The answer is the JSON of
//! the response, or an error with the HTTP status of its gRPC code.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode, Version};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use serde_json::{json, Value};
use tonic::{Code, Status};
use tonic_types::StatusExt;
use tower::ServiceExt;

use super::json_tokens::TokenCache;

/// The gateway: the descriptors of the protos, the gRPC routes, and the short tokens of the API
/// tokens.
#[derive(Clone)]
pub(super) struct Gateway {
    /// The descriptors of the protos.
    pool: DescriptorPool,
    /// The gRPC routes of the server, with their layers.
    pub(super) grpc: Router,
    /// The short tokens of the API tokens that the callers sent.
    tokens: Arc<TokenCache>,
}

/// Makes the route of the gateway on the gRPC routes of the server.
pub(super) fn router(grpc: Router) -> Router {
    let pool = DescriptorPool::decode(scylla_proto::FILE_DESCRIPTOR_SET)
        .expect("the descriptors of the protos are valid");
    let gateway = Gateway {
        pool,
        grpc,
        tokens: Arc::default(),
    };
    Router::new()
        .route("/api/{service}/{method}", post(call))
        .with_state(gateway)
}

/// Calls a unary method with the JSON body, and answers its JSON.
async fn call(
    State(gateway): State<Gateway>,
    Path((service, method)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Find a unary method, and read the body as its input.
    let found = gateway
        .pool
        .get_service_by_name(&service)
        .and_then(|found| found.methods().find(|item| item.name() == method));
    let Some(descriptor) = found else {
        return error(&Status::not_found(format!("no method {service}/{method}")));
    };
    if descriptor.is_client_streaming() || descriptor.is_server_streaming() {
        return error(&Status::invalid_argument(
            "the gateway calls unary methods only",
        ));
    }
    let input = match read(descriptor.input(), &body) {
        Ok(input) => input,
        Err(status) => return error(&status),
    };

    // Use the token of the caller, or the short token of its API token.
    let authorization = match gateway.authorization(&headers).await {
        Ok(authorization) => authorization,
        Err(status) => return error(&status),
    };

    // Call the method, then write its output.
    let path = format!("/{service}/{method}");
    match gateway
        .unary(&path, authorization, input.encode_to_vec())
        .await
    {
        Ok(bytes) => match DynamicMessage::decode(descriptor.output(), bytes) {
            Ok(output) => Json(output).into_response(),
            Err(_) => error(&Status::internal("the method answered badly")),
        },
        Err(status) => error(&status),
    }
}

impl Gateway {
    /// Calls a unary gRPC method of the server with a protobuf message, and returns the bytes
    /// of its answer, or its status.
    pub(super) async fn unary(
        &self,
        path: &str,
        authorization: Option<HeaderValue>,
        message: Vec<u8>,
    ) -> Result<Bytes, Status> {
        let length =
            u32::try_from(message.len()).map_err(|_| Status::invalid_argument("too large"))?;
        let mut frame = Vec::with_capacity(message.len() + 5);
        frame.push(0);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&message);
        let mut request = Request::post(path)
            .version(Version::HTTP_2)
            .header(header::CONTENT_TYPE, "application/grpc")
            .header(header::TE, "trailers")
            .body(Body::from(frame))
            .map_err(|_| Status::internal("bad request"))?;
        if let Some(value) = authorization {
            request.headers_mut().insert(header::AUTHORIZATION, value);
        }
        let response = match self.grpc.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let (parts, body) = response.into_parts();
        let collected = body
            .collect()
            .await
            .map_err(|_| Status::internal("the method stopped"))?;
        let trailers = collected.trailers().cloned().unwrap_or_default();
        let status = Status::from_header_map(&trailers)
            .or_else(|| Status::from_header_map(&parts.headers))
            .unwrap_or_else(|| Status::internal("the method gave no status"));
        if status.code() != Code::Ok {
            return Err(status);
        }
        let bytes = collected.to_bytes();
        Ok(bytes.slice(5.min(bytes.len())..))
    }

    /// Returns the authorization to send: the header of the caller, or a short token for its
    /// API token.
    async fn authorization(&self, headers: &HeaderMap) -> Result<Option<HeaderValue>, Status> {
        let Some(value) = headers.get(header::AUTHORIZATION) else {
            return Ok(None);
        };
        let secret = value
            .to_str()
            .ok()
            .and_then(|text| text.strip_prefix("Bearer "));
        match secret.filter(|secret| TokenCache::is_api_token(secret)) {
            Some(secret) => self.tokens.short_token(self, secret).await.map(Some),
            None => Ok(Some(value.clone())),
        }
    }
}

/// Reads a JSON body as a message. An empty body is an empty message.
fn read(descriptor: MessageDescriptor, body: &[u8]) -> Result<DynamicMessage, Status> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(DynamicMessage::new(descriptor));
    }
    let mut json = serde_json::Deserializer::from_slice(body);
    let message = DynamicMessage::deserialize(descriptor, &mut json)
        .and_then(|message| json.end().map(|()| message))
        .map_err(|error| {
            Status::invalid_argument(format!("the JSON body is not valid: {error}"))
        })?;
    Ok(message)
}

/// Makes the answer of an error: its code, its message, and the reason and the fields of its
/// details.
fn error(status: &Status) -> Response {
    let mut body = json!({
        "code": code_name(status.code()),
        "message": status.message(),
    });
    if let Some(info) = status.get_details_error_info() {
        body["reason"] = Value::String(info.reason);
    }
    if let Some(bad) = status.get_details_bad_request() {
        let fields = bad.field_violations.into_iter().map(
            |violation| json!({ "field": violation.field, "description": violation.description }),
        );
        body["fieldViolations"] = Value::Array(fields.collect());
    }
    (http_status(status.code()), Json(body)).into_response()
}

/// Returns the name of a gRPC code, for example `PERMISSION_DENIED`.
fn code_name(code: Code) -> String {
    let mut name = String::new();
    for (index, letter) in format!("{code:?}").chars().enumerate() {
        if letter.is_ascii_uppercase() && index > 0 {
            name.push('_');
        }
        name.push(letter.to_ascii_uppercase());
    }
    name
}

/// Returns the HTTP status of a gRPC code.
fn http_status(code: Code) -> StatusCode {
    match code {
        Code::Ok => StatusCode::OK,
        Code::InvalidArgument | Code::FailedPrecondition | Code::OutOfRange => {
            StatusCode::BAD_REQUEST
        }
        Code::Unauthenticated => StatusCode::UNAUTHORIZED,
        Code::PermissionDenied => StatusCode::FORBIDDEN,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::AlreadyExists | Code::Aborted => StatusCode::CONFLICT,
        Code::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        Code::Unimplemented => StatusCode::NOT_IMPLEMENTED,
        Code::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
