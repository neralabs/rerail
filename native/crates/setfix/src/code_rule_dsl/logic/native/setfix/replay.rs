//! The join lab's scalar replay (its design's §5, the `scalar` engine): a stratum known by what it is in its run, and
//! what it made, so a launch the device ran is matched with the stratum the scalar joins run in a run of their own.
//!
//! A stratum's identity is its name, its relations' tables and its batch's documents (their paths): the same in every
//! run of one command over one corpus, whichever joins run it. Not how many opened before it: the program's joins
//! open a stratum again where the scalar joins do not (planned again, or parsed again with the CPU's indexes where the
//! device refused it). A stratum of one identity done more than once is recorded the first time, in either run. It is made where a stratum opens, only while captures are on (`RERAIL_JOIN_CAPTURE`, `capture_to`)
//! or a replay is (`RERAIL_JOIN_REPLAY`), and held in its store (`Store::stratum`), so the device names it on each run
//! it encodes (`join_capture::Label`).
//!
//! What a stratum made is each of its relations' live rows when it is done: their count, and an order-free digest of
//! them (each row's hash summed, a symbol's by its text): the program and the scalar joins place rows and intern
//! symbols in orders of their own.
//!
//! A capturing run writes a line for each stratum one of its captured launches ran (`captured`) to `<dir>/strata.txt`;
//! a replay (`RERAIL_JOIN_REPLAY=<dir>`: that capture) writes one for each stratum it names to
//! `RERAIL_JOIN_REPLAY_OUT`, with the thread's CPU seconds its runs took.
//! `stratum <identity> <name> <cpu seconds> <relations> [<table> <rows> <digest>]...`

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;

/// The machine's identities and records (decision 0442: its own, not a process's).
#[derive(Default)]
struct State(parking_lot::Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// Whether strata are identified at all: read once, from the environment or a caller (`capture_to`).
    on: Option<bool>,
    /// The strata captured launches ran (`captured`), each with its captures' directory, each written once.
    captured: HashMap<u64, PathBuf>,
    written: HashSet<u64>,
    /// A replay's: the strata its capture names, and where their records go.
    wanted: Option<(HashSet<u64>, PathBuf)>,
    /// The replay's strata recorded so far: each the first time it is done.
    replayed: HashSet<u64>,
    /// The thread CPU seconds each identified stratum's runs took so far (`Charge`).
    cpu: HashMap<u64, f64>,
}

fn state() -> std::sync::Arc<State> {
    crate::code_rule_dsl::logic::profile::machine::<State>()
}

fn hash_one(bytes: &[u8]) -> u64 {
    use std::hash::BuildHasher as _;
    foldhash::fast::FixedState::with_seed(0x6a6f_696e_6c61_62).hash_one(bytes)
}

/// One stratum's record: its identity, its name, the CPU its runs took, each relation's table, live rows and digest.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub stratum: u64,
    pub name: String,
    pub cpu: f64,
    pub rels: Vec<(String, u64, u64)>,
}

impl Record {
    pub fn line(&self) -> String {
        let mut s = format!("stratum {:016x} {} {:.6} {}", self.stratum, self.name, self.cpu, self.rels.len());
        for (t, n, d) in &self.rels {
            s.push_str(&format!(" {t} {n} {d:016x}"));
        }
        s
    }

    pub fn parse(line: &str) -> Option<Record> {
        let mut w = line.split_ascii_whitespace();
        if w.next()? != "stratum" {
            return None;
        }
        let stratum = u64::from_str_radix(w.next()?, 16).ok()?;
        let name = w.next()?.to_owned();
        let cpu = w.next()?.parse().ok()?;
        let k: usize = w.next()?.parse().ok()?;
        let mut rels = Vec::with_capacity(k);
        for _ in 0..k {
            rels.push((w.next()?.to_owned(), w.next()?.parse().ok()?, u64::from_str_radix(w.next()?, 16).ok()?));
        }
        Some(Record { stratum, name, cpu, rels })
    }

    /// The first relation whose rows differ from `other`'s, as a line naming it: `None` where every one is the same.
    pub fn differs(&self, other: &Record) -> Option<String> {
        if self.rels.len() != other.rels.len() {
            return Some(format!("{} relations, {} in the other", self.rels.len(), other.rels.len()));
        }
        self.rels.iter().zip(&other.rels).find(|(a, b)| a != b).map(|((t, n, d), (u, m, e))| {
            if t != u { format!("relation {t}, {u} in the other") } else { format!("{t}: {n} rows (digest {d:016x}), {m} rows (digest {e:016x}) in the other") }
        })
    }
}

