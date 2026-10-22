//! Fact files: one per content hash, `<store>/<hash[0..2]>/<hash>.facts`, in the layout agreed
//! with Spike B (whose exported facts use the same layout, `.xfacts`):
//!
//! magic `b"RRFACTS\0"`, u32 schema_version, [u8; 32] blake3 content hash, u32 n_strings, strings
//! as (u32 len, utf8), u32 n_relations, per relation: u32 name idx, u32 n_rows, u32 n_cols, per
//! column (u32 name idx, u8 type: 0=u32 1=u16 2=u8 3=sym 4=opt_u32 5=opt_sym), then each column
//! at native width; a u16 or u8 column is followed by zero bytes up to a 4-byte file offset (the
//! 4-byte columns are not padded: the 5-byte column headers leave them at any offset). All
//! little-endian; `opt` none is `u32::MAX`.
//!
//! Spike A's relations (schema v0, syntactic part): `node` (row = node id), `identifier`,
//! `line`, `syntax_error` (one row when the tree has errors). The children CSR and kind postings
//! are derived on read (`facts::index`); node text is the source slice, never stored.
use crate::facts::{self, Facts, GRAMMAR, NONE};
use crate::repo::walk;
use crate::arg;
use rerail_engine::code_rule_dsl::lines::{LineTable, Terminators};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub const MAGIC: &[u8; 8] = b"RRFACTS\0";
pub const SCHEMA_VERSION: u32 = crate::facts::SCHEMA_VERSION;

enum Col {
    U32(Vec<u32>),
    U16(Vec<u16>),
    U8(Vec<u8>),
    Sym(Vec<u32>),
    OptU32(Vec<u32>),
    OptSym(Vec<u32>),
}

impl Col {
    fn code(&self) -> u8 {
        match self {
            Col::U32(_) => 0,
            Col::U16(_) => 1,
            Col::U8(_) => 2,
            Col::Sym(_) => 3,
            Col::OptU32(_) => 4,
            Col::OptSym(_) => 5,
        }
    }
}

#[derive(Default)]
struct Strings {
    list: Vec<String>,
    index: rustc_hash::FxHashMap<String, u32>,
}

impl Strings {
    fn sym(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(s.to_owned());
        self.index.insert(s.to_owned(), i);
        i
    }
}

/// A fact-file layout: v0 (Spike A/B's), or v1 (SCHEMA.md § 5): `kind` as a u16 and `field` as
/// a u8 file-string index, `parent`, `start_line`, `end_line` and `line` derived on read, an
/// optional zstd body. `named_only` drops anonymous nodes (with an `id` column): measured, not
/// decodable to `Facts` without a re-parse.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub version: u32,
    pub named_only: bool,
    pub zstd: Option<i32>,
}
pub const V0: Layout = Layout { version: 0, named_only: false, zstd: None };
/// The layout `store` writes (report § 5.2: every node stored, zstd level 3; 186 MB for the
/// corpus against v0's 1,048 MB, read p50 0.08 ms).
pub const V1: Layout = Layout { version: 1, named_only: false, zstd: Some(3) };

/// A file's facts in the default layout (`V1`).
pub fn encode(hash: &[u8; 32], f: &Facts) -> Vec<u8> {
    encode_with(hash, f, V1)
}

