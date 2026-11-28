//! The join lab: a launch of the join program the engine captured (`join_capture`, under `RERAIL_JOIN_CAPTURE`),
//! replayed apart from the engine. A capture's directory holds `blocks/` (each block its launches reached, frozen)
//! and `w/<launch>/` (`wave.txt`, its own buffers' written pages, its relocations, their after-images); `Wave::read`
//! reads one launch, `Wave::load` places each region in this process (a block mapped from its file, or read where
//! the launch wrote it; a buffer of the program's own in a reservation, its written pages put back), then binds its
//! addresses again where the regions now lie. `Loaded::run` runs its chain through the join program's Rust
//! (`join_run`), each dispatch's groups as the phase before it set them, as the CPU device runs it; `Loaded::compare`
//! holds what it left to what the engine's launch left.

pub mod page;

use rerail_blocks::block::{Block, PAGE};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

/// A run its launch encoded: its shape (`s-<key>`, shapes.yaml's), its rule, the rows into its stages' inputs.
#[derive(Clone, Debug)]
pub struct Label {
    pub shape: String,
    pub rule: String,
    pub work: u64,
}

/// A region a launch reached (`region` lines): its name, its role (one of `join_run`'s binds, `top`, `hold`, `text`,
/// `table`, `lease`, `block`), where it lay in the engine, its bytes, its block's kind, and its bytes' source.
#[derive(Clone, Debug)]
pub struct Region {
    pub name: String,
    pub role: String,
    pub base: u64,
    /// Where the engine's device reached it: the address its words held (the host's on the CPU device).
    pub dev: u64,
    pub len: usize,
    pub kind: u32,
    pub source: String,
}

impl Region {
    /// Its bytes are runs of written pages (`<name>.zst`): a buffer of the program's own, or a lease.
    pub fn sparse(&self) -> bool {
        self.source.ends_with(".zst")
    }
}

/// One captured launch (`wave.txt`).
#[derive(Clone, Debug)]
pub struct Wave {
    pub dir: PathBuf,
    /// The capture's directory (its `blocks/`).
    pub root: PathBuf,
    pub format: u32,
    /// The program's `ProgP`: its phases, its chain's dispatches, the phases its walk takes, 0.
    pub args: [u32; 4],
    /// Each dispatch's groups' three words, as byte offsets into its control words.
    pub offsets: Vec<u64>,
    /// `join_run`'s nine buffers, by role.
    pub binds: Vec<String>,
    pub labels: Vec<Label>,
    pub regions: Vec<Region>,
    /// Each region the launch may have written, and its after-image's source.
    pub after: Vec<(String, String)>,
    /// Words of its regions past 4 GB that no region holds (data that looks like an address, or memory no ledger
    /// knew: a replay that reads one fails there).
    pub unresolved: u64,
    /// Words inside a small region that look like an address and are not bound again (`join_capture`'s check): an
    /// address its encoding wrote without naming it. A replay of one with any may read the engine's memory.
    pub unbound: u64,
    /// Its after-images written: the engine's launch done.
    pub done: bool,
    /// The views' directories' homes of the binary's codes' directory (decision 0697): each its region, the home's
    /// offset there, and the directory's digest, bound to the lab's own (`rerail_tape_format::codes_dir`).
    pub codes: Vec<(String, usize, u64)>,
    /// The atoms directories its capture bound (`atoms` lines): each its region and its root's offset.
    pub atoms: Vec<(String, usize)>,
}

