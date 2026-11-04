//! The conversion of an API token to its proto message.

use scylla_domain::identity::api_tokens::ApiTokenInfo;
use scylla_proto::auth::v1 as pb;

use crate::api::parse::{self, ToProto};

impl ToProto<pb::ApiToken> for ApiTokenInfo {
    fn to_proto(self) -> pb::ApiToken {
        pb::ApiToken {
            id: Some(self.id.to_proto()),
            actor_id: Some(self.actor_id.to_proto()),
            name: self.name,
            hint: self.hint,
            created_by: Some(self.created_by.to_proto()),
            created_at: Some(parse::timestamp(self.created_at)),
            expires_at: Some(parse::timestamp(self.expires_at)),
            last_used_at: self.last_used_at.map(parse::timestamp),
            revoked_at: self.revoked_at.map(parse::timestamp),
        }
    }
}
