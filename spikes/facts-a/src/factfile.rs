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
pub const SCHEMA_VERSION: u32 = 0;

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

/// A file's facts in the agreed layout.
pub fn encode(hash: &[u8; 32], f: &Facts) -> Vec<u8> {
    let mut s = Strings::default();
    let n = f.len();
    // Process symbols to file symbols, once per distinct kind or field.
    let mut kind_map: rustc_hash::FxHashMap<u32, u32> = Default::default();
    let kinds: Vec<u32> = f.kind.iter().map(|&k| *kind_map.entry(k).or_insert_with(|| s.sym(&facts::name(k)))).collect();
    let mut field_map: rustc_hash::FxHashMap<u16, u32> = Default::default();
    let fields: Vec<u32> = f
        .field
        .iter()
        .map(|&c| if c == 0 { NONE } else { *field_map.entry(c).or_insert_with(|| s.sym(GRAMMAR.kinds.field_name(c).unwrap_or(""))) })
        .collect();
    let identifier = facts::lookup("identifier");
    let ids: Vec<u32> = (0..n as u32).filter(|&i| Some(f.kind[i as usize]) == identifier && f.named(i)).collect();
    let id_names: Vec<u32> = ids.iter().map(|&i| s.sym(f.text(i))).collect();
    let source = f.source.as_bytes();
    let line_count = f.lines.line(source.len()) + 1;
    let (mut starts, mut ends, mut indents) = (Vec::with_capacity(line_count), Vec::with_capacity(line_count), Vec::with_capacity(line_count));
    for l in 0..line_count {
        let (a, b) = (f.lines.start(l), f.lines.end(l));
        starts.push(a as u32);
        ends.push(b as u32);
        indents.push(source[a..b].iter().take_while(|&&c| c == b' ' || c == b'\t').count() as u16);
    }
    let rels: Vec<(&str, usize, Vec<(&str, Col)>)> = vec![
        (
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
        ),
        ("identifier", ids.len(), vec![("id", Col::U32(ids)), ("name", Col::Sym(id_names))]),
        ("line", line_count, vec![("start", Col::U32(starts)), ("end", Col::U32(ends)), ("indent", Col::U16(indents))]),
        ("syntax_error", f.syntax_errors as usize, vec![]),
    ];
    let rels: Vec<(u32, u32, Vec<(u32, Col)>)> = rels.into_iter().map(|(name, rows, cols)| (s.sym(name), rows as u32, cols.into_iter().map(|(c, v)| (s.sym(c), v)).collect())).collect();

    let mut out = Vec::with_capacity(64 + n * 34);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(hash);
    out.extend_from_slice(&(s.list.len() as u32).to_le_bytes());
    for st in &s.list {
        out.extend_from_slice(&(st.len() as u32).to_le_bytes());
        out.extend_from_slice(st.as_bytes());
    }
    out.extend_from_slice(&(rels.len() as u32).to_le_bytes());
    for (name, rows, cols) in &rels {
        out.extend_from_slice(&name.to_le_bytes());
        out.extend_from_slice(&rows.to_le_bytes());
        out.extend_from_slice(&(cols.len() as u32).to_le_bytes());
        for (cn, col) in cols {
            out.extend_from_slice(&cn.to_le_bytes());
            out.push(col.code());
        }
        for (_, col) in cols {
            match col {
                Col::U32(v) | Col::Sym(v) | Col::OptU32(v) | Col::OptSym(v) => v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
                Col::U16(v) => v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
                Col::U8(v) => out.extend_from_slice(v),
            }
            // Only the narrow columns pad, to a 4-byte file offset (Spike B's writer and reader).
            if matches!(col, Col::U16(_) | Col::U8(_)) {
                while out.len() % 4 != 0 {
                    out.push(0);
                }
            }
        }
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

/// The header, string table and every relation of a fact file.
pub fn read(bytes: &[u8]) -> Result<([u8; 32], Vec<String>, BTreeMap<String, Relation>), String> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.bytes(8)? != MAGIC {
        return Err("bad magic".into());
    }
    let version = r.u32()?;
    if version != SCHEMA_VERSION {
        return Err(format!("schema version {version}, expected {SCHEMA_VERSION}"));
    }
    let hash: [u8; 32] = r.bytes(32)?.try_into().unwrap();
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
    Ok((hash, strings, rels))
}

/// A file's `Facts` from its fact file and its source (node text is the source slice).
pub fn decode(bytes: &[u8], source: Arc<String>) -> Result<Facts, String> {
    let (hash, strings, mut rels) = read(bytes)?;
    if *blake3::hash(source.as_bytes()).as_bytes() != hash {
        return Err("content hash does not match the source".into());
    }
    let mut node = rels.remove("node").ok_or("no `node` relation")?;
    let mut col = |name: &str| node.cols.remove(name).map(|c| c.1).ok_or_else(|| format!("node.{name} missing"));
    let mut kind_map: rustc_hash::FxHashMap<u32, u32> = Default::default();
    let kind = col("kind")?.into_iter().map(|k| *kind_map.entry(k).or_insert_with(|| facts::intern(&strings[k as usize]))).collect();
    let mut field_map: rustc_hash::FxHashMap<u32, u16> = Default::default();
    let mut field = Vec::new();
    for x in col("field")? {
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
    let mut f = Facts {
        lines: Arc::new(LineTable::new(&source, Terminators::Newlines)),
        source,
        kind,
        parent: col("parent")?,
        start: col("start")?,
        end: col("end")?,
        start_line: col("start_line")?,
        end_line: col("end_line")?,
        subtree_end: col("subtree_end")?,
        flags: col("flags")?.into_iter().map(|x| x as u8).collect(),
        field,
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        syntax_errors: rels.get("syntax_error").is_some_and(|r| r.rows > 0),
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
