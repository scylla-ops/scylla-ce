//! The `TestApp` helpers of `AppService`, of the App RPCs of `TokenService`, and of the stream of
//! `AgentService`.

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey};
use prost::Message;
use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::agents::v1::agent_service_client::AgentServiceClient;
use scylla_proto::agents::v1::{OpenStreamRequest, OpenStreamResponse};
use scylla_proto::apps::v1::app_service_client::AppServiceClient;
use scylla_proto::apps::v1::{
    AddAppKeyRequest, App, AppAssertion, AppKey, CreateAppRequest, CreateAppResponse,
    CreateJoinCodeRequest, DeleteAppRequest, DisableAppRequest, EnableAppRequest, GetAppRequest,
    GetAppResponse, ListAppsRequest, ListAppsResponse, RevokeAppKeyRequest,
};
use scylla_proto::auth::v1::{ExchangeAppAssertionRequest, JoinAppRequest};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::{Status, Streaming};

use crate::rpc::page;
use crate::{id, text, with_token, TestApp};

/// The machine of an App in a test: the ids and the private key of one of its keys.
#[derive(Debug)]
pub struct TestKey {
    /// The id of the App.
    pub app: String,
    /// The id of the key.
    pub key_id: String,
    /// The private key.
    pub key: SigningKey,
}

impl TestKey {
    /// Returns the actor id of the App.
    pub fn actor(&self) -> ActorId {
        ActorId::parse(&self.app).expect("app id")
    }

    /// Signs an assertion that expires after `seconds`, with a new nonce.
    pub fn assertion(&self, seconds: i64) -> ExchangeAppAssertionRequest {
        self.assertion_with(seconds, &ActorId::generate().to_string())
    }

    /// Signs an assertion that expires after `seconds`, with a nonce.
    pub fn assertion_with(&self, seconds: i64, nonce: &str) -> ExchangeAppAssertionRequest {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after 1970")
            .as_secs() as i64;
        let claims = AppAssertion {
            app_id: Some(id(self.app.clone())),
            key_id: Some(id(self.key_id.clone())),
            audience: "scylla".to_owned(),
            expires_at: Some(Timestamp {
                seconds: now + seconds,
                nanos: 0,
            }),
            nonce: nonce.to_owned(),
        };
        let bytes = claims.encode_to_vec();
        ExchangeAppAssertionRequest {
            signature: self.key.sign(&bytes).to_bytes().to_vec(),
            assertion: bytes,
        }
    }
}

/// Makes a new Ed25519 key pair.
pub fn new_key() -> SigningKey {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("random bytes");
    SigningKey::from_bytes(&bytes)
}

impl TestApp {
    /// Returns a client of `AppService`.
    fn apps(&self) -> AppServiceClient<Channel> {
        AppServiceClient::new(self.channel.clone())
    }

