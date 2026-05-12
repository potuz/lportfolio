//! Local stand-in for `rustls-platform-verifier`. Patched in via
//! `[patch.crates-io]` so the Android cross-compile of `lportfolio` does not
//! require the Kotlin/Java component the upstream crate needs to verify
//! certificates against the Android system store.
//!
//! This crate exposes only the surface that `reqwest 0.13.3` uses:
//!   - `Verifier::new(provider)`
//!   - `Verifier::new_with_extra_roots(roots, provider)`
//! Both return a verifier that trusts Mozilla's webpki-roots set on every
//! platform — the same cert source used by `rustls-tls-webpki-roots` in
//! reqwest 0.12.

#![forbid(unsafe_code)]

use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme};

#[derive(Debug)]
pub struct Verifier {
    inner: Arc<WebPkiServerVerifier>,
}

impl Verifier {
    pub fn new(crypto_provider: Arc<CryptoProvider>) -> Result<Self, TlsError> {
        Self::new_with_extra_roots(std::iter::empty(), crypto_provider)
    }

    pub fn new_with_extra_roots(
        extra_roots: impl IntoIterator<Item = CertificateDer<'static>>,
        crypto_provider: Arc<CryptoProvider>,
    ) -> Result<Self, TlsError> {
        let mut root_store = RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for cert in extra_roots {
            root_store.add(cert)?;
        }
        let inner = WebPkiServerVerifier::builder_with_provider(root_store.into(), crypto_provider)
            .build()
            .map_err(|e| TlsError::Other(rustls::OtherError(Arc::new(e))))?;
        Ok(Self { inner })
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}
