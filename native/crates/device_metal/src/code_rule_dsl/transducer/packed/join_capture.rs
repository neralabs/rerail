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
//!   `join_run` binds, and each region it reaches (`region <name> <role> <base> <len> <kind> <source>`), its labels.
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
    let wins: Vec<&Label> = labels.iter().filter(|l| l.work >= cfg.min && st.best.get(&l.shape).is_none_or(|b| l.work > b.0)).collect();
    if wins.is_empty() {
        return None;
    }
    let id = st.next;
    st.next += 1;
    let mut gone = Vec::new();
    for l in wins {
        if let Some((_, old)) = st.best.insert(l.shape.clone(), (l.work, id)) {
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

/// A buffer of the program's own as a region (`<name>.zst`), once.
fn own(regions: &mut Vec<Region>, name: String, role: &'static str, b: &Buffer) {
    let (base, len) = (b.contents() as usize, b.length() as usize);
    if len == 0 || regions.iter().any(|r| r.base == base && r.len == len) {
        return;
    }
    regions.push(Region { source: format!("{name}.zst"), name, role, base, len, kind: 0, written: true });
}

/// The word at byte `o` of region `r`.
fn word(r: &Region, o: usize) -> usize {
    u64::from_ne_bytes(r.bytes(o, 8).try_into().expect("8 bytes")) as usize
}

fn write_before(cfg: &Config, id: u64, labels: &[Label], reach: &Reached<'_>) -> std::io::Result<Pending> {
    let dir = wave_dir(cfg, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(cfg.dir.join("blocks"))?;
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
    // The blocks and leases they and its jobs reach, as regions too.
    let mut targets: Vec<(usize, bool)> = reach.uses.iter().map(|(b, w)| (b.contents() as usize, *w)).collect();
    targets.extend(refs.iter().map(|&(i, o)| (word(&regions[i], o), false)).filter(|&(a, _)| a >= 1 << 32));
    targets.extend(reach.reached.iter().map(|&a| (a as usize, false)));
    let mut written = 0u64;
    for (addr, w) in targets {
        if let Some(r) = regions.iter_mut().find(|r| r.base <= addr && addr < r.base + r.len) {
            r.written |= w;
            continue;
        }
        let Some(p) = rerail_blocks::ledger::placed_at(addr) else { continue };
        let name = format!("r{}", regions.len());
        let region = if p.kind == 0 {
            Region { source: format!("{name}.zst"), name, role: "lease", base: p.start, len: p.end - p.start, kind: 0, written: w }
        } else {
            let len = if p.top > 0 { p.top.min(p.end - p.start) } else { p.end - p.start };
            let source = match &p.path {
                Some(path) => format!("file:{}", path.display()),
                None => {
                    if len as u64 > cfg.budget {
                        return Err(std::io::Error::other(format!("a block of {} MB, past the capture's budget", len >> 20)));
                    }
                    // SAFETY: the block's used bytes, before the launch.
                    let bytes = unsafe { std::slice::from_raw_parts(p.start as *const u8, len) };
                    written += write_block(cfg, bytes)?;
                    format!("blocks/{:016x}.blk", digest(bytes))
                }
            };
            Region { name, role: "block", base: p.start, len, kind: p.kind, source, written: w }
        };
        regions.push(region);
    }
    // The sparse regions' runs, and each address word as (region, offset, region it points into, offset there).
    for r in regions.iter().filter(|r| r.sparse()) {
        written += write_runs(&dir.join(&r.source), r, &r.runs())?;
    }
    let mut relocs: Vec<u8> = Vec::new();
    // Address words no region holds: memory no ledger knew (a replay that reads one fails there).
    let mut unresolved = 0u64;
    let mut unresolved_in: Vec<(usize, u64, Vec<(usize, usize)>)> = Vec::new();
    for &(i, o) in &refs {
        let a = word(&regions[i], o);
        // A word of the program's a launch never reads (a descriptor's address of a buffer it has none of: 0).
        if a < 1 << 32 {
            continue;
        }
        match regions.iter().position(|t| t.base <= a && a < t.base + t.len) {
            Some(t) => {
                for v in [i as u64, o as u64, t as u64, (a - regions[t].base) as u64] {
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
                if a >= 1 << 32 && !bound.contains(&(i, o)) && regions.iter().any(|t| t.len <= 64 << 20 && t.sparse() && t.base <= a && a < t.base + t.len) {
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
    for (i, n, some) in &unresolved_in {
        let some: Vec<String> = some.iter().map(|(o, a)| format!("{o}:{a:#x}")).collect();
        writeln!(w, "unresolved_in {} {n} {}", regions[*i].name, some.join(" "))?;
    }
    for l in labels {
        writeln!(w, "label {} {} {}", l.shape, l.rule.replace(' ', "_"), l.work)?;
    }
    for r in &regions {
        writeln!(w, "region {} {} {:#x} {} {} {}", r.name, r.role, r.base, r.len, r.kind, r.source)?;
    }
    w.flush()?;
    chosen().0.lock().unwrap_or_else(|e| e.into_inner()).bytes += written;
    Ok(Pending { dir, root: cfg.dir.clone(), regions })
}

/// A block's used bytes as a block file (`blocks/<digest>.blk`), its header frozen at its length
/// (`rerail_blocks::block::freeze_copy`), once a directory: the bytes written (0 where it was there).
fn write_block(cfg: &Config, bytes: &[u8]) -> std::io::Result<u64> {
    let d = digest(bytes);
    if !chosen().0.lock().unwrap_or_else(|e| e.into_inner()).blocks.insert(d) {
        return Ok(0);
    }
    let path = cfg.dir.join("blocks").join(format!("{d:016x}.blk"));
    if path.exists() {
        return Ok(0);
    }
    let mut copy = bytes.to_vec();
    rerail_blocks::block::freeze_copy(&mut copy);
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, &copy)?;
    std::fs::rename(&tmp, &path)?;
    Ok(copy.len() as u64)
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
        let cfg = Config { dir: self.root.clone(), only: None, min: 0, budget: u64::MAX };
        for r in self.regions.iter().filter(|r| r.written) {
            let source = if r.sparse() {
                let s = format!("{}.after.zst", r.name);
                write_runs(&self.dir.join(&s), r, &r.runs())?;
                s
            } else if r.source.starts_with("file:") {
                // A mapped file's block, written in place: its bytes after are the file's.
                continue;
            } else {
                // SAFETY: the block's used bytes, its launch done.
                let bytes = unsafe { std::slice::from_raw_parts(r.base as *const u8, r.len) };
                write_block(&cfg, bytes)?;
                format!("blocks/{:016x}.blk", digest(bytes))
            };
            writeln!(w, "after {} {}", r.name, source)?;
        }
        writeln!(w, "done")?;
        Ok(())
    }
}
