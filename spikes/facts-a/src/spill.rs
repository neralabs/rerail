//! Repository-partition rows (Spike A3, per-file phase at O(one file)). During the file phase
//! every repository-partitioned clause's per-file input rows and anchors go to a sink: in
//! `repo`, a per-run temporary directory with one file per (worker, clause); in `replay`,
//! memory. `eval::finish` loads one clause at a time, so only that clause's rows are resident.
use crate::sql::value::V;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// One file's rows for one repository clause: per input table, its rows; and its anchors
/// (start, end, line).
pub struct FileRows {
    pub path: Rc<str>,
    pub tables: Vec<Vec<Box<[V]>>>,
    pub anchors: Vec<(u32, u32, u32)>,
}

pub trait RepoSink {
    fn push(&mut self, clause: (usize, usize), rows: FileRows);
}

/// In-memory sink (`replay`: a recorded call's few files).
#[derive(Default)]
pub struct Memory(pub BTreeMap<(usize, usize), Vec<FileRows>>);

impl RepoSink for Memory {
    fn push(&mut self, clause: (usize, usize), rows: FileRows) {
        self.0.entry(clause).or_default().push(rows);
    }
}

/// A worker's spill files in the run's temporary directory.
pub struct Spill {
    dir: PathBuf,
    worker: usize,
    files: BTreeMap<(usize, usize), BufWriter<std::fs::File>>,
    buf: Vec<u8>,
    pub bytes: u64,
}

fn file_name(dir: &Path, worker: usize, clause: (usize, usize)) -> PathBuf {
    dir.join(format!("w{worker}-r{}-c{}.spill", clause.0, clause.1))
}

const NULL: u8 = 0;
const INT: u8 = 1;
const REAL: u8 = 2;
const TEXT: u8 = 3;
/// The record's path (rows repeat it in `__path`).
const PATH: u8 = 4;

fn put_u32(out: &mut Vec<u8>, x: u32) {
    out.extend_from_slice(&x.to_le_bytes());
}

impl Spill {
    pub fn new(dir: &Path, worker: usize) -> Self {
        Self { dir: dir.to_path_buf(), worker, files: BTreeMap::new(), buf: Vec::new(), bytes: 0 }
    }

    /// Flushes and closes every file; returns the bytes written.
    pub fn finish(mut self) -> u64 {
        for (_, mut w) in std::mem::take(&mut self.files) {
            w.flush().expect("spill flush");
        }
        self.bytes
    }
}

impl RepoSink for Spill {
    fn push(&mut self, clause: (usize, usize), rows: FileRows) {
        if rows.anchors.is_empty() && rows.tables.iter().all(Vec::is_empty) {
            return;
        }
        let buf = &mut self.buf;
        buf.clear();
        put_u32(buf, rows.path.len() as u32);
        buf.extend_from_slice(rows.path.as_bytes());
        put_u32(buf, rows.tables.len() as u32);
        for t in &rows.tables {
            put_u32(buf, t.len() as u32);
            for r in t {
                put_u32(buf, r.len() as u32);
                for v in r.iter() {
                    match v {
                        V::Null => buf.push(NULL),
                        V::Int(i) => {
                            buf.push(INT);
                            buf.extend_from_slice(&i.to_le_bytes());
                        }
                        V::Real(x) => {
                            buf.push(REAL);
                            buf.extend_from_slice(&x.to_bits().to_le_bytes());
                        }
                        V::Text(s) if **s == *rows.path => buf.push(PATH),
                        V::Text(s) => {
                            buf.push(TEXT);
                            put_u32(buf, s.len() as u32);
                            buf.extend_from_slice(s.as_bytes());
                        }
                    }
                }
            }
        }
        put_u32(buf, rows.anchors.len() as u32);
        for (s, e, l) in &rows.anchors {
            put_u32(buf, *s);
            put_u32(buf, *e);
            put_u32(buf, *l);
        }
        let (dir, worker) = (&self.dir, self.worker);
        let w = self.files.entry(clause).or_insert_with(|| BufWriter::with_capacity(1 << 16, std::fs::File::create(file_name(dir, worker, clause)).expect("spill create")));
        w.write_all(&self.buf).expect("spill write");
        self.bytes += self.buf.len() as u64;
    }
}

/// A buffered reader over one spill file.
struct In {
    r: BufReader<std::fs::File>,
    left: u64,
}

impl In {
    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).expect("spill read");
        self.left -= N as u64;
        b
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes::<4>())
    }
    fn text<'b>(&mut self, scratch: &'b mut Vec<u8>) -> &'b str {
        let n = self.u32() as usize;
        scratch.resize(n, 0);
        self.r.read_exact(scratch).expect("spill read");
        self.left -= n as u64;
        std::str::from_utf8(scratch).expect("spill utf-8")
    }
}

/// Every worker's records for one clause, in worker order then file order. Short texts (kinds,
/// names) share one `Rc` again; a record's path is one `Rc` for all its rows.
pub fn load(dir: &Path, workers: usize, clause: (usize, usize)) -> Vec<FileRows> {
    let mut out = Vec::new();
    let mut shared: FxHashMap<Box<str>, Rc<str>> = FxHashMap::default();
    let mut scratch = Vec::new();
    for w in 0..workers {
        let Ok(file) = std::fs::File::open(file_name(dir, w, clause)) else { continue };
        let left = file.metadata().expect("spill metadata").len();
        let mut r = In { r: BufReader::with_capacity(1 << 16, file), left };
        while r.left > 0 {
            let path: Rc<str> = Rc::from(r.text(&mut scratch));
            let nt = r.u32() as usize;
            let mut tables = Vec::with_capacity(nt);
            for _ in 0..nt {
                let nr = r.u32() as usize;
                let mut rows = Vec::with_capacity(nr);
                for _ in 0..nr {
                    let width = r.u32() as usize;
                    let mut row = Vec::with_capacity(width);
                    for _ in 0..width {
                        let [tag] = r.bytes::<1>();
                        row.push(match tag {
                            NULL => V::Null,
                            INT => V::Int(i64::from_le_bytes(r.bytes::<8>())),
                            REAL => V::Real(f64::from_bits(u64::from_le_bytes(r.bytes::<8>()))),
                            PATH => V::Text(path.clone()),
                            TEXT => {
                                let s = r.text(&mut scratch);
                                if s.len() <= 48 {
                                    match shared.get(s) {
                                        Some(x) => V::Text(x.clone()),
                                        None => {
                                            let x: Rc<str> = Rc::from(s);
                                            shared.insert(Box::from(s), x.clone());
                                            V::Text(x)
                                        }
                                    }
                                } else {
                                    V::Text(Rc::from(s))
                                }
                            }
                            t => panic!("spill tag {t}"),
                        });
                    }
                    rows.push(row.into_boxed_slice());
                }
                tables.push(rows);
            }
            let na = r.u32() as usize;
            let mut anchors = Vec::with_capacity(na);
            for _ in 0..na {
                let (s, e, l) = (r.u32(), r.u32(), r.u32());
                anchors.push((s, e, l));
            }
            out.push(FileRows { path, tables, anchors });
        }
    }
    out
}
