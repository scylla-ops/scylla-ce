use anyhow::{Context, Result};
use clap::{CommandFactory, FromArgMatches, Parser};
use scylla_core::config::{ControlPlaneConfig, MASTER_KEY_ENV};
use std::path::Path;

#[derive(Parser, Debug)]
pub struct Cli {
    /// Print an example configuration file and exit
    #[arg(short = 'e', long = "print-example-config")]
    pub print_example_config: bool,

    /// Path to the configuration file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Serve the API only, without the web UI (the scylla-web dev server loop). Overrides `[ui].enabled`.
    #[arg(long = "no-ui")]
    pub no_ui: bool,
}

impl Cli {
    /// Not `Cli::parse()`: `--help` and `--version` would print this library's name and version.
    #[must_use]
    pub fn parse_as(name: &'static str, version: &'static str, about: &'static str) -> Self {
        let matches = Self::command()
            .name(name)
            .version(version)
            .about(about)
            .get_matches();
        Self::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
    }

    pub fn load_config(&self) -> Result<ControlPlaneConfig> {
        let mut config = if let Some(path) = &self.config {
            ControlPlaneConfig::from_file(Path::new(path))
                .with_context(|| format!("Failed to load configuration from {path}"))?
        } else {
            tracing::info!("No configuration file provided, using defaults");
            ControlPlaneConfig::default()
        };

        config.apply_env_overrides();

        // The flag only turns the UI off: it cannot resurrect assets never built in.
        if self.no_ui {
            config.ui.enabled = false;
        }

        if config.uses_dev_master_key() {
            tracing::error!(
                "SECURITY: project secrets are encrypted with the PUBLIC dev master key, so they \
                 are NOT confidential. Set {MASTER_KEY_ENV} to a unique 64-hex-char key before \
                 exposing this instance."
            );
        }

        // Never debug-dump the config: it holds the master key and the other secrets.
        tracing::info!(
            ui = config.ui.enabled,
            secrets = config.secrets.is_some(),
            webhook = config.webhook.is_some(),
            "configuration loaded",
        );

        Ok(config)
    }
}

/// `RUST_LOG` wins. Otherwise `audit=warn`: grants are duplicate detail (the audit_log table stays complete), denials stay visible.
pub fn init_tracing(edition_targets: &[&str]) {
    let core_targets = [
        "scylla_server",
        "scylla_core",
        "scylla_extension",
        "scylla_auth",
        "scylla_db",
        "scylla_domain",
    ];
    let at_info = edition_targets
        .iter()
        .chain(core_targets.iter())
        .map(|target| format!("{target}=info"))
        .collect::<Vec<_>>()
        .join(",");
    let filter = format!("audit=warn,{at_info},warn");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .init();
}