fn bad(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

impl Wave {
    /// The launch captured in `dir` (`<capture>/w/<launch>`).
    pub fn read(dir: &Path) -> io::Result<Wave> {
        let text = read_named(&dir.join("wave.txt")).and_then(|b| String::from_utf8(b).map_err(|e| bad(format!("wave.txt: {e}"))))?;
        let root = dir.parent().and_then(Path::parent).ok_or_else(|| bad(format!("{}: not in a capture's w/", dir.display())))?.to_path_buf();
        let mut w = Wave { dir: dir.to_path_buf(), root, format: 0, args: [0; 4], offsets: Vec::new(), binds: Vec::new(), labels: Vec::new(), regions: Vec::new(), after: Vec::new(), unresolved: 0, unbound: 0, done: false, codes: Vec::new(), atoms: Vec::new() };
        let num = |s: &str| s.parse::<u64>().map_err(|e| bad(format!("{}: {s}: {e}", dir.display())));
        for line in text.lines() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "capture" => {}
                "format" => w.format = num(rest)? as u32,
                "args" => {
                    for (i, a) in rest.split(' ').enumerate().take(4) {
                        w.args[i] = num(a)? as u32;
                    }
                }
                "offsets" => w.offsets = rest.split(' ').filter(|s| !s.is_empty()).map(num).collect::<io::Result<_>>()?,
                "binds" => w.binds = rest.split(' ').map(str::to_owned).collect(),
                "unresolved" => w.unresolved = num(rest)?,
                "unbound" => w.unbound = num(rest)?,
                // Where they lie, and a few of them: what a capture's reader looks into.
                "unresolved_in" => {}
                "label" => {
                    let mut p = rest.rsplitn(3, ' ');
                    let (work, rule, shape) = (p.next(), p.next(), p.next());
                    let (Some(work), Some(rule), Some(shape)) = (work, rule, shape) else { return Err(bad(format!("label: {line}"))) };
                    w.labels.push(Label { shape: shape.to_owned(), rule: rule.replace('_', " "), work: num(work)? });
                }
                "region" => {
                    let f: Vec<&str> = rest.splitn(7, ' ').collect();
                    let [name, role, base, dev, len, kind, source] = f[..] else { return Err(bad(format!("region: {line}"))) };
                    let hex = |h: &str| u64::from_str_radix(h.trim_start_matches("0x"), 16).map_err(|e| bad(format!("region: {line}: {e}")));
                    w.regions.push(Region { name: name.to_owned(), role: role.to_owned(), base: hex(base)?, dev: hex(dev)?, len: num(len)? as usize, kind: num(kind)? as u32, source: source.to_owned() });
                }
                "after" => {
                    let (name, source) = rest.split_once(' ').ok_or_else(|| bad(format!("after: {line}")))?;
                    w.after.push((name.to_owned(), source.to_owned()));
                }
                "done" => w.done = true,
                "codes" => {
                    let f: Vec<&str> = rest.split(' ').collect();
                    let [region, at, digest] = f[..] else { return Err(bad(format!("codes: {line}"))) };
                    w.codes.push((region.to_owned(), num(at)? as usize, u64::from_str_radix(digest, 16).map_err(|e| bad(format!("codes: {line}: {e}")))?));
                }
                // The atoms directories bound (`atoms <region> <root offset> <runs>`): what it found, for `list`.
                "atoms" => {
                    let f: Vec<&str> = rest.split(' ').collect();
                    let [region, at, _] = f[..] else { return Err(bad(format!("atoms: {line}"))) };
                    w.atoms.push((region.to_owned(), num(at)? as usize));
                }
                _ => return Err(bad(format!("{}: an unknown line: {line}", dir.display()))),
            }
        }
        if w.binds.len() != 9 {
            return Err(bad(format!("{}: {} binds, not join_run's 9", dir.display(), w.binds.len())));
        }
        Ok(w)
    }

    /// Every launch captured under `capture` (its `w/`), in its directory names' order.
    pub fn all(capture: &Path) -> io::Result<Vec<Wave>> {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(capture.join("w"))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("wave.txt").exists()).collect();
        dirs.sort();
        dirs.iter().map(|d| Wave::read(d)).collect()
    }

    fn source_path(&self, source: &str) -> PathBuf {
        match source.strip_prefix("file:") {
            Some(p) => PathBuf::from(p),
            None if source.starts_with("blocks/") => self.root.join(source),
            None => self.dir.join(source),
        }
    }

    /// Its regions placed in this process, their addresses bound again where they lie now.
    pub fn load(&self) -> io::Result<Loaded> {
        if !self.done {
            return Err(bad(format!("{}: its capture did not finish (no after-images): the engine's launch never completed", self.dir.display())));
        }
        let written: std::collections::HashSet<&str> = self.after.iter().map(|(n, _)| n.as_str()).collect();
        let relocs = zstd::stream::decode_all(open_named(&self.dir.join("relocs.zst"))?)?;
        // The regions the lab writes into: those the launch writes, and those holding a word bound again.
        let mut bound_in: std::collections::HashSet<usize> = relocs.chunks_exact(32).map(|q| u64::from_le_bytes(q[..8].try_into().expect("8 bytes")) as usize).collect();
        bound_in.extend(self.codes.iter().filter_map(|c| self.regions.iter().position(|r| r.name == c.0)));
        let mut mems = Vec::with_capacity(self.regions.len());
        for r in &self.regions {
            let path = self.source_path(&r.source);
            let mem = if r.sparse() {
                let m = Mem::reserve(r.len);
                read_runs(&path, |at, bytes| m.write(at, bytes))?;
                m
            } else if !written.contains(r.name.as_str()) && !bound_in.contains(&mems.len()) {
                // A block the launch only reads: mapped from its file, read-only.
                let b = Block::map_file(&path, r.kind, false).map_err(|e| bad(format!("{}: {e}", r.name)))?;
                Mem::Mapped(b)
            } else {
                // One it writes: its bytes read into memory of the lab's (its file is the capture's).
                let m = Mem::reserve(r.len);
                let mut f = open_named(&path)?;
                let mut at = 0;
                let mut buf = vec![0u8; 1 << 20];
                while at < r.len {
                    let n = f.read(&mut buf[..(r.len - at).min(1 << 20)])?;
                    if n == 0 {
                        break;
                    }
                    m.write(at, &buf[..n]);
                    at += n;
                }
                m
            };
            mems.push(mem);
        }
        let mut bound = 0;
        for q in relocs.chunks_exact(32) {
            let v = |i: usize| u64::from_le_bytes(q[8 * i..8 * i + 8].try_into().expect("8 bytes"));
            // The target's host bit (a view's home's host word): on the program's Rust, the host's address is the
            // device's.
            let (i, o, t, off) = (v(0) as usize, v(1) as usize, (v(2) & !(HOST | RELATIVE)) as usize, v(3) as usize);
            let to = mems.get(t).ok_or_else(|| bad(format!("a relocation into region {t} of {}", mems.len())))?.base() + off;
            let m = mems.get(i).ok_or_else(|| bad(format!("a relocation in region {i}")))?;
            // A relative word (an atoms directory's span): the target's address less the word's own.
            let value = if v(2) & RELATIVE != 0 { (to as u64).wrapping_sub((m.base() + o) as u64) } else { to as u64 };
            m.write(o, &value.to_ne_bytes());
            bound += 1;
        }
        let cd = rerail_tape_format::codes_dir::bytes();
        for (region, at, digest) in &self.codes {
            if *digest != capture_digest(cd) {
                return Err(bad(format!("the codes' directory is not the engine's (its digest {:016x}, the capture's {digest:016x}): a lab of another build", capture_digest(cd))));
            }
            let i = self.regions.iter().position(|r| r.name == *region).ok_or_else(|| bad(format!("codes: no region {region}")))?;
            mems[i].write(*at, &(cd.as_ptr() as u64).to_ne_bytes());
            mems[i].write(*at + 8, &0u64.to_ne_bytes());
            bound += 1;
        }
        let mut binds = [0u64; 9];
        for (k, role) in self.binds.iter().enumerate() {
            let i = self.regions.iter().position(|r| r.role == *role).ok_or_else(|| bad(format!("no region for the bind {role}")))?;
            binds[k] = mems[i].base() as u64;
        }
        Ok(Loaded { wave: self.clone(), mems, binds, relocations: bound, reverse: false })
    }
}

