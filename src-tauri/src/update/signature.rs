//! Checks an installer against its signature before anything runs it. The releases are signed with a
//! key kept off the repository; the matching public key is built in, so a changed or swapped file (or
//! a manifest that points elsewhere) cannot make the island run something it was not given.
//!
//! Format: what `tauri signer` writes. Both the key and the signature are the base64 text of a
//! minisign file.

use crate::fault::{Cause, Failed};
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};

/// The text of `update.key.pub`, base64. Public: it can only check, never sign.
pub const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDUzRkI2NDlCMjhCNDJGQzUKUldURkw3UW9tMlQ3VTN0VkZVRDhOQWltTmp4N1o1QWlXTk9FYlMzcmhyMzN5NDBBeXRHTmRqZkgK";

pub fn verify(installer: &[u8], signature: &str) -> Result<(), Failed> {
    verify_with(PUBLIC_KEY, installer, signature)
}

/// With another key: for the release check and the tests.
pub fn verify_with(key: &str, installer: &[u8], signature: &str) -> Result<(), Failed> {
    let refused = |detail: String| Failed::because(Cause::Unknown, format!("update signature: {detail}"));
    let text = |base64: &str| {
        STANDARD
            .decode(base64.trim())
            .map_err(|e| e.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
    };
    let key = PublicKey::decode(&text(key).map_err(refused)?).map_err(|e| refused(e.to_string()))?;
    let signature =
        Signature::decode(&text(signature).map_err(refused)?).map_err(|e| refused(e.to_string()))?;
    key.verify(installer, &signature, true)
        .map_err(|e| refused(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made by `tauri signer` with a throwaway key, the way a release is signed.
    const TEST_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVCMzBENTMzNThCRkQwMzAKUldRdzBMOVlNOVV3NndjVzl0dkx6cVdYODRxTy9qeTE4dDZPMkdaUnRTaW5waTdyOGJ3K25FMWEK";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRdzBMOVlNOVV3Nnl1SitLRXNhS1BmNGdSOHJWMFc1Z2pDZHRkZ24zL1Rxa1p6RHZ5YjlFSU1KWXY1MWpJakxOWG50aGRZVUFPcE9UV09SS1Z1QVMrTFZ3YjVRUnZmVHdNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzk0ODc2NTI1CWZpbGU6Zml4dHVyZS5iaW4KaklNeU5CY2ZmTUluM3Vqb05hNE1qMXVHQ1RlSEJkYUYzL0N4Z2c1SUZPZEZrYU0vMVBGQWFaakpabmdoUjkxTnZYNGpBK0Nmb1hBUmpGZjUvVlNqQ1E9PQo=";
    const INSTALLER: &[u8] = b"Great Island test installer\n";

    #[test]
    fn a_file_signed_with_the_key_passes() {
        verify_with(TEST_KEY, INSTALLER, TEST_SIGNATURE).expect("signed");
        // The .sig file ends with a newline.
        verify_with(TEST_KEY, INSTALLER, &format!("{TEST_SIGNATURE}\n")).expect("signed");
    }

    #[test]
    fn a_changed_file_is_refused() {
        let mut changed = INSTALLER.to_vec();
        changed[0] ^= 1;
        assert!(verify_with(TEST_KEY, &changed, TEST_SIGNATURE).is_err());
        assert!(verify_with(TEST_KEY, b"", TEST_SIGNATURE).is_err());
    }

    #[test]
    fn a_signature_of_another_key_is_refused() {
        assert!(verify(INSTALLER, TEST_SIGNATURE).is_err());
    }

    #[test]
    fn rubbish_is_refused() {
        for signature in ["", "not base64!", "bm90IGEgc2lnbmF0dXJl"] {
            assert!(verify_with(TEST_KEY, INSTALLER, signature).is_err(), "{signature}");
        }
        assert!(verify_with("garbage", INSTALLER, TEST_SIGNATURE).is_err());
    }

    #[test]
    fn the_built_in_key_is_well_formed() {
        let text = String::from_utf8(STANDARD.decode(PUBLIC_KEY).expect("base64")).expect("text");
        PublicKey::decode(&text).expect("a minisign key");
    }
}
