use scylla_core::app::Edition;
use scylla_core::features::users;
use scylla_domain::users::UserId;
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use scylla_proto::projects::v1::project_service_client::ProjectServiceClient;
use scylla_proto::projects::v1::{
    CreateProjectRequest, DeleteProjectRequest, ListProjectsRequest, ListProjectsResponse, Project,
};
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::transport::{Channel, Endpoint};
use tonic::Status;

pub struct TestApp {
    pub pool: PgPool,
    pub channel: Channel,
    pub projects: ProjectServiceClient<Channel>,
    pub capabilities: CapabilityServiceClient<Channel>,
}

impl TestApp {
    pub async fn spawn(pool: PgPool, edition: Edition) -> Self {
        scylla_core::db::migrate(&pool, edition.migrations)
            .await
            .expect("migrations apply");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(scylla_core::server::serve(listener, pool.clone(), edition));
        let channel = Endpoint::from_shared(format!("http://{addr}"))
            .expect("endpoint")
            .connect()
            .await
            .expect("connect");
        Self {
            pool,
            projects: ProjectServiceClient::new(channel.clone()),
            capabilities: CapabilityServiceClient::new(channel.clone()),
            channel,
        }
    }

    pub async fn user(&self, name: &str) -> UserId {
        let mut tx = self.pool.begin().await.expect("begin");
        let id = users::insert(&mut tx, name).await.expect("insert user");
        tx.commit().await.expect("commit");
        id
    }

    pub async fn create_project(&self, owner: UserId, name: &str) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            owner_id: owner.0.to_string(),
            name: name.to_owned(),
        };
        let response = self.projects.clone().create_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    pub async fn list_projects(
        &self,
        owner: UserId,
        page_size: i32,
        page_token: &str,
    ) -> Result<ListProjectsResponse, Status> {
        let request = ListProjectsRequest {
            owner_id: owner.0.to_string(),
            page_size,
            page_token: page_token.to_owned(),
        };
        Ok(self
            .projects
            .clone()
            .list_projects(request)
            .await?
            .into_inner())
    }

    pub async fn delete_project(&self, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: project_id.to_owned(),
        };
        self.projects.clone().delete_project(request).await?;
        Ok(())
    }

    pub async fn get_capabilities(&self) -> Result<GetCapabilitiesResponse, Status> {
        Ok(self
            .capabilities
            .clone()
            .get_capabilities(GetCapabilitiesRequest {})
            .await?
            .into_inner())
    }

    pub async fn project_count(&self, owner: UserId) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM projects WHERE owner_id = $1"#,
            owner.0,
        )
        .fetch_one(&self.pool)
        .await
        .expect("count projects")
    }
}
