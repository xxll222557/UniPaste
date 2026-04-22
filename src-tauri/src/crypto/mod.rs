use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    Key, XChaCha20Poly1305, XNonce,
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rand_core::{OsRng, RngCore};
use rcgen::generate_simple_self_signed;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
};
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::error::{AppError, AppResult};
use crate::sync::protocol::HandshakeHello;

const HANDSHAKE_MAX_SKEW_MS: u64 = 30_000;

pub struct HandshakeBundle {
    pub local_secret: EphemeralSecret,
    pub hello: HandshakeHello,
}

pub fn build_handshake(
    device_id: uuid::Uuid,
    device_name: &str,
    signing_key: &SigningKey,
) -> HandshakeBundle {
    let secret = EphemeralSecret::random_from_rng(OsRng);
    let public_key = PublicKey::from(&secret);
    let mut nonce = [0u8; 32];
    OsRng.fill_bytes(&mut nonce);
    let timestamp_ms = now_ms();
    let message = handshake_message(device_id, timestamp_ms, public_key.as_bytes(), &nonce);
    let signature = signing_key.sign(&message);

    HandshakeBundle {
        local_secret: secret,
        hello: HandshakeHello {
            device_id,
            device_name: device_name.to_string(),
            timestamp_ms,
            public_key: STANDARD.encode(signing_key.verifying_key().to_bytes()),
            eph_public_key: STANDARD.encode(public_key.as_bytes()),
            nonce: STANDARD.encode(nonce),
            signature: STANDARD.encode(signature.to_bytes()),
        },
    }
}

pub fn verify_handshake(hello: &HandshakeHello) -> AppResult<VerifyingKey> {
    let public_key = decode_32(&hello.public_key)?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|error| AppError::Crypto(error.to_string()))?;
    let eph_public_key = decode_32(&hello.eph_public_key)?;
    let nonce = decode_32(&hello.nonce)?;
    let signature_bytes = STANDARD.decode(&hello.signature)?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let message = handshake_message(hello.device_id, hello.timestamp_ms, &eph_public_key, &nonce);
    verifying_key
        .verify(&message, &signature)
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let age = now_ms().abs_diff(hello.timestamp_ms);
    if age > HANDSHAKE_MAX_SKEW_MS {
        return Err(AppError::Crypto("stale handshake detected".into()));
    }
    Ok(verifying_key)
}

pub fn derive_session_key(local_secret: EphemeralSecret, remote_eph_public_key: &str) -> AppResult<[u8; 32]> {
    let remote_bytes = decode_32(remote_eph_public_key)?;
    let remote_public = PublicKey::from(remote_bytes);
    let shared = local_secret.diffie_hellman(&remote_public);
    let hk = Hkdf::<Sha256>::new(Some(b"unipaste-lan-v2"), shared.as_bytes());
    let mut output = [0u8; 32];
    hk.expand(b"clipboard-sync", &mut output)
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    Ok(output)
}

pub fn encrypt(key_bytes: &[u8; 32], plaintext: &[u8]) -> AppResult<(String, String)> {
    let key = Key::from_slice(key_bytes);
    let cipher = XChaCha20Poly1305::new(key);
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    Ok((STANDARD.encode(nonce), STANDARD.encode(ciphertext)))
}

pub fn decrypt(key_bytes: &[u8; 32], nonce_b64: &str, ciphertext_b64: &str) -> AppResult<Vec<u8>> {
    let key = Key::from_slice(key_bytes);
    let cipher = XChaCha20Poly1305::new(key);
    let nonce_bytes = STANDARD.decode(nonce_b64)?;
    let nonce_array: [u8; 24] = nonce_bytes
        .try_into()
        .map_err(|_| AppError::Invalid("expected 24-byte nonce".into()))?;
    let ciphertext = STANDARD.decode(ciphertext_b64)?;
    cipher
        .decrypt(XNonce::from_slice(&nonce_array), ciphertext.as_ref())
        .map_err(|error| AppError::Crypto(error.to_string()))
}

pub fn build_quic_server_config() -> AppResult<quinn::ServerConfig> {
    let certified = generate_simple_self_signed(vec!["unipaste.local".into()])
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let cert_der = CertificateDer::from(certified.cert.der().to_vec());
    let key_der = PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der());
    let rustls_server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der.into())
        .map_err(|error| AppError::Crypto(error.to_string()))?;
    let server = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(rustls_server).map_err(|error| AppError::Crypto(error.to_string()))?,
    ));
    Ok(server)
}

pub fn build_quic_client_config() -> AppResult<quinn::ClientConfig> {
    let rustls_client = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    let client = quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(rustls_client).map_err(|error| AppError::Crypto(error.to_string()))?,
    ));
    Ok(client)
}

pub fn pairing_code(local_public_key: &[u8; 32], remote_public_key: &[u8; 32]) -> String {
    let (first, second) = if local_public_key <= remote_public_key {
        (local_public_key, remote_public_key)
    } else {
        (remote_public_key, local_public_key)
    };
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(first);
    bytes.extend_from_slice(second);
    let hash = blake3::hash(&bytes);
    let number = u32::from_be_bytes([hash.as_bytes()[0], hash.as_bytes()[1], hash.as_bytes()[2], hash.as_bytes()[3]])
        % 1_000_000;
    format!("{number:06}")
}

fn decode_32(value: &str) -> AppResult<[u8; 32]> {
    let bytes = STANDARD.decode(value)?;
    bytes
        .try_into()
        .map_err(|_| AppError::Invalid("expected 32-byte value".into()))
}

fn handshake_message(
    device_id: uuid::Uuid,
    timestamp_ms: u64,
    eph_public_key: &[u8; 32],
    nonce: &[u8; 32],
) -> Vec<u8> {
    let mut message = b"unipaste-handshake-v2".to_vec();
    message.extend_from_slice(device_id.as_bytes());
    message.extend_from_slice(&timestamp_ms.to_be_bytes());
    message.extend_from_slice(eph_public_key);
    message.extend_from_slice(nonce);
    message
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug)]
struct SkipServerVerification(Arc<rustls::crypto::CryptoProvider>);

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self(Arc::new(rustls::crypto::ring::default_provider())))
    }
}

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
