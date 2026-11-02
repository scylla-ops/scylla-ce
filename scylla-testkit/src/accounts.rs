//! The `TestApp` helpers of the account RPCs: sign-up, sign-in, sessions, profile, invitations,
//! and the identity providers.

use scylla_core::features::providers::{self, GitHubSettings, OidcSettings, ProviderRow};
use scylla_domain::actors::ActorId;
use scylla_domain::users::ProviderId;
use scylla_domain::workspaces::WorkspaceId;
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::accounts::v1::profile_service_client::ProfileServiceClient;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{RefreshRequest, RefreshResponse, SignOutRequest};
use scylla_proto::workspaces::v1::invitation_service_client::InvitationServiceClient;
use scylla_proto::workspaces::v1::{AcceptInvitationRequest, CreateInvitationRequest, Invitation};
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, text, with_token, TestApp};

impl TestApp {
    /// Calls `SignUp`. An empty invitation token means none.
    pub async fn sign_up(
        &self,
        email: &str,
        password: &str,
        invitation: &str,
    ) -> Result<(), Status> {
        let request = pb::SignUpRequest {
            email: email.to_owned(),
            password: password.to_owned(),
            display_name: "New User".to_owned(),
            invitation_token: invitation.to_owned(),
        };
        self.accounts().sign_up(request).await?;
        Ok(())
    }

    /// Calls `VerifyEmail`.
    pub async fn verify_email(&self, token: &str) -> Result<(), Status> {
        let request = pb::VerifyEmailRequest {
            token: token.to_owned(),
        };
        self.accounts().verify_email(request).await?;
        Ok(())
    }

    /// Calls `SignIn` for a client that keeps the refresh token.
    pub async fn sign_in(&self, email: &str, password: &str) -> Result<pb::SignInResponse, Status> {
        let request = pb::SignInRequest {
            email: email.to_owned(),
            password: password.to_owned(),
            cookie: false,
        };
        Ok(self.accounts().sign_in(request).await?.into_inner())
    }

    /// Calls `RequestPasswordReset`.
    pub async fn request_reset(&self, email: &str) -> Result<(), Status> {
        let request = pb::RequestPasswordResetRequest {
            email: email.to_owned(),
        };
        self.accounts().request_password_reset(request).await?;
        Ok(())
    }

    /// Calls `ResetPassword`.
    pub async fn reset_password(&self, token: &str, password: &str) -> Result<(), Status> {
        let request = pb::ResetPasswordRequest {
            token: token.to_owned(),
            password: password.to_owned(),
        };
        self.accounts().reset_password(request).await?;
        Ok(())
    }

    /// Calls `Refresh` with a refresh token.
    pub async fn refresh(&self, refresh: &str) -> Result<RefreshResponse, Status> {
        let request = RefreshRequest {
            refresh_token: refresh.to_owned(),
        };
        Ok(self.tokens().refresh(request).await?.into_inner())
    }

    /// Calls `SignOut` with a refresh token.
    pub async fn sign_out(&self, refresh: &str) -> Result<(), Status> {
        let request = SignOutRequest {
            refresh_token: refresh.to_owned(),
        };
        self.tokens().sign_out(request).await?;
        Ok(())
    }

