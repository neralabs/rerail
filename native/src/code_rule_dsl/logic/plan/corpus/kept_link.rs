//! The combine's last link kept, and an edit's programs spliced into it (decision 0373), as 0368 keeps the parts'.
//!
//! The link of a run's programs is kept by the run's shape (`family`): the engine, the number of programs, the rows
//! the run adds and the library's images. Its record holds, for each program, its image's key and its interface:
//! what the merge reads of the program besides its rules' bodies (`interface`). An edit lowers only the programs
//! whose key changed; where each one's interface is its slot's, their rules are written in their slots' places and
//! every other program's are copied from the last link as they are (`Part::spliced`). The header, the requests, the
//! kind sets and the policies are the last link's: a program's part of each is in its interface.

use super::added::Added;
use super::preparation::parts::{hex, keep, kept, ordered, rows_of, str, unhex};
use super::scoping::Policy;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::program_facts::Facts;
use crate::code_rule_dsl::logic::program_image::Version;
use crate::content_store::{ContentStore, HashKey, SourceId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// A program's place in the kept link: its image's key and its interface.
#[derive(Clone)]
pub(super) struct Slot {
    pub key: [u8; 32],
    pub interface: Interface,
}

/// The terms of a program's interface, in the merge's order (`interface`'s arguments).
pub(super) const TERMS: [&str; 12] = ["its place", "its bound", "its guests", "its request asked", "its policy", "its text globs", "its kind sets", "their shared names", "its own names", "its request", "its header", "its core rules"];

/// A program's interface: each term's digest (`TERMS`), so that a slot whose interface changed names what changed.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Interface(Vec<[u8; 32]>);

impl Interface {
    /// The terms that differ from `was`'s.
    pub(super) fn changed(&self, was: &Interface) -> Vec<&'static str> {
        if self.0.len() != was.0.len() {
            return vec!["its terms"];
        }
        self.0.iter().zip(&was.0).enumerate().filter(|(_, (a, b))| a != b).map(|(t, _)| TERMS.get(t).copied().unwrap_or("a term")).collect()
    }

    fn hex(&self) -> String {
        self.0.iter().map(|d| hex(d)).collect::<Vec<_>>().join(":")
    }

    fn unhex(text: &str) -> Option<Interface> {
        text.split(':').map(unhex).collect::<Option<Vec<_>>>().map(Interface)
    }
}

/// What a kept link holds besides its image.
pub(super) struct Record {
    /// How many of the linked rules each part's are, in the link's order: the header's, the library's (held once),
    /// then each program's.
    pub counts: Vec<usize>,
    /// Each program's slot, in order.
    pub slots: Vec<Slot>,
    /// Each request's key (decision 0089 §3): its number, and the program that asked it first (the one its copy is).
    pub requests: BTreeMap<[u8; 32], (usize, usize)>,
    /// Each kind set by its kinds: its shared name.
    pub sets: BTreeMap<Vec<String>, String>,
    /// The library part's name for each of its kind sets: its shared name. A program reading a set the library holds
    /// names it so (the merge's `set_of` starts from these), in a spliced program as in a linked one.
    pub library_sets: BTreeMap<String, String>,
    /// The bound every program is read under (`hiding::bound`).
    pub bound: u32,
    /// The tail's: the bound the combined program is hidden to, and the policies.
    pub hide: Option<u32>,
    pub policies: Vec<Policy>,
}

/// The key a run's link is kept under: what an edit of a program's rules leaves as it is.
pub(super) fn family(programs: usize, extra: &Added, library: &[HashKey]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/combine-link/v3\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update((programs as u64).to_le_bytes());
    extra.hash(&mut h);
    for k in library {
        h.update(format!("{k:?}\0").as_bytes());
    }
    // The switches the combine reads.
    for var in ["RERAIL_REQUESTS", "RERAIL_LIBRARIES"] {
        h.update(format!("{var}={:?}\0", std::env::var(var).ok()).as_bytes());
    }
    h.finalize().into()
}

/// A program's interface: everything the merge reads of it besides its rules' bodies, in the merge's order (`TERMS`),
/// each term's digest. Two programs of one interface in one slot merge alike: the same header, requests, kind sets,
/// policy and core rules.
pub(super) fn interface(parts: &[&dyn std::fmt::Debug; 12]) -> Interface {
    use sha2::Digest as _;
    Interface(parts.iter().map(|p| sha2::Sha256::digest(format!("{p:?}").as_bytes()).into()).collect())
}

fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join("\u{1f}")
}

fn split(s: &str) -> BTreeSet<String> {
    if s.is_empty() { BTreeSet::new() } else { s.split('\u{1f}').map(str::to_owned).collect() }
}

/// The record's facts, its image's digest beside them.
fn facts_of(r: &Record, digest: &[u8; 32]) -> Facts {
    let mut f = Facts::new();
    let mut put = |rel: &str, rows: Vec<Vec<String>>| {
        f.insert(rel.to_owned(), rows.into_iter().map(|r| r.iter().map(|c| str(c)).collect::<Vec<_>>().into()).collect());
    };
    put("part_program", vec![vec![hex(digest)]]);
    put("combine_count", r.counts.iter().enumerate().map(|(i, n)| vec![i.to_string(), n.to_string()]).collect());
    put("combine_slot", r.slots.iter().enumerate().map(|(i, s)| vec![i.to_string(), hex(&s.key), s.interface.hex()]).collect());
    put("combine_request", r.requests.iter().map(|(key, (k, first))| vec![k.to_string(), hex(key), first.to_string()]).collect());
    put("combine_set", r.sets.iter().map(|(kinds, name)| vec![name.clone(), kinds.join("\u{1f}")]).collect());
    put("combine_library_set", r.library_sets.iter().map(|(name, shared)| vec![name.clone(), shared.clone()]).collect());
    put("combine_bound", vec![vec![r.bound.to_string(), r.hide.map_or("-".to_owned(), |b| b.to_string())]]);
    put(
        "combine_policy",
        r.policies
            .iter()
            .enumerate()
            .map(|(i, p)| vec![i.to_string(), p.prefix.clone(), join(&p.own), join(&p.named), join(&p.library), serde_json::to_string(&p.scope).unwrap_or_default()])
            .collect(),
    );
    f
}

fn record_of(facts: &Facts) -> Option<Record> {
    let counts = ordered(facts, "combine_count").into_iter().map(|r| r.get(1)?.parse().ok()).collect::<Option<Vec<usize>>>()?;
    let slots = ordered(facts, "combine_slot").into_iter().map(|r| Some(Slot { key: unhex(r.get(1)?)?, interface: Interface::unhex(r.get(2)?)? })).collect::<Option<Vec<_>>>()?;
    let requests = rows_of(facts, "combine_request").map(|r| Some((unhex(r.get(1)?)?, (r.first()?.parse().ok()?, r.get(2)?.parse().ok()?)))).collect::<Option<BTreeMap<_, _>>>()?;
    let sets = rows_of(facts, "combine_set").map(|r| Some((r.get(1).map(|k| if k.is_empty() { Vec::new() } else { k.split('\u{1f}').map(str::to_owned).collect() })?, r.first()?.clone()))).collect::<Option<BTreeMap<_, _>>>()?;
    let library_sets = rows_of(facts, "combine_library_set").map(|r| Some((r.first()?.clone(), r.get(1)?.clone()))).collect::<Option<BTreeMap<_, _>>>()?;
    let b = rows_of(facts, "combine_bound").next()?;
    let bound = b.first()?.parse().ok()?;
    let hide = match b.get(1)?.as_str() {
        "-" => None,
        n => Some(n.parse().ok()?),
    };
    let policies = ordered(facts, "combine_policy")
        .into_iter()
        .map(|r| Some(Policy { prefix: r.get(1)?.clone(), own: split(r.get(2)?), named: split(r.get(3)?), library: split(r.get(4)?), scope: serde_json::from_str(r.get(5)?).ok()? }))
        .collect::<Option<Vec<_>>>()?;
    Some(Record { counts, slots, requests, sets, library_sets, bound, hide, policies })
}

/// The link kept under `family`, its record, and the sources it was read from.
pub(super) fn lookup(store: &ContentStore, family: &[u8; 32]) -> Option<(Arc<Version>, Record, Vec<SourceId>)> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "the combine's last link, from the store (decision 0373)".into());
    let (version, facts, sources) = kept(store, family).ok().flatten()?;
    Some((version, record_of(&facts)?, sources))
}

/// Keeps `linked` and its record under `family`, written by the store's writer.
pub(super) fn record(store: &ContentStore, family: &[u8; 32], linked: &Arc<Version>, r: &Record) {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the combine's link (decision 0373)".into());
    keep(store, family, linked, |digest| facts_of(r, digest));
}
