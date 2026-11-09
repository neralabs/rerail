//! Public-key verification of existing signed artifact bytes. Never signs or reads keys from the environment.
use base64::{Engine, alphabet, engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig}};
use ed25519_dalek::{Signature, VerifyingKey};

/// CPython's non-strict base64 transport ignores ASCII noise and data after completed padding.
/// Canonicalize that transport, then let the base64 library decode it. Signature bytes stay unchanged.
fn signature_bytes(text: &str) -> Result<Vec<u8>, String> {
    if !text.is_ascii() {
        return Err("string argument should contain only ASCII characters".to_owned());
    }
    let mut encoded = Vec::with_capacity(88);
    let mut data = 0usize;
    let mut pads = 0;
    let mut complete = false;
    for byte in text.bytes() {
        if byte == b'=' {
            let position = data % 4;
            pads = (pads + 1).min(4);
            if position >= 2 && position + pads >= 4 {
                if data <= 86 {
                    encoded.extend(std::iter::repeat_n(b'=', 4 - position));
                }
                complete = true;
                break;
            }
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/') {
            pads = 0;
            data += 1;
            // Retain only signature-sized payloads. Count larger inputs to report
            // their transport error without allocating their decoded bytes.
            if data <= 86 {
                encoded.push(byte);
            }
        }
    }
    let remainder = data % 4;
    if !complete && remainder != 0 {
        return Err(if remainder == 1 {
            format!("Invalid base64-encoded string: number of data characters ({data}) cannot be 1 more than a multiple of 4")
        } else {
            "Incorrect padding".to_owned()
        });
    }
    let bytes = data / 4 * 3 + if complete { remainder - 1 } else { 0 };
    if bytes != 64 {
        return Err("The signature must be exactly 64 bytes long".to_owned());
    }
    let decoder = GeneralPurpose::new(&alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true));
    decoder.decode(encoded).map_err(|_| "Incorrect padding".to_owned())
}

fn verify(public_key: &str, message: &str, signature: &str) -> Result<(), String> {
    let invalid_key = || "invalid Ed25519 public key".to_owned();
    if public_key.len() != 64 {
        return Err(invalid_key());
    }
    let mut bytes = [0; 32];
    for (byte, hex) in bytes.iter_mut().zip(public_key.as_bytes().chunks_exact(2)) {
        let digit = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'), b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10), _ => None,
        };
        let (Some(high), Some(low)) = (digit(hex[0]), digit(hex[1])) else { return Err(invalid_key()) };
        *byte = high * 16 + low;
    }
    let key = VerifyingKey::from_bytes(&bytes).map_err(|_| invalid_key())?;
    let bytes = signature_bytes(signature)?;
    let signature = Signature::from_slice(&bytes)
        .map_err(|_| "The signature must be exactly 64 bytes long".to_owned())?;
    key.verify_strict(message.as_bytes(), &signature)
        .map_err(|_| "Signature was forged or corrupt".to_owned())
}

pub(super) fn verifies(public_key: &str, message: &str, signature: &str) -> bool {
    verify(public_key, message, signature).is_ok()
}

pub(super) fn error(public_key: &str, message: &str, signature: &str) -> String {
    verify(public_key, message, signature).err().unwrap_or_default()
}