    /// Calls `CreateApp` as `actor` in a workspace.
    pub async fn create_app(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        runner: bool,
    ) -> Result<CreateAppResponse, Status> {
        let request = CreateAppRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: "build machine".to_owned(),
            runner,
        };
        let request = self.authed(actor, request).await;
        Ok(self.apps().create_app(request).await?.into_inner())
    }

    /// Calls `GetApp` as `actor`.
    pub async fn get_app(&self, actor: ActorId, app: &str) -> Result<GetAppResponse, Status> {
        let request = GetAppRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.apps().get_app(request).await?.into_inner())
    }

    /// Calls `ListApps` as `actor` for one page of a workspace.
    pub async fn list_apps(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        runner: Option<bool>,
        size: i32,
        token: &str,
    ) -> Result<ListAppsResponse, Status> {
        let request = ListAppsRequest {
            workspace_id: Some(id(workspace.to_string())),
            runner,
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.apps().list_apps(request).await?.into_inner())
    }

    /// Calls `DeleteApp` as `actor`.
    pub async fn delete_app(&self, actor: ActorId, app: &str) -> Result<(), Status> {
        let request = DeleteAppRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        self.apps().delete_app(request).await?;
        Ok(())
    }

    /// Calls `DisableApp` as `actor`, and returns the App.
    pub async fn disable_app(&self, actor: ActorId, app: &str) -> Result<App, Status> {
        let request = DisableAppRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        let response = self.apps().disable_app(request).await?.into_inner();
        Ok(response.app.expect("the app"))
    }

    /// Calls `EnableApp` as `actor`, and returns the App.
    pub async fn enable_app(&self, actor: ActorId, app: &str) -> Result<App, Status> {
        let request = EnableAppRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        let response = self.apps().enable_app(request).await?.into_inner();
        Ok(response.app.expect("the app"))
    }

    /// Calls `CreateJoinCode` as `actor`, and returns the code.
    pub async fn create_join_code(&self, actor: ActorId, app: &str) -> Result<String, Status> {
        let request = CreateJoinCodeRequest {
            app_id: Some(id(app)),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .apps()
            .create_join_code(request)
            .await?
            .into_inner()
            .join_code)
    }

    /// Calls `AddAppKey` as `actor` with the public key of `key`.
    pub async fn add_app_key(
        &self,
        actor: ActorId,
        app: &str,
        key: &SigningKey,
    ) -> Result<AppKey, Status> {
        let request = AddAppKeyRequest {
            app_id: Some(id(app)),
            public_key: key.verifying_key().to_bytes().to_vec(),
        };
        let request = self.authed(actor, request).await;
        let response = self.apps().add_app_key(request).await?.into_inner();
        Ok(response.key.expect("key in response"))
    }

    /// Calls `RevokeAppKey` as `actor`.
    pub async fn revoke_app_key(&self, actor: ActorId, app: &str, key: &str) -> Result<(), Status> {
        let request = RevokeAppKeyRequest {
            app_id: Some(id(app)),
            key_id: Some(id(key)),
        };
        let request = self.authed(actor, request).await;
        self.apps().revoke_app_key(request).await?;
        Ok(())
    }

    /// Calls `JoinApp` with a code and the public key of a new key pair.
    pub async fn join_app(&self, code: &str) -> Result<TestKey, Status> {
        let key = new_key();
        let request = JoinAppRequest {
            join_code: code.to_owned(),
            public_key: key.verifying_key().to_bytes().to_vec(),
        };
        let response = self.tokens().join_app(request).await?.into_inner();
        Ok(TestKey {
            app: text(&response.app_id).to_owned(),
            key_id: text(&response.key_id).to_owned(),
            key,
        })
    }

    /// Calls `ExchangeAppAssertion`, and returns the token.
    pub async fn exchange(&self, request: ExchangeAppAssertionRequest) -> Result<String, Status> {
        let response = self.tokens().exchange_app_assertion(request).await?;
        Ok(response.into_inner().token.expect("token").value)
    }

    /// Creates an App as `owner` in a workspace, joins it with a new key pair, and returns the
    /// machine of the App and a token of the App.
    pub async fn app_in(
        &self,
        owner: ActorId,
        workspace: WorkspaceId,
        runner: bool,
    ) -> (TestKey, String) {
        let created = self
            .create_app(owner, workspace, runner)
            .await
            .expect("app");
        let key = self.join_app(&created.join_code).await.expect("join");
        let token = self.exchange(key.assertion(60)).await.expect("token");
        (key, token)
    }

    /// Calls `OpenStream` with a token. Returns the sender of the agent messages and the
    /// stream of the server messages.
    pub async fn open_stream(
        &self,
        token: &str,
    ) -> Result<
        (
            mpsc::Sender<OpenStreamRequest>,
            Streaming<OpenStreamResponse>,
        ),
        Status,
    > {
        let (sender, messages) = mpsc::channel(8);
        let request = with_token(token, ReceiverStream::new(messages));
        let mut client = AgentServiceClient::new(self.channel.clone());
        let response = client.open_stream(request).await?;
        Ok((sender, response.into_inner()))
    }
}
