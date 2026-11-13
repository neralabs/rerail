//! The codecs T1 compares, over a block's laid-out bytes, with an optional dictionary (T4).

use crate::lp::{self, Dict, Refs};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::Raw => "raw",
            Codec::Zstd => "zstd",
            Codec::LzOutput => "lz-out",
            Codec::LpLz => "lp-lz",
            Codec::LpReplay => "lp-replay",
        }
    }
    pub const ALL: [Codec; 5] = [Codec::Raw, Codec::Zstd, Codec::LzOutput, Codec::LpLz, Codec::LpReplay];
    /// Whether one byte can be read without decoding the block before it.
    pub fn random_access(self) -> bool {
        matches!(self, Codec::Raw | Codec::LpLz | Codec::LpReplay)
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
        _ => {
            let all = decompress(codec, c, dict, len_hint);
            out.clear();
            out.extend_from_slice(&all[offset..offset + len]);
        }
    }
}