/// The file at `path`, its error naming it.
fn open_named(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

/// The bytes of the file at `path`, its error naming it.
fn read_named(path: &Path) -> io::Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))
}

/// `relocs.zst`'s host bit on a target (`join_capture::HOST`).
const HOST: u64 = 1 << 63;

/// `relocs.zst`'s relative bit on a target (`join_capture::RELATIVE`).
const RELATIVE: u64 = 1 << 62;

/// `join_capture`'s digest of bytes (FxHash, and the length).
fn capture_digest(bytes: &[u8]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    std::hash::Hasher::write(&mut h, bytes);
    std::hash::Hasher::finish(&h) ^ (bytes.len() as u64).rotate_left(32)
}

/// A region's runs (`join_capture`'s `<name>.zst`: each its offset and length, u64s, then its bytes), each given to
/// `put`.
fn read_runs(path: &Path, mut put: impl FnMut(usize, &[u8])) -> io::Result<()> {
    let mut z = zstd::stream::read::Decoder::new(open_named(path)?)?;
    let mut head = [0u8; 16];
    let mut buf = Vec::new();
    loop {
        match z.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        }
        let at = u64::from_le_bytes(head[..8].try_into().expect("8 bytes")) as usize;
        let n = u64::from_le_bytes(head[8..].try_into().expect("8 bytes")) as usize;
        buf.resize(n, 0);
        z.read_exact(&mut buf)?;
        put(at, &buf);
    }
}

