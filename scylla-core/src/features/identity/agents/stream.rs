//! The life of the stream of an agent: the hello, the renewals of its token, and its end at the
//! expiry of the token or at a stop of the server.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::identity::agents::AgentCaller;
use scylla_domain::identity::apps::{App, AppId};
use scylla_extension::{Gate, Principal};
use scylla_proto::agents::v1 as pb;
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio::time::{sleep_until, timeout, Instant};
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Status, Streaming};

use super::Registry;
use crate::api::parse;
use crate::features::identity::apps;
use crate::Error;

/// The longest wait for the hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);

/// The channel of the messages of the server to the agent.
pub(super) type Outbox = mpsc::Sender<Result<pb::OpenStreamResponse, Status>>;

/// What the stream of one agent uses.
pub(super) struct Stream {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate, for the renewals.
    pub(super) gate: Arc<dyn Gate>,
    /// The open streams of the server.
    pub(super) registry: Arc<Registry>,
    /// The agent.
    pub(super) app: AppId,
    /// The principal of the current token of the stream.
    pub(super) principal: Principal,
}

impl Stream {
    /// Runs the stream until its end, then forgets it. Each end sends its status to the agent.
    pub(super) async fn run(mut self, mut inbound: Streaming<pb::OpenStreamRequest>, out: Outbox) {
        // Wait for the hello, then record the stream.
        let ending = match self.hello(&mut inbound).await {
            Ok(()) => {
                let ticket = self.registry.open(self.app);
                tracing::info!(event = "agent.connected", app = %self.app);
                let ending = self.serve(&mut inbound, &out, ticket.stopped).await;
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

    /// Reads the first message: it must be a hello. Stores the host of the agent.
    async fn hello(&self, inbound: &mut Streaming<pb::OpenStreamRequest>) -> Result<(), Status> {
        let first = timeout(HELLO_TIMEOUT, inbound.message())
            .await
            .map_err(|_| Status::deadline_exceeded("no hello in 30 seconds"))??;
        let Some(pb::OpenStreamRequest {
            message: Some(pb::open_stream_request::Message::Hello(hello)),
        }) = first
        else {
            return Err(Status::invalid_argument(
                "the first message must be a hello",
            ));
        };
        let host = parse::required(hello.host, "hello.host")?;
        apps::hello(&self.pool, self.app, &host).await?;
        Ok(())
    }

    /// Answers the messages of the agent until the stream ends. Returns the status to send, or
    /// `None` when the agent closed the stream.
    async fn serve(
        &mut self,
        inbound: &mut Streaming<pb::OpenStreamRequest>,
        out: &Outbox,
        mut stopped: tokio::sync::oneshot::Receiver<&'static str>,
    ) -> Option<Status> {
        // Welcome the agent with the expiry of its token.
        if send(out, welcome(self.principal.expires_at()))
            .await
            .is_err()
        {
            return None;
        }

        // Renew the token on each Renew, until the expiry, a stop, or the end of the agent.
        loop {
            let expiry = deadline(self.principal.expires_at());
            tokio::select! {
                message = inbound.message() => match message {
                    Ok(Some(request)) => match self.answer(request).await {
                        Ok(response) => {
                            if send(out, response).await.is_err() {
                                return None;
                            }
                        }
                        Err(status) => return Some(status),
                    },
                    Ok(None) | Err(_) => return None,
                },
                reason = &mut stopped => {
                    let reason = reason.unwrap_or("the server closed the stream");
                    return Some(Status::aborted(reason));
                }
                () = sleep_until(expiry) => {
                    return Some(Status::unauthenticated("the token of the stream expired"));
                }
            }
        }
    }

    /// Answers one message after the hello: a Renew.
    async fn answer(
        &mut self,
        request: pb::OpenStreamRequest,
    ) -> Result<pb::OpenStreamResponse, Status> {
        let Some(pb::open_stream_request::Message::Renew(renew)) = request.message else {
            return Err(Status::invalid_argument(
                "after the hello, send only renewals",
            ));
        };
        let principal = self.renew(&renew.token).await?;
        let expires_at = principal.expires_at();
        self.principal = principal;
        apps::touch(&self.pool, self.app).await?;
        Ok(pb::OpenStreamResponse {
            message: Some(pb::open_stream_response::Message::Renewed(pb::Renewed {
                expires_at: Some(parse::timestamp(expires_at)),
            })),
        })
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
            return Err(scylla_extension::AccessError::Unauthenticated(reason).into());
        }
        let check = AgentCaller::OPEN_STREAM.on(principal.actor.id);
        self.gate.authorize(&principal, check).await?;
        Ok(principal)
    }
}

/// Sends one message to the agent. Fails when the agent is gone.
async fn send(out: &Outbox, message: pb::OpenStreamResponse) -> Result<(), ()> {
    out.send(Ok(message)).await.map_err(|_| ())
}

/// Makes the answer to the hello.
fn welcome(expires_at: OffsetDateTime) -> pb::OpenStreamResponse {
    pb::OpenStreamResponse {
        message: Some(pb::open_stream_response::Message::Welcome(pb::Welcome {
            expires_at: Some(parse::timestamp(expires_at)),
        })),
    }
}

/// Returns the instant of an expiry, or now if it is in the past.
fn deadline(expires_at: OffsetDateTime) -> Instant {
    let left = expires_at - OffsetDateTime::now_utc();
    Instant::now() + left.try_into().unwrap_or(Duration::ZERO)
}
