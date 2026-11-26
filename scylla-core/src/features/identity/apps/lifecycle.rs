//! The handlers of `AppService` for the life of an App: its delete, its disable, its enable, and
//! the labels and the icon of the admins.

use scylla_domain::identity::agents::AgentLabels;
use scylla_domain::identity::apps::{App, AppId};
use scylla_domain::platform::images::IconChange;
use scylla_proto::apps::v1 as pb;
use tonic::Request;

use super::grpc::AppsGrpc;
use super::{icon_service, labels_service, service};
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
            app: self.messages(vec![app]).await?.pop(),
        })
    }

    /// Handles `SetAppLabels`: the labels of the admins replace the labels of the App.
    pub(super) async fn set_labels(
        &self,
        request: Request<pb::SetAppLabelsRequest>,
    ) -> Result<pb::SetAppLabelsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let labels = AgentLabels::parse(request.labels).map_err(|(index, error)| {
            let field = index.map_or("labels".to_owned(), |index| format!("labels[{index}]"));
            Error::invalid(field, error.to_string())
        })?;

        // Check the route permission, then replace the labels.
        self.authorize(&caller, App::SET_APP_LABELS, id).await?;
        let app = labels_service::set_labels(&self.pool, &self.registry, id, labels).await?;
        Ok(pb::SetAppLabelsResponse {
            app: self.messages(vec![app]).await?.pop(),
        })
    }

    /// Handles `SetAppIcon`: the icon of the admins replaces the icon of the App.
    pub(super) async fn set_icon(
        &self,
        request: Request<pb::SetAppIconRequest>,
    ) -> Result<pb::SetAppIconResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let change = match request.icon {
            Some(icon) => parse::field(icon, "icon")?,
            None => IconChange::Default,
        };

        // Check the route permission, then change the icon.
        self.authorize(&caller, App::SET_APP_ICON, id).await?;
        let app = icon_service::set_icon(&self.pool, caller.actor.id, id, change).await?;
        Ok(pb::SetAppIconResponse {
            app: self.messages(vec![app]).await?.pop(),
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
