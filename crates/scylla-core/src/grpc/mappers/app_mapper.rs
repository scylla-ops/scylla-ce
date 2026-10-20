//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::PrincipalAccess;
use crate::application::app::{
    CreateApp, CreateAppSecret, DeleteApp, GetApp, IssueAppToken, ListAppAccess, ListAppSecrets,
    ListApps, RevokeAppSecret, SetAppActive, SetAppSecretEnabled, SetAppSecretLabel,
};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use scylla_domain::domain::app::{App, AppCredential, AppName, AppSecret, AppSecretLabel};
use scylla_proto::app::v1::{
    App as ProtoApp, AppAccess as ProtoAppAccess, AppSecret as ProtoAppSecret, CreateAppRequest,
    CreateAppSecretRequest, DeleteAppRequest, GetAppRequest, IssueTokenRequest,
    ListAppAccessRequest, ListAppSecretsRequest, ListAppsRequest, RevokeAppSecretRequest,
    SetAppActiveRequest, SetAppSecretEnabledRequest, SetAppSecretLabelRequest,
};
use tonic::Status;

pub fn app_to_proto(a: &App) -> ProtoApp {
    ProtoApp {
        app_id: wrap(a.id().to_string()),
        organization_id: wrap(a.organization_id().to_string()),
        name: a.name().as_str().to_string(),
        is_active: a.is_active(),
        created_at: ts(a.created_at()),
        updated_at: ts(a.updated_at()),
    }
}

pub fn app_credential_to_proto(c: &AppCredential) -> ProtoAppSecret {
    ProtoAppSecret {
        app_secret_id: wrap(c.id().to_string()),
        app_id: wrap(c.app_id().to_string()),
        label: c.label().as_str().to_string(),
        enabled: c.is_enabled(),
        created_at: ts(c.created_at()),
        updated_at: ts(c.updated_at()),
    }
}

pub fn app_access_to_proto(access: &PrincipalAccess) -> ProtoAppAccess {
    access_to_proto!(ProtoAppAccess, access)
}

impl Parse for CreateAppRequest {
    type Into = CreateApp;

    fn parse(self) -> Result<CreateApp, Status> {
        Ok(CreateApp {
            organization_id: id(self.organization_id, "organization_id")?,
            name: valid(self.name, AppName::new)?,
            secret_label: self
                .secret_label
                .map(|label| valid(label, AppSecretLabel::new))
                .transpose()?,
        })
    }
}

parse!(GetAppRequest => GetApp { id: id(app_id) });
parse!(ListAppsRequest => ListApps { organization_id: id(organization_id) });
parse!(DeleteAppRequest => DeleteApp { id: id(app_id) });

parse!(SetAppActiveRequest => SetAppActive {
    id: id(app_id),
    is_active: copy,
});

impl Parse for CreateAppSecretRequest {
    type Into = CreateAppSecret;

    fn parse(self) -> Result<CreateAppSecret, Status> {
        Ok(CreateAppSecret {
            app_id: id(self.app_id, "app_id")?,
            label: valid(self.label, AppSecretLabel::new)?,
        })
    }
}

parse!(ListAppSecretsRequest => ListAppSecrets { app_id: id(app_id) });
parse!(RevokeAppSecretRequest => RevokeAppSecret { id: id(app_secret_id) });

parse!(SetAppSecretEnabledRequest => SetAppSecretEnabled {
    id: id(app_secret_id),
    enabled: copy,
});

impl Parse for SetAppSecretLabelRequest {
    type Into = SetAppSecretLabel;

    fn parse(self) -> Result<SetAppSecretLabel, Status> {
        Ok(SetAppSecretLabel {
            id: id(self.app_secret_id, "app_secret_id")?,
            label: valid(self.label, AppSecretLabel::new)?,
        })
    }
}

parse!(ListAppAccessRequest => ListAppAccess { app_id: id(app_id) });

impl Parse for IssueTokenRequest {
    type Into = IssueAppToken;

