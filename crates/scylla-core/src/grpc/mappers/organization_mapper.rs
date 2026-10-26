use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use crate::grpc::mappers::{domain_to_proto_metadata, proto_to_domain_pagination};
use scylla_domain::domain::ids::{OrganizationId, UserId};
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
    type Into = (OrganizationName, Option<OrganizationDescription>);

    fn parse(self) -> Result<Self::Into, Status> {
        let name = valid(self.name, OrganizationName::new)?;
        let description = self
            .description
            .map(|d| valid(d, OrganizationDescription::new))
            .transpose()?;
        Ok((name, description))
    }
}

impl Parse for GetOrganizationRequest {
    type Into = OrganizationId;

    fn parse(self) -> Result<OrganizationId, Status> {
        id(self.organization_id, "organization_id")
    }
}

impl Parse for UpdateOrganizationRequest {
    type Into = (
        OrganizationId,
        Option<OrganizationName>,
        Option<Option<OrganizationDescription>>,
    );

    // A description that is set but empty clears it: the wire has no way to send `Some(None)`.
    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        let name = self
            .name
            .map(|n| valid(n, OrganizationName::new))
            .transpose()?;
        let description = self
            .description
            .map(|d| valid(d, OrganizationDescription::new).map(Some))
            .transpose()?;
        Ok((organization_id, name, description))
    }
}

impl Parse for SetOrganizationActiveRequest {
    type Into = (OrganizationId, bool);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        Ok((organization_id, self.is_active))
    }
}

impl Parse for DeleteOrganizationRequest {
    type Into = OrganizationId;

    fn parse(self) -> Result<OrganizationId, Status> {
        id(self.organization_id, "organization_id")
    }
}

impl Parse for ListOrganizationsRequest {
    type Into = Option<PaginationParams>;

    fn parse(self) -> Result<Option<PaginationParams>, Status> {
        Ok(proto_to_domain_pagination(self.pagination))
    }
}

impl Parse for ListOrganizationMembersRequest {
    type Into = (OrganizationId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        Ok((organization_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListUserOrganizationsRequest {
    type Into = (UserId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let user_id = id(self.user_id, "user_id")?;
        Ok((user_id, proto_to_domain_pagination(self.pagination)))
    }
}

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
    fn a_create_request_becomes_the_validated_inputs() {
        let (name, description) = CreateOrganizationRequest {
            name: "  acme ".into(),
            description: Some("widgets".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(name.as_str(), "acme");
        assert_eq!(
            description.as_ref().map(OrganizationDescription::as_str),
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

    fn update(description: Option<&str>) -> Option<Option<OrganizationDescription>> {
        let (_, name, description) = UpdateOrganizationRequest {
            organization_id: wrap("org-1"),
            name: None,
            description: description.map(str::to_owned),
        }
        .parse()
        .unwrap();
        assert!(name.is_none());
        description
    }

    #[test]
    fn an_update_without_a_description_leaves_it() {
        assert!(update(None).is_none());
    }

    #[test]
    fn an_update_with_an_empty_description_carries_it_as_a_change() {
        let description = update(Some(""));

        assert_eq!(
            description
                .as_ref()
                .map(|d| d.as_ref().map(OrganizationDescription::as_str)),
            Some(Some(""))
        );
    }

    #[test]
    fn an_update_with_a_description_set_carries_the_value() {
        let description = update(Some("widgets"));

        assert_eq!(
            description
                .as_ref()
                .map(|d| d.as_ref().map(OrganizationDescription::as_str)),
            Some(Some("widgets"))
        );
    }
}
