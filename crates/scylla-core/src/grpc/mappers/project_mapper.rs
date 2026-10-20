//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::pagination::PaginatedResult;
use crate::application::project::{
    CreateProject, DeleteProject, GetProject, ListOrganizationProjects, ListProjectMembers,
    ListProjects, ListUserProjects, SetProjectActive, UpdateProject,
};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use crate::grpc::mappers::domain_to_proto_metadata;
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
    type Into = CreateProject;

    fn parse(self) -> Result<CreateProject, Status> {
        Ok(CreateProject {
            organization_id: id(self.organization_id, "organization_id")?,
            name: valid(self.name, ProjectName::new)?,
            description: self
                .description
                .map(|d| valid(d, ProjectDescription::new))
                .transpose()?,
        })
    }
}

parse!(GetProjectRequest => GetProject { id: id(project_id) });

impl Parse for UpdateProjectRequest {
    type Into = UpdateProject;

    // A description that is set but empty clears it: the wire has no way to send `Some(None)`.
    fn parse(self) -> Result<UpdateProject, Status> {
        Ok(UpdateProject {
            id: id(self.project_id, "project_id")?,
            name: self.name.map(|n| valid(n, ProjectName::new)).transpose()?,
            description: self
                .description
                .map(|d| valid(d, ProjectDescription::new).map(Some))
                .transpose()?,
        })
    }
}

parse!(SetProjectActiveRequest => SetProjectActive {
    id: id(project_id),
    is_active: copy,
});

parse!(DeleteProjectRequest => DeleteProject { id: id(project_id) });
parse!(ListProjectsRequest => ListProjects { pagination: page });

parse!(ListOrganizationProjectsRequest => ListOrganizationProjects {
    organization_id: id(organization_id),
    pagination: page,
});

parse!(ListProjectMembersRequest => ListProjectMembers {
    project_id: id(project_id),
    pagination: page,
});

parse!(ListUserProjectsRequest => ListUserProjects {
    user_id: id(user_id),
    pagination: page,
});

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
    fn a_create_request_becomes_a_command_with_validated_fields() {
        let command = CreateProjectRequest {
            organization_id: wrap("org-1"),
            name: "  rocket ".into(),
            description: Some("to the moon".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(command.organization_id.as_str(), "org-1");
        assert_eq!(command.name.as_str(), "rocket");
        assert_eq!(
            command.description.as_ref().map(ProjectDescription::as_str),
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
        let command = UpdateProjectRequest {
            project_id: wrap("p-1"),
            name: None,
            description: Some(String::new()),
        }
        .parse()
        .unwrap();

        assert!(command.name.is_none());
        assert!(matches!(command.description, Some(Some(_))));
    }
}
