use crate::application::PrincipalAccess;
use crate::application::user::{
    ChangePassword, CreateUser, DeleteAccount, DeleteUser, GetMe, GetUser, ListUserAccess,
    ListUserSessions, ListUsers, PasswordResetDelivery, RevokeUserSession, RevokeUserSessions,
    SetUserActive, UpdateUser, UserSession, reset::SendPasswordReset, wrong_current_password,
    wrong_password,
};
use crate::grpc::convert::{Parse, ParseInSession, id, optional, required, ts, valid, wrap};
use crate::grpc::mappers::domain_error_to_status;
use scylla_domain::domain::errors::DomainError;
use scylla_domain::domain::ids::SessionId;
use scylla_domain::domain::user::{DisplayName, Email, Password, User, Username};
use scylla_proto::auth::v1::PasswordResetDelivery as ProtoDelivery;
use scylla_proto::common::v1 as common;
use scylla_proto::user::v1::{
    ChangePasswordRequest, CreateUserRequest, DeleteAccountRequest, DeleteUserRequest,
    GetMeRequest, GetUserRequest, ListUserAccessRequest, ListUserAccessResponse,
    ListUserSessionsRequest, ListUserSessionsResponse, ListUsersRequest, ListUsersResponse,
    RevokeUserSessionRequest, RevokeUserSessionsRequest, SendPasswordResetRequest,
    SetUserActiveRequest, UpdateUserRequest, User as ProtoUser, UserAccess as ProtoUserAccess,
    UserSession as ProtoUserSession,
};
use tonic::Status;

pub fn user_to_proto(user: &User) -> ProtoUser {
    ProtoUser {
        user_id: wrap(user.id().to_string()),
        username: user.username().to_string(),
        is_active: user.is_active(),
        created_at: ts(user.created_at()),
        updated_at: ts(user.updated_at()),
        email: user.email().map(|e| common::Email {
            value: e.as_str().to_string(),
        }),
        display_name: user.display_name().map(ToString::to_string),
    }
}

pub fn user_access_to_proto(access: &PrincipalAccess) -> ProtoUserAccess {
    access_to_proto!(ProtoUserAccess, access)
}

/// No field holds the token.
pub fn user_session_to_proto(session: &UserSession) -> ProtoUserSession {
    ProtoUserSession {
        session_id: wrap(session.id.to_string()),
        created_at: ts(session.created_at),
        last_active_at: ts(session.last_active_at),
        expires_at: ts(session.expires_at),
        user_agent: session.client.user_agent().unwrap_or_default().to_owned(),
        ip_address: session
            .client
            .ip_address()
            .map(|ip| ip.to_string())
            .unwrap_or_default(),
        current: session.current,
    }
}

#[must_use]
pub fn delivery_to_proto(delivery: PasswordResetDelivery) -> i32 {
    match delivery {
        PasswordResetDelivery::Mail => ProtoDelivery::Mail,
        PasswordResetDelivery::ServerLog => ProtoDelivery::ServerLog,
    }
    .into()
}

/// Blank is no display name.
fn display_name(value: String) -> Result<Option<DisplayName>, Status> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    valid(value, DisplayName::new).map(Some)
}

/// A value that is no valid password is a wrong password, not a malformed request.
fn attempt(value: String, wrong: fn() -> DomainError) -> Result<Password, Status> {
    Password::new(value).map_err(|_| domain_error_to_status(wrong()))
}

impl Parse for CreateUserRequest {
    type Into = CreateUser;

    fn parse(self) -> Result<CreateUser, Status> {
        Ok(CreateUser {
            username: valid(self.username, Username::new)?,
            password: valid(self.password, Password::new)?,
            email: valid(required(self.email, "email")?, Email::new)?,
            display_name: self.display_name.map(display_name).transpose()?.flatten(),
        })
    }
}

parse!(GetUserRequest => GetUser { id: id(user_id) });
parse!(GetMeRequest => GetMe);

