//! Join captures (the join lab's, its design's §3): a launch of the join program written as what it reads, so the
//! lab replays it apart from the engine. `RERAIL_JOIN_CAPTURE=<dir>` turns it on (read once a batch); a launch is
//! captured where one of the shapes its runs encoded (`Label`) does more work there than in any launch captured
//! before (by default: the rows of the inputs its stages read), or as `_ONLY=` (a text its labels hold) and `_MIN=`
//! (the least work) select. A capture past the size budget (`_MB=`, 2048 by default) is refused, and the profile
//! names it.
//!
//! What it writes, under `<dir>`:
//! - `blocks/<digest>.blk`: each anonymous block the launch reaches, its bytes up to its used length with its header
//!   frozen (`Block::map_file` maps it back), written once however many launches reach it. A block mapped from a file
//!   (the tape store's, the atoms') is not copied: the capture names its path.
//! - `w/<id>/wave.txt`: the launch: its program's format and arguments, its chain's count offsets, the nine buffers
//!   `join_run` binds, and each region it reaches (`region <name> <role> <base> <device base> <len> <kind> <source>`),
//!   its labels.
//! - `w/<id>/<name>.zst`: a region of the program's own (its descriptors, items, phases, control words, counts, the
//!   text arena) or a lease (memory a ledger holds with no block's header), as it was at the launch: its written
//!   pages' runs (`rerail_blocks::mem::written_runs`; a reservation of gigabytes is mostly never touched), each as its
//!   offset and length (u64s) then its bytes, zstd; the pages between read as fresh memory does. `<name>.after.zst`
//!   as the launch left it, and each written block's bytes after it as `blocks/<digest>.blk` (`after <name>
//!   <source>`).
//! - `w/<id>/relocs.zst`: every address the program's regions hold (its buffers' address words by their structures,
//!   and its parameters' words that hold an address its jobs reach, `Job::reach`), as (region, offset, region it
//!   points into, offset there): decision 0353's two-part references, bound again where the replay places each
//!   region.

use rerail_device::device::Buffer;
use rerail_reads::views::ViewDir;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// A run encoded into a batch: its shape (`rerail_join_shapes`), its rule, the work it does (its inputs' rows).
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Label {
    pub(in crate::code_rule_dsl) shape: String,
    pub(in crate::code_rule_dsl) rule: String,
    pub(in crate::code_rule_dsl) work: u64,
}

/// Where captures go, and what selects them (`RERAIL_JOIN_CAPTURE`, `_ONLY`, `_MIN`, `_MB`): read once a batch.
#[derive(Clone, Debug)]
pub(in crate::code_rule_dsl) struct Config {
    dir: PathBuf,
    only: Option<String>,
    min: u64,
    budget: u64,
}

impl Config {
    pub(in crate::code_rule_dsl) fn from_env() -> Option<Config> {
        let set = crate::code_rule_dsl::logic::profile::machine::<Chosen>().0.lock().unwrap_or_else(|e| e.into_inner()).dir.clone();
        let dir = match set {
            Some(d) => d,
            None => PathBuf::from(std::env::var_os("RERAIL_JOIN_CAPTURE").filter(|d| !d.is_empty())?),
        };
        let num = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        Some(Config { dir, only: std::env::var("RERAIL_JOIN_CAPTURE_ONLY").ok().filter(|s| !s.is_empty()), min: num("RERAIL_JOIN_CAPTURE_MIN", 0), budget: num("RERAIL_JOIN_CAPTURE_MB", 2048) << 20 })
    }
}

/// The process's captures so far: each shape's best launch (its work, its capture), the launches each capture is
/// best for, the blocks written, the bytes written.
#[derive(Default)]
struct Chosen(std::sync::Mutex<ChosenState>);

#[derive(Default)]
struct ChosenState {
    /// Where the machine's captures go, set by its caller (`capture_to`): `RERAIL_JOIN_CAPTURE` where none is.
    dir: Option<PathBuf>,
    best: HashMap<String, (u64, u64)>,
    holds: HashMap<u64, usize>,
    blocks: std::collections::HashSet<u64>,
    bytes: u64,
    next: u64,
}

/// The machine's join launches captured under `dir` from its next batch on (`RERAIL_JOIN_CAPTURE`'s, set by a caller).
pub fn capture_to(dir: PathBuf) {
    crate::code_rule_dsl::logic::profile::machine::<Chosen>().0.lock().unwrap_or_else(|e| e.into_inner()).dir = Some(dir);
}

