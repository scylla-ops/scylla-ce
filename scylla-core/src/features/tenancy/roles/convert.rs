//! The proto messages of the catalog and of the roles.

use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::roles::Role;
use scylla_proto::access::v1 as pb;

use crate::api::parse::ToProto;

/// Makes the message of an area of the catalog.
pub(super) fn area(area: &'static catalog::Area) -> pb::CatalogArea {
    pb::CatalogArea {
        id: area.id.to_owned(),
        level: area.level.as_str().to_owned(),
        machine: area.machine,
        permissions: area
            .permissions
            .iter()
            .map(|permission| pb::CatalogPermission {
                id: permission.id.to_owned(),
                action: permission.action.to_owned(),
                depends_on: permission
                    .depends_on
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect(),
                implicit: permission.implicit,
            })
            .collect(),
    }
}

impl ToProto<pb::Role> for Role {
    fn to_proto(self) -> pb::Role {
        pb::Role {
            key: self.key,
            name: self.name,
            description: self.description,
            level: self.level.as_str().to_owned(),
            holder: self.holder.as_str().to_owned(),
            grant_name: self.grant_name,
            builtin: self.builtin,
            full_control: self.full_control,
            permissions: self.permissions,
            holder_count: self.holder_count,
            workspace_id: self.workspace.map(ToProto::to_proto),
        }
    }
}
