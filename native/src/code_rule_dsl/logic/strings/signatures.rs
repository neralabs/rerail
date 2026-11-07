//! Public-key verification of existing signed artifact bytes. Never signs or reads keys from the environment.
use base64::{Engine, alphabet, engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig}};
use ed25519_dalek::{Signature, VerifyingKey};

/// CPython's non-strict base64 transport ignores ASCII noise and data after completed padding.
/// Canonicalize that transport, then let the base64 library decode it. Signature bytes stay unchanged.
fn signature_bytes(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let mut encoded = Vec::with_capacity(88);
    let mut pads = 0;
    let mut complete = false;
    for byte in text.bytes() {
        if byte == b'=' {
            let position = encoded.len() % 4;
            pads += 1;
            if position >= 2 && position + pads >= 4 {
                encoded.extend(std::iter::repeat_n(b'=', 4 - position));
                complete = true;
                break;
            }
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/') {
            pads = 0;
            encoded.push(byte);
            // More than 86 payload characters cannot decode to a 64-byte signature.
            if encoded.len() > 86 {
                return None;
            }
        }
    }
    if !complete && encoded.len() % 4 != 0 {
        return None;
    }
    let decoder = GeneralPurpose::new(&alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true));
    decoder.decode(encoded).ok()
}

pub(super) fn verifies(public_key: &str, message: &str, signature: &str) -> bool {
    if public_key.len() != 64 {
        return false;
    }
    let mut bytes = [0; 32];
    for (byte, hex) in bytes.iter_mut().zip(public_key.as_bytes().chunks_exact(2)) {
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'), b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10), _ => None,
        };
        let (Some(high), Some(low)) = (digit(hex[0]), digit(hex[1])) else { return false };
        *byte = high * 16 + low;
    }
    let Ok(key) = VerifyingKey::from_bytes(&bytes) else { return false };
    let Some(bytes) = signature_bytes(signature) else { return false };
    let Ok(signature) = Signature::from_slice(&bytes) else { return false };
    key.verify_strict(message.as_bytes(), &signature).is_ok()
}
