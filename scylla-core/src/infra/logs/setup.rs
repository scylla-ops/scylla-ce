//! The start of the logs: the filter of `RUST_LOG` or the default filter, and the layer in the
//! style of the settings.

use tracing::subscriber::DefaultGuard;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Registry;

use super::format::{LogFormat, Style};
use super::layer::LogLayer;

/// The filter when `RUST_LOG` is not set: INFO, and WARN for the libraries that log each
/// connection, request or query.
pub const DEFAULT_FILTER: &str =
    "info,sqlx=warn,hyper=warn,h2=warn,tonic=warn,tower=warn,tower_http=warn";

/// Starts the logs on stderr in a format, with the filter of `RUST_LOG`, else the default
/// filter. A `RUST_LOG` that is not valid gives the default filter and a warning.
pub fn init(format: LogFormat) {
    let custom = std::env::var("RUST_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let parsed = custom.as_deref().map(str::parse::<Targets>);
    let filter = match &parsed {
        Some(Ok(filter)) => filter.clone(),
        _ => default_filter(),
    };
    let layer = LogLayer::new(format.style(), std::io::stderr);
    let _ = Registry::default().with(filter).with(layer).try_init();
    if let Some(Err(error)) = parsed {
        tracing::warn!(%error, "RUST_LOG is not valid: the default filter applies");
    }
}

/// Captures the log lines of this thread in a style, with the default filter, until the guard
/// drops. A test uses it to read the lines of its server.
pub fn capture<W>(style: Style, writer: W) -> DefaultGuard
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let layer = LogLayer::new(style, writer);
    tracing::subscriber::set_default(Registry::default().with(default_filter()).with(layer))
}

/// Returns the default filter.
fn default_filter() -> Targets {
    DEFAULT_FILTER.parse().expect("a valid filter")
}