    /// Calls `GetMe` with an access token.
    pub async fn get_me(&self, token: &str) -> Result<pb::User, Status> {
        let request = with_token(token, pb::GetMeRequest {});
        let response = self.profile().get_me(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `ChangePassword` with an access token.
    pub async fn change_password(
        &self,
        token: &str,
        current: &str,
        new: &str,
    ) -> Result<(), Status> {
        let request = pb::ChangePasswordRequest {
            current_password: current.to_owned(),
            new_password: new.to_owned(),
        };
        self.profile()
            .change_password(with_token(token, request))
            .await?;
        Ok(())
    }

    /// Calls `ListSessions` with an access token.
    pub async fn list_sessions(&self, token: &str) -> Result<Vec<pb::Session>, Status> {
        let request = with_token(token, pb::ListSessionsRequest {});
        let response = self.profile().list_sessions(request).await?;
        Ok(response.into_inner().sessions)
    }

    /// Calls `RevokeSession` with an access token. The id is text, so a test can send a bad id.
    pub async fn revoke_session(&self, token: &str, session: &str) -> Result<(), Status> {
        let request = pb::RevokeSessionRequest {
            session_id: Some(id(session)),
        };
        self.profile()
            .revoke_session(with_token(token, request))
            .await?;
        Ok(())
    }

    /// Calls `ListIdentities` with an access token.
    pub async fn list_identities(&self, token: &str) -> Result<pb::ListIdentitiesResponse, Status> {
        let request = with_token(token, pb::ListIdentitiesRequest {});
        Ok(self.profile().list_identities(request).await?.into_inner())
    }

    /// Calls `UnlinkIdentity` with an access token.
    pub async fn unlink_identity(
        &self,
        token: &str,
        provider: &str,
        subject: &str,
    ) -> Result<(), Status> {
        let request = pb::UnlinkIdentityRequest {
            provider: provider.to_owned(),
            subject: subject.to_owned(),
        };
        self.profile()
            .unlink_identity(with_token(token, request))
            .await?;
        Ok(())
    }

    /// Calls `CreateInvitation` as `actor`.
    pub async fn create_invitation(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        email: &str,
        role: &str,
    ) -> Result<Invitation, Status> {
        let request = CreateInvitationRequest {
            workspace_id: Some(id(workspace.to_string())),
            email: email.to_owned(),
            role: role.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.invitations().create_invitation(request).await?;
        Ok(response
            .into_inner()
            .invitation
            .expect("invitation in response"))
    }

    /// Calls `AcceptInvitation` with an access token, and returns the id of the workspace.
    pub async fn accept_invitation(&self, token: &str, invitation: &str) -> Result<String, Status> {
        let request = AcceptInvitationRequest {
            token: invitation.to_owned(),
        };
        let response = self
            .invitations()
            .accept_invitation(with_token(token, request))
            .await?;
        Ok(text(&response.into_inner().workspace_id).to_owned())
    }

    /// Adds an OIDC provider, as `provider add-oidc` does.
    pub async fn add_oidc_provider(
        &self,
        slug: &str,
        issuer: &str,
        client_id: &str,
        secret: Option<&str>,
        allow_sign_up: bool,
    ) {
        let settings = OidcSettings {
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            scopes: ["openid", "email", "profile"].map(str::to_owned).to_vec(),
        };
        let settings = serde_json::to_value(settings).expect("settings");
        self.add_provider(slug, "oidc", settings, secret, allow_sign_up)
            .await;
    }

    /// Adds a GitHub provider at a base URL for the web and the API, as `provider add-github`
    /// does.
    pub async fn add_github_provider(&self, slug: &str, url: &str, client_id: &str, secret: &str) {
        let settings = GitHubSettings {
            client_id: client_id.to_owned(),
            web_url: url.to_owned(),
            api_url: url.to_owned(),
        };
        let settings = serde_json::to_value(settings).expect("settings");
        self.add_provider(slug, "github", settings, Some(secret), true)
            .await;
    }

    /// Writes a provider that is on.
    async fn add_provider(
        &self,
        slug: &str,
        kind: &str,
        settings: serde_json::Value,
        secret: Option<&str>,
        allow_sign_up: bool,
    ) {
        let row = ProviderRow {
            id: ProviderId::generate(),
            slug: slug.to_owned(),
            kind: kind.to_owned(),
            display_name: slug.to_owned(),
            settings,
            client_secret: secret.map(|secret| self.sealer.seal(secret)),
            allow_sign_up,
            enabled: true,
        };
        let mut tx = self.pool.begin().await.expect("transaction");
        providers::upsert(&mut tx, &row).await.expect("provider");
        tx.commit().await.expect("commit");
    }

    /// Returns a client of `AccountService`.
    fn accounts(&self) -> AccountServiceClient<Channel> {
        AccountServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `ProfileService`.
    fn profile(&self) -> ProfileServiceClient<Channel> {
        ProfileServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `TokenService`.
    fn tokens(&self) -> TokenServiceClient<Channel> {
        TokenServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `InvitationService`.
    fn invitations(&self) -> InvitationServiceClient<Channel> {
        InvitationServiceClient::new(self.channel.clone())
    }
}
