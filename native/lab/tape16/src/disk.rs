//! E18 (t39 step 10): what "same structure, same bits" buys on disk, under zstd and LZ4.
//!
//! LZ compressors replace a byte run seen before by a copy of it. A tape form whose repeated subtrees are the
//! same bits gives them more to copy, but only where those bits are also the same bytes: a field of w bits
//! starts at bit `i * w`, so at a width that is not a multiple of 8 two copies of a run line up as bytes only
//! when they start at the same bit phase. Four forms, each kept as the arrays it reads in place, one stream an
//! array:
//!
//! - **F15**: format 15's image as the capture holds it (explicit columns at byte widths, its code table).
//! - **XF**: the earlier candidate: codes at the file's width, slots in per-block PFOR, starts by FOR, lengths.
//! - **new, bit-packed**: codes and slots (by sibling, K = 8) at one width a language with escapes that name
//!   nothing, starts by gap with a start every 8, lengths; widths as E16 chose them.
//! - **new, byte-aligned**: the same, every width rounded up to a multiple of 8.
//!
//! Every stream is compressed alone, file by file (zstd level 3; LZ4 fast and HC level 9; each with and without
//! a 64 KiB dictionary trained on the other half of the files), and the streams of all measured files are also
//! compressed end to end (zstd with a 128 MiB window and long-distance matching; LZ4 HC, whose window is
//! 64 KiB). For each: bytes a node, decode time a node, and the share of bytes the compressor's matches cover.
//! Every stream is decoded and checked equal to its input.

use crate::capture::File;
use crate::codec::Dictionary;
use crate::inplace::{Codes, Cols, Image, PFor, Packed};
use crate::measure::cost;
use crate::relative::{gaps, sib_slots};
use crate::repeat::{Esc, Hist};
use std::ffi::{c_char, c_int, c_void};

pub const FORMS: [&str; 4] = ["F15", "XF", "new, bit-packed", "new, byte-aligned"];
pub const STREAMS: [&[&str]; 4] = [
    &["image"],
    &["codes", "slots", "starts", "lengths"],
    &["codes", "slots", "gaps", "lengths", "checkpoints"],
    &["codes", "slots", "gaps", "lengths", "checkpoints"],
];
pub const SIB_K: usize = 8;
pub const POS_K: usize = 8;
const DICT: usize = 64 << 10;

/// How a stream is compressed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Comp {
    Zstd { dict: bool },
    /// Every measured file's stream end to end, one frame.
    ZstdLong,
    Lz4 { hc: bool, dict: bool },
    Lz4Corpus,
}

pub const COMPS: [Comp; 8] = [
    Comp::Zstd { dict: false },
    Comp::Zstd { dict: true },
    Comp::ZstdLong,
    Comp::Lz4 { hc: false, dict: false },
    Comp::Lz4 { hc: false, dict: true },
    Comp::Lz4 { hc: true, dict: false },
    Comp::Lz4 { hc: true, dict: true },
    Comp::Lz4Corpus,
];

impl Comp {
    pub fn name(self) -> &'static str {
        match self {
            Comp::Zstd { dict: false } => "zstd 3",
            Comp::Zstd { dict: true } => "zstd 3, dict",
            Comp::ZstdLong => "zstd 3 long, end to end",
            Comp::Lz4 { hc: false, dict: false } => "lz4",
            Comp::Lz4 { hc: false, dict: true } => "lz4, dict",
            Comp::Lz4 { hc: true, dict: false } => "lz4 hc 9",
            Comp::Lz4 { hc: true, dict: true } => "lz4 hc 9, dict",
            Comp::Lz4Corpus => "lz4 hc 9, end to end",
        }
    }
    fn dict(self) -> bool {
        matches!(self, Comp::Zstd { dict: true } | Comp::Lz4 { dict: true, .. })
    }
    fn corpus(self) -> bool {
        matches!(self, Comp::ZstdLong | Comp::Lz4Corpus)
    }
}