/// A region's memory in the lab: a reservation of its own (fresh pages read as zero), or a block mapped from a file.
pub enum Mem {
    Reserved { p: *mut u8, len: usize },
    Mapped(Block),
}

impl Mem {
    fn reserve(len: usize) -> Mem {
        let len = len.max(1).next_multiple_of(PAGE);
        Mem::Reserved { p: rerail_blocks::mem::reserve(len), len }
    }

    pub fn base(&self) -> usize {
        match self {
            Mem::Reserved { p, .. } => *p as usize,
            Mem::Mapped(b) => b.bytes(0, 0).as_ptr() as usize,
        }
    }

    fn len(&self) -> usize {
        match self {
            Mem::Reserved { len, .. } => *len,
            Mem::Mapped(b) => b.used(),
        }
    }

    fn write(&self, at: usize, bytes: &[u8]) {
        let Mem::Reserved { p, len } = self else { panic!("a write into a mapped block") };
        assert!(at + bytes.len() <= *len, "a write past a region: {at} + {} of {len}", bytes.len());
        // SAFETY: within the reservation, which only the lab's thread touches while it is loaded.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(at), bytes.len()) };
    }

    fn bytes(&self, at: usize, n: usize) -> &[u8] {
        assert!(at + n <= self.len(), "a read past a region: {at} + {n} of {}", self.len());
        // SAFETY: within the region's memory.
        unsafe { std::slice::from_raw_parts((self.base() + at) as *const u8, n) }
    }
}

impl Drop for Mem {
    fn drop(&mut self) {
        if let Mem::Reserved { p, len } = self {
            // SAFETY: the reservation's, released once, nothing reading it after.
            unsafe { rerail_blocks::mem::release(*p, *len) };
        }
    }
}

/// A launch placed in the lab, ready to run (`run`), then to compare (`compare`).
pub struct Loaded {
    pub wave: Wave,
    pub mems: Vec<Mem>,
    /// `join_run`'s nine buffers' addresses, where its regions lie now.
    binds: [u64; 9],
    /// The addresses bound again (`relocs.zst`'s entries).
    pub relocations: usize,
    /// Each dispatch's groups run last first (`join-lab run --reverse`).
    pub reverse: bool,
}

/// What a replay ran: its dispatches (those whose groups were not none), its groups, its wall time.
#[derive(Clone, Debug, Default)]
pub struct Ran {
    pub dispatches: usize,
    pub groups: u64,
    pub seconds: f64,
}

/// A region the replay left otherwise than the engine's launch: its bytes compared, those that differ, the first.
#[derive(Clone, Debug)]
pub struct Diff {
    pub region: String,
    pub compared: usize,
    pub differ: usize,
    pub first: Option<usize>,
    /// The first words that differ: their offsets, the replay's and the engine's.
    pub words: Vec<(usize, u64, u64)>,
    /// The first of them before the launch (`replay`'s).
    pub before: Option<u64>,
}

/// The join program's group size (`join_stages`' `JOIN_TG`, `join_run`'s).
const TG: u32 = 256;

