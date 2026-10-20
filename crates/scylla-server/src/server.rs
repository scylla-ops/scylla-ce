use crate::feature::{Context, Feature};
use crate::startup::{init_services, run_server, shutdown_signal};
use crate::surface::Surface;
use anyhow::{Context as _, Result};
use scylla_core::application::PasswordResetSender;
use scylla_core::config::ControlPlaneConfig;
use scylla_core::infrastructure::LogPasswordResetSender;
use scylla_extension::{Extension, Hooks};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// An agent stream or a log tail never ends by itself, so the graceful drain waits only this
/// long. Shorter than the 10 seconds a container runtime gives before it kills the process.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub struct Server {
    config: ControlPlaneConfig,
    db: PgPool,
    hooks: Hooks,
    features: Vec<Box<dyn Feature>>,
    surface: Surface,
    password_reset_sender: Arc<dyn PasswordResetSender>,
}

impl Server {
    #[must_use]
    pub fn new(config: ControlPlaneConfig, db: PgPool) -> Self {
        Self {
            config,
            db,
            hooks: Hooks::new(),
            features: Vec::new(),
            surface: Surface::default(),
            password_reset_sender: Arc::new(LogPasswordResetSender),
        }
    }

    /// Replaces the default sender, which writes each reset link in the server log.
    #[must_use]
    pub fn password_reset_sender(mut self, sender: Arc<dyn PasswordResetSender>) -> Self {
        self.password_reset_sender = sender;
        self
    }

    /// Registers before `Feature::hooks`, which runs at `serve`; within one position, hooks run
    /// in registration order.
    #[must_use]
    pub fn extension<E: Extension>(mut self, extension: &Arc<E>) -> Self {
        extension.register(&mut self.hooks);
        self
    }

    #[must_use]
    pub fn feature(mut self, feature: impl Feature) -> Self {
        self.features.push(Box::new(feature));
        self
    }

    #[must_use]
    pub fn grpc_service<S>(mut self, service: S) -> Self
    where
        S: tower::Service<http::Request<tonic::body::Body>, Error = std::convert::Infallible>
            + tonic::server::NamedService
            + Clone
            + Send
            + Sync
            + 'static,
        S::Response: axum::response::IntoResponse,
        S::Future: Send + 'static,
    {
        self.surface.grpc_service(service);
        self
    }

    #[must_use]
    pub fn authenticated_grpc_service<S>(mut self, service: S) -> Self
    where
        S: tower::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = std::convert::Infallible,
            > + tonic::server::NamedService
            + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        self.surface.authenticated_grpc_service(service);
        self
    }

    #[must_use]
    pub fn file_descriptor_set(mut self, set: &'static [u8]) -> Self {
        self.surface.file_descriptor_set(set);
        self
    }

    #[must_use]
    pub fn http_routes(mut self, router: axum::Router) -> Self {
        self.surface.http_routes(router);
        self
    }

    pub async fn serve(self) -> Result<()> {
        let Self {
            config,
            db,
            mut hooks,
            features,
            mut surface,
            password_reset_sender,
        } = self;

        for feature in &features {
            feature.hooks(&mut hooks);
        }
        for feature in &features {
            feature
                .prepare(&db)
                .await
                .context("feature prepare failed")?;
        }

        let services = init_services(&config, db.clone(), Arc::new(hooks), password_reset_sender)
            .await
            .context("init_services failed")?;

        let ctx = Context {
            db: db.clone(),
            actions: services.actions.clone(),
            permissions: services.permission_checker.clone(),
            visibility: services.permission_checker.clone(),
            trust_forwarded_headers: config.server.trust_forwarded_headers,
        };
        for feature in features {
            feature.install(&ctx, &mut surface);
        }

        let token = CancellationToken::new();
        let signal_token = token.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            signal_token.cancel();
        });

        let server_token = token.clone();
        let server = run_server(&config, &services, surface, async move {
            server_token.cancelled().await;
        });
        let grace_token = token.clone();
        let grace = async move {
            grace_token.cancelled().await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        };
        let result = tokio::select! {
            result = server => result,
            () = grace => {
                warn!(
                    grace_secs = SHUTDOWN_GRACE.as_secs(),
                    "connections still open after the shutdown grace period; closing them"
                );
                Ok(())
            }
        };

        token.cancel();

        info!("closing database pool");
        scylla_db::close_db(&db).await;

        result.context("run_server failed")?;
        Ok(())
    }
}
