//! The codecs T1 compares, over a block's laid-out bytes, with an optional dictionary (T4).

use crate::lp::{self, Dict, Refs};
use crate::span::{self, Ptr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    Raw,
    /// zstd at level 3, the store's.
    Zstd,
    /// LZ with references into decoded output (LZ4's rule), checkpoints every K.
    LzOutput,
    /// Literal-pool LZ: references into literals only, checkpoints every K.
    LpLz,
    /// Literal-pool LZ that may also replay earlier sequences (pointers into the compressed block).
    LpReplay,
    /// Span LZ (`span`): copies of runs of compressed sequences, nested to `depth`, reaching earlier blocks
    /// of the file when `cross`, its pointers in the form `ptr`.
    LpSpan { depth: u8, cross: bool, ptr: Ptr },
}

impl Codec {
    pub fn name(self) -> String {
        match self {
            Codec::Raw => "raw".into(),
            Codec::Zstd => "zstd".into(),
            Codec::LzOutput => "lz-out".into(),
            Codec::LpLz => "lp-lz".into(),
            Codec::LpReplay => "lp-replay".into(),
            Codec::LpSpan { depth, cross, ptr } => format!("span{depth}{}{}", if cross { "x" } else { "" }, ptr.name()),
        }
    }
    pub const ALL: [Codec; 8] = [
        Codec::Raw,
        Codec::Zstd,
        Codec::LzOutput,
        Codec::LpLz,
        Codec::LpReplay,
        Codec::span(1, false),
        Codec::span(4, false),
        Codec::span(4, true),
    ];
    /// Span LZ with the first pointer form.
    pub const fn span(depth: u8, cross: bool) -> Codec {
        Codec::LpSpan { depth, cross, ptr: Ptr::Full }
    }
    /// Whether one byte can be read without decoding the block before it.
    pub fn random_access(self) -> bool {
        matches!(self, Codec::Raw | Codec::LpLz | Codec::LpReplay | Codec::LpSpan { .. })
    }
    /// Whether the codec keeps checkpoints every K sequences (and so has a K).
    pub fn has_k(self) -> bool {
        matches!(self, Codec::LzOutput | Codec::LpLz | Codec::LpReplay | Codec::LpSpan { .. })
    }
}

/// A language's dictionary, for every codec: zstd's (trained) and the pool (the same bytes, indexed).
pub struct Dictionary {
    pub bytes: Vec<u8>,
    pub zstd_c: Option<zstd::dict::EncoderDictionary<'static>>,
    pub zstd_d: Option<zstd::dict::DecoderDictionary<'static>>,
    pub pool: Dict,
}

impl Dictionary {
    pub fn none() -> Self {
        Dictionary { bytes: Vec::new(), zstd_c: None, zstd_d: None, pool: Dict::empty() }
    }

    pub fn new(bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return Self::none();
        }
        Dictionary {
            zstd_c: Some(zstd::dict::EncoderDictionary::copy(&bytes, 3)),
            zstd_d: Some(zstd::dict::DecoderDictionary::copy(&bytes)),
            pool: Dict::new(bytes.clone()),
            bytes,
        }
    }

    /// Trained on `samples` by fastCover (zstd's `--train`: d 8, f 20, k optimized over 4 steps), `size` bytes.
    /// Its content is the pool's too.
    pub fn train(samples: &[Vec<u8>], size: usize) -> Self {
        if size == 0 || samples.len() < 8 {
            return Self::none();
        }
        let flat: Vec<u8> = samples.concat();
        let sizes: Vec<usize> = samples.iter().map(Vec::len).collect();
        let mut dict = vec![0u8; size];
        // SAFETY: zeroed parameters are the library's defaults; the buffers are as long as stated.
        let mut params: zstd_sys::ZDICT_fastCover_params_t = unsafe { std::mem::zeroed() };
        (params.d, params.f, params.steps, params.nbThreads, params.accel) = (8, 20, 4, 4, 1);
        params.zParams.compressionLevel = 3;
        let n = unsafe {
            zstd_sys::ZDICT_optimizeTrainFromBuffer_fastCover(
                dict.as_mut_ptr().cast(),
                dict.len(),
                flat.as_ptr().cast(),
                sizes.as_ptr(),
                sizes.len() as u32,
                &mut params,
            )
        };
        if unsafe { zstd_sys::ZDICT_isError(n) } != 0 {
            eprintln!("dictionary training ({} samples, {size} B) failed", samples.len());
            return Self::none();
        }
        dict.truncate(n);
        Self::new(dict)
    }
}

