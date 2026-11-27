//! A row of the `workspaces` table, and the workspace that it makes.

use scylla_domain::platform::images::{Icon, ImageId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use time::OffsetDateTime;

/// A row of the `workspaces` table.
pub(super) struct Row {
    /// The id.
    pub(super) id: uuid::Uuid,
    /// The display name.
    pub(super) name: String,
    /// The slug.
    pub(super) slug: String,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The personal token switch.
    pub(super) personal_tokens_enabled: bool,
    /// The name of the time zone.
    pub(super) time_zone: String,
    /// The name of the glyph of the icon, if it has one.
    pub(super) icon_name: Option<String>,
    /// The color of the glyph of the icon, if it has one.
    pub(super) icon_color: Option<String>,
    /// The image of the icon, if it has one.
    pub(super) icon_image_id: Option<uuid::Uuid>,
}

impl Row {
    /// Makes the workspace of the row.
    pub(super) fn into_workspace(self) -> Workspace {
        Workspace {
            id: WorkspaceId::from_db(self.id),
            name: self.name,
            slug: self.slug,
            created_at: self.created_at,
            personal_tokens_enabled: self.personal_tokens_enabled,
            time_zone: self.time_zone,
            icon: Icon::of_columns(
                self.icon_name,
                self.icon_color,
                self.icon_image_id.map(ImageId::from_db),
            ),
        }
    }
}
