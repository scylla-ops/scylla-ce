//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::auth::{Login, RevokeToken, ValidateToken};
use crate::application::user::reset::{RequestPasswordReset, ResetPassword};
use crate::grpc::convert::{Parse, ParseWithClient, required, valid};
use scylla_domain::domain::session::SessionClient;
use scylla_domain::domain::user::{Email, Password, ResetToken};
use scylla_proto::auth::v1::{
    LoginRequest, RequestPasswordResetRequest, ResetPasswordRequest, RevokeTokenRequest,
    ValidateTokenRequest,
};
use tonic::Status;

impl ParseWithClient for LoginRequest {
    type Into = Login;

    fn parse_with_client(self, client: SessionClient) -> Result<Login, Status> {
        Ok(Login {
            identifier: self.identifier,
            password: valid(self.password, Password::new)?,
            client,
        })
    }
}

parse!(ValidateTokenRequest => ValidateToken { token: copy });

impl Parse for RevokeTokenRequest {
    type Into = RevokeToken;

    fn parse(self) -> Result<RevokeToken, Status> {
        if self.token.is_empty() {
            return Err(Status::invalid_argument("Token cannot be empty"));
        }
        Ok(RevokeToken { token: self.token })
    }
}

impl Parse for RequestPasswordResetRequest {
    type Into = RequestPasswordReset;

    fn parse(self) -> Result<RequestPasswordReset, Status> {
        Ok(RequestPasswordReset {
            email: valid(required(self.email, "email")?, Email::new)?,
        })
    }
}

/// A token of another shape is an unknown link: FAILED_PRECONDITION, not INVALID_ARGUMENT.
impl Parse for ResetPasswordRequest {
    type Into = ResetPassword;

    fn parse(self) -> Result<ResetPassword, Status> {
        Ok(ResetPassword {
            token: valid(self.token, ResetToken::new)?,
            new_password: valid(self.new_password, Password::new)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::convert::wrap;
    use scylla_domain::domain::user::RESET_LINK_INVALID;
    use tonic::Code;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z";

    #[test]
    fn a_reset_request_needs_a_valid_email() {
        let command = RequestPasswordResetRequest {
            email: wrap("Kevin@Example.com"),
        }
        .parse()
        .unwrap();
        assert_eq!(command.email.as_str(), "kevin@example.com");

        for email in [None, wrap("nope")] {
            let Err(err) = RequestPasswordResetRequest { email }.parse() else {
                panic!("a missing or malformed email must not parse");
            };
            assert_eq!(err.code(), Code::InvalidArgument);
        }
    }

    #[test]
    fn a_token_of_another_shape_is_an_invalid_link() {
        for token in [String::new(), "x".into(), format!("{TOKEN}x")] {
            let Err(err) = (ResetPasswordRequest {
                token,
                new_password: "SecurePass123!".into(),
            })
            .parse() else {
                panic!("a malformed token must not parse");
            };
            assert_eq!(err.code(), Code::FailedPrecondition);
            assert_eq!(err.message(), RESET_LINK_INVALID);
        }
    }

    #[test]
    fn a_new_password_that_breaks_a_rule_is_an_invalid_argument() {
        let Err(err) = (ResetPasswordRequest {
            token: TOKEN.into(),
            new_password: "short".into(),
        })
        .parse() else {
            panic!("a short password must not parse");
        };
        assert_eq!(err.code(), Code::InvalidArgument);

        let command = ResetPasswordRequest {
            token: TOKEN.into(),
            new_password: "SecurePass123!".into(),
        }
        .parse()
        .unwrap();
        assert_eq!(command.token.as_str(), TOKEN);
    }

    #[test]
    fn a_login_request_keeps_the_identifier_and_the_client() {
        let client = SessionClient::new(Some("curl/8.0"), Some("192.0.2.1"));
        let command = LoginRequest {
            identifier: "kevin@example.com".into(),
            password: "SecurePass123!".into(),
        }
        .parse_with_client(client.clone())
        .unwrap();

        assert_eq!(command.identifier, "kevin@example.com");
        assert_eq!(command.client, client);
    }

    #[test]
    fn an_invalid_password_is_an_invalid_argument() {
        let Err(err) = LoginRequest {
            identifier: "kevin".into(),
            password: String::new(),
        }
        .parse_with_client(SessionClient::default()) else {
            panic!("an empty password must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_empty_token_to_revoke_is_an_invalid_argument() {
        let Err(err) = RevokeTokenRequest {
            token: String::new(),
        }
        .parse() else {
            panic!("an empty token must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "Token cannot be empty");
    }
}