// ---- zstd, through the library's own API (dictionaries prepared once, sequences for the coverage) ----

mod z {
    use zstd_sys as s;

    pub struct Ctx {
        c: *mut s::ZSTD_CCtx,
        d: *mut s::ZSTD_DCtx,
        cd: *mut s::ZSTD_CDict,
        dd: *mut s::ZSTD_DDict,
    }

    fn ok(n: usize) -> usize {
        // SAFETY: a plain query of a return code.
        assert!(unsafe { s::ZSTD_isError(n) } == 0, "zstd: {}", unsafe { std::ffi::CStr::from_ptr(s::ZSTD_getErrorName(n)) }.to_string_lossy());
        n
    }

    impl Ctx {
        /// Level 3; `long`: a 128 MiB window and long-distance matching.
        pub fn new(dict: &[u8], long: bool) -> Ctx {
            // SAFETY: the contexts are created here and freed in Drop; the dictionary is copied by the library.
            unsafe {
                let c = s::ZSTD_createCCtx();
                let d = s::ZSTD_createDCtx();
                ok(s::ZSTD_CCtx_setParameter(c, s::ZSTD_cParameter::ZSTD_c_compressionLevel, 3));
                if long {
                    ok(s::ZSTD_CCtx_setParameter(c, s::ZSTD_cParameter::ZSTD_c_windowLog, 27));
                    ok(s::ZSTD_CCtx_setParameter(c, s::ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching, 1));
                    ok(s::ZSTD_DCtx_setParameter(d, s::ZSTD_dParameter::ZSTD_d_windowLogMax, 27));
                }
                let (mut cd, mut dd) = (std::ptr::null_mut(), std::ptr::null_mut());
                if !dict.is_empty() {
                    cd = s::ZSTD_createCDict(dict.as_ptr().cast(), dict.len(), 3);
                    dd = s::ZSTD_createDDict(dict.as_ptr().cast(), dict.len());
                    ok(s::ZSTD_CCtx_refCDict(c, cd));
                    ok(s::ZSTD_DCtx_refDDict(d, dd));
                }
                Ctx { c, d, cd, dd }
            }
        }
        pub fn compress(&mut self, data: &[u8]) -> Vec<u8> {
            // SAFETY: the output buffer is the bound the library states for this input.
            unsafe {
                let cap = s::ZSTD_compressBound(data.len());
                let mut out = vec![0u8; cap];
                let n = ok(s::ZSTD_compress2(self.c, out.as_mut_ptr().cast(), cap, data.as_ptr().cast(), data.len()));
                out.truncate(n);
                out
            }
        }
        pub fn decompress(&mut self, c: &[u8], out: &mut [u8]) -> usize {
            // SAFETY: `out` is writable for its length, which the library is told.
            unsafe { ok(s::ZSTD_decompressDCtx(self.d, out.as_mut_ptr().cast(), out.len(), c.as_ptr().cast(), c.len())) }
        }
        /// The bytes the compressor's matches cover (into the data or the dictionary).
        pub fn matched(&mut self, data: &[u8]) -> u64 {
            if data.is_empty() {
                return 0;
            }
            // SAFETY: the sequence buffer is the bound the library states for this input.
            unsafe {
                let cap = s::ZSTD_sequenceBound(data.len());
                let mut seqs = vec![s::ZSTD_Sequence { offset: 0, litLength: 0, matchLength: 0, rep: 0 }; cap];
                let n = ok(s::ZSTD_generateSequences(self.c, seqs.as_mut_ptr(), cap, data.as_ptr().cast(), data.len()));
                seqs[..n].iter().map(|q| u64::from(q.matchLength)).sum()
            }
        }
    }

