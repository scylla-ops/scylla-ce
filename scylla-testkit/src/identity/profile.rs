//! The `TestApp` helpers of `ProfileService`, sent with an access token: the profile, the
//! email, the password, the identities and the sessions of the caller.

use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::profile_service_client::ProfileServiceClient;
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, with_token, TestApp};

impl TestApp {
    /// Calls `GetMe` with an access token.
    pub async fn get_me(&self, token: &str) -> Result<pb::User, Status> {
        let request = with_token(token, pb::GetMeRequest {});
        let response = self.profile().get_me(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `UpdateProfile` with an access token. `None` leaves a field as it is.
    pub async fn update_profile(
        &self,
        token: &str,
        display_name: Option<&str>,
        handle: Option<&str>,
    ) -> Result<pb::User, Status> {
        let request = pb::UpdateProfileRequest {
            display_name: display_name.map(str::to_owned),
            handle: handle.map(str::to_owned),
        };
        let response = self
            .profile()
            .update_profile(with_token(token, request))
            .await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `DeleteAccount` with an access token.
    pub async fn delete_account(&self, token: &str, password: &str) -> Result<(), Status> {
        let request = pb::DeleteAccountRequest {
            password: password.to_owned(),
        };
        self.profile()
            .delete_account(with_token(token, request))
            .await?;
        Ok(())
    }

    /// Calls `RequestEmailChange` with an access token and the current password. The server
    /// sends the mail after its answer.
    pub async fn request_email_change(
        &self,
        token: &str,
        email: &str,
        password: &str,
    ) -> Result<(), Status> {
        let request = pb::RequestEmailChangeRequest {
            new_email: email.to_owned(),
            password: password.to_owned(),
        };
        self.profile()
            .request_email_change(with_token(token, request))
            .await?;
        Ok(())
    }

    /// Calls `ConfirmEmailChange` with an access token and the token of the link.
    pub async fn confirm_email_change(&self, token: &str, link: &str) -> Result<pb::User, Status> {
        let request = pb::ConfirmEmailChangeRequest {
            token: link.to_owned(),
        };
        let response = self
            .profile()
            .confirm_email_change(with_token(token, request))
            .await?;
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

    /// Calls `RevokeOtherSessions` with an access token.
    pub async fn revoke_other_sessions(&self, token: &str) -> Result<(), Status> {
        let request = with_token(token, pb::RevokeOtherSessionsRequest {});
        self.profile().revoke_other_sessions(request).await?;
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

    /// Returns a client of `ProfileService`.
    fn profile(&self) -> ProfileServiceClient<Channel> {
        ProfileServiceClient::new(self.channel.clone())
    }
}
