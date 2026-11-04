//! The handlers of `AppService` for the life of an App: its delete, its disable and its enable.

use scylla_domain::identity::apps::{App, AppId};
use scylla_proto::apps::v1 as pb;
use tonic::Request;

use super::grpc::AppsGrpc;
use super::{convert, service};
use crate::api::parse;
use crate::Error;

impl AppsGrpc {
    /// Handles `DeleteApp`: the App goes with its keys, its roles and its stream.
    pub(super) async fn delete(
        &self,
        request: Request<pb::DeleteAppRequest>,
    ) -> Result<pb::DeleteAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::DELETE_APP, id).await?;
        let gate = self.gate.as_ref();
        service::delete(&self.pool, gate, &self.registry, id).await?;
        Ok(pb::DeleteAppResponse {})
    }

    /// Handles `DisableApp`: its tokens, its stream and its jobs stop.
    pub(super) async fn disable(
        &self,
        request: Request<pb::DisableAppRequest>,
    ) -> Result<pb::DisableAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::DISABLE_APP, id).await?;
        let app = service::disable(&self.pool, &self.registry, id).await?;
        Ok(pb::DisableAppResponse {
            app: Some(convert::app(app, false, 0)),
        })
    }

    /// Handles `EnableApp`: the App gets tokens again.
    pub(super) async fn enable(
        &self,
        request: Request<pb::EnableAppRequest>,
    ) -> Result<pb::EnableAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::ENABLE_APP, id).await?;
        let app = service::enable(&self.pool, id).await?;
        Ok(pb::EnableAppResponse {
            app: self.messages(vec![app]).await?.pop(),
        })
    }
}