impl Loaded {
    /// Its chain through `join_run`, as the CPU device runs it: each dispatch's groups the three words at its offset
    /// of the control words, which the phases before it wrote; its groups one after another on this thread.
    pub fn run(&mut self) -> Ran {
        let ctl = self.binds[7] as usize;
        let prog = self.wave.args;
        // The shim's two arguments past the kernel's: no groups' counts, and the first group's position 0.
        let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
        let mut argv: Vec<*const u8> = self.binds.iter().map(|b| (b as *const u64).cast::<u8>()).collect();
        argv.push(prog.as_ptr().cast::<u8>());
        argv.extend([(&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>()]);
        let start = std::time::Instant::now();
        let mut ran = Ran::default();
        for &offset in &self.wave.offsets {
            // SAFETY: three words of the control words, which the dispatches before wrote.
            let grid = unsafe { std::ptr::read_volatile((ctl + offset as usize) as *const [u32; 3]) };
            if grid.contains(&0) {
                continue;
            }
            ran.dispatches += 1;
            let groups = grid.iter().map(|&g| u64::from(g)).product::<u64>();
            ran.groups += groups;
            for k in 0..groups {
                // Its groups in order, or last first (`reverse`: whether what the launch leaves hangs on their order).
                let i = if self.reverse { groups - 1 - k } else { k };
                let (x, rest) = ((i % u64::from(grid[0])) as u32, i / u64::from(grid[0]));
                let (y, z) = ((rest % u64::from(grid[1])) as u32, (rest / u64::from(grid[1])) as u32);
                let l = rerail_ukl_rust::Launch { group: [x, y, z], block: [TG, 1, 1], groups: grid };
                // SAFETY: `argv` holds the program's buffers' addresses where the lab placed them, its `ProgP`, and the
                // shim's two; the regions they reach are the capture's, bound again.
                unsafe { rerail_ukl_rust::join_program::join_run(argv.as_ptr(), &l) };
            }
        }
        ran.seconds = start.elapsed().as_secs_f64();
        ran
    }

    /// Each region the launch may have written, held to its after-image: its runs' bytes (a buffer of its own), or
    /// its used bytes (a block). Every region it names, compared; those equal are no `Diff` with `differ` 0.
    pub fn compare(&self) -> io::Result<Vec<Diff>> {
        let relocated = self.relocated()?;
        let mut out = Vec::new();
        for (name, source) in &self.wave.after {
            let i = self.wave.regions.iter().position(|r| r.name == *name).ok_or_else(|| bad(format!("after: no region {name}")))?;
            let (r, m) = (&self.wave.regions[i], &self.mems[i]);
            // The words bound again hold the lab's addresses, the after-image the engine's: not compared.
            let skip: std::collections::HashSet<usize> = relocated.iter().filter(|q| q.0 == i).map(|q| q.1).collect();
            let mut d = Diff { region: name.clone(), compared: 0, differ: 0, first: None, words: Vec::new(), before: None };
            let mut hold = |at: usize, want: &[u8]| {
                let got = m.bytes(at, want.len());
                for (k, (a, b)) in got.iter().zip(want).enumerate() {
                    if skip.contains(&((at + k) / 8 * 8)) {
                        continue;
                    }
                    if a != b {
                        d.differ += 1;
                        d.first.get_or_insert(at + k);
                        let o = (at + k) / 8 * 8;
                        if d.words.len() < 8 && d.words.last().is_none_or(|w| w.0 != o) && o >= at && o + 8 <= at + want.len() {
                            let g = u64::from_ne_bytes(m.bytes(o, 8).try_into().expect("8 bytes"));
                            let w = u64::from_ne_bytes(want[o - at..o - at + 8].try_into().expect("8 bytes"));
                            d.words.push((o, g, w));
                        }
                    }
                }
                d.compared += want.len();
            };
            let path = self.wave.source_path(source);
            if r.sparse() {
                read_runs(&path, &mut hold)?;
            } else {
                let want = read_named(&path)?;
                hold(0, &want[..want.len().min(r.len)]);
            }
            out.push(d);
        }
        Ok(out)
    }

    /// The words the lab bound again (`relocs.zst`), as (region, offset): where an after-image holds the engine's
    /// address and the replay the lab's.
    /// Its words (each region of at most 64 MB, as loaded and bound) whose values lie in `lo..hi`: each its region,
    /// offset and value (`join-lab words`: which word holds an address a replay faults at).
    pub fn words_in(&self, lo: u64, hi: u64) -> Vec<(String, usize, u64)> {
        let mut out = Vec::new();
        for (r, m) in self.wave.regions.iter().zip(&self.mems).filter(|(_, m)| m.len() <= 64 << 20 || matches!(m, Mem::Mapped(_))) {
            for (i, q) in m.bytes(0, m.len() / 8 * 8).chunks_exact(8).enumerate() {
                let v = u64::from_ne_bytes(q.try_into().expect("8 bytes"));
                if lo <= v && v < hi {
                    out.push((r.name.clone(), 8 * i, v));
                }
            }
        }
        out
    }

    /// The views' directories its bound words reach, each checked as the program's reads take it (`join_program`'s
    /// `reads::doc`): its rows in its block, and each document's homes' host addresses, its tape's, lines' and
    /// source's bytes inside a region the lab placed. Each what it found wrong, the first few of each directory.
    pub fn views(&self) -> io::Result<Vec<String>> {
        use rerail_reads::views::{ViewDir, ViewHome, ViewRow};
        let inside = |a: usize, n: usize| self.mems.iter().position(|m| m.base() <= a && a + n <= m.base() + m.len());
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, o) in self.relocated()? {
            let a = u64::from_ne_bytes(self.mems[i].bytes(o, 8).try_into().expect("8 bytes")) as usize;
            let Some(t) = inside(a, std::mem::size_of::<ViewDir>()).filter(|&t| self.wave.regions[t].role == "block") else { continue };
            let off = a - self.mems[t].base();
            // SAFETY: inside region `t`'s placed bytes, read unaligned.
            let vd: ViewDir = unsafe { std::ptr::read_unaligned(a as *const ViewDir) };
            if vd.at as usize != off || !seen.insert(a) {
                continue;
            }
            let name = format!("the view directory at {} {off} (from {} {o})", self.wave.regions[t].name, self.wave.regions[i].name);
            let (base, len) = (self.mems[t].base(), self.mems[t].len());
            let mut bad = Vec::new();
            let span = vd.span.max(vd.ndocs) as usize;
            if vd.rows as usize + span * std::mem::size_of::<ViewRow>() > len || vd.homes as usize + 16 * vd.nhomes as usize > len || vd.keys as usize + 4 * vd.ndocs as usize > len || vd.order as usize + 4 * vd.ndocs as usize > len {
                bad.push(format!("{span} rows at {}, {} homes at {}, {} keys at {} or their order at {} past its block's {len} bytes", vd.rows, vd.nhomes, vd.homes, vd.ndocs, vd.keys, vd.order));
            } else {
                // SAFETY: the order inside the block, checked above.
                let order = |m: usize| unsafe { std::ptr::read_unaligned((base + vd.order as usize + 4 * m) as *const u32) };
                if let Some(m) = (0..vd.ndocs as usize).find(|&m| order(m) as usize >= span) {
                    bad.push(format!("document {m}'s row {} past its {span} rows", order(m)));
                }
                // SAFETY: the homes inside the block, checked above.
                let home = |h: u32| unsafe { std::ptr::read_unaligned((base + vd.homes as usize + 16 * h as usize) as *const ViewHome) }.host as usize;
                for m in 0..span {
                    // SAFETY: the rows inside the block, checked above.
                    let r: ViewRow = unsafe { std::ptr::read_unaligned((base + vd.rows as usize + m * std::mem::size_of::<ViewRow>()) as *const ViewRow) };
                    if r.key == rerail_reads::views::NO_DOC {
                        continue;
                    }
                    for (part, h, at, n) in [("tape", 0, r.tape, r.bytes as usize), ("lines", 1, r.lines, 8), ("source", 2, r.source, 1)] {
                        let hh = r.homes[h];
                        if hh >= vd.nhomes {
                            bad.push(format!("document {m}'s {part} home {hh} of {}", vd.nhomes));
                        } else if inside(home(hh).wrapping_add(at as usize), n).is_none() {
                            bad.push(format!("document {m}'s {part} (home {hh}: {:#x}, at {at}, {n} bytes) in no region", home(hh)));
                        }
                    }
                    if bad.len() >= 4 {
                        break;
                    }
                }
            }
            if bad.is_empty() {
                bad.push(format!("sound: {} documents, {} rows, {} homes", vd.ndocs, vd.span, vd.nhomes));
            }
            out.extend(bad.into_iter().map(|b| format!("{name}: {b}")));
        }
        Ok(out)
    }

