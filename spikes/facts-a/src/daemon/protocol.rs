//! The daemon's wire format (docs/facts/PROTOCOL.md): over a Unix stream socket, each message is
//! one frame `u32 header length (LE) | header (UTF-8 JSON object) | u32 body length (LE) | body`.
//! The header carries `op` and the message's fields; the body is raw bytes (an edit's content when
//! the daemon does not know its hash) and is empty otherwise.
use serde_json::Value as Json;
use std::io::{self, Read, Write};

/// The largest header or body a frame may carry.
pub const MAX_FRAME: u32 = 256 << 20;

pub struct Frame {
    pub header: Json,
    pub body: Vec<u8>,
}

fn read_u32(r: &mut impl Read) -> io::Result<Option<u32>> {
    let mut b = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut b[got..])? {
            0 if got == 0 => return Ok(None),
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame length")),
            n => got += n,
        }
    }
    Ok(Some(u32::from_le_bytes(b)))
}

fn read_exact_len(r: &mut impl Read, len: u32) -> io::Result<Vec<u8>> {
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame part of {len} bytes exceeds {MAX_FRAME}")));
    }
    let mut v = vec![0u8; len as usize];
    r.read_exact(&mut v)?;
    Ok(v)
}

/// The next frame, or `None` at a clean end of stream (between frames).
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let Some(hl) = read_u32(r)? else { return Ok(None) };
    let header = read_exact_len(r, hl)?;
    let header: Json = serde_json::from_slice(&header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("header: {e}")))?;
    if !header.is_object() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header is not a JSON object"));
    }
    let bl = read_u32(r)?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing body length"))?;
    let body = read_exact_len(r, bl)?;
    Ok(Some(Frame { header, body }))
}

pub fn write_frame(w: &mut impl Write, header: &Json, body: &[u8]) -> io::Result<()> {
    let h = serde_json::to_vec(header).expect("header serialises");
    let mut out = Vec::with_capacity(8 + h.len() + body.len());
    out.extend_from_slice(&(h.len() as u32).to_le_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    w.write_all(&out)?;
    w.flush()
}

pub fn hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_round_trip_and_end_cleanly() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &json!({"op": "edit", "path": "a.py"}), b"x = 1\n").unwrap();
        write_frame(&mut buf, &json!({"op": "stats"}), b"").unwrap();
        let mut r = &buf[..];
        let a = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(a.header["op"], "edit");
        assert_eq!(a.body, b"x = 1\n");
        let b = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(b.header["op"], "stats");
        assert!(b.body.is_empty());
        assert!(read_frame(&mut r).unwrap().is_none());
        // A frame cut inside its header is an error, not a clean end.
        let mut cut = &buf[..6];
        assert!(read_frame(&mut cut).is_err());
    }

    #[test]
    fn hashes_parse_as_64_hex_digits() {
        let h = blake3::hash(b"x");
        assert_eq!(hex32(&h.to_hex()), Some(*h.as_bytes()));
        assert_eq!(hex32("zz"), None);
    }
}
