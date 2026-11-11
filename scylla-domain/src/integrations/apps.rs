//! The GitHub App of a self-hosted instance, made with the manifest flow. The cloud App comes from
//! the settings of the server.

use time::OffsetDateTime;

use super::VcsProvider;
use crate::identity::actors::ActorId;
use crate::platform::Platform;
use crate::Id;

/// The id of the App of the instance in Scylla.
pub type VcsAppId = Id<VcsApp>;

/// The App of the instance on a provider, as the store returns it. No field holds a secret.
#[derive(Clone, Debug)]
pub struct VcsApp {
    /// The id in Scylla.
    pub id: VcsAppId,
    /// The provider.
    pub provider: VcsProvider,
    /// The id of the App on the provider.
    pub app_id: i64,
    /// The slug of the App, for its URLs on the provider.
    pub slug: String,
    /// The display name.
    pub name: String,
    /// The organization that owns the App, or `None` for a user.
    pub owner: Option<String>,
    /// The page of the App on the provider.
    pub web_url: String,
    /// The client id of the App, for the sign-in of a user during an install.
    pub client_id: String,
    /// The instance admin that made the App.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
}

crate::authz! {
    /// The routes of the Git provider settings of the instance. The instance admins pass them,
    /// and a role of the platform level gives them through the catalog.
    extend Platform as VcsAppHolder {
        /// The route `GetVcsApp`.
        permission get_vcs_app = nil;
        /// The route `StartGitHubManifest`.
        permission start_git_hub_manifest = nil;
        /// The route `CompleteGitHubManifest`.
        permission complete_git_hub_manifest = nil;
        /// The route `DeleteVcsApp`.
        permission delete_vcs_app = nil;
    }
}
