//! The Neural Engine's model writer: a bit product's template as a Core ML ML Program package, written without
//! coremltools.
//!
//! This file is shared: the crate uses it as `ane::package`, and the build step (`build/ane.rs`) includes it with
//! `#[path]` to write the ahead-of-time models with the same code. So it uses `std` only.
//!
//! **The template.** A bit product `popcount(a[i] & b[j])` is a matrix product of 0/1 values: `a`'s rows as an fp16
//! input `x` of `m` rows by `k` columns, against `b`'s rows as a constant `W` of `k` rows by `n` columns, `W[c][j]`
//! being bit `c` of `b`'s row `j`. The constant is stored with 1 bit per entry (a palette of two values, 0 and 1),
//! which the Neural Engine keeps compressed and expands on chip. The shared dimension is split into **pieces**:
//!
//! - a piece is at most 16,384 bits (the Neural Engine's limit for a product's shared dimension);
//! - every row of `b` has at most 2,048 ones within a piece, so every partial count is at most 2,048, the largest
//!   count fp16 holds exactly.
//!
//! Each piece is its own input `x{p}`, constant and output `y{p}` (fp16 partial counts), and the host sums the
//! pieces exactly. The template depends only on the constant's shape and content (its pieces) and the tile's rows
//! `m`, never on a case.
//!
//! **The package.** A directory: `Manifest.json`, `Data/com.apple.CoreML/model.mlmodel` (the `Model` protobuf,
//! schema from Apple's coremltools `mlmodel/format/Model.proto` and `MIL.proto`, BSD-3), and
//! `Data/com.apple.CoreML/weights/weight.bin` (the constants). The protobuf is written by the small encoder below.
//! The output equals coremltools 9.0's for the same graph, byte for byte (the test `writer_matches_coremltools`).

#![allow(dead_code)]

use std::path::Path;

/// The largest count fp16 holds exactly on the Neural Engine (its products round above it).
pub const EXACT: u32 = 2048;
/// The Neural Engine's limit on a product's shared dimension, in bits.
pub const MAX_K: usize = 16384;
/// The most pieces a template takes: each piece's partial counts are an output as large as the whole result.
pub const MAX_PIECES: usize = 8;
/// The rows of `a` in one prediction: a wave's rows against one constant are cut into tiles of this many.
pub const TILE_ROWS: usize = 2048;
/// The writer's version, part of every model's key: a change to what is written changes the key.
pub const WRITER_VERSION: u32 = 1;

/// A bit product's template: tile rows, constant rows, and the pieces of the shared dimension as word ranges.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Template {
    pub m: usize,
    pub n: usize,
    pub pieces: Vec<(usize, usize)>,
}

impl Template {
    /// The bits of piece `p`.
    pub fn k(&self, p: usize) -> usize {
        (self.pieces[p].1 - self.pieces[p].0) * 64
    }

    /// Bytes that identify the template, for a model's key.
    pub fn encode(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"rerail-ane bit product");
        v.extend_from_slice(&WRITER_VERSION.to_le_bytes());
        for x in [self.m, self.n, self.pieces.len()] {
            v.extend_from_slice(&(x as u64).to_le_bytes());
        }
        for &(a, b) in &self.pieces {
            v.extend_from_slice(&(a as u64).to_le_bytes());
            v.extend_from_slice(&(b as u64).to_le_bytes());
        }
        v
    }

    /// Floating-point operations of one prediction, counted as dense.
    pub fn flops(&self) -> f64 {
        let k: usize = (0..self.pieces.len()).map(|p| self.k(p)).sum();
        2.0 * self.m as f64 * k as f64 * self.n as f64
    }
}

