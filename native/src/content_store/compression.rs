//! zstd frames as `zstd::stream::encode_all` writes them, from one compression context per thread
//! instead of a fresh one (and its zeroed tables) per call.

use std::cell::RefCell;
use std::io;
use zstd::stream::raw::{Encoder, InBuffer, Operation, OutBuffer};

thread_local! {
    /// This thread's context, with the level it was set to.
    static ENCODER: RefCell<Option<(i32, Encoder<'static>)>> = const { RefCell::new(None) };
}

/// `bytes` as one zstd frame at `level`, byte for byte what `encode_all` writes: streamed, so the
/// frame header carries no content size.
pub(crate) fn compress(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
    ENCODER.with_borrow_mut(|slot| {
        let encoder = match slot {
            Some((known, encoder)) if *known == level => {
                encoder.reinit()?;
                encoder
            }
            _ => &mut slot.insert((level, Encoder::new(level)?)).1,
        };
        let frame = encode(encoder, bytes);
        if frame.is_err() {
            // A context that failed mid-frame is not reused.
            *slot = None;
        }
        frame
    })
}

fn encode(encoder: &mut Encoder<'static>, bytes: &[u8]) -> io::Result<Vec<u8>> {
    // The bound holds any frame of `bytes`; growing is only a fallback.
    let mut frame = Vec::with_capacity(zstd::zstd_safe::compress_bound(bytes.len()) + 64);
    let mut input = InBuffer::around(bytes);
    while input.pos < bytes.len() {
        reserve(&mut frame);
        let at = frame.len();
        encoder.run(&mut input, &mut OutBuffer::around_pos(&mut frame, at))?;
    }
    loop {
        reserve(&mut frame);
        let at = frame.len();
        if encoder.finish(&mut OutBuffer::around_pos(&mut frame, at), true)? == 0 {
            return Ok(frame);
        }
    }
}

/// Room for more output once the frame's is used up.
fn reserve(frame: &mut Vec<u8>) {
    if frame.capacity() == frame.len() {
        frame.reserve(frame.len() / 2 + 64);
    }
}

#[cfg(test)]
mod tests {
    use super::compress;
    use std::io::Cursor;

    #[test]
    fn frames_equal_encode_all_at_every_level_and_size() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut noise = |len: usize| {
            (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect::<Vec<_>>()
        };
        let text = "def run(conn):\n    conn.session('baseline').write_async()\n".repeat(40_000);
        let inputs = [
            Vec::new(),
            b"alpha".to_vec(),
            noise(4096),
            noise(200_000),
            text.as_bytes()[..131_072].to_vec(),
            text.into_bytes(),
        ];
        // Levels interleaved, so a reused context changes level between frames.
        for level in [3, 1, 3, 19, -5, 3] {
            for input in &inputs {
                assert_eq!(
                    compress(input, level).unwrap(),
                    zstd::stream::encode_all(Cursor::new(input), level).unwrap(),
                    "level {level}, {} bytes",
                    input.len()
                );
            }
        }
    }
}
