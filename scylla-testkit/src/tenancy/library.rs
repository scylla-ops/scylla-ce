//! The `TestApp` helpers of `FragmentService`: the library of a workspace, and the nodes and
//! parameters of a fragment.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::library::v1::fragment_service_client::FragmentServiceClient;
use scylla_proto::library::v1::{
    CreateFragmentRequest, DeleteFragmentRequest, Fragment, FragmentParameter, GetFragmentRequest,
    ListFragmentsRequest, ListFragmentsResponse, ParameterKind, PublishFragmentVersionRequest,
    SetFragmentFavoriteRequest,
};
use scylla_proto::pipelines::v1::{
    env_var, CreatePipelineRequest, EnvVar, FragmentLink, FragmentLinkList, Node, NodeList,
    Pipeline, UpdatePipelineRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{exec_node, id, TestApp};

/// Makes a node that runs a program, with literal variables.
pub fn env_node(node_id: &str, needs: &[&str], env: &[(&str, &str)]) -> Node {
    let mut node = exec_node(node_id, needs);
    node.env = env
        .iter()
        .map(|(name, value)| EnvVar {
            name: (*name).to_owned(),
            value: Some(env_var::Value::Literal((*value).to_owned())),
        })
        .collect();
    node
}

/// Makes a literal parameter with a default.
pub fn literal_parameter(key: &str, default: &str) -> FragmentParameter {
    FragmentParameter {
        key: key.to_owned(),
        kind: ParameterKind::Literal.into(),
        default_value: default.to_owned(),
        description: String::new(),
    }
}

/// Makes the request of a fragment with one node `build` that has the variable `MODE`, and a
/// parameter for it.
pub fn fragment_request(workspace: &str, name: &str) -> CreateFragmentRequest {
    CreateFragmentRequest {
        workspace_id: Some(id(workspace)),
        name: name.to_owned(),
        description: "Builds the code".to_owned(),
        tags: vec!["build".to_owned()],
        nodes: vec![env_node("build", &[], &[("MODE", "release")])],
        parameters: vec![literal_parameter("MODE", "release")],
        origin: None,
    }
}

/// Makes a link of a block to a version of a fragment, with the nodes that the block holds.
pub fn fragment_link(block: &str, fragment: &str, version: u32, nodes: &[&str]) -> FragmentLink {
    FragmentLink {
        block_id: block.to_owned(),
        fragment_id: Some(id(fragment)),
        version,
        values: [("MODE".to_owned(), "debug".to_owned())].into(),
        node_ids: nodes.iter().map(|node| (*node).to_owned()).collect(),
        skipped_version: None,
    }
}

impl TestApp {
    /// Calls `CreatePipeline` as `actor` in a project, with nodes and fragment links.
    pub async fn create_linked_pipeline(
        &self,
        actor: ActorId,
        project: &str,
        nodes: Vec<Node>,
        fragment_links: Vec<FragmentLink>,
    ) -> Result<Pipeline, Status> {
        let request = CreatePipelineRequest {
            project_id: Some(id(project)),
            name: "linked".to_owned(),
            nodes,
            layout: None,
            checkout: None,
            fragment_links,
            agent: None,
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().create_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }

    /// Calls `UpdatePipeline` as `actor` with a version, and new nodes or new links or both.
    pub async fn update_pipeline_links(
        &self,
        actor: ActorId,
        pipeline: &str,
        version: i64,
        nodes: Option<Vec<Node>>,
        links: Option<Vec<FragmentLink>>,
    ) -> Result<Pipeline, Status> {
        let request = UpdatePipelineRequest {
            pipeline_id: Some(id(pipeline)),
            version,
            name: None,
            nodes: nodes.map(|nodes| NodeList { nodes }),
            layout: None,
            checkout: None,
            fragment_links: links.map(|links| FragmentLinkList { links }),
            agent: None,
        };
        let request = self.authed(actor, request).await;
        let response = self.pipelines().update_pipeline(request).await?;
        Ok(response
            .into_inner()
            .pipeline
            .expect("pipeline in response"))
    }

    /// A client of `FragmentService`.
    fn library(&self) -> FragmentServiceClient<Channel> {
        FragmentServiceClient::new(self.channel.clone())
    }

    /// Calls `CreateFragment` as `actor` with the request of `fragment_request`.
    pub async fn create_fragment(
        &self,
        actor: ActorId,
        workspace: &str,
        name: &str,
    ) -> Result<Fragment, Status> {
        self.create_fragment_with(actor, fragment_request(workspace, name))
            .await
    }

    /// Calls `CreateFragment` as `actor` with a request, so a test can send a bad value.
    pub async fn create_fragment_with(
        &self,
        actor: ActorId,
        request: CreateFragmentRequest,
    ) -> Result<Fragment, Status> {
        let request = self.authed(actor, request).await;
        let response = self.library().create_fragment(request).await?;
        Ok(response
            .into_inner()
            .fragment
            .expect("fragment in response"))
    }

    /// Calls `GetFragment` as `actor`.
    pub async fn get_fragment(&self, actor: ActorId, fragment: &str) -> Result<Fragment, Status> {
        let request = GetFragmentRequest {
            fragment_id: Some(id(fragment)),
        };
        let request = self.authed(actor, request).await;
        let response = self.library().get_fragment(request).await?;
        Ok(response
            .into_inner()
            .fragment
            .expect("fragment in response"))
    }

    /// Calls `ListFragments` as `actor` for one page.
    pub async fn list_fragments_page(
        &self,
        actor: ActorId,
        workspace: &str,
        size: i32,
        token: &str,
    ) -> Result<ListFragmentsResponse, Status> {
        let request = ListFragmentsRequest {
            workspace_id: Some(id(workspace)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.library().list_fragments(request).await?.into_inner())
    }

    /// Calls `ListFragments` as `actor`, and returns the fragments of the first page.
    pub async fn list_fragments(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Vec<Fragment>, Status> {
        let page = self.list_fragments_page(actor, workspace, 100, "").await?;
        Ok(page.fragments)
    }

    /// Calls `PublishFragmentVersion` as `actor` with nodes, parameters and changes.
    pub async fn publish_fragment_version(
        &self,
        actor: ActorId,
        fragment: &str,
        nodes: Vec<Node>,
        parameters: Vec<FragmentParameter>,
        changes: &[&str],
    ) -> Result<Fragment, Status> {
        let request = PublishFragmentVersionRequest {
            fragment_id: Some(id(fragment)),
            nodes,
            parameters,
            changes: changes.iter().map(|line| (*line).to_owned()).collect(),
        };
        let request = self.authed(actor, request).await;
        let response = self.library().publish_fragment_version(request).await?;
        Ok(response
            .into_inner()
            .fragment
            .expect("fragment in response"))
    }

    /// Calls `SetFragmentFavorite` as `actor`.
    pub async fn set_fragment_favorite(
        &self,
        actor: ActorId,
        fragment: &str,
        favorite: bool,
    ) -> Result<(), Status> {
        let request = SetFragmentFavoriteRequest {
            fragment_id: Some(id(fragment)),
            favorite,
        };
        let request = self.authed(actor, request).await;
        self.library().set_fragment_favorite(request).await?;
        Ok(())
    }

    /// Calls `DeleteFragment` as `actor`.
    pub async fn delete_fragment(&self, actor: ActorId, fragment: &str) -> Result<(), Status> {
        let request = DeleteFragmentRequest {
            fragment_id: Some(id(fragment)),
        };
        let request = self.authed(actor, request).await;
        self.library().delete_fragment(request).await?;
        Ok(())
    }
}