pub fn encode_with(hash: &[u8; 32], f: &Facts, layout: Layout) -> Vec<u8> {
    let mut s = Strings::default();
    let n = f.len();
    let v1 = layout.version >= 1;
    // v1: field names first (u8 index + 1), then kinds (u16 index), then identifier names.
    let mut field_map: rustc_hash::FxHashMap<u16, u32> = Default::default();
    let mut field_sym = |c: u16, s: &mut Strings| if c == 0 { NONE } else { *field_map.entry(c).or_insert_with(|| s.sym(GRAMMAR.kinds.field_name(c).unwrap_or(""))) };
    let fields: Vec<u32> = f.field.iter().map(|&c| field_sym(c, &mut s)).collect();
    let mut kind_map: rustc_hash::FxHashMap<u32, u32> = Default::default();
    let kinds: Vec<u32> = f.kind.iter().map(|&k| *kind_map.entry(k).or_insert_with(|| s.sym(&facts::name(k)))).collect();
    let identifier = facts::lookup("identifier");
    let ids: Vec<u32> = (0..n as u32).filter(|&i| Some(f.kind[i as usize]) == identifier && f.named(i)).collect();
    let id_names: Vec<u32> = ids.iter().map(|&i| s.sym(f.text(i))).collect();
    let mut rels: Vec<(&str, usize, Vec<(&str, Col)>)> = Vec::new();
    if v1 {
        assert!(s.list.len() <= u16::MAX as usize || kind_map.values().all(|&k| k <= u16::MAX as u32));
        let keep: Vec<u32> = if layout.named_only { (0..n as u32).filter(|&i| f.named(i)).collect() } else { (0..n as u32).collect() };
        let pick = |v: &[u32]| keep.iter().map(|&i| v[i as usize]).collect::<Vec<u32>>();
        let mut cols = Vec::new();
        if layout.named_only {
            cols.push(("id", Col::U32(keep.clone())));
        }
        cols.push(("kind", Col::U16(keep.iter().map(|&i| kinds[i as usize] as u16).collect())));
        cols.push(("start", Col::U32(pick(&f.start))));
        cols.push(("end", Col::U32(pick(&f.end))));
        cols.push(("subtree_end", Col::U32(pick(&f.subtree_end))));
        cols.push(("flags", Col::U8(keep.iter().map(|&i| f.flags[i as usize]).collect())));
        cols.push(("field", Col::U8(keep.iter().map(|&i| { let x = fields[i as usize]; if x == NONE { 0 } else { u8::try_from(x + 1).expect("field index") } }).collect())));
        rels.push(("node", keep.len(), cols));
        rels.push(("identifier", ids.len(), vec![("id", Col::U32(ids)), ("name", Col::Sym(id_names))]));
    } else {
        let source = f.source.as_bytes();
        let line_count = f.lines.line(source.len()) + 1;
        let (mut starts, mut ends, mut indents) = (Vec::with_capacity(line_count), Vec::with_capacity(line_count), Vec::with_capacity(line_count));
        for l in 0..line_count {
            let (a, b) = (f.lines.start(l), f.lines.end(l));
            starts.push(a as u32);
            ends.push(b as u32);
            indents.push(source[a..b].iter().take_while(|&&c| c == b' ' || c == b'\t').count() as u16);
        }
        rels.push((
            "node",
            n,
            vec![
                ("kind", Col::Sym(kinds)),
                ("parent", Col::OptU32(f.parent.clone())),
                ("start", Col::U32(f.start.clone())),
                ("end", Col::U32(f.end.clone())),
                ("start_line", Col::U32(f.start_line.clone())),
                ("end_line", Col::U32(f.end_line.clone())),
                ("subtree_end", Col::U32(f.subtree_end.clone())),
                ("flags", Col::U8(f.flags.clone())),
                ("field", Col::OptSym(fields)),
            ],
        ));
        rels.push(("identifier", ids.len(), vec![("id", Col::U32(ids)), ("name", Col::Sym(id_names))]));
        rels.push(("line", line_count, vec![("start", Col::U32(starts)), ("end", Col::U32(ends)), ("indent", Col::U16(indents))]));
    }
    rels.push(("syntax_error", f.syntax_errors as usize, vec![]));
    let rels: Vec<(u32, u32, Vec<(u32, Col)>)> = rels.into_iter().map(|(name, rows, cols)| (s.sym(name), rows as u32, cols.into_iter().map(|(c, v)| (s.sym(c), v)).collect())).collect();

    // The body (strings and relations); narrow columns pad to a 4-byte offset from the body
    // start, which is a 4-byte file offset in both layouts (44- and 48-byte headers).
    let mut body = Vec::with_capacity(64 + n * 20);
    body.extend_from_slice(&(s.list.len() as u32).to_le_bytes());
    for st in &s.list {
        body.extend_from_slice(&(st.len() as u32).to_le_bytes());
        body.extend_from_slice(st.as_bytes());
    }
    body.extend_from_slice(&(rels.len() as u32).to_le_bytes());
    for (name, rows, cols) in &rels {
        body.extend_from_slice(&name.to_le_bytes());
        body.extend_from_slice(&rows.to_le_bytes());
        body.extend_from_slice(&(cols.len() as u32).to_le_bytes());
        for (cn, col) in cols {
            body.extend_from_slice(&cn.to_le_bytes());
            body.push(col.code());
        }
        for (_, col) in cols {
            match col {
                Col::U32(v) | Col::Sym(v) | Col::OptU32(v) | Col::OptSym(v) => v.iter().for_each(|x| body.extend_from_slice(&x.to_le_bytes())),
                Col::U16(v) => v.iter().for_each(|x| body.extend_from_slice(&x.to_le_bytes())),
                Col::U8(v) => body.extend_from_slice(v),
            }
            if matches!(col, Col::U16(_) | Col::U8(_)) {
                while body.len() % 4 != 0 {
                    body.push(0);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(48 + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&layout.version.to_le_bytes());
    out.extend_from_slice(hash);
    if v1 {
        // u32 flags: bit 0 = zstd body (then u32 raw body length, then the frame).
        match layout.zstd {
            None => {
                out.extend_from_slice(&0u32.to_le_bytes());
                out.extend_from_slice(&body);
            }
            Some(level) => {
                out.extend_from_slice(&1u32.to_le_bytes());
                out.extend_from_slice(&(body.len() as u32).to_le_bytes());
                out.extend_from_slice(&zstd::bulk::compress(&body, level).expect("zstd"));
            }
        }
    } else {
        out.extend_from_slice(&body);
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or("truncated fact file")?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn pad(&mut self) {
        self.at = self.at.div_ceil(4) * 4;
    }
}

/// A relation as read: rows and columns by name, every column widened to u32.
pub struct Relation {
    pub rows: u32,
    pub cols: BTreeMap<String, (u8, Vec<u32>)>,
}

/// The header, string table and every relation of a fact file (versions 0 and 1).
pub fn read(bytes: &[u8]) -> Result<([u8; 32], Vec<String>, BTreeMap<String, Relation>), String> {
    Ok(read_versioned(bytes)?.1)
}

/// `read` with the file's schema version.
pub fn read_versioned(bytes: &[u8]) -> Result<(u32, ([u8; 32], Vec<String>, BTreeMap<String, Relation>)), String> {
    let mut h = Reader { b: bytes, at: 0 };
    if h.bytes(8)? != MAGIC {
        return Err("bad magic".into());
    }
    let version = h.u32()?;
    if version > SCHEMA_VERSION {
        return Err(format!("schema version {version}, this reader knows 0..={SCHEMA_VERSION}"));
    }
    let hash: [u8; 32] = h.bytes(32)?.try_into().unwrap();
    let decompressed;
    let body: &[u8] = if version >= 1 {
        let flags = h.u32()?;
        if flags & !1 != 0 {
            return Err(format!("unknown fact-file flags {flags:#x}"));
        }
        if flags & 1 != 0 {
            let raw = h.u32()? as usize;
            decompressed = zstd::bulk::decompress(&bytes[h.at..], raw).map_err(|e| format!("zstd: {e}"))?;
            &decompressed
        } else {
            &bytes[h.at..]
        }
    } else {
        &bytes[h.at..]
    };
    let mut r = Reader { b: body, at: 0 };
    let ns = r.u32()? as usize;
    let mut strings = Vec::with_capacity(ns);
    for _ in 0..ns {
        let l = r.u32()? as usize;
        strings.push(std::str::from_utf8(r.bytes(l)?).map_err(|e| e.to_string())?.to_owned());
    }
    let string = |i: u32| strings.get(i as usize).cloned().ok_or_else(|| format!("string {i} out of range"));
    let nr = r.u32()?;
    let mut rels = BTreeMap::new();
    for _ in 0..nr {
        let name = string(r.u32()?)?;
        let rows = r.u32()?;
        let nc = r.u32()?;
        let mut heads = Vec::with_capacity(nc as usize);
        for _ in 0..nc {
            let cn = string(r.u32()?)?;
            let ty = r.bytes(1)?[0];
            heads.push((cn, ty));
        }
        let mut cols = BTreeMap::new();
        for (cn, ty) in heads {
            let n = rows as usize;
            let v: Vec<u32> = match ty {
                0 | 3 | 4 | 5 => r.bytes(n * 4)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect(),
                1 => r.bytes(n * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes(c.try_into().unwrap()) as u32).collect(),
                2 => r.bytes(n)?.iter().map(|&b| b as u32).collect(),
                t => return Err(format!("column type {t}")),
            };
            if ty == 1 || ty == 2 {
                r.pad();
            }
            cols.insert(cn, (ty, v));
        }
        rels.insert(name, Relation { rows, cols });
    }
    Ok((version, (hash, strings, rels)))
}

/// A file's `Facts` from its fact file and its source (node text is the source slice).
pub fn decode(bytes: &[u8], source: Arc<String>) -> Result<Facts, String> {
    let (version, (hash, strings, mut rels)) = read_versioned(bytes)?;
    if *blake3::hash(source.as_bytes()).as_bytes() != hash {
        return Err("content hash does not match the source".into());
    }
    let mut node = rels.remove("node").ok_or("no `node` relation")?;
    if node.cols.contains_key("id") {
        return Err("named-only fact file: anonymous nodes need a re-parse".into());
    }
    let mut col = |name: &str| node.cols.remove(name).map(|c| c.1).ok_or_else(|| format!("node.{name} missing"));
    let mut kind_map: rustc_hash::FxHashMap<u32, u32> = Default::default();
    let kind: Vec<u32> = col("kind")?.into_iter().map(|k| *kind_map.entry(k).or_insert_with(|| facts::intern(&strings[k as usize]))).collect();
    let mut field_map: rustc_hash::FxHashMap<u32, u16> = Default::default();
    let mut field = Vec::with_capacity(kind.len());
    for x in col("field")? {
        // v0: string index or NONE; v1: string index + 1, 0 none.
        let x = if version >= 1 { if x == 0 { NONE } else { x - 1 } } else { x };
        field.push(if x == NONE {
            0
        } else {
            match field_map.get(&x) {
                Some(&c) => c,
                None => {
                    let c = GRAMMAR.field_code(&strings[x as usize]).ok_or_else(|| format!("unknown field `{}`", strings[x as usize]))?;
                    field_map.insert(x, c);
                    c
                }
            }
        });
    }
    let lines = Arc::new(LineTable::new(&source, Terminators::Newlines));
    let start = col("start")?;
    let end = col("end")?;
    let subtree_end = col("subtree_end")?;
    let (parent, start_line, end_line) = if version >= 1 {
        // Preorder with subtree ends: a node's parent is the innermost open subtree.
        let mut parent = Vec::with_capacity(kind.len());
        let mut open: Vec<u32> = Vec::new();
        for id in 0..kind.len() as u32 {
            while open.last().is_some_and(|&p| subtree_end[p as usize] <= id) {
                open.pop();
            }
            parent.push(open.last().copied().unwrap_or(NONE));
            open.push(id);
        }
        let sl = start.iter().map(|&b| lines.line(b as usize) as u32 + 1).collect();
        let el = end.iter().map(|&b| lines.line(b as usize) as u32 + 1).collect();
        (parent, sl, el)
    } else {
        (col("parent")?, col("start_line")?, col("end_line")?)
    };
    let mut f = Facts {
        lines,
        source,
        kind,
        parent,
        start,
        end,
        start_line,
        end_line,
        subtree_end,
        flags: col("flags")?.into_iter().map(|x| x as u8).collect(),
        field,
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        syntax_errors: rels.get("syntax_error").is_some_and(|r| r.rows > 0),
        tape: None,
        typedefs: Default::default(),
    };
    facts::index(&mut f);
    Ok(f)
}

/// Every column the evaluator reads, compared exactly; the first difference.
fn same(a: &Facts, b: &Facts) -> Result<(), &'static str> {
    let checks: [(&str, bool); 15] = [
        ("kind", a.kind == b.kind),
        ("parent", a.parent == b.parent),
        ("start", a.start == b.start),
        ("end", a.end == b.end),
        ("start_line", a.start_line == b.start_line),
        ("end_line", a.end_line == b.end_line),
        ("subtree_end", a.subtree_end == b.subtree_end),
        ("flags", a.flags == b.flags),
        ("field", a.field == b.field),
        ("child_offsets", a.child_offsets == b.child_offsets),
        ("children", a.children == b.children),
        ("kind_keys", a.kind_keys == b.kind_keys),
        ("kind_offsets", a.kind_offsets == b.kind_offsets),
        ("kind_nodes", a.kind_nodes == b.kind_nodes),
        ("syntax_errors", a.syntax_errors == b.syntax_errors),
    ];
    if let Some((name, _)) = checks.iter().find(|(_, ok)| !ok) {
        return Err(name);
    }
    let n = a.source.len();
    if (0..=n).any(|byte| a.lines.line(byte) != b.lines.line(byte)) {
        return Err("lines");
    }
    Ok(())
}

struct Stat {
    index: u32,
    source_bytes: u32,
    nodes: u32,
    file_bytes: u32,
    encode_ns: u64,
    decode_ns: u64,
    mismatch: Option<String>,
    hash: String,
}

/// `store`: every `.py` file's facts written to the content-addressed store, read back and
/// compared with the facts built from the source, with bytes and encode/decode times per file.
pub fn store(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let root = arg(args, "--root").unwrap_or(format!("{home}/rerail-bench/nera-data-layer-2b1504ad"));
    let root = Path::new(&root);
    let store = PathBuf::from(arg(args, "--store").unwrap_or(format!("{home}/rerail-bench/facts/store")));
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let paths = walk(root);
    let next = AtomicUsize::new(0);
    let t0 = Instant::now();
    let mut stats: Vec<Stat> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut stats = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(index) else { break };
                        let source = Arc::new(std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}")));
                        let Ok(facts::Built::Facts(f)) = facts::build(path, source.clone()) else { panic!("{path}: unparsed") };
                        let t_enc = Instant::now();
                        let hash = blake3::hash(source.as_bytes());
                        let bytes = encode(hash.as_bytes(), &f);
                        let encode_ns = t_enc.elapsed().as_nanos() as u64;
                        let hex = hash.to_hex();
                        let dir = store.join(&hex[..2]);
                        let file = dir.join(format!("{hex}.facts"));
                        // Always (re)written, atomically: a content's facts are a function of it.
                        std::fs::create_dir_all(&dir).unwrap();
                        let tmp = dir.join(format!("{hex}.facts.tmp{index}"));
                        std::fs::write(&tmp, &bytes).unwrap();
                        std::fs::rename(&tmp, &file).unwrap();
                        let on_disk = std::fs::read(&file).unwrap();
                        let t_dec = Instant::now();
                        let decoded = decode(&on_disk, source.clone());
                        let decode_ns = t_dec.elapsed().as_nanos() as u64;
                        let mismatch = match decoded {
                            Ok(d) => same(&f, &d).err().map(str::to_owned),
                            Err(e) => Some(e),
                        };
                        stats.push(Stat { index: index as u32, hash: hex.to_string(), source_bytes: source.len() as u32, nodes: f.len() as u32, file_bytes: on_disk.len() as u32, encode_ns, decode_ns, mismatch });
                    }
                    stats
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let wall_ns = t0.elapsed().as_nanos() as u64;
    stats.sort_by_key(|s| s.index);
    let mut tsv = String::from("path\tsource_bytes\tnodes\tfact_file_bytes\tencode_ns\tdecode_ns\tmismatch\n");
    for s in &stats {
        tsv.push_str(&format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\n", paths[s.index as usize], s.source_bytes, s.nodes, s.file_bytes, s.encode_ns, s.decode_ns, s.mismatch.as_deref().unwrap_or("")));
    }
    std::fs::write(format!("{out_dir}/per-file.tsv"), tsv).unwrap();
    let summary = |v: Vec<u64>| crate::repo::summary(v);
    let mismatches: Vec<_> = stats.iter().filter_map(|s| s.mismatch.as_ref().map(|m| json!([paths[s.index as usize], m]))).collect();
    let ratio: Vec<u64> = stats.iter().map(|s| (s.file_bytes as u64 * 1000) / (s.source_bytes.max(1) as u64)).collect();
    let result = json!({
        "store": store,
        "files": stats.len(),
        "distinct_contents": stats.iter().map(|s| s.hash.as_str()).collect::<std::collections::BTreeSet<_>>().len(),
        "wall_ns": wall_ns,
        "fact_file_bytes": summary(stats.iter().map(|s| s.file_bytes as u64).collect()),
        "source_bytes": summary(stats.iter().map(|s| s.source_bytes as u64).collect()),
        "fact_bytes_per_source_byte_permille": summary(ratio),
        "encode_ns": summary(stats.iter().map(|s| s.encode_ns).collect()),
        "decode_ns": summary(stats.iter().map(|s| s.decode_ns).collect()),
        "roundtrip_mismatches": mismatches,
    });
    std::fs::write(format!("{out_dir}/results.json"), serde_json::to_string_pretty(&result).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}

/// `inspect`: reads every `.facts` and `.xfacts` file of the store with this spike's generic
/// reader (header, string table, relations), the cross-spike readability check. Prints the files
/// read per extension, the relations seen with their total rows, and every failure.
pub fn inspect(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let store = PathBuf::from(arg(args, "--store").unwrap_or(format!("{home}/rerail-bench/facts/store")));
    let mut read_ok: BTreeMap<String, u64> = BTreeMap::new();
    let mut relations: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
    let mut failures = Vec::new();
    for shard in std::fs::read_dir(&store).unwrap() {
        let shard = shard.unwrap().path();
        if !shard.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&shard).unwrap() {
            let path = entry.unwrap().path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()).filter(|e| *e == "facts" || *e == "xfacts") else { continue };
            let bytes = std::fs::read(&path).unwrap();
            match read(&bytes) {
                Ok((hash, _, rels)) => {
                    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                    if blake3::Hash::from_bytes(hash).to_hex().as_str() != stem {
                        failures.push(json!([path, "header hash differs from the file name"]));
                        continue;
                    }
                    *read_ok.entry(ext.to_owned()).or_default() += 1;
                    for (name, r) in rels {
                        let e = relations.entry((ext.to_owned(), name)).or_default();
                        e.0 += 1;
                        e.1 += r.rows as u64;
                    }
                }
                Err(e) => failures.push(json!([path, e])),
            }
        }
    }
    let relations: Vec<_> = relations.into_iter().map(|((ext, name), (files, rows))| json!({"ext": ext, "relation": name, "files": files, "rows": rows})).collect();
    println!("{}", serde_json::to_string_pretty(&json!({"store": store, "read": read_ok, "relations": relations, "failures": failures})).unwrap());
}

/// `factfile-bench`: per file, every candidate layout's bytes, write time (encode, and compress
/// where the layout compresses) and read time (decompress, decode, derive parent/lines, index),
/// with a round-trip check against the built facts; in memory, no disk I/O. The named-only
/// layouts cannot be read back into `Facts`: their read cost is a re-parse (`parse_ns`).
pub fn bench(args: &[String]) {
    let home = std::env::var("HOME").unwrap();
    let root = arg(args, "--root").unwrap_or(format!("{home}/rerail-bench/nera-data-layer-2b1504ad"));
    let root = Path::new(&root);
    let threads: usize = arg(args, "--threads").map_or(4, |t| t.parse().unwrap());
    let out_dir = arg(args, "--out-dir").expect("--out-dir");
    let variants: Vec<(&str, Layout)> = vec![
        ("v0", V0),
        ("v1", Layout { version: 1, named_only: false, zstd: None }),
        ("v1+zstd1", Layout { version: 1, named_only: false, zstd: Some(1) }),
        ("v1+zstd3", Layout { version: 1, named_only: false, zstd: Some(3) }),
        ("v1+zstd9", Layout { version: 1, named_only: false, zstd: Some(9) }),
        ("v1-named", Layout { version: 1, named_only: true, zstd: None }),
        ("v1-named+zstd3", Layout { version: 1, named_only: true, zstd: Some(3) }),
    ];
    let paths = walk(root);
    let next = AtomicUsize::new(0);
    // Per file: source bytes, nodes, named nodes, parse ns; per variant: (bytes, write ns, read ns).
    type Row = (u32, u32, u32, u32, u64, Vec<(u64, u64, u64)>, Vec<String>);
    let mut rows: Vec<Row> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(path) = paths.get(index) else { break };
                        let source = Arc::new(std::fs::read_to_string(root.join(path)).unwrap());
                        let mut ns = facts::BuildNs::default();
                        let Ok(facts::Built::Facts(f)) = facts::build_timed(path, source.clone(), &mut ns) else { continue };
                        let hash = blake3::hash(source.as_bytes());
                        let mut per = Vec::new();
                        let mut bad = Vec::new();
                        for (name, l) in &variants {
                            let t = Instant::now();
                            let bytes = encode_with(hash.as_bytes(), &f, *l);
                            let w = t.elapsed().as_nanos() as u64;
                            let t = Instant::now();
                            let r = if l.named_only { read(&bytes).map(|_| None) } else { decode(&bytes, source.clone()).map(Some) };
                            let rd = t.elapsed().as_nanos() as u64;
                            match r {
                                Ok(Some(d)) => {
                                    if let Err(e) = same(&f, &d) {
                                        bad.push(format!("{path}: {name}: {e}"));
                                    }
                                }
                                Ok(None) => {}
                                Err(e) => bad.push(format!("{path}: {name}: {e}")),
                            }
                            per.push((bytes.len() as u64, w, rd));
                        }
                        let named = (0..f.len() as u32).filter(|&i| f.named(i)).count() as u32;
                        out.push((index as u32, source.len() as u32, f.len() as u32, named, ns.parse + ns.extract, per, bad));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    rows.sort_by_key(|r| r.0);
    let summary = crate::repo::summary;
    let src_total: u64 = rows.iter().map(|r| r.1 as u64).sum();
    let nodes_total: u64 = rows.iter().map(|r| r.2 as u64).sum();
    let mut out = serde_json::Map::new();
    for (vi, (name, l)) in variants.iter().enumerate() {
        let bytes: Vec<u64> = rows.iter().map(|r| r.5[vi].0).collect();
        let total: u64 = bytes.iter().sum();
        out.insert(name.to_string(), json!({
            "layout": format!("{l:?}"),
            "bytes": summary(bytes.clone()),
            "total_bytes": total,
            "bytes_per_source_byte": total as f64 / src_total as f64,
            "bytes_per_node": total as f64 / nodes_total as f64,
            "ratio_permille": summary(rows.iter().map(|r| r.5[vi].0 * 1000 / (r.1.max(1) as u64)).collect()),
            "write_ns": summary(rows.iter().map(|r| r.5[vi].1).collect()),
            "read_ns": summary(rows.iter().map(|r| r.5[vi].2).collect()),
        }));
    }
    let bad: Vec<&String> = rows.iter().flat_map(|r| r.6.iter()).collect();
    let res = json!({
        "files": rows.len(),
        "source_bytes_total": src_total,
        "nodes_total": nodes_total,
        "named_nodes_total": rows.iter().map(|r| r.3 as u64).sum::<u64>(),
        "parse_extract_ns": summary(rows.iter().map(|r| r.4).collect()),
        "variants": out,
        "roundtrip_failures": bad,
    });
    let mut tsv = String::from("path\tsource_bytes\tnodes\tnamed\tparse_extract_ns");
    for (name, _) in &variants {
        tsv.push_str(&format!("\t{name}_bytes\t{name}_write_ns\t{name}_read_ns"));
    }
    tsv.push('\n');
    for r in &rows {
        tsv.push_str(&format!("{}\t{}\t{}\t{}\t{}", paths[r.0 as usize], r.1, r.2, r.3, r.4));
        for v in &r.5 {
            tsv.push_str(&format!("\t{}\t{}\t{}", v.0, v.1, v.2));
        }
        tsv.push('\n');
    }
    std::fs::write(format!("{out_dir}/per-file.tsv"), tsv).unwrap();
    std::fs::write(format!("{out_dir}/results.json"), serde_json::to_string_pretty(&res).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&res).unwrap());
}
