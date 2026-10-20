use std::convert::Infallible;
use tonic::body::Body;
use tonic::server::NamedService;
use tonic::service::RoutesBuilder;
use tower::{Layer, Service};

pub(crate) type Auth = tonic_async_interceptor::AsyncInterceptorLayer<
    scylla_core::grpc::auth_interceptor::AuthInterceptor,
>;

pub(crate) type GrpcService = Box<dyn FnOnce(&mut RoutesBuilder, &Auth) + Send>;

#[derive(Default)]
pub struct Surface {
    pub(crate) grpc: Vec<GrpcService>,
    pub(crate) http: Vec<axum::Router>,
    pub(crate) descriptors: Vec<&'static [u8]>,
}

impl Surface {
    pub fn grpc_service<S>(&mut self, service: S) -> &mut Self
    where
        S: Service<http::Request<Body>, Error = Infallible>
            + NamedService
            + Clone
            + Send
            + Sync
            + 'static,
        S::Response: axum::response::IntoResponse,
        S::Future: Send + 'static,
    {
        self.grpc.push(Box::new(move |routes, _auth| {
            routes.add_service(service);
        }));
        self
    }

    pub fn authenticated_grpc_service<S>(&mut self, service: S) -> &mut Self
    where
        S: Service<http::Request<Body>, Response = http::Response<Body>, Error = Infallible>
            + NamedService
            + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        self.grpc.push(Box::new(move |routes, auth| {
            routes.add_service(auth.layer(service));
        }));
        self
    }

    pub fn file_descriptor_set(&mut self, set: &'static [u8]) -> &mut Self {
        self.descriptors.push(set);
        self
    }

    /// The router must not define a fallback: the web UI owns it.
    pub fn http_routes(&mut self, router: axum::Router) -> &mut Self {
        self.http.push(router);
        self
    }
}
