//! A program's versions (decision 0173 §7): a base image, a view of its rules (ranges), or an overlay (a parent,
//! and a delta image: the rules it adds, the parent's it removes, the parts it replaces). A shared image is never
//! changed; a version is made by an edit, and costs the edit. An overlay whose edit reaches half its parent is folded
//! into a new base.
//!
//! Rule ids are global to a stack: a base's are its rules' indexes; an overlay's added rules follow its parent's id
//! space (`rule_base`). Rule hashes are of the rules' contents, so two images' equal rules hash the same.
use super::{Encoder, Image, Parts, R, Reader, assemble, bad, hash, meta};
use crate::code_rule_dsl::logic::front::{List, Lists, Name, Program, Rule, Value};
use crate::code_rule_dsl::logic::program_facts::Facts;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Most ranges a view may have; past it a subset is an overlay.
pub const MAX_RANGES: usize = 32;

pub enum Version {
    Base(Arc<Image>),
    View { base: Arc<Image>, ranges: Vec<(u32, u32)>, size: usize },
    Overlay { parent: Arc<Version>, delta: Arc<Image>, size: usize, depth: usize },
}

/// One version's change from its parent.
#[derive(Default)]
pub struct Edit<'a> {
    /// Global ids of the parent's rules removed.
    pub removed: Vec<u32>,
    pub added: Vec<&'a Rule>,
    pub parts: Parts<'a>,
}

impl Edit<'_> {
    fn changes_parts(&self) -> bool {
        let p = &self.parts;
        !p.decls.is_empty() || !p.decls_gone.is_empty() || !p.facts.is_empty() || !p.facts_gone.is_empty() || p.inputs.is_some() || p.outputs.is_some() || p.about.is_some()
    }
    /// The edit's size: rules and part rows it states.
    pub fn size(&self) -> usize {
        let p = &self.parts;
        self.removed.len() + self.added.len() + p.decls.len() + p.decls_gone.len() + p.facts.iter().map(|f| f.1.len().max(1)).sum::<usize>() + p.facts_gone.len()
    }
}

/// What made a version: which form an edit took.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Made {
    View,
    Overlay,
    Folded,
}

/// A program's parts other than its rules, merged down a stack.
struct Merged {
    decls: BTreeMap<String, (Name, List<(Name, Name)>, u64)>,
    facts: BTreeMap<(bool, String), (Vec<List<Value>>, u64)>,
    inputs: (Vec<String>, u64),
    outputs: (Vec<String>, u64),
}

fn image_hash(im: &Image, r: R, k: usize) -> u64 {
    im.u64s(r)[k]
}

impl Version {
    pub fn open(bytes: &[u8]) -> Result<Arc<Version>, String> {
        let image = Image::open(bytes)?;
        if image.is_delta() {
            return Err(bad("a delta has no parent here"));
        }
        Ok(Arc::new(Version::Base(Arc::new(image))))
    }

    /// The number of rules.
    pub fn size(&self) -> usize {
        match self {
            Version::Base(im) => im.rules(),
            Version::View { size, .. } | Version::Overlay { size, .. } => *size,
        }
    }
    /// The index in `stack` of this version's top image: a view reads its base's.
    pub fn depth(&self) -> usize {
        match self {
            Version::Base(_) | Version::View { .. } => 0,
            Version::Overlay { depth, .. } => *depth,
        }
    }
    /// The rule ids this version's rules are numbered within.
    pub fn id_space(&self) -> u32 {
        match self {
            Version::Base(im) | Version::View { base: im, .. } => im.rules() as u32,
            Version::Overlay { delta, .. } => delta.rule_base() + delta.rules() as u32,
        }
    }
    /// The images of this version, the root first.
    pub fn stack(&self) -> Vec<Arc<Image>> {
        match self {
            Version::Base(im) | Version::View { base: im, .. } => vec![im.clone()],
            Version::Overlay { parent, delta, .. } => {
                let mut s = parent.stack();
                s.push(delta.clone());
                s
            }
        }
    }
    /// Each rule in order, as (its image in `stack`, its index there, its global id).
    pub fn each_rule(&self, f: &mut dyn FnMut(usize, u32, u32)) {
        match self {
            Version::Base(im) => (0..im.rules() as u32).for_each(|k| f(0, k, k)),
            Version::View { ranges, .. } => ranges.iter().for_each(|&(s, e)| (s..e).for_each(|k| f(0, k, k))),
            Version::Overlay { parent, delta, depth, .. } => {
                let removed = delta.u32s(R::Removed);
                parent.each_rule(&mut |i, k, g| {
                    if removed.binary_search(&g).is_err() {
                        f(i, k, g)
                    }
                });
                let at = *depth;
                (0..delta.rules() as u32).for_each(|k| f(at, k, delta.rule_base() + k));
            }
        }
    }
    /// The rules whose head is `rel`, as `each_rule` gives them, by each image's index.
    pub fn by_head(&self, rel: &str, f: &mut dyn FnMut(usize, u32, u32)) {
        match self {
            Version::Base(im) => {
                if let Some(r) = im.rel(rel) {
                    im.by_head(r).iter().for_each(|&k| f(0, k, k));
                }
            }
            Version::View { base, ranges, .. } => {
                if let Some(r) = base.rel(rel) {
                    for &k in base.by_head(r) {
                        let at = ranges.partition_point(|&(s, _)| s <= k);
                        if at > 0 && k < ranges[at - 1].1 {
                            f(0, k, k);
                        }
                    }
                }
            }
            Version::Overlay { parent, delta, depth, .. } => {
                let removed = delta.u32s(R::Removed);
                parent.by_head(rel, &mut |i, k, g| {
                    if removed.binary_search(&g).is_err() {
                        f(i, k, g)
                    }
                });
                if let Some(r) = delta.rel(rel) {
                    let at = *depth;
                    delta.by_head(r).iter().for_each(|&k| f(at, k, delta.rule_base() + k));
                }
            }
        }
    }
    /// The hash of the rule of global id `g`.
    pub fn rule_hash(&self, g: u32) -> u64 {
        match self {
            Version::Base(im) | Version::View { base: im, .. } => im.rule_hash(g as usize),
            Version::Overlay { parent, delta, .. } => {
                if g >= delta.rule_base() {
                    delta.rule_hash((g - delta.rule_base()) as usize)
                } else {
                    parent.rule_hash(g)
                }
            }
        }
    }

