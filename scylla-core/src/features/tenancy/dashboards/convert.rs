//! The conversions of the dashboards between the domain and the protos.

use scylla_domain::tenancy::dashboards::{Dashboard, Visibility, Widgets, MAX_WIDGETS_BYTES};
use scylla_proto::dashboards::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

/// Reads a visibility of a request. Not set gives `None`, and an unknown value gives
/// `INVALID_ARGUMENT`.
pub(super) fn visibility(raw: i32) -> Result<Option<Visibility>, Error> {
    match pb::DashboardVisibility::try_from(raw) {
        Ok(pb::DashboardVisibility::Unspecified) => Ok(None),
        Ok(pb::DashboardVisibility::Private) => Ok(Some(Visibility::Private)),
        Ok(pb::DashboardVisibility::Workspace) => Ok(Some(Visibility::Workspace)),
        Err(_) => Err(Error::invalid("visibility", "is not a visibility")),
    }
}

/// Reads the widgets of a request: a version of their format and a JSON text of at most 64 KiB.
pub(super) fn widgets(raw: pb::DashboardWidgets) -> Result<Widgets, Error> {
    let schema_version = Widgets::schema_version(raw.schema_version)
        .map_err(|error| Error::invalid("widgets.schema_version", error.to_string()))?;
    let json = parse::json(&raw.widgets_json, "widgets.widgets_json", MAX_WIDGETS_BYTES)?;
    Ok(Widgets {
        schema_version,
        json,
    })
}

impl ToProto<pb::Dashboard> for Dashboard {
    fn to_proto(self) -> pb::Dashboard {
        let visibility = match self.visibility {
            Visibility::Private => pb::DashboardVisibility::Private,
            Visibility::Workspace => pb::DashboardVisibility::Workspace,
        };
        pb::Dashboard {
            id: Some(self.id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            visibility: visibility.into(),
            team_default: self.team_default,
            version: self.version,
            created_at: Some(parse::timestamp(self.created_at)),
            updated_at: Some(parse::timestamp(self.updated_at)),
        }
    }
}

impl ToProto<pb::DashboardWidgets> for Widgets {
    fn to_proto(self) -> pb::DashboardWidgets {
        pb::DashboardWidgets {
            schema_version: self.schema_version.into(),
            widgets_json: self.json.to_string(),
        }
    }
}