/// Plans the pieces of `b` (`rows` rows of `words` words, packed as in `crate::kernel::Bits`): greedily, each piece
/// as many words as keep it within [`MAX_K`] bits and every row's ones within it at most [`EXACT`]. A row's count
/// against any `a` is at most its ones, so every partial count is exact. `Err` when more than [`MAX_PIECES`] are
/// needed.
pub fn plan(rows: usize, words: usize, data: &[u64], m: usize) -> Result<Template, String> {
    assert_eq!(data.len(), rows * words);
    let mut pieces = Vec::new();
    let mut ones = vec![0u32; rows];
    let mut start = 0;
    for w in 0..words {
        let fits = (w + 1 - start) * 64 <= MAX_K && (0..rows).all(|r| ones[r] + data[r * words + w].count_ones() <= EXACT);
        if !fits {
            pieces.push((start, w));
            start = w;
            ones.iter_mut().for_each(|o| *o = 0);
        }
        for r in 0..rows {
            ones[r] += data[r * words + w].count_ones();
        }
    }
    if words > 0 {
        pieces.push((start, words));
    }
    if pieces.len() > MAX_PIECES {
        return Err(format!(
            "the constant needs {} pieces of at most {MAX_K} bits with at most {EXACT} ones per row (at most {MAX_PIECES})",
            pieces.len()
        ));
    }
    Ok(Template { m, n: rows, pieces })
}

// ---------------------------------------------------------------------------------------------------------------
// Protobuf: the wire format's few pieces the Model needs (varints, length-delimited fields, packed scalars).

/// A message being written: fields appended in order.
#[derive(Default)]
pub struct Pb(pub Vec<u8>);

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

