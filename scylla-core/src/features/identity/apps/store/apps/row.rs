//! The row of the `apps` table, and the App that it makes.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::agents::AgentLabels;
use scylla_domain::identity::apps::{App, AppId, Host};
use scylla_domain::platform::images::{GlyphAvatar, Icon, ImageId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use time::OffsetDateTime;

/// A row of the `apps` table.
pub(super) struct Row {
    /// The id.
    pub(super) id: uuid::Uuid,
    /// The workspace.
    pub(super) workspace_id: uuid::Uuid,
    /// The display name.
    pub(super) name: String,
    /// `true` for an agent.
    pub(super) runner: bool,
    /// The author.
    pub(super) created_by: uuid::Uuid,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The last seen time.
    pub(super) last_seen_at: Option<OffsetDateTime>,
    /// The version of the agent.
    pub(super) version: Option<String>,
    /// The operating system.
    pub(super) os: Option<String>,
    /// The CPU architecture.
    pub(super) arch: Option<String>,
    /// The host name.
    pub(super) hostname: Option<String>,
    /// The number of CPUs.
    pub(super) cpu_count: Option<i32>,
    /// The memory in bytes.
    pub(super) memory_bytes: Option<i64>,
    /// The capacity.
    pub(super) capacity: i32,
    /// The container runtime.
    pub(super) container_runtime: Option<String>,
    /// The features of the agent.
    pub(super) features: Vec<String>,
    /// The protocol of the agent.
    pub(super) protocol: i32,
    /// The commit of the build of the agent.
    pub(super) build: Option<String>,
    /// The start of the process of the agent.
    pub(super) started_at: Option<OffsetDateTime>,
    /// The time of the disable.
    pub(super) disabled_at: Option<OffsetDateTime>,
    /// The labels that the admins gave.
    pub(super) labels: Vec<String>,
    /// The name of the glyph of the icon, if it has one.
    pub(super) icon_name: Option<String>,
    /// The color of the glyph of the icon, if it has one.
    pub(super) icon_color: Option<String>,
    /// The image of the icon, if it has one.
    pub(super) icon_image_id: Option<uuid::Uuid>,
    /// The options of the glyph avatar of the image, if it is one.
    pub(super) icon_glyph: Option<serde_json::Value>,
}

impl Row {
    /// Makes the App of the row. An App with no version has no host yet.
    pub(super) fn into_app(self) -> App {
        let host = self.version.map(|version| Host {
            version,
            os: self.os.unwrap_or_default(),
            arch: self.arch.unwrap_or_default(),
            hostname: self.hostname.unwrap_or_default(),
            cpu_count: self.cpu_count.unwrap_or_default().max(0) as u32,
            memory_bytes: self.memory_bytes.unwrap_or_default().max(0) as u64,
            capacity: self.capacity.max(1) as u32,
            container_runtime: self.container_runtime,
            features: self.features,
            protocol: self.protocol.max(0) as u32,
            build: self.build,
            started_at: self.started_at,
        });
        App {
            id: AppId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            name: self.name,
            runner: self.runner,
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
            last_seen_at: self.last_seen_at,
            host,
            disabled_at: self.disabled_at,
            labels: AgentLabels::of_checked(self.labels),
            icon: Icon::of_columns(
                self.icon_name,
                self.icon_color,
                self.icon_image_id.map(ImageId::from_db),
            )
            .map(|icon| icon.with_glyph(self.icon_glyph.and_then(GlyphAvatar::of_json))),
        }
    }
}
