//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::grpc::convert::{Parse, required, valid};
use scylla_domain::domain::user::{Email, Password, ResetToken};
use scylla_proto::auth::v1::{
    LoginRequest, RequestPasswordResetRequest, ResetPasswordRequest, RevokeTokenRequest,
    ValidateTokenRequest,
};
use tonic::Status;

/// The client of the call (for the new session) comes from the context, not from the request.
impl Parse for LoginRequest {
    type Into = (String, Password);

    fn parse(self) -> Result<Self::Into, Status> {
        let password = valid(self.password, Password::new)?;
        Ok((self.identifier, password))
    }
}

/// No check: the use case answers `false` for an empty token.
impl Parse for ValidateTokenRequest {
    type Into = String;

    fn parse(self) -> Result<String, Status> {
        Ok(self.token)
    }
}

impl Parse for RevokeTokenRequest {
    type Into = String;

    fn parse(self) -> Result<String, Status> {
        if self.token.is_empty() {
            return Err(Status::invalid_argument("Token cannot be empty"));
        }
        Ok(self.token)
    }
}

impl Parse for RequestPasswordResetRequest {
    type Into = Email;

    fn parse(self) -> Result<Email, Status> {
        valid(required(self.email, "email")?, Email::new)
    }
}

/// A token of another shape is an unknown link: FAILED_PRECONDITION, not INVALID_ARGUMENT.
impl Parse for ResetPasswordRequest {
    type Into = (ResetToken, Password);

    fn parse(self) -> Result<Self::Into, Status> {
        let token = valid(self.token, ResetToken::new)?;
        let new_password = valid(self.new_password, Password::new)?;
        Ok((token, new_password))
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
        let email = RequestPasswordResetRequest {
            email: wrap("Kevin@Example.com"),
        }
        .parse()
        .unwrap();
        assert_eq!(email.as_str(), "kevin@example.com");

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

        let (token, _) = ResetPasswordRequest {
            token: TOKEN.into(),
            new_password: "SecurePass123!".into(),
        }
        .parse()
        .unwrap();
        assert_eq!(token.as_str(), TOKEN);
    }

    #[test]
    fn a_login_request_keeps_the_identifier() {
        let (identifier, _) = LoginRequest {
            identifier: "kevin@example.com".into(),
            password: "SecurePass123!".into(),
        }
        .parse()
        .unwrap();

        assert_eq!(identifier, "kevin@example.com");
    }

    #[test]
    fn an_invalid_password_is_an_invalid_argument() {
        let Err(err) = LoginRequest {
            identifier: "kevin".into(),
            password: String::new(),
        }
        .parse() else {
            panic!("an empty password must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_empty_token_to_validate_parses() {
        let token = ValidateTokenRequest {
            token: String::new(),
        }
        .parse()
        .unwrap();

        assert!(token.is_empty());
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
