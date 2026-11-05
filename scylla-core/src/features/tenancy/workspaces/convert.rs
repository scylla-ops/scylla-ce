//! The proto messages of the workspaces.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::workspaces::v1 as pb;

use crate::api::parse::{self, ToProto};

impl ToProto<pb::Workspace> for Workspace {
    fn to_proto(self) -> pb::Workspace {
        pb::Workspace {
            id: Some(self.id.to_proto()),
            name: self.name,
            slug: self.slug,
            created_at: Some(parse::timestamp(self.created_at)),
            personal_tokens_enabled: self.personal_tokens_enabled,
        }
    }
}
