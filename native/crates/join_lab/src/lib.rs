//! The join lab: a launch of the join program the engine captured (`join_capture`, under `RERAIL_JOIN_CAPTURE`),
//! replayed apart from the engine. A capture's directory holds `blocks/` (each block its launches reached, frozen)
//! and `w/<launch>/` (`wave.txt`, its own buffers' written pages, its relocations, their after-images); `Wave::read`
//! reads one launch, `Wave::load` places each region in this process (a block mapped from its file, or read where
//! the launch wrote it; a buffer of the program's own in a reservation, its written pages put back), then binds its
//! addresses again where the regions now lie. `Loaded::run` runs its chain through the join program's Rust
//! (`join_run`), each dispatch's groups as the phase before it set them, as the CPU device runs it; `Loaded::compare`
//! holds what it left to what the engine's launch left.

pub mod catalogue;
pub mod counters;
#[cfg(feature = "metal")]
pub mod metal;
pub mod page;
pub mod sample;

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
    /// The engine of the device that ran it (`device`: the CPU device's is the program): its working memory laid out
    /// for that one. None in a capture that does not name it.
    pub device: Option<Engine>,
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
        let mut w = Wave { dir: dir.to_path_buf(), root, format: 0, device: None, args: [0; 4], offsets: Vec::new(), binds: Vec::new(), labels: Vec::new(), regions: Vec::new(), after: Vec::new(), unresolved: 0, unbound: 0, done: false, codes: Vec::new(), atoms: Vec::new() };
        let num = |s: &str| s.parse::<u64>().map_err(|e| bad(format!("{}: {s}: {e}", dir.display())));
        for line in text.lines() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "capture" => {}
                "format" => w.format = num(rest)? as u32,
                "device" => w.device = Some(Engine::named(match rest.trim() { "cpu" => "program", d => d }).map_err(bad)?),
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

    /// Whether engine `e`'s replay is held to the capture byte for byte: the program's (its groups one after another,
    /// in order) of a capture its own device made. A device's threads claim places in the order they come, run to run
    /// (a Metal capture replayed on Metal differs so), so another's is held to it by what it makes (`compare`).
    pub fn exact_on(&self, e: Engine) -> bool {
        e == Engine::Program && self.device == Some(Engine::Program)
    }

    /// Its regions placed in this process, their addresses bound again where they lie now.
    pub fn load(&self) -> io::Result<Loaded> {
        self.load_on(Engine::Program)
    }

    /// `load`, for engine `e`: its addresses those its kernels reach the regions by (the device's for `metal`, its
    /// wraps', a view's home's host word the host's).
    pub fn load_on(&self, e: Engine) -> io::Result<Loaded> {
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
        // Where the engine's kernels reach each region: the host's address (the program's Rust), or its wrap's.
        #[cfg(feature = "metal")]
        let wraps = match e {
            Engine::Metal => Some(metal::Wraps::of(&mems).map_err(bad)?),
            Engine::Program | Engine::Class => None,
        };
        #[cfg(not(feature = "metal"))]
        if e == Engine::Metal {
            return Err(bad("the metal engine: this lab was built without its `metal` feature".into()));
        }
        let dev = |i: usize| -> u64 {
            #[cfg(feature = "metal")]
            if let Some(w) = &wraps {
                return w.dev[i];
            }
            mems[i].base() as u64
        };
        let mut bound = 0;
        // The words a relocation binds, by place: a view's home whose device word none binds is the CPU device's
        // (none: its kernels read the host's word), given the device's address of what its host word names.
        let placed: std::collections::HashSet<(usize, usize)> = relocs.chunks_exact(32).map(|q| (u64::from_le_bytes(q[..8].try_into().expect("8 bytes")) as usize, u64::from_le_bytes(q[8..16].try_into().expect("8 bytes")) as usize)).collect();
        for q in relocs.chunks_exact(32) {
            let v = |i: usize| u64::from_le_bytes(q[8 * i..8 * i + 8].try_into().expect("8 bytes"));
            // The target's host bit (a view's home's host word): on the program's Rust, the host's address is the
            // device's.
            let (i, o, t, off) = (v(0) as usize, v(1) as usize, (v(2) & !(HOST | RELATIVE)) as usize, v(3) as usize);
            let host = mems.get(t).ok_or_else(|| bad(format!("a relocation into region {t} of {}", mems.len())))?.base() + off;
            let m = mems.get(i).ok_or_else(|| bad(format!("a relocation in region {i}")))?;
            let to = dev(t) + off as u64;
            // A relative word (an atoms directory's span): the target's address less the word's own, both as the
            // kernels reach them.
            let value = if v(2) & HOST != 0 {
                host as u64
            } else if v(2) & RELATIVE != 0 {
                to.wrapping_sub(dev(i) + o as u64)
            } else {
                to
            };
            m.write(o, &value.to_ne_bytes());
            bound += 1;
            if v(2) & HOST != 0 && e == Engine::Metal && !placed.contains(&(i, o + 8)) {
                m.write(o + 8, &to.to_ne_bytes());
                bound += 1;
            }
        }
        let cd = rerail_tape_format::codes_dir::bytes();
        for (region, at, digest) in &self.codes {
            if *digest != capture_digest(cd) {
                return Err(bad(format!("the codes' directory is not the engine's (its digest {:016x}, the capture's {digest:016x}): a lab of another build", capture_digest(cd))));
            }
            let i = self.regions.iter().position(|r| r.name == *region).ok_or_else(|| bad(format!("codes: no region {region}")))?;
            // Its home's host word, and its device word: the device's wrap of it (`Layer::codes`), none on the host.
            #[allow(unused_mut)]
            let mut gpu = 0u64;
            #[cfg(feature = "metal")]
            if let Some(w) = &wraps {
                gpu = w.codes_address().map_err(bad)?;
            }
            mems[i].write(*at, &(cd.as_ptr() as u64).to_ne_bytes());
            mems[i].write(*at + 8, &gpu.to_ne_bytes());
            bound += 1;
        }
        let (mut binds, mut bind_regions) = ([0u64; 9], [0usize; 9]);
        for (k, role) in self.binds.iter().enumerate() {
            let i = self.regions.iter().position(|r| r.role == *role).ok_or_else(|| bad(format!("no region for the bind {role}")))?;
            binds[k] = mems[i].base() as u64;
            bind_regions[k] = i;
        }
        Ok(Loaded {
            #[cfg(feature = "metal")]
            wraps,
            wave: self.clone(),
            mems,
            binds,
            bind_regions,
            relocations: bound,
            reverse: false,
        })
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

/// A made text's value (`join_program::arena_value`): its offset in the text arena's bytes and its length.
fn made_text(v: u64) -> Option<(usize, usize)> {
    (v >> 63 == 1).then(|| (((v >> 24) & 0x7f_ffff_ffff) as usize, (v & 0xff_ffff) as usize))
}

#[cfg(test)]
mod made_text_tests {
    #[test]
    fn a_made_texts_value_names_its_offset_and_length() {
        // `join_program::arena_value(0x961, 4)`.
        assert_eq!(super::made_text(0x8000_0009_6100_0004), Some((0x961, 4)));
        assert_eq!(super::made_text(0x4_0000_0001), None);
    }
}

/// The roles of the program's memory whose bytes hang on the order its groups ran in (a device's threads claim
/// places in the arena, its holds and its texts in the order they come), held to another engine's by what they make
/// (the launch's relations, its texts by value) and not by their bytes.
const ORDERED: &[&str] = &["counts", "hold", "text"];

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

    pub fn len(&self) -> usize {
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
    /// Its regions' wraps for the device (the `metal` engine's): released before the memory they wrap, as a field
    /// before `mems`.
    #[cfg(feature = "metal")]
    wraps: Option<metal::Wraps>,
    pub wave: Wave,
    pub mems: Vec<Mem>,
    /// `join_run`'s nine buffers' addresses, where its regions lie now, and their regions.
    binds: [u64; 9],
    bind_regions: [usize; 9],
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
    /// Its items' time by kind (the program's, `join_run_timed`): each kind's name, nanoseconds and items.
    pub kinds: Vec<(&'static str, u64, u64)>,
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
    /// Its chain on engine `e` (the one it was loaded for, `Wave::load_on`).
    pub fn run_on(&mut self, e: Engine) -> io::Result<Ran> {
        match e {
            Engine::Program => Ok(self.run()),
            Engine::Class => Err(bad(match self.classes() {
                Err(why) => why,
                Ok(_) => "the classes' executor (the design's step 7) is not built".into(),
            })),
            #[cfg(feature = "metal")]
            Engine::Metal => {
                let w = self.wraps.as_ref().ok_or_else(|| bad("the metal engine: a launch loaded for another engine".into()))?;
                metal::run(self, w, &self.bind_regions).map_err(bad)
            }
            #[cfg(not(feature = "metal"))]
            Engine::Metal => Err(bad("the metal engine: this lab was built without its `metal` feature".into())),
        }
    }

    /// The classes of its runs' shapes (`Wave::classes`).
    pub fn classes(&self) -> Result<Vec<&'static rerail_join_shapes::parts::Class>, String> {
        self.wave.classes()
    }

    /// Its chain through `join_run`, as the CPU device runs it: each dispatch's groups the three words at its offset
    /// of the control words, which the phases before it wrote; its groups one after another on this thread.
    pub fn run(&mut self) -> Ran {
        self.run_timed(false)
    }

    /// `run`, with its items' time by kind where `timed` (`join_run_timed`).
    pub fn run_timed(&mut self, timed: bool) -> Ran {
        let mut clock = rerail_ukl_rust::join_program::ItemClock::default();
        let clock_at: *mut rerail_ukl_rust::join_program::ItemClock = if timed { &raw mut clock } else { std::ptr::null_mut() };
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
                unsafe { rerail_ukl_rust::join_program::join_run_timed(argv.as_ptr(), &l, clock_at) };
            }
        }
        ran.seconds = start.elapsed().as_secs_f64();
        if timed {
            ran.kinds = (0..64).filter(|&k| clock.items[k] > 0).map(|k| (rerail_ukl_rust::join_program::item_kind_name(k), clock.nanos[k], clock.items[k])).collect();
        }
        ran
    }

    /// Each region the launch may have written, held to its after-image: its runs' bytes (a buffer of its own), or
    /// its used bytes (a block). Every region it names, compared; those equal are no `Diff` with `differ` 0. Where
    /// it is not held byte for byte (`across`: `Wave::exact_on`), the regions whose bytes hang on the order its groups ran in
    /// (`ORDERED`) are not compared, and a made text's value is held to the engine's by its text.
    pub fn compare(&self, across: bool) -> io::Result<Vec<Diff>> {
        let relocated = self.relocated()?;
        let mut out = Vec::new();
        // The text arena's bytes, the replay's and the engine's (its after-image's runs), for made texts' values.
        let text = self.wave.regions.iter().position(|r| r.role == "text");
        let mut theirs: Vec<(usize, Vec<u8>)> = Vec::new();
        if let (true, Some(t)) = (across, text)
            && let Some((_, source)) = self.wave.after.iter().find(|(n, _)| *n == self.wave.regions[t].name)
        {
            read_runs(&self.wave.source_path(source), |at, b| theirs.push((at, b.to_vec())))?;
        }
        let same_text = |u: u64, w: u64| -> bool {
            let (Some(t), Some((a, n)), Some((b, k))) = (text, made_text(u), made_text(w)) else { return false };
            let mine = &self.mems[t];
            let at = |o: usize| 16 + o;
            n == k && at(a) + n <= mine.len() && theirs.iter().any(|(r, bytes)| *r <= at(b) && at(b) + k <= r + bytes.len() && mine.bytes(at(a), n) == &bytes[at(b) - r..at(b) - r + k])
        };
        for (name, source) in &self.wave.after {
            let i = self.wave.regions.iter().position(|r| r.name == *name).ok_or_else(|| bad(format!("after: no region {name}")))?;
            let (r, m) = (&self.wave.regions[i], &self.mems[i]);
            if across && ORDERED.contains(&r.role.as_str()) {
                continue;
            }
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
                        let o = (at + k) / 8 * 8;
                        if across && o >= at && o + 8 <= at + want.len() {
                            let g = u64::from_ne_bytes(m.bytes(o, 8).try_into().expect("8 bytes"));
                            let w = u64::from_ne_bytes(want[o - at..o - at + 8].try_into().expect("8 bytes"));
                            if same_text(g, w) {
                                continue;
                            }
                        }
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

    /// The words the lab bound again (`relocs.zst`, and each view's home's device word), as (region, offset): where
    /// an after-image holds the engine's address and the replay the lab's.
    pub fn relocated(&self) -> io::Result<Vec<(usize, usize)>> {
        let relocs = zstd::stream::decode_all(open_named(&self.wave.dir.join("relocs.zst"))?)?;
        let w = |q: &[u8], k: usize| u64::from_le_bytes(q[8 * k..8 * k + 8].try_into().expect("8"));
        // And each view's home's device word beside its host word (`load_on` binds it on a device where none does).
        Ok(relocs.chunks_exact(32).flat_map(|q| {
            let at = (w(q, 0) as usize, w(q, 1) as usize);
            std::iter::once(at).chain((w(q, 2) & HOST != 0).then_some((at.0, at.1 + 8)))
        }).collect())
    }
}

impl Loaded {
    /// Its size script's ops as loaded (`SizeOp`: an address `w`, a word `c`, its op and registers), each address
    /// placed in the region whose wrap or memory holds it as the engine reaches them, and the word there as the host
    /// reads it: how a device's address that reaches no region shows (its reads zeros).
    pub fn script_ops(&self) -> Vec<String> {
        let script = self.bind_regions[6];
        let m = &self.mems[script];
        let reach = |i: usize| -> u64 {
            #[cfg(feature = "metal")]
            if let Some(w) = &self.wraps {
                return w.dev[i];
            }
            self.mems[i].base() as u64
        };
        let place = |a: u64| self.mems.iter().enumerate().find(|(i, mm)| reach(*i) <= a && a < reach(*i) + mm.len() as u64).map(|(i, _)| (i, (a - reach(i)) as usize));
        let word = |b: &[u8], k: usize| u64::from_ne_bytes(b[k..k + 8].try_into().expect("8 bytes"));
        let mut out = Vec::new();
        for i in 0..m.len() / 32 {
            let b = m.bytes(32 * i, 32);
            let (w, c, op) = (word(b, 0), word(b, 8), u32::from_ne_bytes(b[16..20].try_into().expect("4 bytes")));
            if w == 0 && c == 0 && op == 0 {
                continue;
            }
            let at = |a: u64| match place(a) {
                Some((r, o)) => format!("{}+{o} ({:#x})", self.wave.regions[r].name, u32::from_ne_bytes(self.mems[r].bytes(o, 4).try_into().expect("4 bytes"))),
                None if a == 0 => "0".to_owned(),
                None => format!("{a:#x} IN NO REGION"),
            };
            let k = |f: usize| u32::from_ne_bytes(b[f..f + 4].try_into().expect("4 bytes")) & 15;
            out.push(format!("op {i:>3}: {op:>2} r{} x{} y{} w {} c {}", k(20), k(24), k(28), at(w), if c > 1 << 32 { at(c) } else { format!("{c}") }));
        }
        out
    }
}

/// Where two engines' replays of a launch first part (`first_divergence`): the dispatch after which their regions
/// differ, the phase it ran and its items' kinds, and the regions that differ then (each its name, bytes, first word).
#[derive(Clone, Debug)]
pub struct Divergence {
    pub dispatch: usize,
    pub phase: u32,
    pub kinds: Vec<&'static str>,
    /// Within the dispatch, the phase after which they first differ (its walk ended past it), and its items' kinds.
    pub last: u32,
    pub walked: Vec<&'static str>,
    pub regions: Vec<(String, usize, Option<(usize, u64, u64)>)>,
}

impl Divergence {
    pub fn line(&self) -> String {
        let regions: Vec<String> = self.regions.iter().map(|(n, b, w)| match w {
            Some((o, x, y)) => format!("{n}: {b} bytes, first word at {o}: {x:#x} against {y:#x}"),
            None => format!("{n}: {b} bytes"),
        }).collect();
        format!("first differs after dispatch {} (phase {}, items {}), in it after phase {} (items {}): {}", self.dispatch, self.phase, self.kinds.join(" "), self.last, self.walked.join(" "), regions.join("; "))
    }
}

impl Loaded {
    /// Its chain's first `k` dispatches only.
    pub fn upto(&mut self, k: usize) {
        self.wave.offsets.truncate(k);
    }

    /// Its walk ended before phase `p` (`ProgP::end`, the program's own stop: a phase past it runs only inside the
    /// program's tail).
    pub fn end_at(&mut self, p: u32) {
        self.wave.args[2] = p;
    }

    /// Phase `p`'s items as loaded: each its kind's name, its `x` (a size item's first op, another's job), its group
    /// (a size item's ops) and its segment.
    pub fn phase_items(&self, p: u32) -> Vec<(&'static str, u32, u32, u32)> {
        let word = |region: usize, at: usize| u32::from_ne_bytes(self.mems[region].bytes(at, 4).try_into().expect("4 bytes"));
        let (phases, segs, items) = (self.bind_regions[2], self.bind_regions[3], self.bind_regions[1]);
        let at = 16 * p as usize;
        if at + 16 > self.mems[phases].len() {
            return Vec::new();
        }
        let [first, count, s0, ns] = [0, 1, 2, 3].map(|k| word(phases, at + 4 * k));
        let runs: Vec<(u32, u32)> = if ns == 0 { vec![(first, count)] } else { (s0..s0 + ns).map(|s| (word(segs, 24 * s as usize + 8), word(segs, 24 * s as usize + 12))).collect() };
        runs.iter().flat_map(|&(f, n)| f..f + n).map(|i| {
            let what = word(items, 16 * i as usize);
            (rerail_ukl_rust::join_program::item_kind_name((what >> 24) as usize), what & 0xff_ffff, word(items, 16 * i as usize + 4), word(items, 16 * i as usize + 12))
        }).collect()
    }

    /// The phase its next dispatch runs (its control words' first, `ctl(0)`), and that phase's items' kinds.
    fn next_phase(&self) -> (u32, Vec<&'static str>) {
        let word = |region: usize, at: usize| u32::from_ne_bytes(self.mems[region].bytes(at, 4).try_into().expect("4 bytes"));
        let (ctl, phases, segs, items) = (self.bind_regions[7], self.bind_regions[2], self.bind_regions[3], self.bind_regions[1]);
        let p = word(ctl, 0);
        let at = 16 * p as usize;
        if at + 16 > self.mems[phases].len() {
            return (p, Vec::new());
        }
        let [first, count, s0, ns] = [0, 1, 2, 3].map(|k| word(phases, at + 4 * k));
        // Its items: its own run of them, or each segment's (`Seg`: its count's address, then its first and items).
        let runs: Vec<(u32, u32)> = if ns == 0 { vec![(first, count)] } else { (s0..s0 + ns).map(|s| (word(segs, 24 * s as usize + 8), word(segs, 24 * s as usize + 12))).collect() };
        let mut kinds: Vec<&'static str> = runs.iter().flat_map(|&(f, n)| (f..f + n).map(|i| rerail_ukl_rust::join_program::item_kind_name((word(items, 16 * i as usize) >> 24) as usize))).collect();
        kinds.dedup();
        (p, kinds)
    }

    /// Its regions held to `other`'s (the same launch, loaded again: its regions elsewhere): each that differs, a word
    /// counted equal where each holds an address at the same offset of the same region (an address its kernel wrote).
    /// With `across` (either not held byte for byte, `Wave::exact_on`), the regions `ORDERED` names not compared, and a made text's value held to the
    /// other's by its text.
    fn against(&self, other: &Loaded, across: bool) -> Vec<(String, usize, Option<(usize, u64, u64)>)> {
        // The words bound again, by the capture's relocations and the codes' homes (whose device word is each
        // engine's own: none on the host, its wrap's on the device).
        let mut relocated: std::collections::HashSet<(usize, usize)> = self.relocated().unwrap_or_default().into_iter().collect();
        for (region, at, _) in &self.wave.codes {
            if let Some(i) = self.wave.regions.iter().position(|r| r.name == *region) {
                relocated.extend([(i, *at), (i, *at + 8)]);
            }
        }
        let place = |l: &Loaded, a: u64| l.mems.iter().position(|m| (m.base() as u64) <= a && a < (m.base() + m.len()) as u64).map(|i| (i, a - l.mems[i].base() as u64));
        let text = self.wave.regions.iter().position(|r| r.role == "text");
        let same_text = |u: u64, v: u64| -> bool {
            let (Some(t), Some((a, n)), Some((b, k))) = (text, made_text(u), made_text(v)) else { return false };
            let (x, y) = (&self.mems[t], &other.mems[t]);
            n == k && 16 + a + n <= x.len() && 16 + b + k <= y.len() && x.bytes(16 + a, n) == y.bytes(16 + b, k)
        };
        let mut out = Vec::new();
        for (i, r) in self.wave.regions.iter().enumerate() {
            let (a, b) = (&self.mems[i], &other.mems[i]);
            if matches!(a, Mem::Mapped(_)) || (across && ORDERED.contains(&r.role.as_str())) {
                continue;
            }
            // The pages either holds (a reservation's others read zero in both, and reading them would make them).
            let n = r.len.min(a.len()).min(b.len()) / 8 * 8;
            let mut pages = std::collections::BTreeSet::new();
            for m in [a, b] {
                for (o, len) in rerail_blocks::mem::written_runs(m.base(), n) {
                    pages.extend(o / PAGE..(o + len).div_ceil(PAGE));
                }
            }
            let (mut differ, mut first) = (0usize, None);
            for o in pages.iter().flat_map(|&p| (p * PAGE..((p + 1) * PAGE).min(n)).step_by(8)) {
                let (x, y) = (a.bytes(o, 8), b.bytes(o, 8));
                if x == y {
                    continue;
                }
                let (u, v) = (u64::from_ne_bytes(x.try_into().expect("8")), u64::from_ne_bytes(y.try_into().expect("8")));
                if u == v || relocated.contains(&(i, o)) || (u > 1 << 32 && place(self, u).is_some() && place(self, u) == place(other, v)) || (across && same_text(u, v)) {
                    continue;
                }
                differ += 8;
                first.get_or_insert((o, u, v));
            }
            if differ > 0 {
                out.push((r.name.clone(), differ, first));
            }
        }
        out
    }
}

/// The first dispatch after which `a` and `b` leave `w`'s regions otherwise (each run of a dispatch's prefix loaded
/// afresh): bisected over its dispatches, a difference once made taken to stay. None where they agree after them all.
pub fn first_divergence(w: &Wave, a: Engine, b: Engine) -> io::Result<Option<Divergence>> {
    let after = |k: usize| -> io::Result<(Loaded, Loaded)> {
        let mut x = w.load_on(a)?;
        x.upto(k);
        x.run_on(a)?;
        let mut y = w.load_on(b)?;
        y.upto(k);
        y.run_on(b)?;
        Ok((x, y))
    };
    let n = w.offsets.len();
    let (x, y) = after(n)?;
    if x.against(&y, !(w.exact_on(a) && w.exact_on(b))).is_empty() {
        return Ok(None);
    }
    drop((x, y));
    // `lo` agree, `hi` differ.
    let (mut lo, mut hi) = (0usize, n);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let (x, y) = after(mid)?;
        if x.against(&y, !(w.exact_on(a) && w.exact_on(b))).is_empty() { lo = mid } else { hi = mid }
    }
    let mut before = w.load_on(a)?;
    before.upto(lo);
    before.run_on(a)?;
    let (phase, kinds) = before.next_phase();
    // Within dispatch `hi` (a dispatch walks phase after phase): the first phase after which they differ, each walk
    // ended there (`end_at`). `plo` agree, `phi` differ.
    let ended = |p: u32| -> io::Result<bool> {
        let run = |e: Engine| -> io::Result<Loaded> {
            let mut l = w.load_on(e)?;
            l.upto(hi);
            l.end_at(p);
            l.run_on(e)?;
            Ok(l)
        };
        Ok(!run(a)?.against(&run(b)?, !(w.exact_on(a) && w.exact_on(b))).is_empty())
    };
    let (mut plo, mut phi) = (phase, w.args[2]);
    if phi > plo && ended(phi)? {
        while phi - plo > 1 {
            let mid = (plo + phi) / 2;
            if ended(mid)? { phi = mid } else { plo = mid }
        }
    }
    let last = phi.saturating_sub(1);
    let walked = before.phase_items(last).iter().map(|i| i.0).collect::<Vec<_>>();
    let (x, y) = after(hi)?;
    Ok(Some(Divergence { dispatch: hi, phase, kinds, last, walked: walked.into_iter().fold(Vec::new(), |mut v, k| { if v.last() != Some(&k) { v.push(k) } v }), regions: x.against(&y, !(w.exact_on(a) && w.exact_on(b))) }))
}