    fn merged(&self, lists: &Lists) -> Result<Merged, String> {
        match self {
            Version::Base(im) | Version::View { base: im, .. } => {
                let mut m = Merged { decls: BTreeMap::new(), facts: BTreeMap::new(), inputs: (Vec::new(), 0), outputs: (Vec::new(), 0) };
                apply(&mut m, im, lists);
                Ok(m)
            }
            Version::Overlay { parent, delta, .. } => {
                let mut m = parent.merged(lists)?;
                apply(&mut m, delta, lists);
                Ok(m)
            }
        }
    }

    /// The program this version is.
    pub fn program(&self, lists: &Lists) -> Result<Program, String> {
        let stack = self.stack();
        let mut readers: Vec<Reader<'_>> = stack.iter().map(|im| Reader::new(im, lists)).collect();
        let mut at = Vec::with_capacity(self.size());
        self.each_rule(&mut |i, k, _| at.push((i, k)));
        let mut rules = Vec::with_capacity(at.len());
        for (i, k) in at {
            rules.push(readers[i].rule(k as usize)?);
        }
        drop(readers);
        let m = self.merged(lists)?;
        let mut facts = BTreeMap::new();
        let mut about = Facts::new();
        for ((is_about, rel), (rows, _)) in m.facts {
            if is_about {
                about.insert(rel, rows);
            } else {
                facts.insert(rel, rows);
            }
        }
        let decls = m.decls.into_values().map(|(n, c, _)| (n, c)).collect();
        assemble(rules, decls, facts, about, m.inputs.0, m.outputs.0)
    }

    /// The version's digest: its rules' hashes and its parts' hashes, summed. An edit's digest is its parent's, less
    /// what it removes, plus what it adds (`edit` keeps it so; here it is read whole).
    pub fn digest(&self, lists: &Lists) -> Result<u64, String> {
        let mut d = 0u64;
        self.each_rule(&mut |_, _, g| d = d.wrapping_add(self.rule_hash(g)));
        let m = self.merged(lists)?;
        for (_, _, h) in m.decls.values() {
            d = d.wrapping_add(*h);
        }
        for (_, h) in m.facts.values() {
            d = d.wrapping_add(*h);
        }
        Ok(d.wrapping_add(m.inputs.1).wrapping_add(m.outputs.1))
    }

