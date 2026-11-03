//! The `TestApp` helpers of `AccountService` and of the sessions of `TokenService`: sign-up,
//! sign-in, reset, refresh and sign-out.

use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::auth::v1::token_service_client::TokenServiceClient;
use scylla_proto::auth::v1::{RefreshRequest, RefreshResponse, SignOutRequest};
use tonic::transport::Channel;
use tonic::Status;

use crate::TestApp;

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

    /// Calls `SignIn` with an email or a handle, for a client that keeps the refresh token.
    pub async fn sign_in(
        &self,
        identifier: &str,
        password: &str,
    ) -> Result<pb::SignInResponse, Status> {
        let request = pb::SignInRequest {
            identifier: identifier.to_owned(),
            password: password.to_owned(),
            cookie: false,
        };
        Ok(self.accounts().sign_in(request).await?.into_inner())
    }

    /// Calls `RequestPasswordReset`. The server sends the mail after its answer.
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

    /// Returns a client of `AccountService`.
    fn accounts(&self) -> AccountServiceClient<Channel> {
        AccountServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `TokenService`.
    pub(crate) fn tokens(&self) -> TokenServiceClient<Channel> {
        TokenServiceClient::new(self.channel.clone())
    }
}
