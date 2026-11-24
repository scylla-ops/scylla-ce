//! The notifications of a user: one row for each person and each group of facts.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::identity::actors::Actor;
use crate::projects::ProjectId;
use crate::tenancy::invitations::InvitationId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::Id;

/// The id of a notification.
pub type NotificationId = Id<Notification>;

/// The most ids in one call on the rows.
pub const MAX_IDS: usize = 100;

/// The number at which the count of the unread rows stops.
pub const UNREAD_CAP: i32 = 100;

/// The time, in seconds, between two reads of the counts.
pub const POLL_AFTER_SECONDS: i32 = 30;

/// One row of the inbox of a user, with what the read joins.
#[derive(Clone, Debug)]
pub struct Notification {
    /// The id.
    pub id: NotificationId,
    /// The key of the kind.
    pub kind: String,
    /// Why the user gets the row.
    pub reason: String,
    /// The display name of the actor, if the user still exists.
    pub actor_name: Option<String>,
    /// The workspace of the fact.
    pub workspace_id: Option<WorkspaceId>,
    /// The slug of the workspace.
    pub workspace_slug: Option<String>,
    /// The name of the workspace.
    pub workspace_name: Option<String>,
    /// The project of the fact.
    pub project_id: Option<ProjectId>,
    /// The name of the project.
    pub project_name: Option<String>,
    /// The parameters of the kind.
    pub params: BTreeMap<String, String>,
    /// The number of facts in the row.
    pub occurrences: i32,
    /// `true` if the row waits for an action of the user.
    pub needs_action: bool,
    /// The time of the first fact.
    pub first_at: OffsetDateTime,
    /// The time of the last fact.
    pub last_at: OffsetDateTime,
    /// The time at which the user read the row.
    pub read_at: Option<OffsetDateTime>,
    /// The time at which the user marked the row as done.
    pub done_at: Option<OffsetDateTime>,
    /// The invitation of the row, as it is now.
    pub invitation: Option<RowInvitation>,
}

/// The invitation of a row.
#[derive(Clone, Debug)]
pub struct RowInvitation {
    /// The id of the invitation.
    pub id: InvitationId,
    /// The role that it gives.
    pub role: String,
    /// The state of the invitation.
    pub state: InvitationState,
}

/// The state of the invitation of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitationState {
    /// It waits for an answer until this time.
    Open(OffsetDateTime),
    /// A user accepted it at this time.
    Accepted(OffsetDateTime),
    /// It ended at this time with no answer.
    Expired(OffsetDateTime),
}

/// The filters of a list of notifications.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// `true` lists the done rows, `false` the inbox.
    pub done: bool,
    /// `true` lists only the unread rows.
    pub unread_only: bool,
    /// `true` lists only the rows that need an action.
    pub needs_action_only: bool,
    /// Lists only the rows of this workspace.
    pub workspace: Option<WorkspaceId>,
    /// Lists only the rows of these kinds.
    pub kinds: Option<Vec<String>>,
}

/// The counts of an inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    /// The number of unread rows, up to `UNREAD_CAP`.
    pub unread: i32,
    /// The number of rows that need an action.
    pub needs_action: i32,
}

crate::authz! {
    /// The routes of a user on its own inbox.
    extend Actor as NotificationCaller {
        /// The route `GetNotificationSummary`.
        permission get_notification_summary = nil;
        /// The route `ListNotifications`.
        permission list_notifications = nil;
        /// The route `MarkNotificationsRead`.
        permission mark_notifications_read = nil;
        /// The route `MarkAllNotificationsRead`.
        permission mark_all_notifications_read = nil;
        /// The route `SetNotificationsDone`.
        permission set_notifications_done = nil;
    }
}
