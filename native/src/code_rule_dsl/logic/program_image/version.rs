//! A program's versions (decision 0173 §7): a base image, a view of a parent's rules (ranges of its rule ids; the
//! parent a base or an overlay, decision 0244), or an overlay (a parent and a delta image: the rules it adds, the
//! parent's it removes, the parts it replaces). A shared image is never
//! changed; a version is made by an edit (`build::Builder`) and costs the edit. An overlay whose edit reaches half its
//! parent is folded into a new base, which keeps the stack's atom and relation ids.
//!
//! Everything is looked up by id, through the stack: texts, relations, rules (by id, by head, by the literals that
//! read a relation), declarations, facts, inputs and outputs. A version's digest is kept as it is made: its parent's,
//! less what the edit removes or replaces, plus what it adds.
use super::handles::{AtomRef, RuleRef};
use super::{AtomId, Cell, Image, RelId, RuleId, bad};
use crate::code_rule_dsl::logic::front::{Lists, Program};
use std::collections::{BTreeMap, HashSet};
use super::manager::Store;
use std::sync::Arc;

/// Fewest ranges a view may have; it may have up to a quarter of its parent's rules (`max_ranges`, decision 0244).
/// Past that a subset is an overlay.
pub const MAX_RANGES: usize = 32;

/// Most ranges a view of a parent of `rules` rules may have: a quarter of them, never fewer than `MAX_RANGES`.
pub fn max_ranges(rules: usize) -> usize {
    (rules / 4).max(MAX_RANGES)
}

/// Each form holds its analyses (`analyses`, decision 0281 §2), made as they are asked for and dropped with it.
pub enum Version {
    Base { image: Arc<Image>, digest: u64, analyses: Store },
    /// Ranges of `parent`'s rule ids, ascending; `parent` is never a view (a view of a view is of its parent).
    View { parent: Arc<Version>, ranges: Vec<(u32, u32)>, size: usize, digest: u64, analyses: Store },
    Overlay { parent: Arc<Version>, delta: Arc<Image>, size: usize, depth: usize, digest: u64, analyses: Store },
}

/// What made a version: which form an edit took.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Made {
    View,
    Overlay,
    Folded,
}

/// A declaration: its relation's name, and its columns' names and types.
#[derive(Clone, Copy)]
pub struct DeclRef<'v> {
    im: &'v Image,
    k: usize,
}

impl<'v> DeclRef<'v> {
    pub fn name(self) -> AtomId {
        self.im.decl_name(self.k)
    }
    pub fn cols(self) -> impl ExactSizeIterator<Item = (AtomId, AtomId)> + 'v {
        self.im.decl_cols(self.k)
    }
    pub fn arity(self) -> usize {
        self.cols().len()
    }
    pub fn hash(self) -> u64 {
        self.im.decl_hash(self.k)
    }
}

/// A fact relation: its name, whether it is one of the `.about` rows, and its rows.
#[derive(Clone, Copy)]
pub struct FactsRef<'v> {
    im: &'v Image,
    k: usize,
}

impl<'v> FactsRef<'v> {
    pub fn name(self) -> AtomId {
        self.im.fact_key(self.k).0
    }
    pub fn about(self) -> bool {
        self.im.fact_key(self.k).1
    }
    pub fn rows(self) -> impl ExactSizeIterator<Item = impl ExactSizeIterator<Item = Cell> + 'v> + 'v {
        self.im.fact_rows(self.k)
    }
    pub fn hash(self) -> u64 {
        self.im.fact_hash(self.k)
    }
}

/// A version's images, its own first, then its parent's.
struct Down<'v>(Option<&'v Version>);

impl<'v> Iterator for Down<'v> {
    type Item = &'v Image;
    fn next(&mut self) -> Option<&'v Image> {
        loop {
            match self.0? {
                Version::Base { image, .. } => {
                    self.0 = None;
                    return Some(image);
                }
                // A view has no image of its own: its ids are its parent's.
                Version::View { parent, .. } => self.0 = Some(parent),
                Version::Overlay { parent, delta, .. } => {
                    self.0 = Some(parent);
                    return Some(delta);
                }
            }
        }
    }
}