/// One region a launch reaches: its role, its memory, and where its bytes are kept.
struct Region {
    name: String,
    role: &'static str,
    base: usize,
    /// Where the device reaches it (`Buffer::device_address`): the address the program's words hold, the host's
    /// (`base`) on the CPU device, the GPU's on Metal.
    dev: usize,
    len: usize,
    kind: u32,
    /// Its bytes' source: a block file (`blocks/<digest>.blk`), a mapped file (`file:<path>`), or its own runs
    /// (`<name>.zst`).
    source: String,
    /// Whether the launch may write it: its bytes after it are kept too.
    written: bool,
}

impl Region {
    /// Its bytes kept as runs of its written pages (`<name>.zst`): a buffer of the program's own, or a lease.
    fn sparse(&self) -> bool {
        self.source.ends_with(".zst")
    }

    /// Its written pages' runs, as (offset, bytes).
    fn runs(&self) -> Vec<(usize, usize)> {
        rerail_blocks::mem::written_runs(self.base, self.len)
    }

    /// Its bytes `at..at + n`.
    fn bytes(&self, at: usize, n: usize) -> &[u8] {
        // SAFETY: a run of the region's written pages, read while no work of the launch runs (before its submission,
        // after its completion).
        unsafe { std::slice::from_raw_parts((self.base + at) as *const u8, n) }
    }
}

/// A launch being captured: its directory, its regions; finished once the launch is done (`after`).
pub(in crate::code_rule_dsl) struct Pending {
    dir: PathBuf,
    root: PathBuf,
    regions: Vec<Region>,
}

