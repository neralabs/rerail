//! The store's zstd frames: how sources and tapes are compressed, with or without a shipped
//! dictionary, and how every frame is read back.
//!
//! - **Plain frames** are what `zstd::stream::encode_all` writes, from one compression context
//!   per thread instead of a fresh one (and its zeroed tables) per call.
//! - **Dictionary frames.** A source or tape under [`SMALL`] bytes is also compressed with its
//!   language's dictionary, and the smaller frame is kept. A small frame has little history of its
//!   own to copy from; a dictionary trained on the language supplies it (t28: 25–45% of frames
//!   under 1 KB).
//! - A dictionary frame names its dictionary in the zstd frame header (its dictionary id, two
//!   bytes), so `codec_id` stays [`super::CODEC_ZSTD`] and readers need no other column:
//!   [`decompress`] picks the dictionary from the frame.
//! - **Dictionaries are compiled in and append-only** (`native/dictionaries`). A retrained
//!   dictionary takes a new id and the old one stays, so every stored frame stays readable.
//!
//! Every reader of a store frame goes through [`decompress`], [`decompress_into`],
//! [`decompress_head`] or [`decompress_chunks`].

use std::collections::HashMap;
use std::cell::RefCell;
use std::io;
use zstd::stream::raw::{Decoder, Encoder, InBuffer, Operation, OutBuffer};

/// Frames at least this long are stored plain: a dictionary saves nothing on them, and can cost.
pub(crate) const SMALL: usize = 64 * 1024;

/// What a dictionary was trained on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Content {
    Source,
    Tape,
}

/// A shipped dictionary: its zstd dictionary id (in each frame it compressed), the languages
/// (tape tags) and content it serves, and its bytes.
struct Dictionary {
    id: u32,
    tags: &'static [u8],
    content: Content,
    bytes: &'static [u8],
}

macro_rules! dictionary {
    ($id:expr, $tags:expr, $content:ident, $file:literal) => {
        Dictionary {
            id: $id,
            tags: $tags,
            content: Content::$content,
            bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../dictionaries/", $file)),
        }
    };
}

/// Append-only: a new dictionary takes the next id, and a writer uses the last one listed for its
/// language. Tape tags: Python 1, TypeScript 2, TSX 3, JavaScript 6, PHP 7, Rust 9, SQL 10
/// (`language::tape_tag`). Trained with `zstd --train -3` on redistributable text: Rerail's own
/// files, and CPython's standard library for Python (`native/dictionaries/README.md`).
static DICTIONARIES: &[Dictionary] = &[
    dictionary!(0x8001, &[1], Source, "python-source.zdict"),
    dictionary!(0x8002, &[1], Tape, "python-tape.zdict"),
    dictionary!(0x8003, &[2, 3], Source, "typescript-source.zdict"),
    dictionary!(0x8004, &[2, 3], Tape, "typescript-tape.zdict"),
    dictionary!(0x8005, &[6], Source, "javascript-source.zdict"),
    dictionary!(0x8006, &[6], Tape, "javascript-tape.zdict"),
    dictionary!(0x8007, &[9], Source, "rust-source.zdict"),
    dictionary!(0x8008, &[9], Tape, "rust-tape.zdict"),
    dictionary!(0x8009, &[7], Source, "php-source.zdict"),
    dictionary!(0x800A, &[7], Tape, "php-tape.zdict"),
    dictionary!(0x800B, &[10], Tape, "sql-tape.zdict"),
];

/// The dictionary a writer uses for `content` of the language with tape tag `tag`.
fn dictionary_for(tag: u8, content: Content) -> Option<&'static Dictionary> {
    DICTIONARIES.iter().rev().find(|d| d.content == content && d.tags.contains(&tag))
}

fn dictionary_by_id(id: u32) -> io::Result<&'static Dictionary> {
    DICTIONARIES
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("frame names unknown dictionary {id:#x}")))
}