    /// The version `edit` makes of `parent`: a view when it only drops rules of a base, in few ranges; else an
    /// overlay; folded into a new base when the edit reaches half the parent.
    pub fn edit(parent: &Arc<Version>, edit: Edit<'_>, lists: &Lists) -> Result<(Arc<Version>, Made), String> {
        let mut removed = edit.removed.clone();
        removed.sort_unstable();
        removed.dedup();
        let size = parent.size() - removed.len() + edit.added.len();
        if edit.added.is_empty() && !edit.changes_parts()
            && let Version::Base(base) = &**parent
        {
            let mut ranges = Vec::new();
            let mut start = 0u32;
            for &r in &removed {
                if r > start {
                    ranges.push((start, r));
                }
                start = r + 1;
            }
            if (start as usize) < base.rules() {
                ranges.push((start, base.rules() as u32));
            }
            if ranges.len() <= MAX_RANGES {
                return Ok((Arc::new(Version::View { base: base.clone(), ranges, size }), Made::View));
            }
        }
        let mut e = Encoder::default();
        for r in &edit.added {
            e.rule(r)?;
        }
        let bytes = e.finish(&edit.parts, Some((parent.id_space(), &removed)));
        let delta = Arc::new(Image::open(&bytes)?);
        let overlay = Arc::new(Version::Overlay { parent: parent.clone(), delta, size, depth: parent.depth() + 1 });
        if 2 * edit.size() >= parent.size().max(1) {
            let folded = super::encode(&overlay.program(lists)?)?;
            return Ok((Version::open(&folded)?, Made::Folded));
        }
        Ok((overlay, Made::Overlay))
    }

    /// `to` as an edit of this version: its rules matched by hash, its parts by relation. For a caller that holds
    /// both programs (a test, the trace's replay); a pass states its edit instead.
    pub fn diff<'p>(&self, to: &'p Program, about: &'p Facts, lists: &Lists) -> Result<Edit<'p>, String> {
        let mut want: HashMap<u64, Vec<usize>> = HashMap::new();
        for (k, r) in to.rules.iter().enumerate() {
            want.entry(hash(r)).or_default().push(k);
        }
        let mut kept = vec![false; to.rules.len()];
        let mut removed = Vec::new();
        self.each_rule(&mut |_, _, g| match want.get_mut(&self.rule_hash(g)).and_then(Vec::pop) {
            Some(k) => kept[k] = true,
            None => removed.push(g),
        });
        let added = to.rules.iter().zip(&kept).filter(|(_, k)| !**k).map(|(r, _)| r).collect();
        let m = self.merged(lists)?;
        let mut parts = Parts::default();
        for (n, c) in &to.decls {
            if m.decls.get(n.as_str()).is_none_or(|(_, _, h)| *h != hash(&(n.as_str(), c.as_slice()))) {
                parts.decls.push((n, c.as_slice()));
            }
        }
        for n in m.decls.keys() {
            if !to.decls.contains_key(n.as_str()) {
                parts.decls_gone.push(n.clone());
            }
        }
        let mut facts: Vec<(bool, &str, &[List<Value>])> = to.facts.iter().map(|(r, rows)| (false, r.as_str(), rows.as_slice())).collect();
        facts.extend(about.iter().map(|(r, rows)| (true, r.as_str(), rows.as_slice())));
        for &(is_about, rel, rows) in &facts {
            if m.facts.get(&(is_about, rel.to_owned())).is_none_or(|(_, h)| *h != hash(&(rel, rows, is_about))) {
                parts.facts.push((rel, rows, is_about));
            }
        }
        for (is_about, rel) in m.facts.keys() {
            if !facts.iter().any(|(a, r, _)| a == is_about && r == rel) {
                parts.facts_gone.push((rel.clone(), *is_about));
            }
        }
        if m.inputs.1 != hash(&to.inputs) {
            parts.inputs = Some(&to.inputs);
        }
        if m.outputs.1 != hash(&to.outputs) {
            parts.outputs = Some(&to.outputs);
        }
        Ok(Edit { removed, added, parts })
    }
}

/// `im`'s parts over `m`: its declarations and fact relations replace those of their names, its gone ones go, and its
/// inputs and outputs replace the parent's where it has its own.
fn apply(m: &mut Merged, im: &Image, lists: &Lists) {
    let mut r = Reader::new(im, lists);
    let (decls, gone) = r.decls();
    for g in gone {
        m.decls.remove(&g);
    }
    for (k, (n, c)) in decls.into_iter().enumerate() {
        m.decls.insert(n.to_string(), (n, c, image_hash(im, R::DeclHash, k)));
    }
    let (facts, gone) = r.facts();
    for (rel, about) in gone {
        m.facts.remove(&(about, rel));
    }
    let owns = im.u64s(R::Meta)[meta::OWNS];
    if owns & 4 != 0 {
        m.facts.retain(|(about, _), _| !about);
    }
    for (k, (rel, about, rows)) in facts.into_iter().enumerate() {
        m.facts.insert((about, rel), (rows, image_hash(im, R::FactHash, k)));
    }
    if owns & 1 != 0 {
        m.inputs = (r.names(R::Inputs), im.u64s(R::Meta)[meta::INPUTS_HASH]);
    }
    if owns & 2 != 0 {
        m.outputs = (r.names(R::Outputs), im.u64s(R::Meta)[meta::OUTPUTS_HASH]);
    }
}