    /// A malformed secret is an auth failure, so the response never says why.
    fn parse(self) -> Result<IssueAppToken, Status> {
        Ok(IssueAppToken {
            app_id: id(self.app_id, "app_id")?,
            secret: AppSecret::new(self.secret)
                .map_err(|_| Status::unauthenticated("Invalid app credentials"))?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_auth::authz::Scope;
    use scylla_domain::domain::ids::{OrganizationId, ProjectId};
    use scylla_domain::domain::organization::OrganizationName;
    use scylla_domain::domain::project::ProjectName;
    use scylla_domain::domain::role::{RoleDisplayName, RoleName};
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    #[test]
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateAppRequest {
            organization_id: wrap("acme"),
            name: "ci-bot".into(),
            secret_label: Some(" build-01 ".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(command.organization_id.as_str(), "acme");
        assert_eq!(command.name.as_str(), "ci-bot");
        assert_eq!(command.secret_label.unwrap().as_str(), "build-01");
    }

    #[test]
    fn a_create_request_without_a_label_leaves_it_to_the_command() {
        let command = CreateAppRequest {
            organization_id: wrap("acme"),
            name: "ci-bot".into(),
            secret_label: None,
        }
        .parse()
        .unwrap();

        assert!(command.secret_label.is_none());
    }

    #[test]
    fn a_blank_first_label_is_an_invalid_argument() {
        let Err(err) = CreateAppRequest {
            organization_id: wrap("acme"),
            name: "ci-bot".into(),
            secret_label: Some("  ".into()),
        }
        .parse() else {
            panic!("a blank label must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_set_active_request_keeps_the_flag() {
        let command = SetAppActiveRequest {
            app_id: wrap("app-1"),
            is_active: false,
        }
        .parse()
        .unwrap();

        assert_eq!(command.id.as_str(), "app-1");
        assert!(!command.is_active);
    }

    #[test]
    fn a_missing_app_id_is_an_invalid_argument() {
        let err = GetAppRequest {
            app_id: None::<common::AppId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing app_id");
    }

    #[test]
    fn an_invalid_name_is_an_invalid_argument() {
        let Err(err) = CreateAppRequest {
            organization_id: wrap("acme"),
            name: String::new(),
            secret_label: None,
        }
        .parse() else {
            panic!("an empty name must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_malformed_app_secret_is_unauthenticated_without_a_reason() {
        let Err(err) = IssueTokenRequest {
            app_id: wrap("app-1"),
            secret: "short".into(),
        }
        .parse() else {
            panic!("a malformed secret must not parse");
        };

        assert_eq!(err.code(), Code::Unauthenticated);
        assert_eq!(err.message(), "Invalid app credentials");
    }

    #[test]
    fn an_invalid_label_is_an_invalid_argument() {
        let Err(err) = CreateAppSecretRequest {
            app_id: wrap("app-1"),
            label: String::new(),
        }
        .parse() else {
            panic!("an empty label must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_label_change_parses_the_secret_and_the_label() {
        let command = SetAppSecretLabelRequest {
            app_secret_id: wrap("secret-1"),
            label: " build-01.example.com ".into(),
        }
        .parse()
        .unwrap();

        assert_eq!(command.id.as_str(), "secret-1");
        assert_eq!(command.label.as_str(), "build-01.example.com");
    }

    #[test]
    fn a_label_change_refuses_a_bad_label_or_no_secret() {
        let Err(long) = SetAppSecretLabelRequest {
            app_secret_id: wrap("secret-1"),
            label: "a".repeat(65),
        }
        .parse() else {
            panic!("a label over 64 bytes must not parse");
        };
        let Err(missing) = SetAppSecretLabelRequest {
            app_secret_id: None::<common::AppSecretId>,
            label: "ci".into(),
        }
        .parse() else {
            panic!("a request without a secret must not parse");
        };

        assert_eq!(long.code(), Code::InvalidArgument);
        assert_eq!(missing.code(), Code::InvalidArgument);
        assert_eq!(missing.message(), "missing app_secret_id");
    }

    #[test]
    fn an_access_request_without_an_app_is_an_invalid_argument() {
        let err = ListAppAccessRequest {
            app_id: None::<common::AppId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing app_id");
    }

    #[test]
    fn an_access_row_names_its_scope() {
        let proto = app_access_to_proto(&PrincipalAccess {
            grant_id: "g1".into(),
            scope: Scope::Project(ProjectId::new("p1")),
            organization_id: Some(OrganizationId::new("o1")),
            organization_name: Some(OrganizationName::new("Acme").unwrap()),
            project_name: Some(ProjectName::new("Web").unwrap()),
            role: RoleName::new("project-agent").unwrap(),
            role_name: RoleDisplayName::new("Project agent").unwrap(),
        });

        assert_eq!(proto.grant_id.unwrap().value, "g1");
        assert_eq!(proto.organization_id.unwrap().value, "o1");
        assert_eq!(proto.organization_name, "Acme");
        assert_eq!(proto.project_name, "Web");
        assert_eq!(proto.role_id.unwrap().value, "project-agent");
        assert_eq!(proto.role_name, "Project agent");
        assert!(proto.scope.is_some());
    }
}