thread_local! {
    /// This thread's plain context, with the level it was set to.
    static ENCODER: RefCell<Option<(i32, Encoder<'static>)>> = const { RefCell::new(None) };
    /// This thread's contexts with a dictionary loaded, by dictionary id: (level, context).
    static DICT_ENCODERS: RefCell<HashMap<u32, (i32, Encoder<'static>)>> = RefCell::new(HashMap::new());
    /// This thread's decompression contexts, by dictionary id (0: none).
    static DECODERS: RefCell<HashMap<u32, Decoder<'static>>> = RefCell::new(HashMap::new());
}

/// `bytes` as one plain zstd frame at `level`, byte for byte what `encode_all` writes: streamed,
/// so the frame header carries no content size. For store-internal blobs (results, pages, memo
/// entries); sources and tapes use [`compress_source`] and [`compress_tape`].
pub fn compress(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
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

/// A source's frame. `tag` is its language's tape tag when the writer knows it (from the file's
/// path, or from its tape: [`tape_tag`]).
pub(crate) fn compress_source(bytes: &[u8], level: i32, tag: Option<u8>) -> io::Result<Vec<u8>> {
    compress_as(bytes, level, tag.and_then(|tag| dictionary_for(tag, Content::Source)))
}

/// A tape's frame: its language is in its own header.
pub(crate) fn compress_tape(bytes: &[u8], level: i32) -> io::Result<Vec<u8>> {
    compress_as(bytes, level, tape_tag(bytes).and_then(|tag| dictionary_for(tag, Content::Tape)))
}

/// How a writer finds a frame's language: the tape tag of a stored tape image, and of the language a file at a
/// path is parsed as. The engine knows both (`code_rule_dsl::tape`'s header, its languages) and sets them as it
/// loads. Before that, or without them, a writer uses no dictionary; every reader decodes either frame.
static LANGUAGE_TAGS: std::sync::OnceLock<(fn(&[u8]) -> Option<u8>, fn(&str) -> Option<u8>)> = std::sync::OnceLock::new();

/// Sets how a writer finds a frame's language (`LANGUAGE_TAGS`); the first setting stays.
pub fn set_language_tags(tape: fn(&[u8]) -> Option<u8>, path: fn(&str) -> Option<u8>) {
    let _ = LANGUAGE_TAGS.set((tape, path));
}

/// The tape tag of a stored tape image, if `bytes` is one.
pub(crate) fn tape_tag(bytes: &[u8]) -> Option<u8> {
    LANGUAGE_TAGS.get().and_then(|(tape, _)| tape(bytes))
}

/// The tape tag of the language a file at `path` is parsed as.
pub(crate) fn path_tag(path: &str) -> Option<u8> {
    LANGUAGE_TAGS.get().and_then(|(_, of_path)| of_path(path))
}

/// The tape tag of a file at `path` whose tape (if it has one) is `tape`: the tape's own, else
/// the path's.
pub(crate) fn file_tag(path: &str, tape: Option<&[u8]>) -> Option<u8> {
    tape.and_then(tape_tag).or_else(|| path_tag(path))
}

fn compress_as(bytes: &[u8], level: i32, dictionary: Option<&'static Dictionary>) -> io::Result<Vec<u8>> {
    let plain = compress(bytes, level)?;
    let Some(dictionary) = dictionary.filter(|_| bytes.len() < SMALL) else {
        return Ok(plain);
    };
    let framed = DICT_ENCODERS.with_borrow_mut(|encoders| {
        let encoder = match encoders.get_mut(&dictionary.id) {
            Some((known, encoder)) if *known == level => {
                encoder.reinit()?;
                encoder
            }
            _ => {
                encoders.insert(dictionary.id, (level, Encoder::with_dictionary(level, dictionary.bytes)?));
                &mut encoders.get_mut(&dictionary.id).expect("inserted").1
            }
        };
        let frame = encode(encoder, bytes);
        if frame.is_err() {
            encoders.remove(&dictionary.id);
        }
        frame
    })?;
    Ok(if framed.len() < plain.len() { framed } else { plain })
}

/// A store frame's bytes, plain or with the dictionary its header names. `capacity` is the
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
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated zstd frame"));
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
                    Err(io::Error::new(io::ErrorKind::InvalidData, "frame is shorter than its slot"))
                };
            }
            if full || input.pos == frame.len() && output.pos() < output.capacity() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "frame does not fit its slot"));
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
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "frame is shorter than the head"));
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
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated zstd frame"));
            }
        }
    })
}