/// An engine a launch is replayed on: the join program's Rust (`join_run`, the CPU device's), or its kernel on the
/// device (`metal`, a lab built with its `metal` feature, on a Mac).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Program,
    Metal,
    /// Its shapes' classes (`rerail_join_shapes::parts::CLASSES`), where each has one.
    Class,
}

impl Engine {
    /// The engine `name` names (`--engine`).
    pub fn named(name: &str) -> Result<Engine, String> {
        match name {
            "program" => Ok(Engine::Program),
            "metal" => Ok(Engine::Metal),
            "class" => Ok(Engine::Class),
            _ => Err(format!("no engine {name} (engines: class, program, metal)")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Program => "program",
            Engine::Metal => "metal",
            Engine::Class => "class",
        }
    }
}

impl Wave {
    /// Its directory's name: the process and capture number (`<pid>-<id>`).
    pub fn name(&self) -> String {
        self.dir.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
    }

    /// The classes of its runs' shapes (`rerail_join_shapes::parts::CLASSES`), or the shapes that have none.
    pub fn classes(&self) -> Result<Vec<&'static rerail_join_shapes::parts::Class>, String> {
        let (mut found, mut none) = (Vec::new(), Vec::new());
        for l in &self.labels {
            match rerail_join_shapes::parts::key_of(&l.shape).and_then(rerail_join_shapes::parts::class_of) {
                Some(c) => found.push(c),
                None => none.push(l.shape.as_str()),
            }
        }
        if none.is_empty() { Ok(found) } else { Err(format!("no class: {}", none.join(" "))) }
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
    let mut l = w.load_on(e)?;
    l.reverse = reverse;
    let ran = l.run_on(e)?;
    let mut diffs = l.compare(!w.exact_on(e))?;
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
    /// The median run's instructions and cycles (`counters`), 0 where the host gives none.
    pub instructions: u64,
    pub cycles: u64,
    pub bytes: u64,
    pub same: bool,
    /// Its items' time by kind, of a run of its own (the program's; the clock's own time is not in the others'):
    /// each kind's name, nanoseconds and items.
    pub kinds: Vec<(&'static str, u64, u64)>,
}

impl Profile {
    pub fn header() -> String {
        "launch\tengine\tdispatches\tgroups\trepeat\tbest_ms\tmedian_ms\tcpu_ms\tinstructions\tcycles\tbytes\tsame\tshapes\tkinds".to_owned()
    }

