//! The saved layouts of the dashboard of a workspace. The server keeps their widgets and does not
//! read them.

use serde_json::Value;
use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{DomainError, Id, Name, Page, PageRequest, ResourceKind};

/// The kind of a dashboard, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("dashboard");

/// The largest JSON text of the widgets of a dashboard, in bytes.
pub const MAX_WIDGETS_BYTES: usize = 64 << 10;

/// The id of a dashboard.
pub type DashboardId = Id<Dashboard>;

/// The display name of a dashboard.
pub type DashboardName = Name;

/// Who sees a dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// Only its creator, also against an owner of the workspace or an instance admin.
    Private,
    /// Each actor that can read the dashboards of the workspace.
    Workspace,
}

impl Visibility {
    /// Returns the name of the visibility in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Workspace => "workspace",
        }
    }

    /// Reads the name of a visibility in the database. Another text gives `Private`.
    pub fn of_db(text: &str) -> Self {
        match text {
            "workspace" => Self::Workspace,
            _ => Self::Private,
        }
    }
}

/// A dashboard, as the store returns it, with no widgets.
#[derive(Clone, Debug)]
pub struct Dashboard {
    /// The id of the dashboard.
    pub id: DashboardId,
    /// The workspace of the dashboard.
    pub workspace_id: WorkspaceId,
    /// The actor that created the dashboard.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// Who sees the dashboard.
    pub visibility: Visibility,
    /// `true` for the team default of the workspace.
    pub team_default: bool,
    /// The version: 1 at the creation, and one more at each change.
    pub version: i64,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The time of the last change.
    pub updated_at: OffsetDateTime,
}

impl Dashboard {
    /// Returns `true` if the actor created the dashboard: its routes pass on the actor itself.
    pub fn is_created_by(&self, actor: ActorId) -> bool {
        self.created_by == actor
    }

    /// Returns `true` if the dashboard is private and the actor did not create it. No role shows
    /// it, so its routes answer as for an unknown dashboard.
    pub fn is_hidden_from(&self, actor: ActorId) -> bool {
        self.visibility == Visibility::Private && !self.is_created_by(actor)
    }
}

/// The widgets of a dashboard: a JSON value that the web app writes and reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Widgets {
    /// The version of the format of the JSON value, that the web app sets.
    pub schema_version: u16,
    /// The widgets. The server does not read them.
    pub json: Value,
}

impl Widgets {
    /// Checks the version of the format of the widgets: 1 to 65535.
    pub fn schema_version(raw: u32) -> Result<u16, DomainError> {
        u16::try_from(raw)
            .ok()
            .filter(|version| *version >= 1)
            .ok_or(DomainError::OutOfRange {
                min: 1,
                max: u16::MAX.into(),
            })
    }
}

/// The input of a dashboard creation.
#[derive(Clone, Debug)]
pub struct NewDashboard {
    /// The workspace of the dashboard.
    pub workspace_id: WorkspaceId,
    /// The actor that creates the dashboard.
    pub created_by: ActorId,
    /// The display name.
    pub name: DashboardName,
    /// Who sees the dashboard.
    pub visibility: Visibility,
    /// The widgets.
    pub widgets: Widgets,
}

/// A change of a dashboard. A field that is `None` does not change.
#[derive(Clone, Debug)]
pub struct DashboardChange {
    /// The version that the client read.
    pub version: i64,
    /// The new display name.
    pub name: Option<DashboardName>,
    /// The new visibility. A private dashboard is not the team default.
    pub visibility: Option<Visibility>,
    /// The new widgets.
    pub widgets: Option<Widgets>,
}

/// The input of a list of the dashboards of a workspace.
#[derive(Clone, Copy, Debug)]
pub struct ListDashboards {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The actor that reads the list: its private dashboards are in it.
    pub caller: ActorId,
    /// The page to return.
    pub page: PageRequest<DashboardId>,
}

/// One page of dashboards.
pub type DashboardPage = Page<Dashboard, DashboardId>;

crate::authz! {
    /// The routes of the dashboards on their workspace. A private dashboard is not in reach of a
    /// role: only its creator sees it.
    extend Workspace as DashboardHolder {
        /// The route `ListDashboards`: the shared dashboards of this workspace.
        permission list_dashboards = read;
        /// The route `GetDashboard`, on each shared dashboard of this workspace.
        permission get_dashboard = read;
        /// The route `CreateDashboard`, in this workspace.
        permission create_dashboard = work;
        /// The route `UpdateDashboard`, on each shared dashboard of this workspace.
        permission update_dashboard = manage;
        /// The route `DeleteDashboard`, on each shared dashboard of this workspace.
        permission delete_dashboard = manage;
        /// The route `SetTeamDefaultDashboard`, in this workspace.
        permission set_team_default_dashboard = manage;
    }

    /// The routes of a dashboard on its creator: the creator always reads, changes and deletes
    /// it.
    extend Actor as DashboardCaller {
        /// The route `GetDashboard`, on a dashboard that the caller created.
        permission get_dashboard = nil;
        /// The route `UpdateDashboard`, on a dashboard that the caller created.
        permission update_dashboard = nil;
        /// The route `DeleteDashboard`, on a dashboard that the caller created.
        permission delete_dashboard = nil;
    }
}