fn finished(input: &InBuffer<'_>, frame: &[u8]) -> io::Result<()> {
    if input.pos == frame.len() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, "bytes after the zstd frame"))
    }
}

/// Runs `read` with this thread's decoder for `frame`'s dictionary, reset for a new frame.
fn with_decoder<T>(frame: &[u8], read: impl FnOnce(&mut Decoder<'static>) -> io::Result<T>) -> io::Result<T> {
    let id = zstd::zstd_safe::get_dict_id_from_frame(frame).map_or(0, |id| id.get());
    DECODERS.with_borrow_mut(|decoders| {
        let decoder = match decoders.entry(id) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                let decoder = entry.into_mut();
                decoder.reinit()?;
                decoder
            }
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(if id == 0 {
                Decoder::new()?
            } else {
                Decoder::with_dictionary(dictionary_by_id(id)?.bytes)?
            }),
        };
        let result = read(decoder);
        if result.is_err() {
            // A context that failed mid-frame is not reused.
            decoders.remove(&id);
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

    #[test]
    fn source_and_tape_frames_read_back_with_and_without_dictionaries() {
        let python = b"from __future__ import annotations\n\nimport os\n\n\ndef main() -> None:\n    print(os.getcwd())\n";
        let small = compress_source(python, 3, Some(1)).unwrap();
        assert!(small.len() < compress(python, 3).unwrap().len(), "a small Python source gains from its dictionary");
        assert_eq!(zstd::zstd_safe::get_dict_id_from_frame(&small).map(|id| id.get()), Some(0x8001));
        assert_eq!(decompress(&small, 0).unwrap(), python);
        let mut slot = vec![0; python.len()];
        decompress_into(&small, &mut slot).unwrap();
        assert_eq!(slot, python);
        assert!(decompress_into(&small, &mut vec![0; python.len() - 1]).is_err());
        assert!(decompress_into(&small, &mut vec![0; python.len() + 1]).is_err());
        // No language, a language without a dictionary, and a large input: plain frames.
        for (bytes, tag) in [(python.to_vec(), None), (python.to_vec(), Some(12)), (python.repeat(1000), Some(1))] {
            let frame = compress_source(&bytes, 3, tag).unwrap();
            assert_eq!(frame, compress(&bytes, 3).unwrap());
            assert_eq!(decompress(&frame, bytes.len()).unwrap(), bytes);
        }
        // Every shipped dictionary loads, and has a distinct id.
        let mut ids: Vec<_> = DICTIONARIES.iter().map(|d| d.id).collect();
        for d in DICTIONARIES {
            assert_eq!(zstd::zstd_safe::get_dict_id(d.bytes).map(|id| id.get()), Some(d.id));
        }
        ids.dedup();
        assert_eq!(ids.len(), DICTIONARIES.len());
        assert!(decompress(b"not a frame", 0).is_err());
        assert_eq!(decompress_head(&small, 10).unwrap(), &python[..10]);
        assert!(decompress_head(&small, python.len() + 1).is_err());
        let large = python.repeat(3000);
        let mut joined = Vec::new();
        decompress_chunks(&compress_source(&large, 3, Some(1)).unwrap(), |chunk| joined.extend_from_slice(chunk)).unwrap();
        assert_eq!(joined, large);
    }
}
