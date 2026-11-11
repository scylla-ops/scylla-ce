//! The life of the stream of an agent: the hello, the renewals of its token, the commands of the
//! server, and its end at the expiry of the token or at a stop of the server.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::identity::agents::AgentCaller;
use scylla_domain::identity::apps::{App, AppId};
use scylla_domain::projects::jobs::RunScope;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::agents::v1 as pb;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::{interval, sleep_until, timeout, Instant};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Status, Streaming};

use super::registry::Command;
use super::{work, Registry};
use crate::api::parse;
use crate::features::identity::apps;
use crate::features::integrations::IntegrationContext;
use crate::infra::secret::Sealer;
use crate::Error;

/// The longest wait for the hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// The time between two looks for pending jobs with no wake.
const TICK: Duration = Duration::from_secs(30);

/// The channel of the messages of the server to the agent.
pub(super) type Outbox = mpsc::Sender<Result<pb::OpenStreamResponse, Status>>;

/// What the stream of one agent uses and knows.
pub(super) struct Stream {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate, for the renewals and the role of the agent.
    pub(super) gate: Arc<dyn Gate>,
    /// The open streams of the server.
    pub(super) registry: Arc<Registry>,
    /// The key of the secrets, for the dispatch.
    pub(super) sealer: Option<Arc<Sealer>>,
    /// What the Git integration shares, for the tokens of the checkouts.
    pub(super) vcs: IntegrationContext,
    /// The agent.
    pub(super) app: AppId,
    /// The principal of the current token of the stream.
    pub(super) principal: Principal,
    /// The workspace of the agent, known after the hello.
    pub(super) workspace: Option<WorkspaceId>,
    /// The largest number of jobs at the same time, from the hello.
    pub(super) capacity: usize,
    /// `true` if the agent has a container runtime.
    pub(super) containers: bool,
    /// `true` if the agent gets the code of a job before its first node.
    pub(super) checkout: bool,
    /// The jobs that the agent may take, from its roles `runner`.
    pub(super) scope: RunScope,
}

impl Stream {
    /// Runs the stream until its end, then forgets it. Each end sends its status to the agent.
    pub(super) async fn run(mut self, mut inbound: Streaming<pb::OpenStreamRequest>, out: Outbox) {
        // Wait for the hello, then record the stream.
        let ending = match self.hello(&mut inbound, &out).await {
            Ok(workspace) => {
                let ticket = self.registry.open(self.app, workspace, self.checkout);
                tracing::info!(event = "agent.connected", app = %self.app);
                let ending = self.serve(&mut inbound, &out, ticket.commands).await;
                self.registry.close(self.app, ticket.number);
                if let Err(error) = apps::touch(&self.pool, self.app).await {
                    tracing::warn!(%error, app = %self.app, "the last seen time did not change");
                }
                tracing::info!(event = "agent.disconnected", app = %self.app);
                ending
            }
            Err(status) => Some(status),
        };

        // Send the status of the end, if the agent did not close the stream itself.
        if let Some(status) = ending {
            let _ = out.send(Err(status)).await;
        }
    }

    /// Reads the first message: it must be a hello. Stores the host, checks the jobs that the
    /// agent still runs, and learns the place, the features and the role of the agent. Returns
    /// the workspace of the agent.
    async fn hello(
        &mut self,
        inbound: &mut Streaming<pb::OpenStreamRequest>,
        out: &Outbox,
    ) -> Result<WorkspaceId, Status> {
        // Read the hello.
        let first = timeout(HELLO_TIMEOUT, inbound.message())
            .await
            .map_err(|_| Status::deadline_exceeded("no hello in 30 seconds"))??;
        let Some(pb::OpenStreamRequest {
            message: Some(Up::Hello(hello)),
        }) = first
        else {
            return Err(Status::invalid_argument(
                "the first message must be a hello",
            ));
        };
        let host: scylla_domain::identity::apps::Host = parse::required(hello.host, "hello.host")?;
        let running = hello
            .running_jobs
            .into_iter()
            .map(|id| parse::field(id, "hello.running_jobs"))
            .collect::<Result<Vec<_>, _>>()?;

        // Store the host, then learn the workspace and the role.
        apps::hello(&self.pool, self.app, &host).await?;
        let app = apps::get(&self.pool, self.app)
            .await?
            .ok_or_else(|| Status::unauthenticated("the app was deleted"))?;
        self.workspace = Some(app.workspace_id);
        self.capacity = host.capacity as usize;
        self.containers = host.container_runtime.is_some();
        self.checkout = host.can_check_out();
        self.scope = self.run_scope().await;

        // Welcome the agent, then stop the jobs that it must not run.
        work::send(out, work::welcome(self.principal.expires_at())).await;
        for job in work::reconcile(self, &running).await? {
            work::send(out, work::cancel(job)).await;
        }
        Ok(app.workspace_id)
    }