impl Parse for UpdateUserRequest {
    type Into = UpdateUser;

    fn parse(self) -> Result<UpdateUser, Status> {
        Ok(UpdateUser {
            id: id(self.user_id, "user_id")?,
            username: self.username.map(|u| valid(u, Username::new)).transpose()?,
            display_name: self.display_name.map(display_name).transpose()?,
            email: optional(self.email)
                .map(|e| valid(e, Email::new))
                .transpose()?,
        })
    }
}

parse!(DeleteUserRequest => DeleteUser { id: id(user_id) });
parse!(ListUsersRequest => ListUsers { pagination: page });
page_response!(User => users: user_to_proto; ListUsersResponse);

impl ParseInSession for ChangePasswordRequest {
    type Into = ChangePassword;

    fn parse_in_session(self, session: Option<SessionId>) -> Result<ChangePassword, Status> {
        Ok(ChangePassword {
            current_password: attempt(self.current_password, wrong_current_password)?,
            new_password: valid(self.new_password, Password::new)?,
            session,
        })
    }
}

parse!(SendPasswordResetRequest => SendPasswordReset { id: id(user_id) });

impl ParseInSession for RevokeUserSessionsRequest {
    type Into = RevokeUserSessions;

    fn parse_in_session(self, session: Option<SessionId>) -> Result<RevokeUserSessions, Status> {
        Ok(RevokeUserSessions {
            id: id(self.user_id, "user_id")?,
            session,
        })
    }
}

parse!(SetUserActiveRequest => SetUserActive { id: id(user_id), is_active: copy });

impl Parse for DeleteAccountRequest {
    type Into = DeleteAccount;

    fn parse(self) -> Result<DeleteAccount, Status> {
        Ok(DeleteAccount {
            password: attempt(self.password, wrong_password)?,
        })
    }
}

parse!(ListUserAccessRequest => ListUserAccess { id: id(user_id) });

pub fn user_access_response(access: &[PrincipalAccess]) -> ListUserAccessResponse {
    ListUserAccessResponse {
        access: access.iter().map(user_access_to_proto).collect(),
    }
}

impl ParseInSession for ListUserSessionsRequest {
    type Into = ListUserSessions;

    fn parse_in_session(self, session: Option<SessionId>) -> Result<ListUserSessions, Status> {
        Ok(ListUserSessions {
            id: id(self.user_id, "user_id")?,
            session,
        })
    }
}

pub fn user_sessions_response(sessions: &[UserSession]) -> ListUserSessionsResponse {
    ListUserSessionsResponse {
        sessions: sessions.iter().map(user_session_to_proto).collect(),
    }
}