impl Version {
    /// The base `bytes` hold.
    pub fn open(bytes: &[u8]) -> Result<Arc<Version>, String> {
        let image = Image::open(bytes)?;
        if image.is_delta() {
            return Err(bad("a delta is opened over its parent (`Version::over`)"));
        }
        let digest = image.own_sum();
        Ok(Arc::new(Version::Base { image: Arc::new(image), digest, analyses: Store::default() }))
    }

    /// The overlay of `parent` and the delta `bytes` hold: the delta's bases must be `parent`'s id spaces, and what
    /// it removes `parent`'s rules.
    pub fn over(parent: &Arc<Version>, bytes: &[u8]) -> Result<Arc<Version>, String> {
        let delta = Image::open(bytes)?;
        if !delta.is_delta() || delta.atom_base() != parent.atom_end() || delta.rel_base() != parent.rel_end() || delta.rule_base() != parent.rule_end() {
            return Err(bad("a delta over another parent"));
        }
        let removed = delta.removed();
        if !removed.iter().all(|&g| parent.contains(RuleId(g))) {
            return Err(bad("a delta removes a rule its parent does not have"));
        }
        let mut d = parent.digest();
        for &g in removed {
            d = d.wrapping_sub(parent.rule(RuleId(g)).expect("checked").hash());
        }
        for k in 0..delta.decl_count() {
            if let Some(old) = parent.decl(delta.decl_name(k)) {
                d = d.wrapping_sub(old.hash());
            }
        }
        for &a in delta.u32s(super::R::DeclGone) {
            if let Some(old) = parent.decl(AtomId(a)) {
                d = d.wrapping_sub(old.hash());
            }
        }
        for k in 0..delta.fact_count() {
            let (a, about) = delta.fact_key(k);
            if !about && let Some(old) = parent.facts(a) {
                d = d.wrapping_sub(old.hash());
            }
        }
        for &key in delta.u32s(super::R::FactGone) {
            if let Some(old) = parent.facts(AtomId(key)) {
                d = d.wrapping_sub(old.hash());
            }
        }
        if delta.owns() & 1 != 0 {
            d = d.wrapping_sub(parent.inputs_hash());
        }
        if delta.owns() & 2 != 0 {
            d = d.wrapping_sub(parent.outputs_hash());
        }
        if delta.owns() & 4 != 0 {
            for f in parent.about_facts() {
                d = d.wrapping_sub(f.hash());
            }
        }
        d = d.wrapping_add(delta.own_sum());
        let size = parent.size() - removed.len() + delta.rules();
        Ok(Arc::new(Version::Overlay { parent: parent.clone(), size, depth: parent.depth() + 1, digest: d, delta: Arc::new(delta), analyses: Store::default() }))
    }

    /// A view of `parent`'s rules in `ranges` (of its rule ids), of `size` rules and digest `digest` (the builder's).
    pub(crate) fn view(parent: &Arc<Version>, ranges: Vec<(u32, u32)>, size: usize, digest: u64) -> Arc<Version> {
        debug_assert!(!matches!(**parent, Version::View { .. }), "a view of a view is of its parent");
        Arc::new(Version::View { parent: parent.clone(), ranges, size, digest, analyses: Store::default() })
    }
    /// Whether rule id `k` is in `ranges`.
    fn in_ranges(ranges: &[(u32, u32)], k: u32) -> bool {
        let at = ranges.partition_point(|&(s, _)| s <= k);
        at > 0 && k < ranges[at - 1].1
    }

