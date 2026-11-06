//! The gRPC channel to the server, with TLS for an `https` URL.

use std::error::Error;

use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::agent::AgentError;

/// Connects to the server.
pub(crate) async fn connect(url: &str) -> Result<Channel, AgentError> {
    let fail = |error: tonic::transport::Error| {
        AgentError::Connection(format!("{url}: {}", causes(&error)))
    };
    let mut endpoint = Endpoint::from_shared(url.to_owned()).map_err(fail)?;
    if url.starts_with("https://") {
        let tls = ClientTlsConfig::new().with_webpki_roots();
        endpoint = endpoint.tls_config(tls).map_err(fail)?;
    }
    endpoint.connect().await.map_err(fail)
}

/// The text of an error and of each cause under it, with no text two times in a row.
fn causes(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.ends_with(&cause_text) {
            text = format!("{text}: {cause_text}");
        }
        source = cause.source();
    }
    text
}
