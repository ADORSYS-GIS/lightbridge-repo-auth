//! GitHub webhook signature verification (`X-Hub-Signature-256`).

use ring::hmac;

use crate::error::{Error, Result};

/// Verify the `X-Hub-Signature-256: sha256=<hex>` header against the raw body.
///
/// `ring::hmac::verify` is constant-time, so we don't leak the expected digest
/// through timing. The raw body bytes MUST be exactly what GitHub sent —
/// re-serializing parsed JSON would change the signature.
pub fn verify_signature(secret: &str, body: &[u8], signature_header: Option<&str>) -> Result<()> {
    let sig = signature_header.ok_or(Error::BadSignature)?;
    let hex_digest = sig.strip_prefix("sha256=").ok_or(Error::BadSignature)?;
    let expected = hex::decode(hex_digest).map_err(|_| Error::BadSignature)?;

    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    hmac::verify(&key, body, &expected).map_err(|_| Error::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture from GitHub's docs: secret "It's a Secret to Everybody",
    // body "Hello, World!".
    const SIG: &str = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";

    #[test]
    fn accepts_valid_signature() {
        assert!(verify_signature("It's a Secret to Everybody", b"Hello, World!", Some(SIG)).is_ok());
    }

    #[test]
    fn rejects_tampered_body() {
        assert!(verify_signature("It's a Secret to Everybody", b"Goodbye, World!", Some(SIG)).is_err());
    }

    #[test]
    fn rejects_missing_header() {
        assert!(verify_signature("x", b"y", None).is_err());
    }
}
