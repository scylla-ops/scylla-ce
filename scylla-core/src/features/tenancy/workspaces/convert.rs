//! The proto messages of the workspaces and of their owners.

use scylla_domain::tenancy::owners::WorkspaceOwner;
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
        }
    }
}

impl ToProto<pb::WorkspaceOwner> for WorkspaceOwner {
    fn to_proto(self) -> pb::WorkspaceOwner {
        pb::WorkspaceOwner {
            user_id: Some(self.user_id.to_proto()),
            display_name: self.display_name,
            since: Some(parse::timestamp(self.since)),
        }
    }
}
