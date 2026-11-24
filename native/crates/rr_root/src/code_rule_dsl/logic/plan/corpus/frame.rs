//! A run's frame kept across processes (t33 K4): the leaves the run read, each with its digest (`Report::consulted`),
//! its plans, its program's summary (what `changes_of` reads of it) and the output image it wrote, under its
//! identity. A later process whose run of the same identity finds every leaf as it was (`leaves::still`) takes the
//! image as the run's and evaluates nothing: the empty leaf delta stops before the program is prepared.
//!
//! One mechanism for every run kept this way (decision 0659): the whole run's (`reuse`), phase 1's
//! (`phases::phase_one`), the constants' (`constants`) and a command's (`reuse::command`). Kept facts
//! of the store (`plan::kept_entry`), a partition a frame, held by a small source and the frames' plan, which the
//! run's manifest names (`named`), so collection keeps a frame while it keeps the run.
//!
//! The image is a derived fact, written once where the run computed it (decision 0353: not a copy): phase 1's
//! exports are read off it as its run read them (`phases::read_exports`), and the whole run writes it as its output.

use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::plan::kept_entry::Entries;
use crate::code_rule_dsl::logic::program_facts::Facts;
use crate::content_store::{ContentStore, HashKey};
use sha2::{Digest as _, Sha256};

const ENTRIES: Entries = Entries::new("rerail.frame");
/// `(kind, path, digest)`: a leaf the run read.
const READ: &str = "frame_read";
/// The run's plans, in hex: a run that takes the frame names them as its own.
const PLAN: &str = "frame_plan";
/// The program's parts `changes_of` reads (`Summary`), as their bytes.
const SUMMARY: &str = "frame_summary";
/// The output image's length in bytes, one row.
const SIZE: &str = "frame_size";
/// The output image, a word a row (eight bytes, little-endian), in order.
const IMAGE: &str = "frame_image";
/// The frame this one leads to, in hex: a command's frame names its run's (`reuse::command`).
const THEN: &str = "frame_then";

fn source_text(key: &[u8; 32]) -> String {
    format!("rerail/frame\0{}", crate::content_store::observe::hex(key))
}

/// The frames' plan: the build and the schema.
fn plan() -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"rerail.frame/v1\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.finalize().into()
}

/// The small source and the plan the frame `key` is held by: the run's manifest names both.
pub fn named(key: &[u8; 32]) -> (String, [u8; 32]) {
    (source_text(key), plan())
}

/// A run's frame: the leaves it read, its plans, its program's summary and its output image; or a command's: the
/// leaves it read before its run (its rule files), and the run's frame (`then`).
#[derive(Clone, Default)]
pub struct Frame {
    pub reads: Vec<(String, String, String)>,
    pub plans: rerail_blocks::vector::Vector<[u8; 32]>,
    pub summary: Vec<u8>,
    pub image: Vec<u8>,
    pub then: Option<[u8; 32]>,
}

impl Frame {
    /// The frame's facts, as kept.
    fn facts(&self) -> Facts {
        let s = |x: &str| Value::Str(x.to_owned().into());
        let mut facts = Facts::new();
        facts.insert(READ.to_owned(), self.reads.iter().map(|(k, p, d)| vec![s(k), s(p), s(d)].into()).collect());
        facts.insert(PLAN.to_owned(), self.plans.iter().map(|p| vec![s(&crate::content_store::observe::hex(p))].into()).collect());
        facts.insert(SUMMARY.to_owned(), vec![vec![s(&String::from_utf8_lossy(&self.summary))].into()]);
        if let Some(then) = &self.then {
            facts.insert(THEN.to_owned(), vec![vec![s(&crate::content_store::observe::hex(then))].into()]);
        }
        facts.insert(SIZE.to_owned(), vec![vec![Value::Int(self.image.len() as i64)].into()]);
        facts.insert(IMAGE.to_owned(), self.image.chunks(8).map(|w| {
            let mut b = [0u8; 8];
            b[..w.len()].copy_from_slice(w);
            vec![Value::Int(i64::from_le_bytes(b))].into()
        }).collect());
        facts
    }
}

/// Keeps `frame` as the frame `key`, behind the run.
pub fn put(store: &ContentStore, key: &[u8; 32], frame: &Frame) {
    let holder = HashKey::sha256(source_text(key).as_bytes()).sha256_bytes().expect("a sha256 key");
    ENTRIES.put(store, Entries::part(key), holder, Some(plan()), &frame.facts());
}

/// The frame `key`, read where the store keeps it; `None` where it keeps none, or one that does not read.
pub fn get(store: &ContentStore, key: &[u8; 32]) -> Option<Frame> {
    let (mut reads, mut plans, mut words) = (Vec::new(), Vec::new(), Vec::new());
    let (mut summary, mut size, mut then) = (None, None, None);
    let mut whole = true;
    let (found, bad) = ENTRIES.visit(store, Some(&[Entries::part(key)]), &|_| true, &mut |_, rel, row| {
        match rel {
            READ => match (row.text(0), row.text(1), row.text(2)) {
                (Some(k), Some(p), Some(d)) => reads.push((row.at, (k.to_string(), p.to_string(), d.to_string()))),
                _ => whole = false,
            },
            PLAN => match row.text(0).and_then(|h| crate::content_store::observe::unhex(&h)) {
                Some(p) => plans.push((row.at, p)),
                None => whole = false,
            },
            SUMMARY => summary = row.text(0).map(|t| t.to_string().into_bytes()),
            SIZE => size = row.int(0),
            THEN => then = row.text(0).and_then(|h| crate::content_store::observe::unhex(&h)),
            IMAGE => match row.int(0) {
                Some(w) => words.push((row.at, w)),
                None => whole = false,
            },
            _ => {}
        }
        true
    });
    if found.is_empty() || !bad.is_empty() || !whole {
        return None;
    }
    let size = usize::try_from(size?).ok()?;
    let mut image: Vec<u8> = ordered(words).into_iter().flat_map(|w: i64| w.to_le_bytes()).collect();
    if image.len() < size || image.len() > size + 7 {
        return None;
    }
    image.truncate(size);
    Some(Frame { reads: ordered(reads), plans: ordered(plans).into_iter().collect(), summary: summary?, image, then })
}

/// Rows read with their places, in their relation's order.
fn ordered<T>(mut rows: Vec<(i64, T)>) -> Vec<T> {
    rows.sort_by_key(|(at, _)| *at);
    rows.into_iter().map(|(_, x)| x).collect()
}
