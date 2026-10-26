//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::application::pagination::{PaginatedResult, PaginationParams};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use crate::grpc::mappers::{domain_to_proto_metadata, proto_to_domain_pagination};
use scylla_domain::domain::ids::{OrganizationId, ProjectId, UserId};
use scylla_domain::domain::project::{Project, ProjectDescription, ProjectName};
use scylla_domain::domain::user::User;
use scylla_proto::project::v1::{
    CreateProjectRequest, DeleteProjectRequest, GetProjectRequest, ListOrganizationProjectsRequest,
    ListOrganizationProjectsResponse, ListProjectMembersRequest, ListProjectMembersResponse,
    ListProjectsRequest, ListProjectsResponse, ListUserProjectsRequest, ListUserProjectsResponse,
    Project as ProtoProject, ProjectMember, SetProjectActiveRequest, UpdateProjectRequest,
};
use tonic::Status;

pub fn project_to_proto(project: &Project) -> ProtoProject {
    ProtoProject {
        project_id: wrap(project.id().to_string()),
        name: project.name().to_string(),
        description: project
            .description()
            .map(|d| d.as_str().to_string())
            .unwrap_or_default(),
        organization_id: wrap(project.organization_id().to_string()),
        is_active: project.is_active(),
        created_at: ts(project.created_at()),
        updated_at: ts(project.updated_at()),
    }
}

impl Parse for CreateProjectRequest {
    type Into = (OrganizationId, ProjectName, Option<ProjectDescription>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        let name = valid(self.name, ProjectName::new)?;
        let description = self
            .description
            .map(|d| valid(d, ProjectDescription::new))
            .transpose()?;
        Ok((organization_id, name, description))
    }
}

impl Parse for GetProjectRequest {
    type Into = ProjectId;

    fn parse(self) -> Result<ProjectId, Status> {
        id(self.project_id, "project_id")
    }
}

impl Parse for UpdateProjectRequest {
    type Into = (
        ProjectId,
        Option<ProjectName>,
        Option<Option<ProjectDescription>>,
    );

    // A description that is set but empty clears it: the wire has no way to send `Some(None)`.
    fn parse(self) -> Result<Self::Into, Status> {
        let project_id = id(self.project_id, "project_id")?;
        let name = self.name.map(|n| valid(n, ProjectName::new)).transpose()?;
        let description = self
            .description
            .map(|d| valid(d, ProjectDescription::new).map(Some))
            .transpose()?;
        Ok((project_id, name, description))
    }
}

impl Parse for SetProjectActiveRequest {
    type Into = (ProjectId, bool);

    fn parse(self) -> Result<Self::Into, Status> {
        Ok((id(self.project_id, "project_id")?, self.is_active))
    }
}

impl Parse for DeleteProjectRequest {
    type Into = ProjectId;

    fn parse(self) -> Result<ProjectId, Status> {
        id(self.project_id, "project_id")
    }
}

impl Parse for ListProjectsRequest {
    type Into = Option<PaginationParams>;

    fn parse(self) -> Result<Self::Into, Status> {
        Ok(proto_to_domain_pagination(self.pagination))
    }
}

impl Parse for ListOrganizationProjectsRequest {
    type Into = (OrganizationId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        Ok((organization_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListProjectMembersRequest {
    type Into = (ProjectId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let project_id = id(self.project_id, "project_id")?;
        Ok((project_id, proto_to_domain_pagination(self.pagination)))
    }
}

impl Parse for ListUserProjectsRequest {
    type Into = (UserId, Option<PaginationParams>);

    fn parse(self) -> Result<Self::Into, Status> {
        let user_id = id(self.user_id, "user_id")?;
        Ok((user_id, proto_to_domain_pagination(self.pagination)))
    }
}

page_response!(
    Project => projects: project_to_proto;
    ListProjectsResponse,
    ListOrganizationProjectsResponse,
    ListUserProjectsResponse,
);

impl From<PaginatedResult<User>> for ListProjectMembersResponse {
    fn from(page: PaginatedResult<User>) -> Self {
        let (users, metadata) = page.into_parts();
        Self {
            members: users
                .iter()
                .map(|user| ProjectMember {
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
        let (organization_id, name, description) = CreateProjectRequest {
            organization_id: wrap("org-1"),
            name: "  rocket ".into(),
            description: Some("to the moon".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(organization_id.as_str(), "org-1");
        assert_eq!(name.as_str(), "rocket");
        assert_eq!(
            description.as_ref().map(ProjectDescription::as_str),
            Some("to the moon")
        );
    }

    #[test]
    fn a_missing_id_is_an_invalid_argument() {
        let err = CreateProjectRequest {
            organization_id: None::<common::OrganizationId>,
            name: "rocket".into(),
            description: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing organization_id");
    }

    #[test]
    fn a_domain_validation_failure_is_an_invalid_argument() {
        let err = CreateProjectRequest {
            organization_id: wrap("org-1"),
            name: "   ".into(),
            description: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_update_with_a_description_set_carries_it_as_a_change() {
        let (_, name, description) = UpdateProjectRequest {
            project_id: wrap("p-1"),
            name: None,
            description: Some(String::new()),
        }
        .parse()
        .unwrap();

        assert!(name.is_none());
        assert!(matches!(description, Some(Some(_))));
    }
}