/// The program's own buffers a launch binds, in `join_run`'s order, and the rest it reaches.
pub(in crate::code_rule_dsl) struct Reached<'a> {
    pub(in crate::code_rule_dsl) binds: [(&'static str, &'a Buffer); 9],
    /// The words of its own buffers that hold addresses by their structures: each its buffer's role and offset.
    pub(in crate::code_rule_dsl) typed: Vec<(&'static str, u64)>,
    /// The addresses its jobs reach (`Job::reach`): a word of its parameters (`hold`) that holds one is an address.
    pub(in crate::code_rule_dsl) reached: &'a [u64],
    pub(in crate::code_rule_dsl) top: Option<&'a Buffer>,
    pub(in crate::code_rule_dsl) holds: Vec<&'a Buffer>,
    pub(in crate::code_rule_dsl) text: Option<&'a Buffer>,
    pub(in crate::code_rule_dsl) table: Option<&'a Buffer>,
    /// Blocks and leases its jobs reach by address, each with whether it is written.
    pub(in crate::code_rule_dsl) uses: Vec<(&'a Buffer, bool)>,
    pub(in crate::code_rule_dsl) format: u32,
    pub(in crate::code_rule_dsl) args: [u32; 4],
    pub(in crate::code_rule_dsl) offsets: Vec<u64>,
}

fn digest(bytes: &[u8]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    std::hash::Hasher::write(&mut h, bytes);
    std::hash::Hasher::finish(&h) ^ (bytes.len() as u64).rotate_left(32)
}

/// A region's written runs as `path` (each its offset and length, then its bytes; zstd): the bytes written.
fn write_runs(path: &Path, r: &Region, runs: &[(usize, usize)]) -> std::io::Result<u64> {
    let mut z = zstd::stream::write::Encoder::new(std::fs::File::create(path)?, 3)?;
    for &(at, n) in runs {
        z.write_all(&(at as u64).to_le_bytes())?;
        z.write_all(&(n as u64).to_le_bytes())?;
        z.write_all(r.bytes(at, n))?;
    }
    z.finish()?;
    Ok(std::fs::metadata(path)?.len())
}

/// The machine's state, locked.
fn chosen() -> std::sync::Arc<Chosen> {
    crate::code_rule_dsl::logic::profile::machine::<Chosen>()
}

/// Whether to capture a launch of `labels`, and its number: past the selection, the size budget refuses it.
fn choose(cfg: &Config, labels: &[Label]) -> Option<u64> {
    if let Some(only) = &cfg.only
        && !labels.iter().any(|l| l.shape.contains(only.as_str()) || l.rule.contains(only.as_str()))
    {
        return None;
    }
    let chosen = chosen();
    let mut st = chosen.0.lock().unwrap_or_else(|e| e.into_inner());
    if st.bytes >= cfg.budget {
        crate::code_rule_dsl::logic::profile::decision("a join capture refused: past its size budget (RERAIL_JOIN_CAPTURE_MB)", || format!("{} MB written", st.bytes >> 20));
        return None;
    }
    // Each shape once, at its most work in this launch (two runs of a launch may share a shape).
    let mut most: HashMap<&str, u64> = HashMap::new();
    for l in labels.iter().filter(|l| l.work >= cfg.min) {
        let w = most.entry(l.shape.as_str()).or_default();
        *w = (*w).max(l.work);
    }
    let wins: Vec<(&str, u64)> = most.into_iter().filter(|(s, w)| st.best.get(*s).is_none_or(|b| *w > b.0)).collect();
    if wins.is_empty() {
        return None;
    }
    let id = st.next;
    st.next += 1;
    let mut gone = Vec::new();
    for (shape, work) in wins {
        if let Some((_, old)) = st.best.insert(shape.to_owned(), (work, id)) {
            let n = st.holds.get_mut(&old).expect("a capture's count");
            *n -= 1;
            if *n == 0 {
                gone.push(old);
            }
        }
        *st.holds.entry(id).or_default() += 1;
    }
    for old in gone {
        st.holds.remove(&old);
        // No shape's best now: its launch goes (its blocks stay, shared).
        let _ = std::fs::remove_dir_all(wave_dir(cfg, old));
    }
    Some(id)
}

/// A capture's directory: the process's, so processes writing one directory keep apart.
fn wave_dir(cfg: &Config, id: u64) -> PathBuf {
    cfg.dir.join("w").join(format!("{}-{id:06}", std::process::id()))
}

/// Captures the launch `reach` reaches, its runs `labels`, where the selection takes it: its regions written as it is
/// about to run. `None`: not captured (or its writing failed, which the profile names).
pub(in crate::code_rule_dsl) fn before(cfg: &Config, labels: &[Label], reach: &Reached<'_>) -> Option<Pending> {
    let id = choose(cfg, labels)?;
    match write_before(cfg, id, labels, reach) {
        Ok(p) => Some(p),
        Err(e) => {
            crate::code_rule_dsl::logic::profile::decision("a join capture not written", || format!("{e}"));
            let _ = std::fs::remove_dir_all(wave_dir(cfg, id));
            None
        }
    }
}

/// A relocation's target bit (`relocs.zst`'s third word): the word holds the target's host address (a view's home's
/// `host`), not the device's.
const HOST: u64 = 1 << 63;

/// A relocation's relative bit: the word holds the target's address less its own (an atoms directory's span into
/// a run's block, `rerail_blocks::atoms::DirEntry`), in the reader's address space.
const RELATIVE: u64 = 1 << 62;

/// An atoms directory's entry's bytes, and its spans' offsets in it with their targets' element sizes: `ends`,
/// `strings`, `table` (`rerail_blocks::atoms::DirEntry`, each span an `off: i64` then a `len: u64`).
const DIR_ENTRY: usize = 64;
const DIR_SPANS: [(usize, usize); 3] = [(16, 8), (32, 1), (48, 8)];

/// The block or lease `p` holds (its device address `dev`) as region `r<n>`: a lease's written pages as its own runs,
/// a block mapped from a file by its path, any other block's used bytes as a block file (counted in `written`).
fn place(cfg: &Config, p: &rerail_blocks::ledger::Placed, dev: usize, w: bool, n: usize, written: &mut u64) -> std::io::Result<Region> {
    let name = format!("r{n}");
    if p.kind == 0 {
        return Ok(Region { source: format!("{name}.zst"), name, role: "lease", base: p.start, dev, len: p.end - p.start, kind: 0, written: w });
    }
    // Its used bytes: its top, or past it where it was laid out by its own writes, not bumped (an atoms block's
    // directory and atoms: its top its header's), to its last written page.
    let written_to = if p.path.is_none() { rerail_blocks::mem::written_runs(p.start, p.end - p.start).last().map_or(0, |&(at, n)| at + n) } else { 0 };
    let len = if p.top > 0 { p.top.max(written_to).min(p.end - p.start) } else { p.end - p.start };
    let source = match &p.path {
        Some(path) => format!("file:{}", path.display()),
        None => {
            within_budget(cfg, *written, len as u64)?;
            // SAFETY: the block's used bytes, before the launch.
            let bytes = unsafe { std::slice::from_raw_parts(p.start as *const u8, len) };
            let (source, n) = write_block(cfg, bytes)?;
            *written += n;
            source
        }
    };
    Ok(Region { name, role: "block", base: p.start, dev, len, kind: p.kind, source, written: w })
}

/// A buffer of the program's own as a region (`<name>.zst`), once.
fn own(regions: &mut Vec<Region>, name: String, role: &'static str, b: &Buffer) {
    let (base, len) = (b.contents() as usize, b.length() as usize);
    if len == 0 || regions.iter().any(|r| r.base == base && r.len == len) {
        return;
    }
    let dev = b.device_address().map_or(base, |a| a as usize);
    regions.push(Region { source: format!("{name}.zst"), name, role, base, dev, len, kind: 0, written: true });
}

/// The region whose device range holds `a`.
fn at_dev(regions: &[Region], a: usize) -> Option<usize> {
    regions.iter().position(|t| t.dev <= a && a < t.dev + t.len)
}

/// The word at byte `o` of region `r`.
fn word(r: &Region, o: usize) -> usize {
    u64::from_ne_bytes(r.bytes(o, 8).try_into().expect("8 bytes")) as usize
}

fn write_before(cfg: &Config, id: u64, labels: &[Label], reach: &Reached<'_>) -> std::io::Result<Pending> {
    let dir = wave_dir(cfg, id);
    rerail_blocks::mapped::make_folder(&dir)?;
    rerail_blocks::mapped::make_folder(&cfg.dir.join("blocks"))?;
    let mut regions: Vec<Region> = Vec::new();
    for (role, b) in reach.binds {
        own(&mut regions, role.to_owned(), role, b);
    }
    if let Some(t) = reach.top {
        own(&mut regions, "top".into(), "top", t);
    }
    for (i, h) in reach.holds.iter().enumerate() {
        own(&mut regions, format!("hold{i}"), "hold", h);
    }
    if let Some(t) = reach.text {
        own(&mut regions, "text".into(), "text", t);
    }
    if let Some(t) = reach.table {
        own(&mut regions, "table".into(), "table", t);
    }
    // Its words that hold addresses (region, offset): its buffers' by their structures, and its parameters' that
    // hold an address its jobs reach (as `Kit::grow_rooms` finds them). No other word is read as one: a pair of
    // 32-bit values may look like an address inside a reservation of gigabytes.
    let reached: std::collections::HashSet<usize> = reach.reached.iter().map(|&a| a as usize).collect();
    let mut refs: Vec<(usize, usize)> = Vec::new();
    for &(role, o) in &reach.typed {
        if let Some(i) = regions.iter().position(|r| r.role == role)
            && (o as usize) + 8 <= regions[i].len
        {
            refs.push((i, o as usize));
        }
    }
    for (i, r) in regions.iter().enumerate().filter(|(_, r)| r.role == "hold") {
        for (at, n) in r.runs() {
            for o in (at.next_multiple_of(8)..(at + n) / 8 * 8).step_by(8) {
                if reached.contains(&word(r, o)) {
                    refs.push((i, o));
                }
            }
        }
    }
    // The blocks and leases its jobs reach (each buffer with its host and device addresses), and those its words
    // reach: a word holds a device address, the host's only where the device's memory is the host's (the CPU
    // device's), where the ledger finds what holds it.
    let identity = reach.binds[0].1.device_address().is_none_or(|a| a as usize == reach.binds[0].1.contents() as usize);
    let mut targets: Vec<(usize, usize, bool)> = reach.uses.iter().map(|(b, w)| (b.contents() as usize, b.device_address().map_or(b.contents() as usize, |a| a as usize), *w)).collect();
    if identity {
        targets.extend(refs.iter().map(|&(i, o)| word(&regions[i], o)).chain(reach.reached.iter().map(|&a| a as usize)).filter(|&a| a >= 1 << 32).map(|a| (a, a, false)));
    }
    let mut written = 0u64;
    for (addr, dev, w) in targets {
        if let Some(r) = regions.iter_mut().find(|r| r.base <= addr && addr < r.base + r.len) {
            r.written |= w;
            continue;
        }
        let Some(p) = rerail_blocks::ledger::placed_at(addr) else { continue };
        regions.push(place(cfg, &p, dev - (addr - p.start), w, regions.len(), &mut written)?);
    }
    // The views' directories its words reach (each self-describing: its own offset in its block, its homes there),
    // and their homes: each tape's, lines' and source's memory, the 16z cache's (which the reads write), each placed
    // where it lies; the binary's codes' directory, named (`codes`), which a replay binds to its own.
    let mut homes: Vec<(usize, usize, bool)> = Vec::new();
    // An atoms directory's spans whose targets no region holds, or past it.
    let mut unresolved_atoms = 0u64;
    let mut codes: Vec<(usize, usize)> = Vec::new();
    let mut dirs = std::collections::HashSet::new();
    for &(i, o) in &refs.clone() {
        let a = word(&regions[i], o);
        let Some(t) = at_dev(&regions, a).filter(|&t| regions[t].role == "block") else { continue };
        let off = a - regions[t].dev;
        if !dirs.insert((t, off)) || off + std::mem::size_of::<ViewDir>() > regions[t].len {
            continue;
        }
        // SAFETY: within the block's used bytes, read unaligned.
        let vd: ViewDir = unsafe { std::ptr::read_unaligned((regions[t].base + off) as *const ViewDir) };
        let n = vd.nhomes as usize;
        if vd.at as usize != off || n == 0 || vd.codes as usize >= n || vd.homes as usize + 16 * n > regions[t].len {
            continue;
        }
        for h in 0..n {
            let ho = vd.homes as usize + 16 * h;
            let (host, gpu) = (word(&regions[t], ho), word(&regions[t], ho + 8));
            if host == 0 {
                continue;
            }
            if h == vd.codes as usize && host == rerail_tape_format::codes_dir::bytes().as_ptr() as usize {
                codes.push((t, ho));
                continue;
            }
            // The cache's home (the one before the codes'): the reads write it.
            let cache = h + 1 == vd.codes as usize;
            if let Some(r) = regions.iter_mut().find(|r| r.base <= host && host < r.base + r.len) {
                r.written |= cache;
            } else if let Some(p) = rerail_blocks::ledger::placed_at(host) {
                let pdev = if gpu != 0 { gpu - (host - p.start) } else { p.start };
                let r = place(cfg, &p, pdev, cache, regions.len(), &mut written)?;
                regions.push(r);
            }
            homes.push((t, ho, true));
            if gpu != 0 {
                homes.push((t, ho + 8, false));
            }
        }
    }
    // The atoms directories its words reach (each found by its structure: a root in an atoms block whose entries'
    // span is in its own block, as many as its count, their runs in order): each entry's spans into its run's block
    // bound again as relative words; the spans within the directory's block move with it.
    let mut relative: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut roots = std::collections::HashSet::new();
    // Each directory found: its region, its root's offset, its runs.
    let mut atoms_dirs: Vec<(usize, usize, u64)> = Vec::new();
    for a in refs.iter().map(|&(i, o)| word(&regions[i], o)).collect::<Vec<_>>() {
        let Some(i) = at_dev(&regions, a).filter(|&i| regions[i].role == "block" && regions[i].kind == rerail_blocks::block::kind::ATOMS) else { continue };
        let ro = a - regions[i].dev;
        if !roots.insert((i, ro)) || ro + 24 > regions[i].len {
            continue;
        }
        let (n, eoff, elen) = (word(&regions[i], ro), word(&regions[i], ro + 8) as i64, word(&regions[i], ro + 16));
        let eo = (ro as i64 + 8).wrapping_add(eoff);
        if n == 0 || n != elen || eo < 0 || eo as usize + n * DIR_ENTRY > regions[i].len {
            continue;
        }
        let eo = eo as usize;
        // SAFETY: inside the region's used bytes (checked above), read unaligned.
        let u32_at = |o: usize| unsafe { std::ptr::read_unaligned((regions[i].base + o) as *const u32) };
        if (0..n).any(|k| u32_at(eo + k * DIR_ENTRY) > u32_at(eo + k * DIR_ENTRY + 4)) {
            continue;
        }
        atoms_dirs.push((i, ro, n as u64));
        for k in 0..n {
            for (at, size) in DIR_SPANS {
                let so = eo + k * DIR_ENTRY + at;
                let (off, len) = (word(&regions[i], so) as i64, word(&regions[i], so + 8));
                let target = (regions[i].dev as i64 + so as i64).wrapping_add(off) as usize;
                let t = match at_dev(&regions, target) {
                    Some(t) => Some(t),
                    None if identity => match rerail_blocks::ledger::placed_at(target) {
                        Some(p) => {
                            let r = place(cfg, &p, p.start, false, regions.len(), &mut written)?;
                            regions.push(r);
                            Some(regions.len() - 1)
                        }
                        None => None,
                    },
                    None => None,
                };
                match t.filter(|&t| target - regions[t].dev + len * size <= regions[t].len) {
                    Some(t) => relative.push((i, so, t, target - regions[t].dev)),
                    None => unresolved_atoms += 1,
                }
            }
        }
    }
    // The sparse regions' runs, and each address word as (region, offset, region it points into, offset there).
    for r in regions.iter().filter(|r| r.sparse()) {
        let runs = r.runs();
        within_budget(cfg, written, runs.iter().map(|&(_, n)| n as u64).sum())?;
        written += write_runs(&dir.join(&r.source), r, &runs)?;
    }
    let mut relocs: Vec<u8> = Vec::new();
    // Address words no region holds: memory no ledger knew (a replay that reads one fails there).
    let mut unresolved = unresolved_atoms;
    for &(i, o, t, off) in &relative {
        for v in [i as u64, o as u64, t as u64 | RELATIVE, off as u64] {
            // copy: not block data (a capture's relocation list, for the lab alone)
            relocs.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut unresolved_in: Vec<(usize, u64, Vec<(usize, usize)>)> = Vec::new();
    for &(i, o, host) in refs.iter().map(|&(i, o)| (i, o, false)).chain(homes.iter().copied()).collect::<Vec<_>>().iter() {
        let a = word(&regions[i], o);
        // A word of the program's a launch never reads (a descriptor's address of a buffer it has none of: 0).
        if a < 1 << 32 {
            continue;
        }
        // A home's host word points at its host address (its region's `base`), every other word at the device's.
        let found = if host { regions.iter().position(|t| t.base <= a && a < t.base + t.len) } else { at_dev(&regions, a) };
        match found {
            Some(t) => {
                let (t64, off) = if host { (t as u64 | HOST, (a - regions[t].base) as u64) } else { (t as u64, (a - regions[t].dev) as u64) };
                for v in [i as u64, o as u64, t64, off] {
                    // copy: not block data (a capture's relocation list, for the lab alone)
                    relocs.extend_from_slice(&v.to_le_bytes());
                }
            }
            None => {
                unresolved += 1;
                if unresolved_in.last().is_none_or(|u| u.0 != i) {
                    unresolved_in.push((i, 0, Vec::new()));
                }
                let u = unresolved_in.last_mut().expect("its entry");
                u.1 += 1;
                if u.2.len() < 4 {
                    u.2.push((o, a));
                }
            }
        }
    }
    // Words inside a region of its own or a lease (at most 64 MB: not a reservation, whose range a pair of 32-bit
    // values may fall in) that are not bound again: an address its encoding wrote without naming it (`Job::names`),
    // which a replay would read or write in the engine's memory. A capture holds none.
    let bound: std::collections::HashSet<(usize, usize)> = refs.iter().copied().collect();
    let mut unbound = 0u64;
    for (i, r) in regions.iter().enumerate().filter(|(_, r)| r.sparse() && r.role != "text" && r.role != "counts") {
        for (at, n) in r.runs() {
            for o in (at.next_multiple_of(8)..(at + n) / 8 * 8).step_by(8) {
                let a = word(r, o);
                if a >= 1 << 32 && !bound.contains(&(i, o)) && regions.iter().any(|t| t.len <= 64 << 20 && t.sparse() && t.dev <= a && a < t.dev + t.len) {
                    unbound += 1;
                }
            }
        }
    }
    std::fs::write(dir.join("relocs.zst"), zstd::bulk::compress(&relocs, 3)?)?;
    let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join("wave.txt"))?);
    writeln!(w, "capture {id}")?;
    writeln!(w, "format {}", reach.format)?;
    writeln!(w, "args {}", reach.args.map(|a| a.to_string()).join(" "))?;
    writeln!(w, "offsets {}", reach.offsets.iter().map(u64::to_string).collect::<Vec<_>>().join(" "))?;
    writeln!(w, "binds {}", reach.binds.map(|(r, _)| r).join(" "))?;
    writeln!(w, "unresolved {unresolved}")?;
    writeln!(w, "unbound {unbound}")?;
    let cd = rerail_tape_format::codes_dir::bytes();
    for &(t, o, n) in &atoms_dirs {
        writeln!(w, "atoms {} {o} {n}", regions[t].name)?;
    }
    for (t, o) in &codes {
        writeln!(w, "codes {} {o} {:016x}", regions[*t].name, digest(cd))?;
    }
    for (i, n, some) in &unresolved_in {
        let some: Vec<String> = some.iter().map(|(o, a)| format!("{o}:{a:#x}")).collect();
        writeln!(w, "unresolved_in {} {n} {}", regions[*i].name, some.join(" "))?;
    }
    for l in labels {
        writeln!(w, "label {} {} {}", l.shape, l.rule.replace(' ', "_"), l.work)?;
    }
    for r in &regions {
        writeln!(w, "region {} {} {:#x} {:#x} {} {} {}", r.name, r.role, r.base, r.dev, r.len, r.kind, r.source)?;
    }
    w.flush()?;
    chosen().0.lock().unwrap_or_else(|e| e.into_inner()).bytes += written;
    Ok(Pending { dir, root: cfg.dir.clone(), regions })
}

/// Whether `more` bytes fit the capture's budget past what its launches wrote and this one has (`written`): else
/// an error, which abandons the launch's capture (its directory removed), the budget never passed by a before-image.
fn within_budget(cfg: &Config, written: u64, more: u64) -> std::io::Result<()> {
    let done = chosen().0.lock().unwrap_or_else(|e| e.into_inner()).bytes;
    if done + written + more > cfg.budget {
        return Err(std::io::Error::other(format!("past the capture's budget of {} MB: {} MB written, {} MB more", cfg.budget >> 20, (done + written) >> 20, more >> 20)));
    }
    Ok(())
}

/// A block's used bytes as a block file (`blocks/<digest>.blk`), its header frozen at its length
/// (`rerail_blocks::block::freeze_copy`), once a directory: its source (`blocks/<digest>.blk`) and the bytes written
/// (0 where it was there). The bytes are read once, the digest the copy's: a block another thread writes meanwhile
/// is named by what was written.
fn write_block(cfg: &Config, bytes: &[u8]) -> std::io::Result<(String, u64)> {
    // copy: not block data (a capture's frozen image of a block, for the lab alone)
    let mut copy = bytes.to_vec();
    let d = digest(&copy);
    let source = format!("blocks/{d:016x}.blk");
    if !chosen().0.lock().unwrap_or_else(|e| e.into_inner()).blocks.insert(d) {
        return Ok((source, 0));
    }
    let path = cfg.dir.join(&source);
    if path.exists() {
        return Ok((source, 0));
    }
    rerail_blocks::block::freeze_copy(&mut copy);
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, &copy)?;
    std::fs::rename(&tmp, &path)?;
    Ok((source, copy.len() as u64))
}

