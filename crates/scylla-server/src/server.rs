use crate::feature::{Context, Feature};
use crate::startup::{bootstrap, init_services, run_server, shutdown_signal};
use crate::surface::Surface;
use anyhow::{Context as _, Result};
use scylla_core::application::PasswordResetSender;
use scylla_core::config::ControlPlaneConfig;
use scylla_core::error::StartupError;
use scylla_core::infrastructure::LogPasswordResetSender;
use scylla_strata::{EditionHost, Host, ScyllaCeOnDb, Slice};
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
    features: Vec<Box<dyn Feature>>,
    surface: Surface,
    password_reset_sender: Arc<dyn PasswordResetSender>,
    strata: Box<dyn Host>,
}

impl Server {
    #[must_use]
    pub fn new(config: ControlPlaneConfig, db: PgPool) -> Self {
        Self {
            config,
            db,
            features: Vec::new(),
            surface: Surface::default(),
            password_reset_sender: Arc::new(LogPasswordResetSender),
            strata: Box::new(EditionHost::<ScyllaCeOnDb>::new(Slice::ALL)),
        }
    }

    /// The strata edition of the binary. It serves every slice. Without a call, the server serves
    /// the CE edition `ScyllaCeOnDb`.
    #[must_use]
    pub fn strata(mut self, host: impl Host) -> Self {
        self.strata = Box::new(host);
        self
    }

    /// Replaces the default sender, which writes each reset link in the server log.
    #[must_use]
    pub fn password_reset_sender(mut self, sender: Arc<dyn PasswordResetSender>) -> Self {
        self.password_reset_sender = sender;
        self
    }

    #[must_use]
    pub fn feature(mut self, feature: impl Feature) -> Self {
        self.features.push(Box::new(feature));
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
            features,
            mut surface,
            password_reset_sender,
            strata,
        } = self;

        warn_ignored_slices(&config);
        let missing = missing_slices(strata.ready());
        if !missing.is_empty() {
            anyhow::bail!(
                "the strata edition does not serve the slices {}: no other engine serves them",
                missing.join(",")
            );
        }
        scylla_strata::log_engines();

        for feature in &features {
            feature
                .prepare(&db)
                .await
                .context("feature prepare failed")?;
        }

        let mut services = init_services(&config, db.clone(), password_reset_sender)
            .await
            .context("init_services failed")?;

        let built = strata
            .build(&services.shared)
            .await
            .map_err(|e| StartupError::Strata(format!("{e:#}")))
            .context("strata edition build failed")?;
        bootstrap(&config, &*built)
            .await
            .context("bootstrap failed")?;

        let ctx = Context {
            db: db.clone(),
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
        let in_flight = strata::InFlight::waiting(strata::InFlight::DEFAULT_MAX);
        let workers = scylla_strata::Workers {
            wakes: services.wakes.take(),
            stop: token.clone(),
        };
        let server = run_server(
            &config,
            &services,
            surface,
            &*built,
            &in_flight,
            workers,
            async move {
                server_token.cancelled().await;
            },
        );
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

        if tokio::time::timeout(SHUTDOWN_GRACE, in_flight.idle())
            .await
            .is_err()
        {
            warn!(
                running = in_flight.running(),
                "strata calls still run after the shutdown grace period; closing the pool"
            );
        }

        info!("closing database pool");
        scylla_db::close_db(&db).await;

        result.context("run_server failed")?;
        Ok(())
    }
}

/// `[strata].slices` and `SCYLLA_STRATA_SLICES` chose the slices of strata before every slice ran
/// on it. The server accepts them, ignores them and writes one warning. True when it warns.
pub(crate) fn warn_ignored_slices(config: &ControlPlaneConfig) -> bool {
    if config.strata.slices.is_empty() {
        return false;
    }
    warn!("[strata].slices is ignored: every slice runs on strata");
    true
}

/// The keys of the slices of `Slice::ALL` that `ready` does not hold.
pub(crate) fn missing_slices(ready: &[Slice]) -> Vec<&'static str> {
    Slice::ALL
        .iter()
        .filter(|slice| !ready.contains(slice))
        .map(|slice| slice.key())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{missing_slices, warn_ignored_slices};
    use scylla_core::config::ControlPlaneConfig;
    use scylla_strata::Slice;

    const OLD_CONFIG: &str = r#"
[server]
address = "127.0.0.1:8080"

[strata]
slices = ["projects", "secrets", "not-a-slice"]
"#;

    #[test]
    fn an_old_config_with_strata_slices_starts_with_one_warning() {
        let path = std::env::temp_dir().join(format!("scylla-old-{}.toml", std::process::id()));
        std::fs::write(&path, OLD_CONFIG).unwrap();
        let config = ControlPlaneConfig::from_file(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(config.strata.slices.len(), 3);
        assert!(warn_ignored_slices(&config));
    }

    #[test]
    fn the_env_switch_is_also_ignored_with_one_warning() {
        let mut config = ControlPlaneConfig::default();
        assert!(!warn_ignored_slices(&config));

        config.override_strata_slices("projects,jobs");

        assert!(warn_ignored_slices(&config));
    }

    #[test]
    fn each_old_slice_value_is_ignored_and_an_empty_env_value_clears_the_file_list() {
        let with = |slices: &[&str]| ControlPlaneConfig {
            strata: scylla_core::config::StrataConfig {
                slices: slices.iter().map(|s| (*s).to_owned()).collect(),
            },
            ..ControlPlaneConfig::default()
        };
        assert!(warn_ignored_slices(&with(&["projects"])));
        assert!(warn_ignored_slices(&with(&["grnts"])));
        assert!(warn_ignored_slices(&with(&[""])));

        let mut config = with(&["grnts"]);
        config.override_strata_slices("projects");
        assert_eq!(config.strata.slices, ["projects"]);
        assert!(warn_ignored_slices(&config));

        config.override_strata_slices("");
        assert_eq!(config.strata.slices, Vec::<String>::new());
        assert!(!warn_ignored_slices(&config));
    }

    #[test]
    fn an_edition_must_serve_every_slice() {
        assert_eq!(missing_slices(Slice::ALL), Vec::<&str>::new());
        assert_eq!(missing_slices(&Slice::ALL[1..]), [Slice::ALL[0].key()]);
    }
}
