use crate::application::organization::{
    CreateOrganization, DeleteOrganization, GetOrganization, ListOrganizationMembers,
    ListOrganizations, ListUserOrganizations, SetOrganizationActive, UpdateOrganization,
};
use crate::application::pagination::PaginatedResult;
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use crate::grpc::mappers::domain_to_proto_metadata;
use scylla_domain::domain::organization::{
    Organization, OrganizationDescription, OrganizationName,
};
use scylla_domain::domain::user::User;
use scylla_proto::organization::v1::{
    CreateOrganizationRequest, DeleteOrganizationRequest, GetOrganizationRequest,
    ListOrganizationMembersRequest, ListOrganizationMembersResponse, ListOrganizationsRequest,
    ListOrganizationsResponse, ListUserOrganizationsRequest, ListUserOrganizationsResponse,
    Organization as ProtoOrganization, OrganizationMember, SetOrganizationActiveRequest,
    UpdateOrganizationRequest,
};
use tonic::Status;

pub fn organization_to_proto(org: &Organization) -> ProtoOrganization {
    ProtoOrganization {
        organization_id: wrap(org.id().to_string()),
        name: org.name().to_string(),
        description: org
            .description()
            .map(|d| d.as_str().to_string())
            .unwrap_or_default(),
        is_active: org.is_active(),
        created_at: ts(org.created_at()),
        updated_at: ts(org.updated_at()),
    }
}

impl Parse for CreateOrganizationRequest {
    type Into = CreateOrganization;

    fn parse(self) -> Result<CreateOrganization, Status> {
        Ok(CreateOrganization {
            name: valid(self.name, OrganizationName::new)?,
            description: self
                .description
                .map(|d| valid(d, OrganizationDescription::new))
                .transpose()?,
        })
    }
}

parse!(GetOrganizationRequest => GetOrganization { id: id(organization_id) });

impl Parse for UpdateOrganizationRequest {
    type Into = UpdateOrganization;

    // A description that is set but empty clears it: the wire has no way to send `Some(None)`.
    fn parse(self) -> Result<UpdateOrganization, Status> {
        Ok(UpdateOrganization {
            id: id(self.organization_id, "organization_id")?,
            name: self
                .name
                .map(|n| valid(n, OrganizationName::new))
                .transpose()?,
            description: self
                .description
                .map(|d| valid(d, OrganizationDescription::new).map(Some))
                .transpose()?,
        })
    }
}

parse!(SetOrganizationActiveRequest => SetOrganizationActive {
    id: id(organization_id),
    is_active: copy,
});

parse!(DeleteOrganizationRequest => DeleteOrganization { id: id(organization_id) });
parse!(ListOrganizationsRequest => ListOrganizations { pagination: page });

parse!(ListOrganizationMembersRequest => ListOrganizationMembers {
    organization_id: id(organization_id),
    pagination: page,
});

parse!(ListUserOrganizationsRequest => ListUserOrganizations {
    user_id: id(user_id),
    pagination: page,
});

page_response!(
    Organization => organizations: organization_to_proto;
    ListOrganizationsResponse,
    ListUserOrganizationsResponse,
);

impl From<PaginatedResult<User>> for ListOrganizationMembersResponse {
    fn from(page: PaginatedResult<User>) -> Self {
        let (users, metadata) = page.into_parts();
        Self {
            members: users
                .iter()
                .map(|user| OrganizationMember {
                    user_id: wrap(user.id().to_string()),
                    username: user.username().to_string(),
                })
                .collect(),
            pagination: Some(domain_to_proto_metadata(&metadata)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    #[test]
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateOrganizationRequest {
            name: "  acme ".into(),
            description: Some("widgets".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(command.name.as_str(), "acme");
        assert_eq!(
            command
                .description
                .as_ref()
                .map(OrganizationDescription::as_str),
            Some("widgets")
        );
    }

    #[test]
    fn a_missing_id_is_an_invalid_argument() {
        let err = GetOrganizationRequest {
            organization_id: None::<common::OrganizationId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing organization_id");
    }

    #[test]
    fn a_domain_validation_failure_is_an_invalid_argument() {
        let err = CreateOrganizationRequest {
            name: "   ".into(),
            description: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_update_with_a_description_set_carries_it_as_a_change() {
        let command = UpdateOrganizationRequest {
            organization_id: wrap("org-1"),
            name: None,
            description: Some(String::new()),
        }
        .parse()
        .unwrap();

        assert!(command.name.is_none());
        assert!(matches!(command.description, Some(Some(_))));
    }
}
