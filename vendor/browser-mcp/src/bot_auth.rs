//! Web Bot Auth (RFC 9421 HTTP Message Signatures) — outbound request signing.
//!
//! Loads an Ed25519 private key from a PKCS#8 PEM file pointed at by
//! `CPC_WBA_KEY_PATH`, using the key identifier in `CPC_WBA_KEY_ID`. When
//! either env var is unset (or the key fails to load) the signer simply
//! returns `None` from `Signer::from_env()` and the crawler runs unsigned —
//! fully backwards compatible.
//!
//! For every outbound request, `Signer::sign(url)` produces the values for
//! the `Signature-Input` and `Signature` headers (already in the `sig1=...`
//! label form). The crawler attaches them via `reqwest::RequestBuilder::header`.

use ed25519_dalek::pkcs8::DecodePrivateKey;
use indexmap::IndexMap;
use std::time::Duration;
use web_bot_auth::{
    components::{CoveredComponent, DerivedComponent},
    keyring::Algorithm,
    message_signatures::{MessageSigner, UnsignedMessage},
};

/// Lifetime (seconds) of each generated signature — verifiers accept it within this window.
const SIGNATURE_EXPIRY_SECS: u64 = 10;
/// Tag advertised in the signature parameters — fixed by the Web Bot Auth spec.
const TAG: &str = "web-bot-auth";

pub struct Signer {
    keyid: String,
    /// Raw 32-byte Ed25519 private key (seed).
    private_key: [u8; 32],
}

impl Signer {
    /// Build a signer from environment:
    ///   `CPC_WBA_KEY_PATH` — path to a PKCS#8 PEM Ed25519 private key file
    ///   `CPC_WBA_KEY_ID`   — key identifier (matches `kid` in your published JWKS)
    /// Returns `None` if either var is unset/empty or the key file can't be loaded.
    pub fn from_env() -> Option<Self> {
        let path = std::env::var("CPC_WBA_KEY_PATH")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let keyid = std::env::var("CPC_WBA_KEY_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let pem = std::fs::read_to_string(&path).ok()?;
        let signing_key = ed25519_dalek::SigningKey::from_pkcs8_pem(&pem).ok()?;
        Some(Signer {
            keyid,
            private_key: signing_key.to_bytes(),
        })
    }

    /// Build the `Signature-Input` and `Signature` HTTP header values for a
    /// request to `url`. Returns `(signature_input, signature)` — each already
    /// in the `sig1=...` label form, ready to set as headers.
    /// Returns `None` if the URL has no host (e.g. relative) or signing fails.
    pub fn sign(&self, url: &reqwest::Url) -> Option<(String, String)> {
        let host = url.host_str()?;
        let mut authority = host.to_string();
        if let Some(port) = url.port() {
            authority.push(':');
            authority.push_str(&port.to_string());
        }

        let mut msg = OutgoingRequest::new(authority);
        let signer = MessageSigner {
            keyid: self.keyid.clone(),
            nonce: random_nonce(),
            tag: TAG.into(),
        };
        signer
            .generate_signature_headers_content(
                &mut msg,
                Duration::from_secs(SIGNATURE_EXPIRY_SECS),
                Algorithm::Ed25519,
                &self.private_key,
            )
            .ok()?;
        Some((msg.signature_input, msg.signature_header))
    }
}

/// Minimal `UnsignedMessage` impl covering just `@authority` — mirrors the
/// Cloudflare reference example for outbound bot requests. The signature
/// commits to the destination host (so a captured signature can't be replayed
/// against a different site).
struct OutgoingRequest {
    authority: String,
    signature_input: String,
    signature_header: String,
}

impl OutgoingRequest {
    fn new(authority: String) -> Self {
        Self {
            authority,
            signature_input: String::new(),
            signature_header: String::new(),
        }
    }
}

impl UnsignedMessage for OutgoingRequest {
    fn fetch_components_to_cover(&self) -> IndexMap<CoveredComponent, String> {
        IndexMap::from_iter([(
            CoveredComponent::Derived(DerivedComponent::Authority { req: false }),
            self.authority.clone(),
        )])
    }

    fn register_header_contents(&mut self, signature_input: String, signature_header: String) {
        self.signature_input = format!("sig1={signature_input}");
        self.signature_header = format!("sig1={signature_header}");
    }
}

/// 32 bytes of OS randomness, base64-encoded — a fresh nonce per signature.
fn random_nonce() -> String {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let mut bytes = [0u8; 32];
    rand::Rng::fill(&mut rand::thread_rng(), &mut bytes);
    STANDARD.encode(bytes)
}
