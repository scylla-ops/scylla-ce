//! The helpers that make requests: a token in a header, an id message, the text of an id.

use scylla_proto::common::v1 as common;
use tonic::metadata::MetadataValue;
use tonic::Request;

/// Makes a request with a token in `authorization: Bearer`.
pub fn with_token<M>(token: &str, message: M) -> Request<M> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {token}")).expect("ascii token");
    request.metadata_mut().insert("authorization", value);
    request
}

/// Makes an id message from text.
pub fn id(value: impl Into<String>) -> common::Id {
    common::Id {
        value: value.into(),
    }
}

/// Returns the text of an id of a response.
pub fn text(id: &Option<common::Id>) -> &str {
    &id.as_ref().expect("id in response").value
}