    fn down(&self) -> Down<'_> {
        Down(Some(self))
    }
    /// The image this version's own ids are numbered in: a base's, a view's parent's, an overlay's delta.
    fn top(&self) -> &Image {
        self.down().next().expect("a version has an image")
    }
    /// The base image this version is over, at the bottom of its stack.
    pub(crate) fn root(&self) -> &Arc<Image> {
        match self {
            Version::Base { image, .. } => image,
            Version::View { parent, .. } | Version::Overlay { parent, .. } => parent.root(),
        }
    }
    /// Whether `im` is one of this version's images: its ids are this version's.
    pub(crate) fn holds_image(&self, im: &Image) -> bool {
        self.down().any(|x| std::ptr::eq(x, im))
    }

    /// Its analyses (decision 0281 §2): asked for through a pipeline's `Cx`, or `manager::analysis`.
    pub fn analyses(&self) -> &Store {
        match self {
            Version::Base { analyses, .. } | Version::View { analyses, .. } | Version::Overlay { analyses, .. } => analyses,
        }
    }
    /// The number of rules.
    pub fn size(&self) -> usize {
        match self {
            Version::Base { image, .. } => image.rules(),
            Version::View { size, .. } | Version::Overlay { size, .. } => *size,
        }
    }
    /// How many deltas are stacked on the base.
    pub fn depth(&self) -> usize {
        match self {
            Version::Base { .. } => 0,
            Version::View { parent, .. } => parent.depth(),
            Version::Overlay { depth, .. } => *depth,
        }
    }
    /// A base's image bytes, as they are stored (`program_text::store_version`); `None` for a view or an overlay.
    pub fn base_bytes(&self) -> Option<&[u8]> {
        match self {
            Version::Base { image, .. } => Some(image.bytes()),
            _ => None,
        }
    }
    /// One past the last id of each id space: atoms, relations, rules.
    pub fn atom_end(&self) -> u32 {
        self.top().atom_end()
    }
    pub fn rel_end(&self) -> u32 {
        self.top().rel_end()
    }
    pub fn rule_end(&self) -> u32 {
        self.top().rule_end()
    }
    /// The version's digest: the sum of its rules', declarations', fact relations' hashes, and its inputs' and
    /// outputs'. Kept as the version is made; equal programs have equal digests, whatever their stacks.
    pub fn digest(&self) -> u64 {
        match self {
            Version::Base { digest, .. } | Version::View { digest, .. } | Version::Overlay { digest, .. } => *digest,
        }
    }

    // -- names --

    /// The text of atom `a`.
    pub fn text(&self, a: AtomId) -> &str {
        self.down().find(|im| a.0 >= im.atom_base()).expect("an atom of the stack").text(a)
    }
    /// The atom of `text`, if the version holds it.
    pub fn find(&self, text: &str) -> Option<AtomId> {
        self.down().find_map(|im| im.find(text))
    }
    /// The relation atom `a` names, if it names one.
    pub fn rel_of(&self, a: AtomId) -> Option<RelId> {
        self.down().find_map(|im| im.rel_of(a)).flatten()
    }
    /// The relation named `name`.
    pub fn rel(&self, name: &str) -> Option<RelId> {
        self.rel_of(self.find(name)?)
    }
    /// The atom naming relation `r`.
    pub fn rel_atom(&self, r: RelId) -> AtomId {
        self.down().find(|im| r.0 >= im.rel_base()).expect("a relation of the stack").rel_atom(r)
    }
    pub fn rel_name(&self, r: RelId) -> &str {
        self.text(self.rel_atom(r))
    }

    // -- rules --

    /// Each rule, in order: a base's, then each delta's added rules after what is left of its parent's.
    pub fn rules(&self) -> Box<dyn Iterator<Item = RuleRef<'_>> + '_> {
        match self {
            Version::Base { image, .. } => Box::new((0..image.rules() as u32).map(move |k| RuleRef::new(image, k, RuleId(k)))),
            Version::View { parent, ranges, .. } => match &**parent {
                Version::Base { image, .. } => Box::new(ranges.iter().flat_map(move |&(s, e)| (s..e).map(move |k| RuleRef::new(image, k, RuleId(k))))),
                _ => Box::new(parent.rules().filter(move |r| Self::in_ranges(ranges, r.id().0))),
            },
            Version::Overlay { parent, delta, .. } => {
                let removed = delta.removed();
                let base = delta.rule_base();
                Box::new(parent.rules().filter(move |r| removed.binary_search(&r.id().0).is_err()).chain((0..delta.rules() as u32).map(move |k| RuleRef::new(delta, k, RuleId(base + k)))))
            }
        }
    }
    /// Whether rule `id` is one of this version's.
    pub fn contains(&self, id: RuleId) -> bool {
        self.rule(id).is_some()
    }
    /// Rule `id`, if it is one of this version's.
    pub fn rule(&self, id: RuleId) -> Option<RuleRef<'_>> {
        match self {
            Version::Base { image, .. } => (id.index() < image.rules()).then(|| RuleRef::new(image, id.0, id)),
            Version::View { parent, ranges, .. } => Self::in_ranges(ranges, id.0).then(|| parent.rule(id)).flatten(),
            Version::Overlay { parent, delta, .. } => {
                if id.0 >= delta.rule_base() {
                    (id.0 < delta.rule_end()).then(|| RuleRef::new(delta, id.0 - delta.rule_base(), id))
                } else if delta.removed().binary_search(&id.0).is_ok() {
                    None
                } else {
                    parent.rule(id)
                }
            }
        }
    }
    /// The rules whose head is relation `rel`, in the order `rules` gives them.
    pub fn heads(&self, rel: RelId) -> Box<dyn Iterator<Item = RuleRef<'_>> + '_> {
        match self {
            Version::Base { image, .. } => Box::new(image.heads(rel).iter().map(move |&k| RuleRef::new(image, k, RuleId(k)))),
            Version::View { parent, ranges, .. } => Box::new(parent.heads(rel).filter(move |r| Self::in_ranges(ranges, r.id().0))),
            Version::Overlay { parent, delta, .. } => {
                let removed = delta.removed();
                let base = delta.rule_base();
                Box::new(parent.heads(rel).filter(move |r| removed.binary_search(&r.id().0).is_err()).chain(delta.heads(rel).iter().map(move |&k| RuleRef::new(delta, k, RuleId(base + k)))))
            }
        }
    }
    /// The rules that came from rule `src` of the derivation's input (§9), in the order `rules` gives them.
    pub fn derived(&self, src: RuleId) -> Box<dyn Iterator<Item = RuleRef<'_>> + '_> {
        match self {
            Version::Base { image, .. } => Box::new(image.derived(src).map(move |k| RuleRef::new(image, k, RuleId(k)))),
            Version::View { parent, ranges, .. } => Box::new(parent.derived(src).filter(move |r| Self::in_ranges(ranges, r.id().0))),
            Version::Overlay { parent, delta, .. } => {
                let removed = delta.removed();
                let base = delta.rule_base();
                Box::new(parent.derived(src).filter(move |r| removed.binary_search(&r.id().0).is_err()).chain(delta.derived(src).map(move |k| RuleRef::new(delta, k, RuleId(base + k)))))
            }
        }
    }
    /// The atoms in rules' bodies that read relation `rel` (an aggregate's included), each with its rule.
    pub fn readers(&self, rel: RelId) -> Box<dyn Iterator<Item = (RuleRef<'_>, AtomRef<'_>)> + '_> {
        match self {
            Version::Base { image, .. } => Box::new(image.reads(rel).map(move |(k, n)| (RuleRef::new(image, k, RuleId(k)), AtomRef::new(image, n)))),
            Version::View { parent, ranges, .. } => Box::new(parent.readers(rel).filter(move |(r, _)| Self::in_ranges(ranges, r.id().0))),
            Version::Overlay { parent, delta, .. } => {
                let removed = delta.removed();
                let base = delta.rule_base();
                Box::new(parent.readers(rel).filter(move |(r, _)| removed.binary_search(&r.id().0).is_err()).chain(delta.reads(rel).map(move |(k, n)| (RuleRef::new(delta, k, RuleId(base + k)), AtomRef::new(delta, n)))))
            }
        }
    }

    // -- parts --

    /// The declaration of the relation named `a`.
    pub fn decl(&self, a: AtomId) -> Option<DeclRef<'_>> {
        for im in self.down() {
            if let Some(k) = im.decl(a) {
                return Some(DeclRef { im, k });
            }
            if im.decl_gone(a) {
                return None;
            }
        }
        None
    }
    pub fn decl_of(&self, name: &str) -> Option<DeclRef<'_>> {
        self.decl(self.find(name)?)
    }
    /// Every declaration, by atom.
    pub fn decls(&self) -> Vec<DeclRef<'_>> {
        let mut out: BTreeMap<u32, DeclRef<'_>> = BTreeMap::new();
        let mut hidden: HashSet<u32> = HashSet::new();
        for im in self.down() {
            for k in 0..im.decl_count() {
                let a = im.decl_name(k).0;
                if !hidden.contains(&a) {
                    out.entry(a).or_insert(DeclRef { im, k });
                }
            }
            for k in 0..im.decl_count() {
                hidden.insert(im.decl_name(k).0);
            }
            hidden.extend(im.u32s(super::R::DeclGone).iter().copied());
        }
        out.into_values().collect()
    }
    /// The facts of relation `a` (not the `.about` rows).
    pub fn facts(&self, a: AtomId) -> Option<FactsRef<'_>> {
        for im in self.down() {
            if let Some(k) = im.fact(a, false) {
                return Some(FactsRef { im, k });
            }
            if im.fact_gone(a, false) {
                return None;
            }
        }
        None
    }
    /// Every fact relation (not the `.about` rows), by atom.
    pub fn all_facts(&self) -> Vec<FactsRef<'_>> {
        let mut out: BTreeMap<u32, FactsRef<'_>> = BTreeMap::new();
        let mut hidden: HashSet<u32> = HashSet::new();
        for im in self.down() {
            for k in 0..im.fact_count() {
                let (a, about) = im.fact_key(k);
                if !about && !hidden.contains(&a.0) {
                    out.entry(a.0).or_insert(FactsRef { im, k });
                }
            }
            // An image's `.about` rows hide no fact relation of the same name (a program may hold facts named
            // `tape_column`, beside its about rows of that name).
            for k in 0..im.fact_count() {
                let (a, about) = im.fact_key(k);
                if !about {
                    hidden.insert(a.0);
                }
            }
            hidden.extend(im.u32s(super::R::FactGone).iter().copied());
        }
        out.into_values().collect()
    }
    /// The `.about` rows: the program's other fields, as facts; all of them the image's that owns them.
    pub fn about_facts(&self) -> Vec<FactsRef<'_>> {
        let im = self.down().find(|im| im.owns() & 4 != 0).expect("a base owns its `.about` rows");
        (0..im.fact_count()).filter(|&k| im.fact_key(k).1).map(|k| FactsRef { im, k }).collect()
    }
    fn owner(&self, bit: u64) -> &Image {
        self.down().find(|im| im.owns() & bit != 0).expect("a base owns its inputs and outputs")
    }
    pub fn inputs(&self) -> impl ExactSizeIterator<Item = AtomId> + '_ {
        self.owner(1).inputs().iter().map(|&a| AtomId(a))
    }
    pub fn outputs(&self) -> impl ExactSizeIterator<Item = AtomId> + '_ {
        self.owner(2).outputs().iter().map(|&a| AtomId(a))
    }
    pub(crate) fn inputs_hash(&self) -> u64 {
        self.owner(1).inputs_hash()
    }
    pub(crate) fn outputs_hash(&self) -> u64 {
        self.owner(2).outputs_hash()
    }

    /// The program this version is, as the tree: a decode, counted (`trees_decoded`) and, when profiling, named by
    /// the caller's place (the decision "a tree decoded from its version"), so what still reads a tree is seen.
    #[track_caller]
    pub fn program(&self, lists: &Lists) -> Result<Program, String> {
        let at = std::panic::Location::caller();
        DECODED.with(|d| d.set((d.get().0 + 1, Some(at))));
        crate::code_rule_dsl::logic::profile::decision("a tree decoded from its version", || format!("{}:{}", at.file(), at.line()));
        super::Tree::new(self, lists).program()
    }

    /// The program's fields besides its rules (its declarations, facts, inputs, outputs, settings, policies and the
    /// rest of its `.about` rows), as a tree without rules: what a run reads of a program other than its rules, with no
    /// rule decoded (decision 0281's path, step R).
    pub fn header(&self, lists: &Lists) -> Result<Program, String> {
        super::Tree::new(self, lists).program_of(std::iter::empty())
    }

    /// A new base of this version's program, keeping its atom and relation ids (rule ids are renumbered from 0).
    pub fn fold(&self) -> Result<Arc<Version>, String> {
        super::Builder::folding(self).finish().map(|(v, _)| v)
    }
}