    impl Drop for Ctx {
        fn drop(&mut self) {
            // SAFETY: each was created in `new` (a null dictionary is a no-op to free).
            unsafe {
                s::ZSTD_freeCCtx(self.c);
                s::ZSTD_freeDCtx(self.d);
                s::ZSTD_freeCDict(self.cd);
                s::ZSTD_freeDDict(self.dd);
            }
        }
    }
}

// ---- LZ4, the reference library (block format, as GPU decoders read it) ----

use lz4_sys as _;

unsafe extern "C" {
    fn LZ4_compressBound(n: c_int) -> c_int;
    fn LZ4_createStream() -> *mut c_void;
    fn LZ4_freeStream(s: *mut c_void) -> c_int;
    fn LZ4_resetStream_fast(s: *mut c_void);
    fn LZ4_loadDict(s: *mut c_void, dict: *const c_char, n: c_int) -> c_int;
    fn LZ4_compress_fast_continue(s: *mut c_void, src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int, accel: c_int) -> c_int;
    fn LZ4_createStreamHC() -> *mut c_void;
    fn LZ4_freeStreamHC(s: *mut c_void) -> c_int;
    fn LZ4_resetStreamHC_fast(s: *mut c_void, level: c_int);
    fn LZ4_loadDictHC(s: *mut c_void, dict: *const c_char, n: c_int) -> c_int;
    fn LZ4_compress_HC_continue(s: *mut c_void, src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int) -> c_int;
    fn LZ4_decompress_safe_usingDict(src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int, dict: *const c_char, dict_n: c_int) -> c_int;
}

struct Lz4 {
    s: *mut c_void,
    hc: bool,
    dict: Vec<u8>,
}

impl Lz4 {
    fn new(hc: bool, dict: &[u8]) -> Lz4 {
        // LZ4 reads at most the last 64 KiB of a dictionary.
        let dict = dict[dict.len().saturating_sub(64 << 10)..].to_vec();
        // SAFETY: freed in Drop.
        let s = unsafe { if hc { LZ4_createStreamHC() } else { LZ4_createStream() } };
        Lz4 { s, hc, dict }
    }
    fn compress(&mut self, data: &[u8]) -> Vec<u8> {
        // SAFETY: the stream is reset and given the dictionary (which outlives the call) before each block;
        // the output buffer is the library's bound.
        unsafe {
            let cap = LZ4_compressBound(data.len() as c_int);
            let mut out = vec![0u8; cap as usize];
            let (src, dst, n) = (data.as_ptr().cast(), out.as_mut_ptr().cast(), data.len() as c_int);
            let (d, dn) = (self.dict.as_ptr().cast(), self.dict.len() as c_int);
            let got = if self.hc {
                LZ4_resetStreamHC_fast(self.s, 9);
                LZ4_loadDictHC(self.s, d, dn);
                LZ4_compress_HC_continue(self.s, src, dst, n, cap)
            } else {
                LZ4_resetStream_fast(self.s);
                LZ4_loadDict(self.s, d, dn);
                LZ4_compress_fast_continue(self.s, src, dst, n, cap, 1)
            };
            assert!(got > 0 || data.is_empty(), "lz4 compressed nothing");
            out.truncate(got.max(0) as usize);
            out
        }
    }
    fn decompress(&self, c: &[u8], out: &mut [u8]) -> usize {
        // SAFETY: `out` is writable for its length, which the library is told.
        let n = unsafe {
            LZ4_decompress_safe_usingDict(c.as_ptr().cast(), out.as_mut_ptr().cast(), c.len() as c_int, out.len() as c_int, self.dict.as_ptr().cast(), self.dict.len() as c_int)
        };
        assert!(n >= 0, "lz4: a corrupt block");
        n as usize
    }
}

impl Drop for Lz4 {
    fn drop(&mut self) {
        // SAFETY: created in `new`.
        unsafe {
            if self.hc { LZ4_freeStreamHC(self.s) } else { LZ4_freeStream(self.s) };
        }
    }
}

