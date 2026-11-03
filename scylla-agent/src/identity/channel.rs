//! The gRPC channel to the server, with TLS for an `https` URL.

use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::agent::AgentError;

/// Connects to the server.
pub(crate) async fn connect(url: &str) -> Result<Channel, AgentError> {
    let fail = |error: tonic::transport::Error| AgentError::Connection(error.to_string());
    let mut endpoint = Endpoint::from_shared(url.to_owned()).map_err(fail)?;
    if url.starts_with("https://") {
        let tls = ClientTlsConfig::new().with_webpki_roots();
        endpoint = endpoint.tls_config(tls).map_err(fail)?;
    }
    endpoint.connect().await.map_err(fail)
}