/// Every record of `text` (a `strata.txt`, or a replay's), by identity.
pub fn records(text: &str) -> HashMap<u64, Record> {
    text.lines().filter_map(Record::parse).map(|r| (r.stratum, r)).collect()
}

/// Strata identified from now on (`join_capture::capture_to`'s): the records of those its launches ran (`captured`)
/// written under the directory each was captured to.
pub fn capture_to(dir: PathBuf) {
    let s = state();
    let mut s = s.0.lock();
    let _ = dir;
    s.on = Some(true);
}

/// A replay of the capture under `dir` from now on (`RERAIL_JOIN_REPLAY`'s, set by a caller): the strata its
/// `strata.txt` names recorded to `out` when done.
pub fn replay_to(dir: &std::path::Path, out: PathBuf) {
    let wanted = std::fs::read_to_string(dir.join("strata.txt")).map(|t| records(&t).into_keys().collect()).unwrap_or_default();
    let s = state();
    let mut s = s.0.lock();
    s.on = Some(true);
    s.wanted = Some((wanted, out));
    s.replayed.clear();
}

/// Whether strata are identified: captures on, or a replay.
fn on(s: &mut Inner) -> bool {
    *s.on.get_or_insert_with(|| {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        if set("RERAIL_JOIN_REPLAY") {
            let dir = PathBuf::from(std::env::var_os("RERAIL_JOIN_REPLAY").unwrap_or_default());
            let out = std::env::var_os("RERAIL_JOIN_REPLAY_OUT").map_or_else(|| dir.join("replay.txt"), PathBuf::from);
            let wanted = std::fs::read_to_string(dir.join("strata.txt")).map(|t| records(&t).into_keys().collect()).unwrap_or_default();
            s.wanted = Some((wanted, out));
        }
        set("RERAIL_JOIN_CAPTURE") || set("RERAIL_JOIN_REPLAY")
    })
}

/// The identity of a stratum opening, `name` over relations `tables`, its batch's documents `paths`: `0` while
/// strata are not identified.
pub fn opened<'a>(name: &str, tables: impl Iterator<Item = &'a str>, paths: impl FnOnce() -> Vec<String>) -> u64 {
    let s = state();
    let mut s = s.0.lock();
    if !on(&mut s) {
        return 0;
    }
    let mut bytes: Vec<u8> = name.as_bytes().to_vec();
    for t in tables {
        bytes.push(0);
        // copy: not block data (the bytes an identity or a row's digest hashes)
        bytes.extend_from_slice(t.as_bytes());
    }
    bytes.push(1);
    for p in paths() {
        bytes.push(0);
        // copy: not block data (the bytes an identity or a row's digest hashes)
        bytes.extend_from_slice(p.as_bytes());
    }
    // `0` is no identity.
    let id = hash_one(&bytes).max(1);
    // `RERAIL_JOIN_REPLAY_TRACE=<file>`: each identity made, its name and how many relations and documents it is of.
    if let Some(path) = std::env::var_os("RERAIL_JOIN_REPLAY_TRACE").filter(|p| !p.is_empty())
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let tables = bytes.split(|&b| b == 1).next().map_or(0, |t| t.iter().filter(|&&b| b == 0).count());
        let docs = bytes.rsplit(|&b| b == 1).next().map_or(0, |d| d.iter().filter(|&&b| b == 0).count());
        let _ = writeln!(f, "opened {id:016x} {name} {tables} {docs}");
    }
    id
}

/// The strata a captured launch ran (its runs' `Label::stratum`), under captures' directory `dir`: each one's record
/// is written when it is done.
pub fn captured(dir: &std::path::Path, strata: impl Iterator<Item = u64>) {
    let s = state();
    let mut s = s.0.lock();
    for x in strata.filter(|&x| x != 0) {
        s.captured.insert(x, dir.to_owned());
    }
}

/// Whether a stratum done is one whose record is wanted: a captured launch's, or one a replay names.
pub fn wanted(stratum: u64) -> bool {
    if stratum == 0 {
        return false;
    }
    let s = state();
    let s = s.0.lock();
    (s.captured.contains_key(&stratum) && !s.written.contains(&stratum)) || (s.wanted.as_ref().is_some_and(|(w, _)| w.contains(&stratum)) && !s.replayed.contains(&stratum))
}