/// The bytes an LZ4 block's matches cover, read off its sequences.
fn lz4_matched(c: &[u8]) -> u64 {
    let (mut at, mut matched) = (0usize, 0u64);
    let ext = |at: &mut usize, mut n: usize| {
        loop {
            let b = c[*at];
            *at += 1;
            n += b as usize;
            if b != 255 {
                return n;
            }
        }
    };
    while at < c.len() {
        let token = c[at];
        at += 1;
        let mut lit = (token >> 4) as usize;
        if lit == 15 {
            lit = ext(&mut at, lit);
        }
        at += lit;
        if at >= c.len() {
            break;
        }
        at += 2;
        let mut m = (token & 15) as usize;
        if m == 15 {
            m = ext(&mut at, m);
        }
        matched += (m + 4) as u64;
    }
    matched
}

// ---- the forms ----

/// A file's streams in every form.
type Streams = [Vec<Vec<u8>>; 4];

fn img(x: &impl Image) -> Vec<u8> {
    let mut out = Vec::new();
    x.image(&mut out);
    out
}

/// The language's widths for the new form: codes, slots, gaps, lengths.
#[derive(Clone, Copy)]
struct Widths([u32; 4]);

fn streams(f: &File, c: &Cols, w: Widths) -> Streams {
    let n = c.len();
    let xf_slot: Vec<u64> = (0..n).map(|i| {
        if c.size[i] > 1 {
            u64::from(c.size[i]) << 1
        } else if i + 1 < n {
            (((i + 1 - c.parent[i + 1] as usize) as u64) << 1) | 1
        } else {
            1
        }
    }).collect();
    let (gap, len) = gaps(c);
    let xf = vec![img(&Packed::new(&c.code)), img(&PFor::new(&xf_slot, false)), img(&PFor::new(&c.start, true)), img(&PFor::new(&len, false))];
    let sib = sib_slots(c, SIB_K);
    let ckpt: Vec<u64> = c.start.iter().step_by(POS_K).copied().collect();
    let new = |round: bool| {
        let r = |x: u32| if round { x.next_multiple_of(8) } else { x };
        let ck_w = ckpt.iter().map(|&v| crate::inplace::bits_for(v)).max().unwrap_or(0).max(1);
        vec![
            img(&Packed::new_width(&c.code, r(w.0[0]))),
            img(&Esc::new(&sib, r(w.0[1]))),
            img(&Esc::new(&gap, r(w.0[2]))),
            img(&Esc::new(&len, r(w.0[3]))),
            img(&Packed::new_width(&ckpt, r(ck_w))),
        ]
    };
    [vec![f.image.clone()], xf, new(false), new(true)]
}

// ---- the run ----

#[derive(Clone, Default)]
pub struct Cell {
    /// Compressed bytes, per stream.
    pub bytes: Vec<u64>,
    pub raw: u64,
    pub matched: Option<u64>,
    pub decode_ns: u64,
}

pub struct Report {
    pub files: usize,
    pub measured_files: usize,
    pub nodes: u64,
    pub widths: [u32; 4],
    /// In-memory bytes per form and stream, over the measured files.
    pub memory: [Vec<u64>; 4],
    /// `cells[comp][form]`; `None` where the language has too few files for a dictionary.
    pub cells: Vec<[Option<Cell>; 4]>,
    pub wrong: u64,
}

