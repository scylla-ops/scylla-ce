//! The payloads of the GitLab webhooks that Scylla reads: `push`, `tag_push` and
//! `merge_request`. Each struct has only the fields that Scylla uses.

use serde::Deserialize;
use serde_json::Value;

/// A delivery of a project webhook.
#[derive(Deserialize)]
pub(super) struct Payload {
    /// `push`, `tag_push` or `merge_request`.
    pub(super) object_kind: String,
    /// The full ref of a push.
    #[serde(rename = "ref")]
    pub(super) git_ref: Option<String>,
    /// The new head of a push.
    pub(super) after: Option<String>,
    /// The commit that a push checks out.
    pub(super) checkout_sha: Option<String>,
    /// The login of the person that pushed.
    pub(super) user_username: Option<String>,
    /// The avatar of the person that pushed.
    pub(super) user_avatar: Option<String>,
    /// The project of the webhook.
    pub(super) project: Option<Project>,
    /// The commits of a push.
    #[serde(default)]
    pub(super) commits: Vec<Commit>,
    /// The person that changed a merge request.
    pub(super) user: Option<User>,
    /// The merge request.
    pub(super) object_attributes: Option<MergeRequest>,
    /// The fields that an update of a merge request changed.
    pub(super) changes: Option<Value>,
}

/// A GitLab project.
#[derive(Deserialize)]
pub(super) struct Project {
    /// Its id.
    pub(super) id: u64,
    /// Its full path.
    pub(super) path_with_namespace: String,
    /// Its page.
    #[serde(default)]
    pub(super) web_url: String,
}

/// A commit.
#[derive(Deserialize)]
pub(super) struct Commit {
    /// The SHA.
    pub(super) id: String,
    /// The message.
    pub(super) message: String,
}

/// A GitLab user.
#[derive(Deserialize)]
pub(super) struct User {
    /// The login.
    pub(super) username: String,
    /// The address of the avatar.
    pub(super) avatar_url: Option<String>,
}

/// A merge request.
#[derive(Deserialize)]
pub(super) struct MergeRequest {
    /// The number in the project.
    pub(super) iid: u32,
    /// The title.
    pub(super) title: String,
    /// The page.
    pub(super) url: String,
    /// The source branch.
    pub(super) source_branch: String,
    /// The target branch.
    pub(super) target_branch: String,
    /// The project of the source branch.
    pub(super) source_project_id: u64,
    /// The project of the target branch.
    pub(super) target_project_id: u64,
    /// `open`, `update`, `reopen`, `close`, `merge`, or another action.
    pub(super) action: Option<String>,
    /// The old head of an update that added commits.
    pub(super) oldrev: Option<String>,
    /// `true` for a draft.
    #[serde(default)]
    pub(super) draft: bool,
    /// The head commit.
    pub(super) last_commit: Option<Commit>,
    /// The project of the source branch.
    pub(super) source: Option<Project>,
}
