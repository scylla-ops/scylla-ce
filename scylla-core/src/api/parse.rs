//! The conversions between proto and domain types. Each bad field gives `INVALID_ARGUMENT`.

use std::fmt;

use prost_types::Timestamp;
use scylla_domain::identity::user_state::StateKey;
use scylla_domain::identity::users::{Email, Handle, Password};
use scylla_domain::projects::secrets::{SecretName, SecretValue};
use scylla_domain::projects::{ProjectDescription, ProjectName};
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::{DomainError, Id};
use scylla_domain::{PageRequest, PageSize};
use scylla_proto::common::v1 as pb;
use time::OffsetDateTime;

use crate::Error;

/// Why a proto value is not valid.
#[derive(Debug)]
pub struct Invalid {
    /// The field inside the value, for example `size` in a page.
    sub_field: Option<&'static str>,
    /// The reason.
    reason: String,
}

impl Invalid {
    /// Makes the error of the whole value.
    pub fn new(reason: impl fmt::Display) -> Self {
        Self {
            sub_field: None,
            reason: reason.to_string(),
        }
    }

    /// Makes the error of one field inside the value.
    pub fn at(sub_field: &'static str, reason: impl fmt::Display) -> Self {
        Self {
            sub_field: Some(sub_field),
            reason: reason.to_string(),
        }
    }

    /// Makes the request error, with the path of the field.
    fn into_error(self, field: &'static str) -> Error {
        match self.sub_field {
            Some(sub_field) => Error::invalid(format!("{field}.{sub_field}"), self.reason),
            None => Error::invalid(field, self.reason),
        }
    }
}

impl From<DomainError> for Invalid {
    fn from(error: DomainError) -> Self {
        Self::new(error)
    }
}

/// A domain value that comes from a proto value.
pub trait FromProto<P>: Sized {
    /// Checks the proto value and makes the domain value.
    fn from_proto(value: P) -> Result<Self, Invalid>;
}

/// A domain value that becomes a proto value. A local trait, as the orphan rule forbids `From`.
pub trait ToProto<P> {
    /// Makes the proto value.
    fn to_proto(self) -> P;
}

/// Reads a message field that the client must set.
pub fn required<P, T: FromProto<P>>(value: Option<P>, field: &'static str) -> Result<T, Error> {
    let value = value.ok_or_else(|| Error::invalid(field, "is required"))?;
    self::field(value, field)
}

/// Reads a message field, or gives `default` if the client did not set it.
pub fn or<P, T: FromProto<P>>(
    value: Option<P>,
    field: &'static str,
    default: T,
) -> Result<T, Error> {
    value.map_or(Ok(default), |value| self::field(value, field))
}

/// Reads a field that has a default value when it is not set.
pub fn optional<P, T: FromProto<P> + Default>(
    value: Option<P>,
    field: &'static str,
) -> Result<T, Error> {
    or(value, field, T::default())
}

/// Reads a field.
pub fn field<P, T: FromProto<P>>(value: P, field: &'static str) -> Result<T, Error> {
    T::from_proto(value).map_err(|invalid| invalid.into_error(field))
}

/// Makes a proto timestamp.
pub fn timestamp(at: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: at.unix_timestamp(),
        nanos: at.nanosecond() as i32,
    }
}

/// Makes a proto duration.
pub fn duration(length: time::Duration) -> prost_types::Duration {
    prost_types::Duration {
        seconds: length.whole_seconds(),
        nanos: length.subsec_nanoseconds(),
    }
}

impl FromProto<Timestamp> for OffsetDateTime {
    fn from_proto(value: Timestamp) -> Result<Self, Invalid> {
        let seconds = Self::from_unix_timestamp(value.seconds);
        let nanos = u32::try_from(value.nanos).ok();
        match (seconds, nanos) {
            (Ok(at), Some(nanos)) => at
                .replace_nanosecond(nanos)
                .map_err(|_| Invalid::new("must be a valid time")),
            _ => Err(Invalid::new("must be a valid time")),
        }
    }
}

impl<T: ?Sized> FromProto<pb::Id> for Id<T> {
    fn from_proto(message: pb::Id) -> Result<Self, Invalid> {
        Ok(Self::parse(&message.value)?)
    }
}

impl<T: ?Sized> ToProto<pb::Id> for Id<T> {
    fn to_proto(self) -> pb::Id {
        pb::Id {
            value: self.to_string(),
        }
    }
}

impl FromProto<String> for ProjectName {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for ProjectDescription {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for SecretName {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for SecretValue {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for Email {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for Password {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for Handle {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for Slug {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl FromProto<String> for StateKey {
    fn from_proto(value: String) -> Result<Self, Invalid> {
        Ok(Self::parse(value)?)
    }
}

impl<T: ?Sized> FromProto<pb::PageRequest> for PageRequest<Id<T>> {
    fn from_proto(value: pb::PageRequest) -> Result<Self, Invalid> {
        let size = PageSize::parse(value.size).map_err(|error| Invalid::at("size", error))?;
        let after = match value.token.as_str() {
            "" => None,
            token => Some(
                Id::parse(token)
                    .map_err(|_| Invalid::at("token", "is not a token of this list"))?,
            ),
        };
        Ok(Self { after, size })
    }
}

impl<T: ?Sized> ToProto<pb::PageResponse> for Option<Id<T>> {
    fn to_proto(self) -> pb::PageResponse {
        pb::PageResponse {
            next_token: self.map(|id| id.to_string()).unwrap_or_default(),
        }
    }
}

impl FromProto<pb::PageRequest> for PageRequest<SecretName> {
    fn from_proto(value: pb::PageRequest) -> Result<Self, Invalid> {
        let size = PageSize::parse(value.size).map_err(|error| Invalid::at("size", error))?;
        let after = match value.token {
            token if token.is_empty() => None,
            token => Some(
                SecretName::parse(token)
                    .map_err(|_| Invalid::at("token", "is not a token of this list"))?,
            ),
        };
        Ok(Self { after, size })
    }
}

impl ToProto<pb::PageResponse> for Option<SecretName> {
    fn to_proto(self) -> pb::PageResponse {
        pb::PageResponse {
            next_token: self
                .map(|name| name.as_str().to_owned())
                .unwrap_or_default(),
        }
    }
}
