//! The proto messages of the users and of the sessions.

use scylla_domain::identity::users::{ProfileChange, SessionId, User};
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
    })
}