/// Over one language's files.
pub fn run(files: &[&File]) -> Report {
    let read = |f: &File| crate::v15::read(&f.image).unwrap_or_else(|e| panic!("{}: {e}", f.path));
    let mut codes = Codes::default();
    for f in files {
        codes.learn(&read(f));
    }
    // The widths of the new form, over every file of the language (E16's choice).
    let (mut hs, mut hg, mut hl) = (Hist::default(), Hist::default(), Hist::default());
    for f in files {
        let c = Cols::of_tree(&read(f), &codes);
        hs.add(sib_slots(&c, SIB_K).into_iter());
        let (g, l) = gaps(&c);
        hg.add(g.into_iter());
        hl.add(l.into_iter());
    }
    let widths = [codes.width(), hs.width(1.0), hg.width(1.0), hl.width(1.0)];
    // Dictionaries are trained on the even files and measured on the odd; with fewer than 16 files, no
    // dictionary, and every file measured.
    let dicts = files.len() >= 16;
    let mut train: Vec<Streams> = Vec::new();
    let mut test: Vec<Streams> = Vec::new();
    let mut nodes = 0u64;
    for (i, f) in files.iter().enumerate() {
        let c = Cols::of_tree(&read(f), &codes);
        let s = streams(f, &c, Widths(widths));
        if dicts && i % 2 == 0 {
            train.push(s);
        } else {
            nodes += c.len() as u64;
            test.push(s);
        }
    }
    let mut memory: [Vec<u64>; 4] = std::array::from_fn(|f| vec![0; STREAMS[f].len()]);
    for s in &test {
        for f in 0..4 {
            for (q, b) in s[f].iter().enumerate() {
                memory[f][q] += b.len() as u64;
            }
        }
    }
    // A dictionary per form and stream, trained on at most 16 MiB of samples.
    let dict: Vec<Vec<Dictionary>> = (0..4).map(|f| (0..STREAMS[f].len()).map(|q| {
        if !dicts {
            return Dictionary::none();
        }
        let mut samples = Vec::new();
        let mut total = 0;
        for s in &train {
            if total > 16 << 20 {
                break;
            }
            total += s[f][q].len();
            samples.push(s[f][q].clone());
        }
        Dictionary::train(&samples, DICT)
    }).collect()).collect();
    drop(train);
    let mut wrong = 0u64;
    let mut cells = Vec::new();
    for comp in COMPS {
        let mut row: [Option<Cell>; 4] = Default::default();
        if comp.dict() && !dicts {
            cells.push(row);
            continue;
        }
        for f in 0..4 {
            let mut cell = Cell { bytes: vec![0; STREAMS[f].len()], raw: memory[f].iter().sum(), matched: Some(0), decode_ns: 0 };
            for q in 0..STREAMS[f].len() {
                let d: &[u8] = if comp.dict() { &dict[f][q].bytes } else { &[] };
                // The inputs: each measured file's stream, or all of them end to end.
                let inputs: Vec<Vec<u8>> = if comp.corpus() {
                    vec![test.iter().flat_map(|s| s[f][q].iter().copied()).collect()]
                } else {
                    test.iter().map(|s| s[f][q].clone()).collect()
                };
                let (packed, matched): (Vec<Vec<u8>>, Option<u64>) = match comp {
                    Comp::Zstd { .. } | Comp::ZstdLong => {
                        let mut z = z::Ctx::new(d, comp == Comp::ZstdLong);
                        let packed = inputs.iter().map(|x| z.compress(x)).collect();
                        // Sequences are collected without long-distance matching's window, so not for "long".
                        let matched = (comp != Comp::ZstdLong).then(|| inputs.iter().map(|x| z.matched(x)).sum());
                        (packed, matched)
                    }
                    Comp::Lz4 { .. } | Comp::Lz4Corpus => {
                        let hc = matches!(comp, Comp::Lz4 { hc: true, .. } | Comp::Lz4Corpus);
                        let mut l = Lz4::new(hc, d);
                        let packed: Vec<Vec<u8>> = inputs.iter().map(|x| l.compress(x)).collect();
                        let matched = packed.iter().map(|p| lz4_matched(p)).sum();
                        (packed, Some(matched))
                    }
                };
                cell.bytes[q] = packed.iter().map(|p| p.len() as u64).sum();
                cell.matched = cell.matched.zip(matched).map(|(a, b)| a + b);
                // Decoded, checked, and timed (best of three, one thread).
                let mut out: Vec<Vec<u8>> = inputs.iter().map(|x| vec![0u8; x.len()]).collect();
                let mut best = u64::MAX;
                for _ in 0..3 {
                    let ((), t) = match comp {
                        Comp::Zstd { .. } | Comp::ZstdLong => {
                            let mut z = z::Ctx::new(d, comp == Comp::ZstdLong);
                            cost(|| for (p, o) in packed.iter().zip(&mut out) {
                                assert_eq!(z.decompress(p, o), o.len());
                            })
                        }
                        _ => {
                            let l = Lz4::new(true, d);
                            cost(|| for (p, o) in packed.iter().zip(&mut out) {
                                assert_eq!(l.decompress(p, o), o.len());
                            })
                        }
                    };
                    best = best.min(t.nanos);
                }
                cell.decode_ns += best;
                wrong += inputs.iter().zip(&out).filter(|(a, b)| a != b).count() as u64;
            }
            row[f] = Some(cell);
        }
        cells.push(row);
    }
    Report { files: files.len(), measured_files: test.len(), nodes, widths, memory, cells, wrong }
}