/// The record of a stratum done, written where it is wanted (`wanted`, which the caller asked first).
pub fn done(record: &Record) -> std::io::Result<()> {
    let s = state();
    let mut g = s.0.lock();
    let s = &mut *g;
    let mut to = Vec::new();
    if let Some(dir) = s.captured.get(&record.stratum) && s.written.insert(record.stratum) {
        to.push(dir.join("strata.txt"));
    }
    if let Some((w, out)) = &s.wanted && w.contains(&record.stratum) && s.replayed.insert(record.stratum) {
        to.push(out.clone());
    }
    drop(g);
    for path in to {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{}", record.line())?;
    }
    Ok(())
}

/// A run of a stratum, charged its thread's CPU seconds when it returns (`Record::cpu`): `None` for a stratum not
/// identified.
pub struct Charge {
    stratum: u64,
    from: f64,
    /// A replay's stratum, its run watched (`watch`).
    watched: bool,
}

thread_local! {
    /// The join lab's watch of a replay's strata on this thread (`watch`): told as each run of one starts (`true`) and
    /// ends, so the lab's sampler and counters take those runs alone.
    static WATCH: std::cell::RefCell<Option<Box<dyn FnMut(u64, bool)>>> = const { std::cell::RefCell::new(None) };
}

/// `f` told on this thread as each run of a stratum a replay names starts and ends (`None`: no watch).
pub fn watch(f: Option<Box<dyn FnMut(u64, bool)>>) {
    WATCH.with(|w| *w.borrow_mut() = f);
}

fn tell(stratum: u64, start: bool) {
    WATCH.with(|w| {
        if let Some(f) = w.borrow_mut().as_mut() {
            f(stratum, start);
        }
    });
}

impl Charge {
    pub fn new(stratum: u64) -> Option<Charge> {
        if stratum == 0 {
            return None;
        }
        let watched = state().0.lock().wanted.as_ref().is_some_and(|(w, _)| w.contains(&stratum));
        if watched {
            tell(stratum, true);
        }
        Some(Charge { stratum, from: super::thread_cpu(), watched })
    }

    /// The stratum's CPU seconds: its runs' before this one, and this one's so far.
    pub fn so_far(&self) -> f64 {
        state().0.lock().cpu.get(&self.stratum).copied().unwrap_or(0.0) + (super::thread_cpu() - self.from)
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let spent = super::thread_cpu() - self.from;
        if self.watched {
            tell(self.stratum, false);
        }
        *state().0.lock().cpu.entry(self.stratum).or_default() += spent;
    }
}

/// An order-free digest of rows: each row's hash (its values', a symbol's by `text`), summed.
pub fn digest(rows: impl Iterator<Item = impl AsRef<[i64]>>, types: &[u8], text: &mut dyn FnMut(i64) -> String) -> (u64, u64) {
    let (mut n, mut sum) = (0u64, 0u64);
    let mut bytes = Vec::new();
    for row in rows {
        bytes.clear();
        for (&v, &t) in row.as_ref().iter().zip(types) {
            if t == b's' {
                bytes.push(b's');
                // copy: not block data (the bytes an identity or a row's digest hashes)
                bytes.extend_from_slice(text(v).as_bytes());
                bytes.push(0);
            } else {
                bytes.push(b'i');
                // copy: not block data (the bytes an identity or a row's digest hashes)
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        sum = sum.wrapping_add(hash_one(&bytes));
        n += 1;
    }
    (n, sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_reads_back_as_written() {
        let r = Record { stratum: 0xabc, name: "s3".into(), cpu: 0.25, rels: vec![("rr_a".into(), 3, 0x1234), ("rr_b".into(), 0, 0)] };
        assert_eq!(Record::parse(&r.line()), Some(r.clone()));
        assert_eq!(r.differs(&r), None);
        let mut other = r.clone();
        other.rels[1].1 = 1;
        assert!(r.differs(&other).expect("a difference").starts_with("rr_b: 0 rows"));
    }

    #[test]
    fn a_digest_is_of_the_rows_not_their_order_nor_their_symbols_ids() {
        let names = ["x", "y"];
        let a: Vec<[i64; 2]> = vec![[1, 0], [2, 1]];
        let b: Vec<[i64; 2]> = vec![[2, 11], [1, 10]];
        let da = digest(a.iter().map(|r| &r[..]), b"is", &mut |v| names[v as usize].to_owned());
        let db = digest(b.iter().map(|r| &r[..]), b"is", &mut |v| names[(v - 10) as usize].to_owned());
        assert_eq!(da, db);
        let dc = digest(a.iter().map(|r| &r[..]), b"ii", &mut |v| names[v as usize].to_owned());
        assert_ne!(da, dc);
    }
}