pub fn compress(codec: Codec, data: &[u8], dict: &Dictionary, k: usize) -> Vec<u8> {
    match codec {
        Codec::Raw => data.to_vec(),
        Codec::Zstd => match &dict.zstd_c {
            Some(d) => {
                let mut c = zstd::bulk::Compressor::with_prepared_dictionary(d).expect("zstd");
                c.compress(data).expect("zstd")
            }
            None => zstd::bulk::compress(data, 3).expect("zstd"),
        },
        Codec::LzOutput => lp::compress(data, &dict.pool, Refs::Output, k),
        Codec::LpLz => lp::compress(data, &dict.pool, Refs::Literals, k),
        Codec::LpReplay => lp::compress(data, &dict.pool, Refs::Replay, k),
        Codec::LpSpan { depth, ptr, .. } => span::compress_file(&[data.to_vec()], &dict.pool, k, depth, false, ptr).remove(0),
    }
}

pub fn decompress(codec: Codec, c: &[u8], dict: &Dictionary, len_hint: usize) -> Vec<u8> {
    match codec {
        Codec::Raw => c.to_vec(),
        Codec::Zstd => match &dict.zstd_d {
            Some(d) => {
                let mut z = zstd::bulk::Decompressor::with_prepared_dictionary(d).expect("zstd");
                z.decompress(c, len_hint).expect("zstd")
            }
            None => zstd::bulk::decompress(c, len_hint).expect("zstd"),
        },
        Codec::LzOutput => lp::decompress(c, &dict.pool, Refs::Output),
        Codec::LpLz => lp::decompress(c, &dict.pool, Refs::Literals),
        Codec::LpReplay => lp::decompress(c, &dict.pool, Refs::Replay),
        Codec::LpSpan { ptr, .. } => span::decompress(&[c.to_vec()], 0, &dict.pool, ptr),
    }
}

/// `len` bytes at `offset`, the cheapest way the codec allows.
pub fn read_at(codec: Codec, c: &[u8], dict: &Dictionary, len_hint: usize, offset: usize, len: usize, out: &mut Vec<u8>) {
    match codec {
        Codec::Raw => {
            out.clear();
            out.extend_from_slice(&c[offset..offset + len]);
        }
        Codec::LpLz | Codec::LpReplay => lp::read_at(c, &dict.pool, offset, len, out),
        Codec::LpSpan { ptr, .. } => span::read_at(&[c.to_vec()], 0, &dict.pool, ptr, offset, len, out),
        _ => {
            let all = decompress(codec, c, dict, len_hint);
            out.clear();
            out.extend_from_slice(&all[offset..offset + len]);
        }
    }
}

/// A file's blocks compressed in order: for span LZ with `cross`, a block may copy runs of an earlier
/// block's compressed sequences; every other codec compresses each block alone.
pub fn compress_file(codec: Codec, blocks: &[Vec<u8>], dict: &Dictionary, k: usize) -> Vec<Vec<u8>> {
    match codec {
        Codec::LpSpan { depth, cross, ptr } => span::compress_file(blocks, &dict.pool, k, depth, cross, ptr),
        _ => blocks.iter().map(|b| compress(codec, b, dict, k)).collect(),
    }
}

/// Block `b` of a file compressed by `compress_file`.
pub fn decompress_in(codec: Codec, blocks: &[Vec<u8>], b: usize, dict: &Dictionary, len_hint: usize) -> Vec<u8> {
    match codec {
        Codec::LpSpan { ptr, .. } => span::decompress(blocks, b, &dict.pool, ptr),
        _ => decompress(codec, &blocks[b], dict, len_hint),
    }
}

/// `len` bytes at `offset` of block `b` of a file compressed by `compress_file`.
#[allow(clippy::too_many_arguments)]
pub fn read_at_in(codec: Codec, blocks: &[Vec<u8>], b: usize, dict: &Dictionary, len_hint: usize, offset: usize, len: usize, out: &mut Vec<u8>) {
    match codec {
        Codec::LpSpan { ptr, .. } => span::read_at(blocks, b, &dict.pool, ptr, offset, len, out),
        _ => read_at(codec, &blocks[b], dict, len_hint, offset, len, out),
    }
}

/// Every block of a file compressed by `compress_file`, in order (span LZ shares what it learns of earlier
/// blocks).
pub fn decompress_file(codec: Codec, blocks: &[Vec<u8>], dict: &Dictionary, lens: &[usize]) -> Vec<Vec<u8>> {
    match codec {
        Codec::LpSpan { ptr, .. } => span::decode_file(&mut span::sources(blocks), &dict.pool, ptr),
        _ => (0..blocks.len()).map(|b| decompress_in(codec, blocks, b, dict, lens[b])).collect(),
    }
}