thread_local! {
    /// The trees this thread decoded from versions (`Version::program`): a count, and the place of the last.
    static DECODED: std::cell::Cell<(usize, Option<&'static std::panic::Location<'static>>)> = const { std::cell::Cell::new((0, None)) };
}

/// How many trees this thread decoded from versions so far (`Version::program`), and where the last was
/// (`file:line`): a run reads its versions on its own thread, so a check around a run on one thread reads that run's.
pub fn trees_decoded() -> (usize, Option<String>) {
    let (n, at) = DECODED.with(|d| d.get());
    (n, at.map(|l| format!("{}:{}", l.file(), l.line())))
}

/// Forgets this thread's decodes (`trees_decoded`): a check counts a run's alone.
pub fn forget_trees_decoded() {
    DECODED.with(|d| d.set((0, None)));
}

/// What two versions differ in, by text: the rules, declarations, facts and about rows only one has (a debug check's
/// message).
pub fn differences(a: &Version, b: &Version) -> String {
    use std::collections::BTreeSet;
    let texts = |v: &Version| -> BTreeSet<String> {
        let mut out: BTreeSet<String> = v.rules().map(|r| format!("rule {}", v.show_rule(r))).collect();
        for d in v.decls() {
            out.insert(format!("decl {}({})", v.text(d.name()), d.cols().map(|(n, t)| format!("{}: {}", v.text(n), v.text(t))).collect::<Vec<_>>().join(", ")));
        }
        let cell = |c: Cell| match c {
            Cell::Int(i) => i.to_string(),
            Cell::Str(x) => format!("{:?}", v.text(x)),
        };
        for f in v.all_facts().into_iter().chain(v.about_facts()) {
            for (k, row) in f.rows().enumerate() {
                out.insert(format!("{} {} #{k} ({})", if f.about() { "about" } else { "fact" }, v.text(f.name()), row.map(cell).collect::<Vec<_>>().join(", ")));
            }
        }
        out.insert(format!("inputs {:?}", v.inputs().map(|a| v.text(a)).collect::<Vec<_>>()));
        out.insert(format!("outputs {:?}", v.outputs().map(|a| v.text(a)).collect::<Vec<_>>()));
        out
    };
    let (ta, tb) = (texts(a), texts(b));
    let only = |x: &BTreeSet<String>, y: &BTreeSet<String>| x.difference(y).take(12).cloned().collect::<Vec<_>>().join("\n    ");
    format!("\n  only the first:\n    {}\n  only the second:\n    {}", only(&ta, &tb), only(&tb, &ta))
}