parse!(RevokeUserSessionRequest => RevokeUserSession { id: id(user_id), session_id: id(session_id) });

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::user::{WRONG_CURRENT_PASSWORD, WRONG_PASSWORD};
    use scylla_auth::authz::Scope;
    use scylla_domain::domain::ids::{OrganizationId, ProjectId};
    use scylla_domain::domain::organization::OrganizationName;
    use scylla_domain::domain::project::ProjectName;
    use scylla_domain::domain::role::{RoleDisplayName, RoleName};
    use scylla_domain::domain::session::SessionClient;
    use scylla_domain::domain::user::PasswordHash;
    use tonic::Code;

    fn create_request() -> CreateUserRequest {
        CreateUserRequest {
            username: "alice".into(),
            password: "SecurePass123!".into(),
            email: wrap("alice@example.com"),
            display_name: None,
        }
    }

    fn update_request() -> UpdateUserRequest {
        UpdateUserRequest {
            user_id: wrap("user-1"),
            username: None,
            display_name: None,
            email: None,
        }
    }

    #[test]
    fn user_to_proto_maps_all_fields() {
        let user = User::create(
            Username::new("alice").unwrap(),
            None,
            PasswordHash::new("$argon2id$v=19$m=19456,t=2,p=1$abc$def").unwrap(),
        )
        .with_display_name(Some(DisplayName::new("Alice A.").unwrap()));

        let proto = user_to_proto(&user);
        assert_eq!(proto.username, "alice");
        assert_eq!(proto.display_name.as_deref(), Some("Alice A."));
        assert!(proto.is_active);
        assert!(!proto.user_id.unwrap().value.is_empty());
        assert!(proto.created_at.is_some());
        assert!(proto.updated_at.is_some());
    }

    #[test]
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateUserRequest {
            display_name: Some("  Alice  ".into()),
            ..create_request()
        }
        .parse()
        .unwrap();

        assert_eq!(command.username.as_str(), "alice");
        assert_eq!(command.password.as_str(), "SecurePass123!");
        assert_eq!(command.email.as_str(), "alice@example.com");
        assert_eq!(command.display_name.unwrap().as_str(), "Alice");
    }

    #[test]
    fn a_create_request_without_an_email_is_an_invalid_argument() {
        let err = CreateUserRequest {
            email: None,
            ..create_request()
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing email");
    }

    #[test]
    fn an_absent_or_blank_display_name_is_none_at_creation() {
        for display_name in [None, Some(String::new()), Some("   ".into())] {
            let command = CreateUserRequest {
                display_name,
                ..create_request()
            }
            .parse()
            .unwrap();
            assert!(command.display_name.is_none());
        }
    }

    #[test]
    fn a_display_name_that_breaks_a_rule_is_an_invalid_argument() {
        for bad in ["a\nb".to_string(), "é".repeat(101)] {
            let err = CreateUserRequest {
                display_name: Some(bad),
                ..create_request()
            }
            .parse()
            .unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument);
        }
    }

    #[test]
    fn a_missing_id_is_an_invalid_argument() {
        let err = GetUserRequest {
            user_id: None::<common::UserId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing user_id");
    }

    #[test]
    fn a_domain_validation_failure_is_an_invalid_argument() {
        let err = CreateUserRequest {
            password: "short".into(),
            ..create_request()
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_update_without_a_field_changes_nothing() {
        let command = update_request().parse().unwrap();

        assert_eq!(command.id.as_str(), "user-1");
        assert!(command.username.is_none());
        assert!(command.display_name.is_none());
        assert!(command.email.is_none());
    }

    #[test]
    fn an_empty_display_name_in_an_update_removes_it() {
        let command = UpdateUserRequest {
            display_name: Some(String::new()),
            ..update_request()
        }
        .parse()
        .unwrap();
        assert_eq!(command.display_name, Some(None));

        let command = UpdateUserRequest {
            display_name: Some("Bob".into()),
            email: wrap("Bob@Example.com"),
            ..update_request()
        }
        .parse()
        .unwrap();
        assert_eq!(
            command.display_name,
            Some(Some(DisplayName::new("Bob").unwrap()))
        );
        assert_eq!(command.email.unwrap().as_str(), "bob@example.com");
    }

    #[test]
    fn a_current_password_that_is_no_password_is_a_wrong_password() {
        let err = ChangePasswordRequest {
            current_password: "short".into(),
            new_password: "SecurePass123!".into(),
        }
        .parse_in_session(None)
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert_eq!(err.message(), WRONG_CURRENT_PASSWORD);

        let err = DeleteAccountRequest {
            password: String::new(),
        }
        .parse()
        .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert_eq!(err.message(), WRONG_PASSWORD);
    }

    #[test]
    fn a_new_password_that_breaks_a_rule_is_an_invalid_argument() {
        let err = ChangePasswordRequest {
            current_password: "SecurePass123!".into(),
            new_password: "short".into(),
        }
        .parse_in_session(None)
        .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn the_session_of_the_call_goes_into_the_command() {
        let session = SessionId::new("s1");
        let command = RevokeUserSessionsRequest {
            user_id: wrap("user-1"),
        }
        .parse_in_session(Some(session.clone()))
        .unwrap();
        assert_eq!(command.session, Some(session.clone()));

        let command = ChangePasswordRequest {
            current_password: "SecurePass123!".into(),
            new_password: "OtherPass123!".into(),
        }
        .parse_in_session(Some(session.clone()))
        .unwrap();
        assert_eq!(command.session, Some(session));
    }

    #[test]
    fn an_access_row_names_its_scope() {
        let proto = user_access_to_proto(&PrincipalAccess {
            grant_id: "g1".into(),
            scope: Scope::Project(ProjectId::new("p1")),
            organization_id: Some(OrganizationId::new("o1")),
            organization_name: Some(OrganizationName::new("Acme").unwrap()),
            project_name: Some(ProjectName::new("Web").unwrap()),
            role: RoleName::new("project-admin").unwrap(),
            role_name: RoleDisplayName::new("Project admin").unwrap(),
        });
        assert_eq!(proto.organization_id.unwrap().value, "o1");
        assert_eq!(proto.organization_name, "Acme");
        assert_eq!(proto.project_name, "Web");
        assert_eq!(proto.role_id.unwrap().value, "project-admin");
        assert_eq!(proto.role_name, "Project admin");
        assert!(proto.scope.is_some());

        let system = user_access_to_proto(&PrincipalAccess {
            grant_id: "g2".into(),
            scope: Scope::System,
            organization_id: None,
            organization_name: None,
            project_name: None,
            role: RoleName::new("system-admin").unwrap(),
            role_name: RoleDisplayName::new("System admin").unwrap(),
        });
        assert!(system.organization_id.is_none());
        assert!(system.organization_name.is_empty());
        assert!(system.project_name.is_empty());
    }

    #[test]
    fn a_session_row_shows_its_client_and_no_token() {
        let now = chrono::Utc::now();
        let session = UserSession {
            id: SessionId::new("s1"),
            created_at: now,
            last_active_at: now,
            expires_at: now + chrono::Duration::hours(1),
            client: SessionClient::new(Some("Firefox/131.0"), Some("2001:db8::1")),
            current: true,
        };

        let proto = user_session_to_proto(&session);
        assert_eq!(proto.session_id.unwrap().value, "s1");
        assert_eq!(proto.user_agent, "Firefox/131.0");
        assert_eq!(proto.ip_address, "2001:db8::1");
        assert!(proto.current);
        assert_eq!(proto.expires_at, ts(session.expires_at));

        let unknown = user_session_to_proto(&UserSession {
            client: SessionClient::default(),
            current: false,
            ..session
        });
        assert!(unknown.user_agent.is_empty());
        assert!(unknown.ip_address.is_empty());
        assert!(!unknown.current);
    }

    #[test]
    fn a_session_list_names_the_session_of_the_call_and_a_revoke_needs_both_ids() {
        let session = SessionId::new("s1");
        let command = ListUserSessionsRequest {
            user_id: wrap("user-1"),
        }
        .parse_in_session(Some(session.clone()))
        .unwrap();
        assert_eq!(command.id.as_str(), "user-1");
        assert_eq!(command.session, Some(session));

        let command = RevokeUserSessionRequest {
            user_id: wrap("user-1"),
            session_id: wrap("s2"),
        }
        .parse()
        .unwrap();
        assert_eq!(command.id.as_str(), "user-1");
        assert_eq!(command.session_id.as_str(), "s2");

        let err = RevokeUserSessionRequest {
            user_id: wrap("user-1"),
            session_id: None,
        }
        .parse()
        .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing session_id");
    }

    #[test]
    fn each_delivery_has_its_wire_value() {
        assert_eq!(
            delivery_to_proto(PasswordResetDelivery::Mail),
            ProtoDelivery::Mail as i32
        );
        assert_eq!(
            delivery_to_proto(PasswordResetDelivery::ServerLog),
            ProtoDelivery::ServerLog as i32
        );
    }
}
