//! Certificate pin and handshake signature verification.
use crate::catalog::{Pin, PinKind};
use ring::digest::{SHA256, digest};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
pub(super) struct PinVerifier {
    pub(super) pins: Vec<Pin>,
    pub(super) mismatch: Arc<AtomicBool>,
    pub(super) provider: Arc<rustls::crypto::CryptoProvider>,
}
impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        // The configured pins are the trust anchors; CA, hostname and clock checks are not used.
        // Parse the leaf even for a certificate hash so malformed X.509 can never match.
        let parsed = webpki::EndEntityCert::try_from(end_entity).map_err(|_| {
            self.mismatch.store(true, Ordering::SeqCst);
            rustls::Error::General("invalid pinned leaf certificate".into())
        })?;
        let certificate = digest(&SHA256, end_entity.as_ref());
        let spki = digest(&SHA256, parsed.subject_public_key_info().as_ref());
        if !self.pins.iter().any(|pin| match pin.kind {
            PinKind::CertificateSha256 => certificate.as_ref() == pin.sha256.as_slice(),
            PinKind::SpkiSha256 => spki.as_ref() == pin.sha256.as_slice(),
        }) {
            self.mismatch.store(true, Ordering::SeqCst);
            return Err(rustls::Error::General("leaf pin mismatch".into()));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_leaf_cannot_match_even_its_certificate_hash() {
        let malformed = CertificateDer::from(vec![0x30, 0x01, 0x00]);
        let verifier = PinVerifier {
            pins: vec![Pin {
                kind: PinKind::CertificateSha256,
                sha256: digest(&SHA256, malformed.as_ref())
                    .as_ref()
                    .try_into()
                    .unwrap(),
            }],
            mismatch: Arc::new(AtomicBool::new(false)),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        };
        let name = ServerName::try_from("localhost").unwrap();
        assert!(
            verifier
                .verify_server_cert(&malformed, &[], &name, &[], UnixTime::now())
                .is_err()
        );
        assert!(verifier.mismatch.load(Ordering::SeqCst));
    }
}