impl Pending {
    /// The launch done: each region it may have written, as it left it.
    pub(in crate::code_rule_dsl) fn after(self) {
        if let Err(e) = self.write_after() {
            crate::code_rule_dsl::logic::profile::decision("a join capture's outputs not written", || format!("{e}"));
        }
    }

    fn write_after(&self) -> std::io::Result<()> {
        let mut w = std::fs::OpenOptions::new().append(true).open(self.dir.join("wave.txt"))?;
        // Its after-images, written whatever the budget (its before-images are), counted in it: a launch after is
        // refused past it.
        let cfg = Config { dir: self.root.clone(), only: None, min: 0, budget: u64::MAX };
        let mut written = 0u64;
        for r in self.regions.iter().filter(|r| r.written) {
            let source = if r.sparse() {
                let s = format!("{}.after.zst", r.name);
                written += write_runs(&self.dir.join(&s), r, &r.runs())?;
                s
            } else if r.source.starts_with("file:") {
                // A mapped file's block, written in place: its bytes after are the file's.
                continue;
            } else {
                // SAFETY: the block's used bytes, its launch done.
                let bytes = unsafe { std::slice::from_raw_parts(r.base as *const u8, r.len) };
                let (source, n) = write_block(&cfg, bytes)?;
                written += n;
                source
            };
            writeln!(w, "after {} {}", r.name, source)?;
        }
        writeln!(w, "done")?;
        chosen().0.lock().unwrap_or_else(|e| e.into_inner()).bytes += written;
        Ok(())
    }
}
