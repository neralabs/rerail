//! The store's zstd frames for its internal rows (results, pages, memo entries): how they are compressed and read
//! back. Sources and tapes are not compressed: each is kept as it is, in a segment (`object_segments`; decision
//! 0295 step 5, 2026-11-19).
//!
//! - **Plain frames** are what a fresh streaming `zstd::stream::Encoder` writes (no content size in the header), from
//!   one compression context per thread instead of a fresh one (and its zeroed tables) per call. `encode_all` wrote
//!   the same until zstd 0.14.4, which records the size of a source under 128 KiB.
//! - Every reader of a store frame goes through [`decompress`], [`decompress_into`], [`decompress_head`] or
//!   [`decompress_chunks`]. A frame that names a dictionary is refused: the store compresses with none.

use std::cell::{Cell, RefCell};
use std::io;
use zstd::stream::raw::{Decoder, Encoder, InBuffer, Operation, OutBuffer};

thread_local! {
    /// This thread's plain context, with the level it was set to.
    static ENCODER: RefCell<Option<(i32, Encoder<'static>)>> = const { RefCell::new(None) };
    /// This thread's decompression context.
    static DECODER: RefCell<Option<Decoder<'static>>> = const { RefCell::new(None) };
    /// The bytes of the frames this thread has made for the store: a writer's job reads what it encoded as the
    /// difference around it (`write_back`'s accounts), whether its payload came encoded or it encoded it there.
    static ENCODED: Cell<usize> = const { Cell::new(0) };
}

/// The bytes of the frames this thread has made so far ([`compress`]).
pub fn encoded_here() -> usize {
    ENCODED.with(Cell::get)
}

fn counted(frame: io::Result<Vec<u8>>) -> io::Result<Vec<u8>> {
    if let Ok(frame) = &frame {
        ENCODED.with(|n| n.set(n.get() + frame.len()));
    }
    frame
}

/// `bytes` as one plain zstd frame at `level`, byte for byte what a fresh streaming encoder writes: streamed, so the
/// frame header carries no content size. For store-internal blobs (results, pages, memo entries).
pub fn compress(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
    counted(plain(bytes, level))
}

fn plain(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
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

/// A store frame's bytes. `capacity` is the
/// expected length, when the caller knows it (a hint; the frame decides).
pub fn decompress(frame: &[u8], capacity: usize) -> io::Result<Vec<u8>> {
    with_decoder(frame, |decoder| {
        let mut out = Vec::with_capacity(capacity.max(64));
        let mut input = InBuffer::around(frame);
        loop {
            reserve(&mut out);
            let at = out.len();
            let mut output = OutBuffer::around_pos(&mut out, at);
            let hint = decoder.run(&mut input, &mut output)?;
            if hint == 0 {
                return finished(&input, frame).map(|()| out);
            }
            if input.pos == frame.len() && output.pos() < output.capacity() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated zstd frame",
                ));
            }
        }
    })
}

/// A store frame decompressed into `out`, which must be exactly its length.
pub fn decompress_into(frame: &[u8], out: &mut [u8]) -> io::Result<()> {
    with_decoder(frame, |decoder| {
        let mut input = InBuffer::around(frame);
        let mut output = OutBuffer::around(out);
        loop {
            let full = output.pos() == output.capacity();
            let hint = decoder.run(&mut input, &mut output)?;
            if hint == 0 {
                finished(&input, frame)?;
                return if output.pos() == output.capacity() {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "frame is shorter than its slot",
                    ))
                };
            }
            if full || input.pos == frame.len() && output.pos() < output.capacity() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame does not fit its slot",
                ));
            }
        }
    })
}