    /// Its relocations (`relocs.zst`), each its word's region and offset, its target's region (the host bit's
    /// `host`), its offset there, and the word as bound (`join-lab words`).
    pub fn relocations(&self) -> io::Result<Vec<String>> {
        let relocs = zstd::stream::decode_all(open_named(&self.wave.dir.join("relocs.zst"))?)?;
        Ok(relocs.chunks_exact(32).map(|q| {
            let v = |i: usize| u64::from_le_bytes(q[8 * i..8 * i + 8].try_into().expect("8 bytes"));
            let (i, o, t, off) = (v(0) as usize, v(1) as usize, v(2), v(3));
            let now = u64::from_ne_bytes(self.mems[i].bytes(o, 8).try_into().expect("8 bytes"));
            let name = |r: usize| self.wave.regions.get(r).map_or("?", |r| r.name.as_str());
            format!("{} {o} -> {}{}{} {off} = {now:#x}", name(i), name((t & !(HOST | RELATIVE)) as usize), if t & HOST != 0 { " host" } else { "" }, if t & RELATIVE != 0 { " relative" } else { "" })
        }).collect())
    }

    /// The atoms directories it bound, each read as the program reads it (`join_program::reads`): each run's first,
    /// middle and last atom's string found, inside a region the lab placed. Each directory's runs, or what is wrong.
    pub fn atoms(&self) -> Vec<String> {
        use rerail_ukl_rust::join_program::reads::{atoms_len, atoms_str};
        let inside = |a: usize, n: usize| self.mems.iter().any(|m| m.base() <= a && a + n <= m.base() + m.len());
        let mut out = Vec::new();
        for (region, at) in &self.wave.atoms {
            let Some(i) = self.wave.regions.iter().position(|r| &r.name == region) else { continue };
            let dir = (self.mems[i].base() + at) as *const u8;
            let n = u64::from_ne_bytes(self.mems[i].bytes(*at, 8).try_into().expect("8 bytes"));
            let eo = (*at as i64 + 8 + i64::from_ne_bytes(self.mems[i].bytes(at + 8, 8).try_into().expect("8 bytes"))) as usize;
            let mut found = Vec::new();
            for k in 0..n as usize {
                let e = self.mems[i].bytes(eo + 64 * k, 8);
                let (first, end) = (u32::from_ne_bytes(e[..4].try_into().expect("4")), u32::from_ne_bytes(e[4..8].try_into().expect("4")));
                for id in [first.max(1), first.max(1) / 2 + end / 2, end.saturating_sub(1)].into_iter().filter(|&id| id >= first && id < end) {
                    // SAFETY: the directory as bound; its reads stay in its runs (checked below, after each).
                    let (len, at) = unsafe { (atoms_len(dir, id), atoms_str(dir, id) as usize) };
                    let ok = len != u32::MAX && inside(at, len as usize);
                    if !ok || found.len() < 2 {
                        found.push(format!("id {id}: {} at {at:#x}{}", if len == u32::MAX { "none".into() } else { format!("{len} bytes") }, if ok { "" } else { " IN NO REGION" }));
                    }
                }
                found.push(format!("run {k}: {first}..{end}"));
            }
            out.push(format!("atoms at {region} {at}: {n} runs: {}", found.join(", ")));
        }
        out
    }

