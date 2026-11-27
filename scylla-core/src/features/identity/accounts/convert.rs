//! The proto messages of the users, of the sessions and of the acceptances of the terms.

use scylla_domain::identity::terms::Acceptance;
use scylla_domain::identity::users::{ProfileChange, SessionId, User};
use scylla_domain::platform::images::{GlyphAvatar, IconColor, Upload};
use scylla_proto::accounts::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::features::identity::sessions::SessionInfo;
use crate::Error;

impl ToProto<pb::User> for User {
    fn to_proto(self) -> pb::User {
        pb::User {
            id: Some(self.id.to_proto()),
            display_name: self.display_name,
            handle: self.handle.unwrap_or_default(),
            email: self.email.unwrap_or_default(),
            status: self.status.as_str().to_owned(),
            instance_admin: self.instance_admin,
            created_at: Some(parse::timestamp(self.created_at)),
            avatar: self.avatar.map(ToProto::to_proto),
        }
    }
}

/// Makes the proto of a session. `current` is the session of the request, if it has one.
pub(crate) fn session(info: SessionInfo, current: Option<SessionId>) -> pb::Session {
    pb::Session {
        current: current == Some(info.id),
        id: Some(info.id.to_proto()),
        methods: info.methods,
        user_agent: info.user_agent.unwrap_or_default(),
        ip_address: info.ip_address.unwrap_or_default(),
        created_at: Some(parse::timestamp(info.created_at)),
        last_used_at: Some(parse::timestamp(info.last_used_at)),
    }
}

/// Makes the proto of an acceptance of the terms.
pub(crate) fn acceptance(acceptance: Acceptance) -> pb::TermsAcceptance {
    pb::TermsAcceptance {
        version: acceptance.version,
        accepted_at: Some(parse::timestamp(acceptance.accepted_at)),
    }
}

/// Reads the fields of a profile change. A field that is not set does not change, and an empty
/// handle removes the handle.
pub(crate) fn profile_change(
    display_name: Option<String>,
    handle: Option<String>,
) -> Result<ProfileChange, Error> {
    let display_name = display_name
        .map(|name| parse::field(name, "display_name"))
        .transpose()?;
    let handle = handle
        .map(|handle| match handle.trim() {
            "" => Ok(None),
            _ => parse::field(handle, "handle").map(Some),
        })
        .transpose()?;
    Ok(ProfileChange {
        display_name,
        handle,
        ..ProfileChange::default()
    })
}

/// Reads the new image of an avatar. Empty bytes remove the image.
pub(crate) fn avatar_image(bytes: Option<Vec<u8>>) -> Result<Option<Option<Upload>>, Error> {
    bytes
        .map(|bytes| match bytes.is_empty() {
            true => Ok(None),
            false => Upload::parse(bytes).map(Some),
        })
        .transpose()
        .map_err(|error| Error::invalid("avatar_image", error.to_string()))
}

/// Reads a new glyph avatar. It cannot come with a new image.
pub(crate) fn avatar_glyph(
    glyph: Option<scylla_proto::common::v1::GlyphAvatar>,
    with_image: bool,
) -> Result<Option<GlyphAvatar>, Error> {
    if glyph.is_some() && with_image {
        return Err(Error::invalid(
            "avatar_glyph",
            "must not come with avatar_image",
        ));
    }
    glyph
        .map(|glyph| parse::field(glyph, "avatar_glyph"))
        .transpose()
}

/// Reads the new color of the initials of an avatar. An empty text gives the default color.
pub(crate) fn avatar_color(color: Option<String>) -> Result<Option<Option<IconColor>>, Error> {
    color
        .map(IconColor::parse)
        .transpose()
        .map_err(|error| Error::invalid("avatar_color", error.to_string()))
}
