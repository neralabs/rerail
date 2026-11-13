//! Detached public-key verification for signed rule artifacts.
//! This scalar is used over artifact rows, never as an AST-node callback.
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use rusqlite::{Connection, functions::FunctionFlags};

fn verifies(public_key: &str, message: &str, signature: &str) -> bool {
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

pub(super) fn register(db: &Connection) -> rusqlite::Result<()> {
    db.create_scalar_function(
        "sha256",
        1,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let text: Option<String> = context.get(0)?;
            Ok(text.map(|s| super::json::hex_digest(s.as_bytes())))
        },
    )?;
    db.create_scalar_function(
        "verify_ed25519",
        3,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let public_key: Option<String> = context.get(0)?;
            let message: Option<String> = context.get(1)?;
            let signature: Option<String> = context.get(2)?;
            Ok(match (public_key, message, signature) {
                (Some(key), Some(message), Some(signature)) => verifies(&key, &message, &signature),
                _ => false,
            })
        },
    )
}

#[cfg(test)]
pub mod tests {
    #[test]
    fn hashes_signed_artifact_payloads() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        super::register(&db).unwrap();
        let actual: String = db
            .query_row("SELECT sha256('abc')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            actual,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
