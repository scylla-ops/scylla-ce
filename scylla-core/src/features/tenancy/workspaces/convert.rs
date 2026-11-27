//! The proto messages of the workspaces.

use scylla_domain::tenancy::workspaces::lists::{WorkspaceDetails, WorkspaceOrder, WorkspaceOwner};
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::workspaces::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

impl ToProto<pb::Workspace> for Workspace {
    fn to_proto(self) -> pb::Workspace {
        pb::Workspace {
            id: Some(self.id.to_proto()),
            name: self.name,
            slug: self.slug,
            created_at: Some(parse::timestamp(self.created_at)),
            personal_tokens_enabled: self.personal_tokens_enabled,
            time_zone: self.time_zone,
            icon: self.icon.map(ToProto::to_proto),
        }
    }
}

impl ToProto<pb::WorkspaceOwner> for WorkspaceOwner {
    fn to_proto(self) -> pb::WorkspaceOwner {
        pb::WorkspaceOwner {
            user_id: Some(self.user.to_proto()),
            display_name: self.display_name,
            handle: self.handle.unwrap_or_default(),
            email: self.email.unwrap_or_default(),
            avatar: self.avatar.map(ToProto::to_proto),
        }
    }
}

impl ToProto<pb::WorkspaceDetails> for WorkspaceDetails {
    fn to_proto(self) -> pb::WorkspaceDetails {
        pb::WorkspaceDetails {
            workspace_id: Some(self.workspace.to_proto()),
            owner: self.owner.map(ToProto::to_proto),
            member_count: self.member_count,
            project_count: self.project_count,
            last_activity_at: self.last_activity_at.map(parse::timestamp),
            role: self.role.unwrap_or_default(),
        }
    }
}

/// Reads the order of the console of the workspaces. A value that the server does not know gives
/// `INVALID_ARGUMENT`.
pub(super) fn order(raw: i32) -> Result<WorkspaceOrder, Error> {
    match pb::WorkspaceOrder::try_from(raw) {
        Ok(pb::WorkspaceOrder::Unspecified | pb::WorkspaceOrder::Name) => Ok(WorkspaceOrder::Name),
        Ok(pb::WorkspaceOrder::Newest) => Ok(WorkspaceOrder::Newest),
        Ok(pb::WorkspaceOrder::Oldest) => Ok(WorkspaceOrder::Oldest),
        Ok(pb::WorkspaceOrder::LastActivity) => Ok(WorkspaceOrder::LastActivity),
        Err(_) => Err(Error::invalid("order", "is not a known order")),
    }
}
