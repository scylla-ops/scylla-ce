//! The TLS crypto provider of the process: ring, the provider that tonic uses.

/// Installs ring as the TLS crypto provider. A second call does nothing.
pub fn install() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