impl Pb {
    fn key(&mut self, field: u32, wire: u8) {
        varint(&mut self.0, (field as u64) << 3 | wire as u64);
    }
    /// An integer or bool field (varint). Proto3 omits a zero.
    pub fn int(mut self, field: u32, v: u64) -> Self {
        if v != 0 {
            self.key(field, 0);
            varint(&mut self.0, v);
        }
        self
    }
    pub fn bytes(mut self, field: u32, v: &[u8]) -> Self {
        self.key(field, 2);
        varint(&mut self.0, v.len() as u64);
        self.0.extend_from_slice(v);
        self
    }
    pub fn str(self, field: u32, v: &str) -> Self {
        self.bytes(field, v.as_bytes())
    }
    pub fn msg(self, field: u32, m: Pb) -> Self {
        self.bytes(field, &m.0)
    }
    /// A packed repeated varint field.
    pub fn packed(self, field: u32, vs: &[u64]) -> Self {
        let mut b = Vec::new();
        for &v in vs {
            varint(&mut b, v);
        }
        self.bytes(field, &b)
    }
    /// A map entry (`key` 1, `value` 2), in field `field`.
    pub fn entry(self, field: u32, key: &str, value: Pb) -> Self {
        self.msg(field, Pb::default().str(1, key).msg(2, value))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The Model and its MIL program.

/// MIL data types (`MIL.proto` `DataType`).
const MIL_BOOL: u64 = 1;
const MIL_STRING: u64 = 2;
const MIL_FP16: u64 = 10;
const MIL_UINT1: u64 = 37;
/// The feature description's multi-array type (`FeatureTypes.proto` `ArrayDataType.FLOAT16`).
const ARRAY_FP16: u64 = 65552;
/// The weight blob's storage types.
const BLOB_UINT1: u32 = 9;
/// Core ML 8's specification version and opset (macOS 15): the first with `constexpr_lut_to_dense`'s 1-bit form.
const SPEC_VERSION: u64 = 9;
const OPSET: &str = "CoreML8";
const WEIGHTS: &str = "@model_path/weights/weight.bin";

/// `ValueType` of a tensor.
fn tensor_type(dtype: u64, dims: &[usize]) -> Pb {
    let mut t = Pb::default().int(1, dtype).int(2, dims.len() as u64);
    for &d in dims {
        t = t.msg(3, Pb::default().msg(1, Pb::default().int(1, d as u64)));
    }
    Pb::default().msg(1, t)
}

fn named(name: &str, ty: Pb) -> Pb {
    Pb::default().str(1, name).msg(2, ty)
}

/// A `Value` holding a scalar string (an op's `name` attribute).
fn string_value(s: &str) -> Pb {
    let t = Pb::default().msg(4, Pb::default().str(1, s));
    Pb::default().msg(2, tensor_type(MIL_STRING, &[])).msg(3, Pb::default().msg(1, t))
}

fn bool_value(b: bool) -> Pb {
    let t = Pb::default().msg(3, Pb::default().packed(1, &[b as u64]));
    Pb::default().msg(2, tensor_type(MIL_BOOL, &[])).msg(3, Pb::default().msg(1, t))
}

/// An op input bound to a named value, or to a value in place.
fn arg_name(name: &str) -> Pb {
    Pb::default().msg(1, Pb::default().str(1, name))
}

fn arg_value(v: Pb) -> Pb {
    Pb::default().msg(1, Pb::default().msg(2, v))
}

/// An operation: type, inputs (sorted by name, as coremltools' deterministic output has them), outputs, its name.
fn op(ty: &str, inputs: Vec<(&str, Pb)>, outputs: Vec<Pb>, name: &str, mut attrs: Vec<(&str, Pb)>) -> Pb {
    let mut o = Pb::default().str(1, ty);
    let mut inputs = inputs;
    inputs.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in inputs {
        o = o.entry(2, k, v);
    }
    for out in outputs {
        o = o.msg(3, out);
    }
    attrs.push(("name", string_value(name)));
    attrs.sort_by(|a, b| a.0.cmp(b.0));
    for (k, v) in attrs {
        o = o.entry(5, k, v);
    }
    o
}

/// The weight blob: its file header, then each constant after a 64-byte header of its own, 64-byte aligned.
pub struct Blob {
    pub bytes: Vec<u8>,
    count: u32,
}

impl Blob {
    pub fn new() -> Self {
        Blob { bytes: vec![0u8; 64], count: 0 }
    }

    /// Appends a constant; returns the offset its `blobFileValue` names (its header's).
    pub fn add(&mut self, dtype: u32, data: &[u8]) -> u64 {
        let at = self.bytes.len().next_multiple_of(64);
        self.bytes.resize(at, 0);
        let mut h = [0u8; 64];
        h[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        h[4..8].copy_from_slice(&dtype.to_le_bytes());
        h[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());
        h[16..24].copy_from_slice(&((at + 64) as u64).to_le_bytes());
        self.bytes.extend_from_slice(&h);
        self.bytes.extend_from_slice(data);
        self.count += 1;
        self.bytes[0..4].copy_from_slice(&self.count.to_le_bytes());
        self.bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        at as u64
    }
}

/// Piece `p`'s constant as 1-bit indices, row-major over `[k, n]`, least significant bit first.
pub fn piece_bits(t: &Template, p: usize, words: usize, data: &[u64]) -> Vec<u8> {
    let (w0, w1) = t.pieces[p];
    let (k, n) = ((w1 - w0) * 64, t.n);
    let mut out = vec![0u8; (k * n).div_ceil(8)];
    for j in 0..n {
        let row = &data[j * words + w0..j * words + w1];
        for (wi, &w) in row.iter().enumerate() {
            let mut w = w;
            while w != 0 {
                let c = wi * 64 + w.trailing_zeros() as usize;
                w &= w - 1;
                let e = c * n + j;
                out[e / 8] |= 1 << (e % 8);
            }
        }
    }
    out
}

/// The model (`model.mlmodel`) and weights (`weight.bin`) of `t` over `b` (`words` words a row, `t.n` rows).
pub fn write_model(t: &Template, words: usize, data: &[u64]) -> (Vec<u8>, Vec<u8>) {
    assert_eq!(data.len(), t.n * words);
    let mut blob = Blob::new();
    let np = t.pieces.len();
    let feature = |name: &str, dims: &[usize]| {
        let shape: Vec<u64> = dims.iter().map(|&d| d as u64).collect();
        let arr = Pb::default().packed(1, &shape).int(2, ARRAY_FP16);
        Pb::default().str(1, name).msg(3, Pb::default().msg(5, arr))
    };
    let mut desc = Pb::default();
    for p in 0..np {
        desc = desc.msg(1, feature(&format!("x{p}"), &[t.m, t.k(p)]));
    }
    for p in 0..np {
        desc = desc.msg(10, feature(&format!("y{p}"), &[t.m, t.n]));
    }
    let mut block = Pb::default();
    for p in 0..np {
        block = block.str(2, &format!("y{p}"));
    }
    let lut = {
        // fp16 0 and 1, as `[1, 1, 2, 1]`: one palette of two values for the whole tensor.
        let t = Pb::default().msg(7, Pb::default().bytes(1, &[0x00, 0x00, 0x00, 0x3c]));
        Pb::default().msg(2, tensor_type(MIL_FP16, &[1, 1, 2, 1])).msg(3, Pb::default().msg(1, t))
    };
    for p in 0..np {
        let k = t.k(p);
        let off = blob.add(BLOB_UINT1, &piece_bits(t, p, words, data));
        let (y, w, tx, ty) = (format!("y{p}"), format!("y{p}_y_0_palettized"), format!("y{p}_transpose_x_0"), format!("y{p}_transpose_y_0"));
        let indices = Pb::default()
            .msg(2, tensor_type(MIL_UINT1, &[k, t.n]))
            .msg(5, Pb::default().str(1, WEIGHTS).int(2, off));
        block = block.msg(
            3,
            op(
                "constexpr_lut_to_dense",
                vec![("indices", arg_value(indices)), ("lut", arg_value(Pb(lut.0.clone())))],
                vec![named(&w, tensor_type(MIL_FP16, &[k, t.n]))],
                &w,
                vec![],
            ),
        );
        for c in [&tx, &ty] {
            block = block.msg(3, op("const", vec![], vec![named(c, tensor_type(MIL_BOOL, &[]))], c, vec![("val", bool_value(false))]));
        }
        block = block.msg(
            3,
            op(
                "matmul",
                vec![("x", arg_name(&format!("x{p}"))), ("y", arg_name(&w)), ("transpose_x", arg_name(&tx)), ("transpose_y", arg_name(&ty))],
                vec![named(&y, tensor_type(MIL_FP16, &[t.m, t.n]))],
                &y,
                vec![],
            ),
        );
    }
    let mut func = Pb::default();
    for p in 0..np {
        func = func.msg(1, named(&format!("x{p}"), tensor_type(MIL_FP16, &[t.m, t.k(p)])));
    }
    let func = func.str(2, OPSET).entry(3, OPSET, block);
    let program = Pb::default().int(1, 1).entry(2, "main", func);
    let model = Pb::default().int(1, SPEC_VERSION).msg(2, desc).msg(502, program);
    (model.0, blob.bytes)
}

const MANIFEST: &str = r#"{
    "fileFormatVersion": "1.0.0",
    "itemInfoEntries": {
        "6A1E3F52-7D0B-4C1E-9B7A-0E5C2B8D4A11": {
            "author": "com.apple.CoreML",
            "description": "CoreML Model Specification",
            "name": "model.mlmodel",
            "path": "com.apple.CoreML/model.mlmodel"
        },
        "6A1E3F52-7D0B-4C1E-9B7A-0E5C2B8D4A12": {
            "author": "com.apple.CoreML",
            "description": "CoreML Model Weights",
            "name": "weights",
            "path": "com.apple.CoreML/weights"
        }
    },
    "rootModelIdentifier": "6A1E3F52-7D0B-4C1E-9B7A-0E5C2B8D4A11"
}
"#;

/// Writes the `.mlpackage` directory of `t` over `b` at `dir` (replacing it).
pub fn write_package(dir: &Path, t: &Template, words: usize, data: &[u64]) -> std::io::Result<()> {
    let (model, weights) = write_model(t, words, data);
    let _ = std::fs::remove_dir_all(dir);
    let ml = dir.join("Data/com.apple.CoreML");
    std::fs::create_dir_all(ml.join("weights"))?;
    std::fs::write(dir.join("Manifest.json"), MANIFEST)?;
    std::fs::write(ml.join("model.mlmodel"), model)?;
    std::fs::write(ml.join("weights/weight.bin"), weights)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Archives: a compiled model (`.mlmodelc`, a directory) as one byte string, for the model store and the executable.
//
// Layout: the magic `RRMLMC01`, the file count (u32), then per file its relative path's length (u32) and bytes
// (UTF-8, `/`-separated) and its content's length (u64) and bytes. All integers little-endian.

const ARCHIVE_MAGIC: &[u8; 8] = b"RRMLMC01";

/// The files under `dir`, as an archive.
pub fn archive(dir: &Path) -> std::io::Result<Vec<u8>> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
        let mut es: Vec<_> = std::fs::read_dir(d)?.collect::<Result<_, _>>()?;
        es.sort_by_key(|e| e.file_name());
        for e in es {
            let p = e.path();
            if e.file_type()?.is_dir() {
                walk(base, &p, out)?;
            } else {
                let rel = p.strip_prefix(base).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
                out.push((rel, std::fs::read(&p)?));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let mut v = ARCHIVE_MAGIC.to_vec();
    v.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (p, b) in files {
        v.extend_from_slice(&(p.len() as u32).to_le_bytes());
        v.extend_from_slice(p.as_bytes());
        v.extend_from_slice(&(b.len() as u64).to_le_bytes());
        v.extend_from_slice(&b);
    }
    Ok(v)
}

/// Writes an archive's files under `dir` (replacing it).
pub fn unarchive(bytes: &[u8], dir: &Path) -> Result<(), String> {
    let bad = || "a compiled model's archive is malformed".to_string();
    if bytes.len() < 12 || &bytes[..8] != ARCHIVE_MAGIC {
        return Err(bad());
    }
    let mut at = 8;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let s = bytes.get(at..at + n).ok_or_else(bad)?;
        at += n;
        Ok(s)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap());
    let _ = std::fs::remove_dir_all(dir);
    for _ in 0..count {
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let path = std::str::from_utf8(take(n)?).map_err(|_| bad())?.to_string();
        if path.split('/').any(|c| c == ".." || c.is_empty()) {
            return Err(bad());
        }
        let len = u64::from_le_bytes(take(8)?.try_into().unwrap()) as usize;
        let data = take(len)?;
        let p = dir.join(&path);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, data).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Constant files: a fixed relation's rows on disk, for the ahead-of-time manifest (`accel/models/aot.list`).
//
// Layout: the magic `RRBITS01`, rows (u64), words (u64), then the rows' words (u64 each). Little-endian.

const BITS_MAGIC: &[u8; 8] = b"RRBITS01";

/// `(rows, words, data)` of a constant file.
pub fn read_constant(bytes: &[u8]) -> Result<(usize, usize, Vec<u64>), String> {
    if bytes.len() < 24 || &bytes[..8] != BITS_MAGIC {
        return Err("not a constant file (magic RRBITS01)".into());
    }
    let u = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap()) as usize;
    let (rows, words) = (u(8), u(16));
    if bytes.len() != 24 + rows * words * 8 {
        return Err(format!("a constant file of {rows} rows of {words} words has {} bytes", bytes.len()));
    }
    Ok((rows, words, (0..rows * words).map(|i| u(24 + i * 8) as u64).collect()))
}

pub fn write_constant(rows: usize, words: usize, data: &[u64]) -> Vec<u8> {
    let mut v = BITS_MAGIC.to_vec();
    v.extend_from_slice(&(rows as u64).to_le_bytes());
    v.extend_from_slice(&(words as u64).to_le_bytes());
    for w in data {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v
}