pub fn print(title: &str, r: &Report) {
    let per = |b: u64| b as f64 / r.nodes.max(1) as f64;
    println!("### {title}\n");
    println!(
        "{} files, {} measured ({} nodes){}; new form's widths: codes {}, slots {}, gaps {}, lengths {} bits. Answers wrong: {}.\n",
        r.files,
        r.measured_files,
        r.nodes,
        if r.measured_files < r.files { ", dictionaries trained on the others" } else { ", too few for a dictionary" },
        r.widths[0], r.widths[1], r.widths[2], r.widths[3],
        r.wrong
    );
    let head = |what: &str| {
        println!("| {what} | memory | {} |", COMPS.iter().map(|c| c.name()).collect::<Vec<_>>().join(" | "));
        println!("|---|---|{}", "---|".repeat(COMPS.len()));
    };
    head("B a node");
    for f in 0..4 {
        let cells: Vec<String> = r.cells.iter().map(|row| row[f].as_ref().map_or("—".into(), |c| format!("{:.3}", per(c.bytes.iter().sum())))).collect();
        println!("| {} | {:.3} | {} |", FORMS[f], per(r.memory[f].iter().sum()), cells.join(" | "));
    }
    println!();
    head("bytes the matches cover");
    for f in 0..4 {
        let cells: Vec<String> = r.cells.iter().map(|row| row[f].as_ref().and_then(|c| c.matched.map(|m| format!("{:.0}%", 100.0 * m as f64 / c.raw.max(1) as f64))).unwrap_or("—".into())).collect();
        println!("| {} | | {} |", FORMS[f], cells.join(" | "));
    }
    println!();
    head("decode, ns a node");
    for f in 0..4 {
        let cells: Vec<String> = r.cells.iter().map(|row| row[f].as_ref().map_or("—".into(), |c| format!("{:.2}", c.decode_ns as f64 / r.nodes.max(1) as f64))).collect();
        println!("| {} | | {} |", FORMS[f], cells.join(" | "));
    }
    println!();
    // Per stream, for the per-file codecs with a dictionary (or without, if there is none).
    let pick: Vec<usize> = if r.measured_files < r.files { vec![1, 6] } else { vec![0, 5] };
    println!("| stream, B a node | memory | {} |", pick.iter().map(|&i| COMPS[i].name()).collect::<Vec<_>>().join(" | "));
    println!("|---|---|{}", "---|".repeat(pick.len()));
    for f in 1..4 {
        for (q, s) in STREAMS[f].iter().enumerate() {
            let cells: Vec<String> = pick.iter().map(|&i| r.cells[i][f].as_ref().map_or("—".into(), |c| format!("{:.3}", per(c.bytes[q])))).collect();
            println!("| {}: {s} | {:.3} | {} |", FORMS[f], per(r.memory[f][q]), cells.join(" | "));
        }
    }
    println!();
}
