//! `.rr`'s `@ed25519_ok(key_hex, message, sig_b64)`: the detached Ed25519 check W's `verify_ed25519` SQL function
//! made over signed allowlists (d627273f6 `code_rule_dsl/crypto.rs`, its `verifies`, kept as it was): a 64-digit hex
//! public key, the message's UTF-8 bytes, a standard (padded) base64 signature of at most 128 characters, and
//! `verify_strict`. Anything malformed is a failed check, never an error.
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};

pub fn ed25519_ok(public_key: &str, message: &str, signature: &str) -> bool {
    if public_key.len() != 64 || signature.len() > 128 {
        return false;
    }
    let mut bytes = [0u8; 32];
    for (to, from) in bytes.iter_mut().zip(public_key.as_bytes().chunks_exact(2)) {
        let hex = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        };
        let (Some(high), Some(low)) = (hex(from[0]), hex(from[1])) else {
            return false;
        };
        *to = high * 16 + low;
    }
    let Ok(key) = VerifyingKey::from_bytes(&bytes) else {
        return false;
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(signature) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&bytes) else {
        return false;
    };
    key.verify_strict(message.as_bytes(), &signature).is_ok()
}
