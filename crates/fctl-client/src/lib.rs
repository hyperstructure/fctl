//! Networking for fctl's native binaries: an HTTPS client, WebSocket
//! connections, and a typed client for the control plane's REST API.
//!
//! TLS is rustls with the ring provider and Mozilla's root store (plus any
//! certificates in `SSL_CERT_FILE`), so binaries need no system libraries and
//! cross-compile anywhere.

pub mod api;
pub mod http;
pub mod ws;

pub use api::Api;
pub use http::{Error, HttpClient};

use std::sync::{Arc, OnceLock};

/// The TLS configuration shared by HTTP and WebSocket clients.
pub fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            if let Some(path) = std::env::var_os("SSL_CERT_FILE") {
                use rustls_pki_types::pem::PemObject;
                if let Ok(iter) = rustls_pki_types::CertificateDer::pem_file_iter(&path) {
                    for cert in iter.flatten() {
                        let _ = roots.add(cert);
                    }
                }
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map(|b| b.with_root_certificates(roots).with_no_client_auth());
            match config {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    // Only possible if ring lost TLS 1.2 and 1.3 support.
                    tracing::error!("TLS configuration failed: {e}");
                    std::process::exit(70);
                }
            }
        })
        .clone()
}

/// `fctl/<version> (<os>; <arch>)`
pub fn user_agent() -> String {
    format!(
        "fctl/{} ({}; {})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}