/// The first `n` bytes of a store frame's content, decompressing no more of it than they need.
pub fn decompress_head(frame: &[u8], n: usize) -> io::Result<Vec<u8>> {
    with_decoder(frame, |decoder| {
        let mut out = vec![0; n];
        let mut input = InBuffer::around(frame);
        let mut output = OutBuffer::around(&mut out[..]);
        while output.pos() < n {
            let before = (input.pos, output.pos());
            let hint = decoder.run(&mut input, &mut output)?;
            if output.pos() < n && (hint == 0 || (input.pos, output.pos()) == before) {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "frame is shorter than the head",
                ));
            }
        }
        Ok(out)
    })
}

/// A store frame's content in chunks of at most 64 KB, in order, without holding all of it.
pub fn decompress_chunks(frame: &[u8], mut each: impl FnMut(&[u8])) -> io::Result<()> {
    with_decoder(frame, |decoder| {
        let mut chunk = vec![0; 1 << 16];
        let mut input = InBuffer::around(frame);
        loop {
            let mut output = OutBuffer::around(&mut chunk[..]);
            let hint = decoder.run(&mut input, &mut output)?;
            let written = output.pos();
            each(&chunk[..written]);
            if hint == 0 {
                return finished(&input, frame);
            }
            if written == 0 && input.pos == frame.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated zstd frame",
                ));
            }
        }
    })
}

fn finished(input: &InBuffer<'_>, frame: &[u8]) -> io::Result<()> {
    if input.pos == frame.len() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bytes after the zstd frame",
        ))
    }
}

/// Runs `read` with this thread's decoder, reset for a new frame. A frame that names a dictionary is refused.
fn with_decoder<T>(frame: &[u8], read: impl FnOnce(&mut Decoder<'static>) -> io::Result<T>) -> io::Result<T> {
    if let Some(id) = zstd::zstd_safe::get_dict_id_from_frame(frame) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("a frame of dictionary {:#x}: the store uses none", id.get())));
    }
    DECODER.with_borrow_mut(|slot| {
        let decoder = match slot {
            Some(decoder) => {
                decoder.reinit()?;
                decoder
            }
            None => slot.insert(Decoder::new()?),
        };
        let result = read(decoder);
        if result.is_err() {
            // A context that failed mid-frame is not reused.
            *slot = None;
        }
        result
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
    use super::*;

    /// The frame a fresh streaming encoder writes for `input` at `level`: the store's plain frame.
    fn streamed(input: &[u8], level: i32) -> Vec<u8> {
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), level).unwrap();
        std::io::Write::write_all(&mut encoder, input).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn frames_equal_a_fresh_streamed_frame_at_every_level_and_size() {
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
                let frame = compress(input, level).unwrap();
                assert_eq!(
                    frame,
                    streamed(input, level),
                    "level {level}, {} bytes",
                    input.len()
                );
                assert_eq!(
                    decompress(&frame, input.len()).unwrap(),
                    *input,
                    "level {level}, {} bytes",
                    input.len()
                );
            }
        }
    }

    #[test]
    fn frames_read_back_whole_into_a_slot_by_head_and_in_chunks() {
        let python = b"from __future__ import annotations\n\nimport os\n\n\ndef main() -> None:\n    print(os.getcwd())\n";
        let small = compress(python, 3).unwrap();
        assert_eq!(decompress(&small, 0).unwrap(), python);
        let mut slot = vec![0; python.len()];
        decompress_into(&small, &mut slot).unwrap();
        assert_eq!(slot, python);
        assert!(decompress_into(&small, &mut vec![0; python.len() - 1]).is_err());
        assert!(decompress_into(&small, &mut vec![0; python.len() + 1]).is_err());
        assert!(decompress(b"not a frame", 0).is_err());
        assert_eq!(decompress_head(&small, 10).unwrap(), &python[..10]);
        assert!(decompress_head(&small, python.len() + 1).is_err());
        let large = python.repeat(3000);
        let mut joined = Vec::new();
        decompress_chunks(&compress(&large, 3).unwrap(), |chunk| joined.extend_from_slice(chunk)).unwrap();
        assert_eq!(joined, large);
    }
}