    pub fn tsv(&self) -> String {
        let shapes: Vec<String> = self.shapes.iter().map(|(s, r, w)| format!("{s}|{r}|{w}")).collect();
        let kinds: Vec<String> = self.kinds.iter().map(|(k, ns, n)| format!("{k}|{ns}|{n}")).collect();
        format!("{}\t{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}\t{}\t{}\t{}\t{}", self.launch, self.engine, self.dispatches, self.groups, self.repeat, self.best_ms, self.median_ms, self.cpu_ms, self.instructions, self.cycles, self.bytes, self.same, shapes.join(","), kinds.join(","))
    }

    pub fn line(&self) -> String {
        let shapes: Vec<&str> = self.shapes.iter().map(|s| s.0.as_str()).collect();
        format!("{:<16} {:<8} {:>4} disp {:>9} groups  best {:>9.3} ms  median {:>9.3} ms  cpu {:>9.3} ms  {:>7.2} M instr  {:>7.2} M cycles  {}  {}", self.launch, self.engine, self.dispatches, self.groups, self.best_ms, self.median_ms, self.cpu_ms, self.instructions as f64 / 1e6, self.cycles as f64 / 1e6, if self.same { "same" } else { "DIFFERS" }, shapes.join(" "))
    }
}

/// `w` run `repeat` times on `e`, each loaded afresh (the load not timed): its wall times' best and median, the
/// CPU time of its median run, and whether its last run left what the engine's launch left. On the program, one run
/// more gives its items' time by kind. Its runs (not its loads) are `sampler`'s, tagged `tag`, where it is given.
pub fn profile(w: &Wave, e: Engine, repeat: usize, sampler: Option<(&sample::Sampler, u32)>) -> io::Result<Profile> {
    let counters = counters::Counters::open();
    let count = || counters.as_ref().map_or(counters::Counts::default(), counters::Counters::read);
    let mut times = Vec::with_capacity(repeat);
    let mut last = None;
    for _ in 0..repeat.max(1) {
        let mut l = w.load_on(e)?;
        if let Some((s, t)) = sampler {
            s.tag(t);
        }
        let (cpu, c) = (thread_cpu(), count());
        let ran = l.run_on(e);
        let (cpu, c) = (thread_cpu() - cpu, count() - c);
        if let Some((s, _)) = sampler {
            s.tag(0);
        }
        let ran = ran?;
        times.push((ran.seconds, cpu, c));
        last = Some((ran, l));
    }
    let (ran, l) = last.expect("a run");
    let same = l.compare(!w.exact_on(e))?.iter().all(|d| d.differ == 0);
    drop(l);
    let kinds = if e == Engine::Program { w.load()?.run_timed(true).kinds } else { ran.kinds.clone() };
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
        instructions: median.2.instructions,
        cycles: median.2.cycles,
        bytes: w.bytes(),
        same,
        kinds,
    })
}