    /// Answers the messages and the commands until the stream ends. Returns the status to send,
    /// or `None` when the agent closed the stream.
    async fn serve(
        &mut self,
        inbound: &mut Streaming<pb::OpenStreamRequest>,
        out: &Outbox,
        mut commands: mpsc::Receiver<Command>,
    ) -> Option<Status> {
        let mut ticks = interval(TICK);
        loop {
            let expiry = deadline(self.principal.expires_at());
            let step = tokio::select! {
                message = inbound.message() => match message {
                    Ok(Some(request)) => self.answer(request, out).await,
                    Ok(None) | Err(_) => return None,
                },
                command = commands.recv() => match command {
                    Some(Command::Stop(reason)) => Err(Status::aborted(reason)),
                    None => Err(Status::aborted("the server closed the stream")),
                    Some(Command::Cancel(job)) => Ok(work::send(out, work::cancel(job)).await),
                    Some(Command::Wake) => work::fill(self, out).await,
                },
                _ = ticks.tick() => work::fill(self, out).await,
                () = sleep_until(expiry) => {
                    Err(Status::unauthenticated("the token of the stream expired"))
                }
            };
            match step {
                Ok(true) => {}
                Ok(false) => return None,
                Err(status) => return Some(status),
            }
        }
    }

    /// Answers one message after the hello. Returns `false` when the agent is gone.
    async fn answer(
        &mut self,
        request: pb::OpenStreamRequest,
        out: &Outbox,
    ) -> Result<bool, Status> {
        match request.message {
            Some(Up::Renew(renew)) => {
                self.principal = self.renew(&renew.token).await?;
                self.scope = self.run_scope().await;
                apps::touch(&self.pool, self.app).await?;
                let renewed = work::renewed(self.principal.expires_at());
                if !work::send(out, renewed).await {
                    return Ok(false);
                }
                work::fill(self, out).await
            }
            Some(Up::Event(event)) => work::event(self, event, out).await,
            Some(Up::Logs(batch)) => work::logs(self, batch, out).await,
            Some(Up::Hello(_)) | None => Err(Status::invalid_argument(
                "after the hello, send no other hello",
            )),
        }
    }

    /// Checks a new token as the start of the stream did: the same agent, and the route
    /// permission.
    async fn renew(&self, token: &str) -> Result<Principal, Error> {
        let mut metadata = MetadataMap::new();
        let value = MetadataValue::try_from(format!("Bearer {token}"))
            .map_err(|_| Error::invalid("renew.token", "must be a token"))?;
        metadata.insert("authorization", value);
        let operation = self.principal.operation();
        let principal = self.gate.authenticate(&metadata, operation).await?;
        if principal.actor.id != App::actor(self.app) {
            let reason = "the token is of another actor".to_owned();
            return Err(AccessError::Unauthenticated(reason).into());
        }
        let check = AgentCaller::OPEN_STREAM.on(principal.actor.id);
        self.gate.authorize(&principal, check).await?;
        Ok(principal)
    }

    /// Returns the jobs that the agent may take, from its roles `runner`. A failure of the
    /// permission system gives no job until the next renewal.
    async fn run_scope(&self) -> RunScope {
        let Some(workspace) = self.workspace else {
            return RunScope::Nothing;
        };
        match work::run_scope(self.gate.as_ref(), &self.principal, workspace).await {
            Ok(scope) => scope,
            Err(error) => {
                tracing::warn!(%error, app = %self.app, "the role of the agent is unknown");
                RunScope::Nothing
            }
        }
    }
}

/// Returns the instant of an expiry, or now if it is in the past.
fn deadline(expires_at: OffsetDateTime) -> Instant {
    let left = expires_at - OffsetDateTime::now_utc();
    Instant::now() + left.try_into().unwrap_or(Duration::ZERO)
}