    /// Each region where the lab placed it: its name, the lab's address and its length (`join-lab words`).
    pub fn placed(&self) -> Vec<(String, usize, usize)> {
        self.wave.regions.iter().zip(&self.mems).map(|(r, m)| (r.name.clone(), m.base(), m.len())).collect()
    }

    pub fn relocated(&self) -> io::Result<Vec<(usize, usize)>> {
        let relocs = zstd::stream::decode_all(open_named(&self.wave.dir.join("relocs.zst"))?)?;
        Ok(relocs.chunks_exact(32).map(|q| (u64::from_le_bytes(q[..8].try_into().expect("8")) as usize, u64::from_le_bytes(q[8..16].try_into().expect("8")) as usize)).collect())
    }
}

/// An engine a launch is replayed on: the join program's Rust (`join_run`, the CPU device's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Program,
}

impl Engine {
    /// The engine `name` names (`--engine`).
    pub fn named(name: &str) -> Result<Engine, String> {
        match name {
            "program" => Ok(Engine::Program),
            _ => Err(format!("no engine {name} (engines: program)")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Program => "program",
        }
    }
}

impl Wave {
    /// Its directory's name: the process and capture number (`<pid>-<id>`).
    pub fn name(&self) -> String {
        self.dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
    }

    /// The bytes its capture holds on disk: its own files, and the block files it names (each once).
    pub fn bytes(&self) -> u64 {
        let own: u64 = std::fs::read_dir(&self.dir).map(|d| d.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0);
        let blocks: std::collections::BTreeSet<&str> = self.regions.iter().map(|r| r.source.as_str()).chain(self.after.iter().map(|a| a.1.as_str())).filter(|s| s.starts_with("blocks/")).collect();
        own + blocks.iter().filter_map(|b| std::fs::metadata(self.root.join(b)).ok()).map(|m| m.len()).sum::<u64>()
    }
}

/// A replay: what it ran and how what it left compares.
#[derive(Debug)]
pub struct Replay {
    pub ran: Ran,
    pub diffs: Vec<Diff>,
}

impl Replay {
    /// Every region it wrote as the engine's launch left it.
    pub fn same(&self) -> bool {
        self.diffs.iter().all(|d| d.differ == 0)
    }

