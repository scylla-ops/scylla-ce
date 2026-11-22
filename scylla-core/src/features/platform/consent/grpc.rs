//! The gRPC handler of `ConsentService`: anyone records a choice of a browser, under the rate
//! limit of its IP address, and a token of a user names the user in the record.

use std::sync::Arc;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::KIND;
use scylla_domain::platform::consent::ConsentCaller;
use scylla_extension::{AccessError, Gate};
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::consent_service_server::{ConsentService, ConsentServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::rates::Rates;
use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::identity::sessions::Client;
use crate::Error;

/// The header of the token.
const TOKEN: &str = "authorization";

/// Makes the service. `trust_forwarded` reads the IP address of the client from
/// `X-Forwarded-For`, as the sessions do.
pub(crate) fn server(
    pool: &PgPool,
    gate: &Arc<dyn Gate>,
    trust_forwarded: bool,
) -> ConsentServiceServer<ConsentGrpc> {
    ConsentServiceServer::new(ConsentGrpc {
        pool: pool.clone(),
        gate: Arc::clone(gate),
        trust_forwarded,
        rates: Rates::default(),
    })
}

/// The handler.
pub(crate) struct ConsentGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// `true` when the IP address of the client comes from `X-Forwarded-For`.
    trust_forwarded: bool,
    /// The records of each IP address in the current window.
    rates: Rates,
}

#[tonic::async_trait]
impl ConsentService for ConsentGrpc {
    async fn record_consent(
        &self,
        request: Request<pb::RecordConsentRequest>,
    ) -> Result<Response<pb::RecordConsentResponse>, Status> {
        // Refuse an IP address over the limit, then find the user of a token.
        let client = Client::of_grpc(&request, self.trust_forwarded);
        if !self.rates.allow(client.ip) {
            let message = "too many records of the consent from this address, try again later";
            return Err(Status::resource_exhausted(message));
        }
        let user = self.user(&request).await?;

        // Check the record and write it.
        let record = convert::record(request.into_inner())?;
        let (consent, at) = service::record(&self.pool, &record, user).await?;
        Ok(Response::new(pb::RecordConsentResponse {
            consent_id: Some(consent.to_proto()),
            recorded_at: Some(parse::timestamp(at)),
        }))
    }
}

impl ConsentGrpc {
    /// Returns the user of the token, or `None` for a request with no token. The token of an
    /// actor that is not a user is refused.
    async fn user<T: Sync>(&self, request: &Request<T>) -> Result<Option<ActorId>, Error> {
        if !request.metadata().contains_key(TOKEN) {
            return Ok(None);
        }
        let caller = self.gate.caller(request).await?;
        let check = ConsentCaller::RECORD_CONSENT.on(caller.actor.id);
        self.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != KIND {
            let reason = "only a user has an account".to_owned();
            let object = check.object();
            return Err(AccessError::refused(&caller, check.permission(), object, reason).into());
        }
        Ok(Some(caller.actor.id))
    }
}