    /// Its differing regions, the first word of each.
    pub fn differing(&self) -> String {
        self.diffs.iter().filter(|d| d.differ > 0).map(|d| match d.words.first() {
            Some((o, got, want)) => format!("{}: {} bytes, first word at {o}: {got:#x}, the engine's {want:#x}{}", d.region, d.differ, d.before.map_or(String::new(), |b| format!(", before {b:#x}"))),
            None => format!("{}: {} bytes", d.region, d.differ),
        }).collect::<Vec<_>>().join("; ")
    }
}

/// `w` replayed once on `e`, held to what the engine's launch left.
pub fn replay(w: &Wave, e: Engine) -> io::Result<Replay> {
    replay_in(w, e, false)
}

/// `replay`, each dispatch's groups last first where `reverse`.
pub fn replay_in(w: &Wave, e: Engine, reverse: bool) -> io::Result<Replay> {
    let mut l = w.load()?;
    l.reverse = reverse;
    let ran = match e {
        Engine::Program => l.run(),
    };
    let mut diffs = l.compare()?;
    drop(l);
    // Each differing word as it was before the launch: where the replay's is the before's, the replay never wrote
    // it and the engine's came from its launch or another writer of the region.
    if diffs.iter().any(|d| d.differ > 0) {
        let fresh = w.load()?;
        for d in diffs.iter_mut().filter(|d| d.differ > 0) {
            if let (Some(i), Some(&(o, ..))) = (w.regions.iter().position(|r| r.name == d.region), d.words.first()) {
                d.before = Some(u64::from_ne_bytes(fresh.mems[i].bytes(o, 8).try_into().expect("8 bytes")));
            }
        }
    }
    Ok(Replay { diffs, ran })
}

/// The CPU time this thread has used, in seconds (`CLOCK_THREAD_CPUTIME_ID`).
fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a query of this thread's clock into a timespec of ours.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}

/// A launch profiled on an engine: its work, its runs' best and median wall time and CPU time, whether it left
/// what the engine's launch left.
#[derive(Clone, Debug)]
pub struct Profile {
    pub launch: String,
    pub engine: &'static str,
    pub shapes: Vec<(String, String, u64)>,
    pub dispatches: usize,
    pub groups: u64,
    pub repeat: usize,
    pub best_ms: f64,
    pub median_ms: f64,
    pub cpu_ms: f64,
    pub bytes: u64,
    pub same: bool,
}

impl Profile {
    pub fn header() -> String {
        "launch\tengine\tdispatches\tgroups\trepeat\tbest_ms\tmedian_ms\tcpu_ms\tbytes\tsame\tshapes".to_owned()
    }

    pub fn tsv(&self) -> String {
        let shapes: Vec<String> = self.shapes.iter().map(|(s, r, w)| format!("{s}|{r}|{w}")).collect();
        format!("{}\t{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}", self.launch, self.engine, self.dispatches, self.groups, self.repeat, self.best_ms, self.median_ms, self.cpu_ms, self.bytes, self.same, shapes.join(","))
    }

    pub fn line(&self) -> String {
        let shapes: Vec<&str> = self.shapes.iter().map(|s| s.0.as_str()).collect();
        format!("{:<16} {:<8} {:>4} disp {:>9} groups  best {:>9.3} ms  median {:>9.3} ms  cpu {:>9.3} ms  {}  {}", self.launch, self.engine, self.dispatches, self.groups, self.best_ms, self.median_ms, self.cpu_ms, if self.same { "same" } else { "DIFFERS" }, shapes.join(" "))
    }
}

/// `w` run `repeat` times on `e`, each loaded afresh (the load not timed): its wall times' best and median, the
/// CPU time of its median run, and whether its last run left what the engine's launch left.
pub fn profile(w: &Wave, e: Engine, repeat: usize) -> io::Result<Profile> {
    let mut times = Vec::with_capacity(repeat);
    let mut last = None;
    for _ in 0..repeat.max(1) {
        let mut l = w.load()?;
        let cpu = thread_cpu();
        let ran = match e {
            Engine::Program => l.run(),
        };
        times.push((ran.seconds, thread_cpu() - cpu));
        last = Some((ran, l));
    }
    let (ran, l) = last.expect("a run");
    let same = l.compare()?.iter().all(|d| d.differ == 0);
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let median = sorted[sorted.len() / 2];
    Ok(Profile {
        launch: w.name(),
        engine: e.name(),
        shapes: w.labels.iter().map(|l| (l.shape.clone(), l.rule.clone(), l.work)).collect(),
        dispatches: ran.dispatches,
        groups: ran.groups,
        repeat: times.len(),
        best_ms: sorted[0].0 * 1e3,
        median_ms: median.0 * 1e3,
        cpu_ms: median.1 * 1e3,
        bytes: w.bytes(),
        same,
    })
}
