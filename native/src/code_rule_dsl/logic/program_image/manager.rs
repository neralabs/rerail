//! The pass manager (decision 0281): a pass states what it changes, and the manager does the rest.
//!
//! - A **rule pass** (`RulePass`, 0173 §9's local pass) says what becomes of one rule, read alone: keep it, drop it,
//!   or write what replaces it through its `RuleOut`.
//! - A **program pass** (`ProgramPass`) reads analyses of the whole program and states an edit through its
//!   `ProgramW`: a filter, rule passes over a selection, or a derivation written rule by rule.
//! - An **analysis** (`Analysis`) is asked for by type (`cx.get::<Strata>()`), computed once per version and held by
//!   it (`Version::analyses`), and carried to the next version when the edit's `Delta` misses what it reads.
//! - A **pipeline** (`Pipeline`) runs passes in order and owns what every pass did for itself before: the profile
//!   span, the walk over the rules, the writer's form, the delta, the decisions and the stage trace.
//!
//! ```text
//! let mut p = Pipeline::new("normalize", v.clone());
//! p.rules(&mut Normalize::default()).program(&mut StatedApart::default());
//! let (out, stages) = p.finish()?;
//! ```
//!
//! A pipeline made with `keyed` (0281 §6) keys each step by its input, its pass's `NAME`, `VERSION` and `settings`,
//! and the caller's key parts (`key_part`), and keeps each step's output under its key (`Steps`). Asked again, in
//! this process or a new one, it runs only the passes after the latest step the store holds.
//!
//! The manager writes through `Builder` (0173 §6, 0276 §3): a pass never opens one, never copies the rules it
//! keeps, and never picks a view, an overlay or a derivation.
use rerail_blocks::vector::Vector;
use super::build::{Builder, Rewrite, RuleBuilder};
use super::handles::RuleRef;
use super::version::{Made, Version};
use super::{RelId, RuleId};
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, String>;

// ---- dense columns over a stack's ids ----

/// An id global to a stack (`RelId`, `RuleId`): the index of a dense column.
pub trait Dense: Copy {
    fn index(self) -> usize;
    fn at(i: usize) -> Self;
}
impl Dense for RelId {
    fn index(self) -> usize {
        self.0 as usize
    }
    fn at(i: usize) -> Self {
        RelId(i as u32)
    }
}
impl Dense for RuleId {
    fn index(self) -> usize {
        self.0 as usize
    }
    fn at(i: usize) -> Self {
        RuleId(i as u32)
    }
}

/// A set of ids, as a bitset over the stack's ids. An id past its end is not in it, so a set made on one version
/// reads the same on an edit of it that numbered more.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct IdSet<I> {
    bits: Vector<u64>,
    len: usize,
    of: PhantomData<I>,
}
pub type RelSet = IdSet<RelId>;
pub type RuleSet = IdSet<RuleId>;

impl<I: Dense> IdSet<I> {
    pub fn new() -> Self {
        IdSet { bits: rerail_blocks::vector::Vector::new(), len: 0, of: PhantomData }
    }
    pub fn has(&self, i: I) -> bool {
        let k = i.index();
        self.bits.get(k / 64).is_some_and(|w| w >> (k % 64) & 1 == 1)
    }
    /// Adds `i`: whether it was not in the set.
    pub fn insert(&mut self, i: I) -> bool {
        let k = i.index();
        if self.bits.len() <= k / 64 {
            self.bits.resize(k / 64 + 1, 0);
        }
        let (w, b) = (&mut self.bits[k / 64], 1u64 << (k % 64));
        let new = *w & b == 0;
        *w |= b;
        self.len += new as usize;
        new
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// The ids, ascending.
    pub fn iter(&self) -> impl Iterator<Item = I> + '_ {
        self.bits.iter().enumerate().flat_map(|(k, &w)| (0..64).filter(move |b| w >> b & 1 == 1).map(move |b| I::at(k * 64 + b)))
    }
}
impl<I: Dense> FromIterator<I> for IdSet<I> {
    fn from_iter<T: IntoIterator<Item = I>>(it: T) -> Self {
        let mut s = IdSet::new();
        for i in it {
            s.insert(i);
        }
        s
    }
}
impl<I: Dense + std::fmt::Debug> std::fmt::Debug for IdSet<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

/// A value per id, dense: an id past its end, or never set, has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdMap<I, T> {
    at: Vec<Option<T>>,
    of: PhantomData<I>,
}
pub type RelMap<T> = IdMap<RelId, T>;
pub type RuleMap<T> = IdMap<RuleId, T>;

impl<I: Dense, T> Default for IdMap<I, T> {
    fn default() -> Self {
        IdMap { at: Vec::new(), of: PhantomData }
    }
}
impl<I: Dense, T> IdMap<I, T> {
    pub fn get(&self, i: I) -> Option<&T> {
        self.at.get(i.index()).and_then(Option::as_ref)
    }
    pub fn set(&mut self, i: I, t: T) {
        let k = i.index();
        if self.at.len() <= k {
            self.at.resize_with(k + 1, || None);
        }
        self.at[k] = Some(t);
    }
    /// The ids set and their values, ascending.
    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> + '_ {
        self.at.iter().enumerate().filter_map(|(k, t)| t.as_ref().map(|t| (I::at(k), t)))
    }
}

// ---- what an edit touches ----

/// Kinds of a program's fields, by bit: what an analysis reads (`Analysis::READS`) and what an edit changed
/// (`Delta::touched`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Reads(pub u8);
impl Reads {
    pub const NONE: Reads = Reads(0);
    pub const RULES: Reads = Reads(1);
    pub const DECLS: Reads = Reads(2);
    pub const FACTS: Reads = Reads(4);
    pub const INPUTS: Reads = Reads(8);
    pub const OUTPUTS: Reads = Reads(16);
    /// The `.about` rows: natives, lattices, gates, answers and the rest (`About`).
    pub const ABOUT: Reads = Reads(32);
    pub const ALL: Reads = Reads(63);
    pub const fn or(self, o: Reads) -> Reads {
        Reads(self.0 | o.0)
    }
    /// Whether the two share a kind.
    pub const fn meets(self, o: Reads) -> bool {
        self.0 & o.0 != 0
    }
}
impl std::ops::BitOr for Reads {
    type Output = Reads;
    fn bitor(self, o: Reads) -> Reads {
        self.or(o)
    }
}

/// An edit's record (0281 §4): what the output has that the input had not. Read off the version the writer made
/// (an overlay is its delta; a view, its ranges), never by diffing two programs' rules; the fields by their hashes.
#[derive(Clone, Debug, Default)]
pub struct Delta {
    /// The input's rules the output lacks.
    pub removed: RuleSet,
    /// The output's rules the input lacks, in order.
    pub added: Vec<RuleId>,
    /// The output is a new base (a derivation, or a fold): its rules are not numbered as the input's, so every
    /// reader of rules reads them again.
    pub rebuilt: bool,
    /// The relations whose declaration changed (declared, undeclared or redeclared), by the output's ids.
    pub decls: RelSet,
    /// Which kinds changed.
    pub touched: Reads,
    /// The heads of the rules removed (by the input) and added (by the output), by name, where the edit is read off
    /// the output (`None` where it is rebuilt or made otherwise): what a relation's key reads (decision 0385).
    pub heads: Option<std::collections::BTreeSet<String>>,
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.touched == Reads::NONE
    }
    /// The edit's size, in rules removed and added and declarations changed.
    pub fn size(&self) -> usize {
        self.removed.len() + self.added.len() + self.decls.len()
    }

    /// Whether the edit that made `output` from `input` is read off `output` (`of` does not rebuild it).
    pub fn read_off(input: &Arc<Version>, output: &Arc<Version>) -> bool {
        Arc::ptr_eq(input, output)
            || match &**output {
                Version::Overlay { parent, .. } => Arc::ptr_eq(parent, input),
                Version::View { .. } => same_numbering(input, output),
                Version::Base { .. } => false,
            }
    }

    /// The edit that made `output` from `input`.
    pub fn of(input: &Arc<Version>, output: &Arc<Version>) -> Delta {
        let mut d = Delta::default();
        if Arc::ptr_eq(input, output) {
            return d;
        }
        match &**output {
            Version::Overlay { parent, delta, .. } if Arc::ptr_eq(parent, input) => {
                d.removed = delta.removed().iter().map(|&k| RuleId(k)).collect();
                d.added = (0..delta.rules() as u32).map(|k| RuleId(delta.rule_base() + k)).collect();
            }
            Version::View { .. } if same_numbering(input, output) => {
                d.removed = input.rules().map(|r| r.id()).filter(|&id| !output.contains(id)).collect();
            }
            _ => {
                d.rebuilt = true;
                d.removed = input.rules().map(|r| r.id()).collect();
                d.added = output.rules().map(|r| r.id()).collect();
            }
        }
        if d.rebuilt || !d.removed.is_empty() || !d.added.is_empty() {
            d.touched = d.touched | Reads::RULES;
        }
        if !d.rebuilt {
            let removed = d.removed.iter().filter_map(|id| input.rule(id)).map(|r| input.rel_name(r.head().rel()).to_owned());
            let added = d.added.iter().filter_map(|&id| output.rule(id)).map(|r| output.rel_name(r.head().rel()).to_owned());
            d.heads = Some(removed.chain(added).collect());
        }
        // An overlay of the input whose own layer states no declaration or fact (one that removes rules or sets the
        // inputs and outputs, a part's or a selection's) has the input's: those are not compared (each comparison
        // reads every declaration and fact of both).
        let states = match &**output {
            Version::Overlay { parent, delta, .. } if Arc::ptr_eq(parent, input) => States {
                decls: delta.decl_count() > 0 || !delta.u32s(super::R::DeclGone).is_empty(),
                facts: delta.fact_count() > 0 || !delta.u32s(super::R::FactGone).is_empty(),
                about: delta.owns() & 4 != 0,
            },
            _ => States { decls: true, facts: true, about: true },
        };
        fields_delta(input, output, &mut d, states);
        d
    }
}

/// Whether `output` (a view) numbers its rules as `input` does: it is a view of `input`, or of `input`'s parent.
fn same_numbering(input: &Arc<Version>, output: &Arc<Version>) -> bool {
    let Version::View { parent, .. } = &**output else { return false };
    Arc::ptr_eq(parent, input) || matches!(&**input, Version::View { parent: p, .. } if Arc::ptr_eq(p, parent))
}

/// The fields' part of the delta, by each part's hash: declarations by relation, facts, `.about` rows, inputs and
/// outputs. Their count is the program's relations, never its rules.
/// Which of the output's fields its own layer may state apart from the input's (`Delta::of`).
struct States {
    decls: bool,
    facts: bool,
    about: bool,
}

fn fields_delta(input: &Version, output: &Version, d: &mut Delta, states: States) {
    use std::collections::HashMap;
    let decls = |v: &Version| -> HashMap<String, u64> { v.decls().into_iter().map(|x| (v.text(x.name()).to_owned(), x.hash())).collect() };
    let (a, b) = if states.decls { (decls(input), decls(output)) } else { Default::default() };
    for (name, h) in &b {
        if a.get(name) != Some(h) {
            d.decls.insert(output.rel(name).expect("a declared relation has an id"));
        }
    }
    for name in a.keys().filter(|n| !b.contains_key(*n)) {
        if let Some(r) = output.rel(name) {
            d.decls.insert(r);
        }
    }
    if !d.decls.is_empty() {
        d.touched = d.touched | Reads::DECLS;
    }
    let facts = |fs: Vec<super::version::FactsRef<'_>>, v: &Version| -> Vec<(String, u64)> {
        let mut x: Vec<(String, u64)> = fs.into_iter().map(|f| (v.text(f.name()).to_owned(), f.hash())).collect();
        x.sort_unstable();
        x
    };
    if states.facts && facts(input.all_facts(), input) != facts(output.all_facts(), output) {
        d.touched = d.touched | Reads::FACTS;
    }
    if (states.facts || states.about) && facts(input.about_facts(), input) != facts(output.about_facts(), output) {
        d.touched = d.touched | Reads::ABOUT;
    }
    if input.inputs_hash() != output.inputs_hash() {
        d.touched = d.touched | Reads::INPUTS;
    }
    if input.outputs_hash() != output.outputs_hash() {
        d.touched = d.touched | Reads::OUTPUTS;
    }
}

// ---- analyses ----

/// A whole-program analysis (0281 §2): asked for by type, cached per version for one pipeline, carried over an edit
/// whose delta misses what it `READS`, else maintained or computed again. A debug build computes every carried or
/// maintained one again and compares the two, so a wrong `READS` or `maintain` fails a test, not a run.
pub trait Analysis: Sized + Send + Sync + PartialEq + std::fmt::Debug + 'static {
    const NAME: &'static str;
    const READS: Reads;
    fn compute(cx: &Cx<'_>) -> Result<Self>;
    /// It, for the edit's output (`cx`), from its value on the input and the edit: `None` computes it again.
    fn maintain(&self, _cx: &Cx<'_>, _delta: &Delta) -> Option<Self> {
        None
    }
    /// Bumped when what `rows` writes changes: a stored value of another version is not read (decision 0333 §4).
    const VERSION: u32 = 1;
    /// It as rows, to store with a kept run's version (decision 0333 §4); `None`: not stored, computed again.
    fn rows(&self, _v: &Version) -> Option<Vec<Row>> {
        None
    }
    /// It read back from `rows` for `v`, the version it was stored with, reopened (its ids are the same).
    fn of_rows(_v: &Version, _rows: &[Row]) -> Option<Self> {
        None
    }
}

/// A row of a stored analysis or run: values as facts hold them (decision 0333).
pub type Row = Vec<crate::code_rule_dsl::logic::front::Value>;

/// The rows stored for a reopened version's analyses, by name and version (decision 0333 §4).
pub type StoredRows = Arc<dyn Fn(&str, u32) -> Option<Vec<Row>> + Send + Sync>;

type Value = Arc<dyn Any + Send + Sync>;

/// A cached analysis, with what the store needs of its type to carry it.
#[derive(Clone)]
struct Entry {
    value: Value,
    reads: Reads,
    name: &'static str,
    maintain: fn(&Value, &Cx<'_>, &Delta) -> Option<Value>,
    agrees: fn(&Value, &Cx<'_>) -> Result<bool>,
    version: u32,
    rows: fn(&Value, &Version) -> Option<Vec<Row>>,
}

fn rows_as<A: Analysis>(v: &Value, of: &Version) -> Option<Vec<Row>> {
    v.downcast_ref::<A>()?.rows(of)
}

fn maintain_as<A: Analysis>(v: &Value, cx: &Cx<'_>, d: &Delta) -> Option<Value> {
    v.downcast_ref::<A>()?.maintain(cx, d).map(|a| Arc::new(a) as Value)
}
fn get_as<A: Analysis>(cx: &Cx<'_>) -> Result<Value> {
    cx.get::<A>().map(|a| a as Value)
}
fn same_as<A: Analysis>(a: &Value, b: &Value) -> bool {
    Arc::ptr_eq(a, b) || a.downcast_ref::<A>().zip(b.downcast_ref::<A>()).is_some_and(|(a, b)| a == b)
}

/// An analysis a step asked for: what a rerun compares on its new input (`Pipeline::again`).
#[derive(Clone, Copy)]
struct Asked {
    t: TypeId,
    name: &'static str,
    get: fn(&Cx<'_>) -> Result<Value>,
    same: fn(&Value, &Value) -> bool,
}

fn agrees_as<A: Analysis>(v: &Value, cx: &Cx<'_>) -> Result<bool> {
    Ok(v.downcast_ref::<A>().is_some_and(|a| A::compute(cx).is_ok_and(|fresh| fresh == *a)))
}

/// A version's analyses, from its slot (`Version::held`), where the first ask puts an empty store.
pub trait VersionAnalyses {
    fn analyses(&self) -> &Store;
}

impl VersionAnalyses for Version {
    fn analyses(&self) -> &Store {
        self.held().0.get_or_init(|| Box::new(Store::default())).downcast_ref::<Store>().expect("a version's slot holds the manager's store")
    }
}

/// A version's analyses (`VersionAnalyses`), held by the version beside its indexes and dropped with it: not a
/// process cache (0103), and never keyed by anything but the version that holds it. A pipeline carries them from a
/// pass's input to its output (`carry`).
#[derive(Default)]
pub struct Store {
    of: parking_lot::Mutex<HashMap<TypeId, Entry>>,
    /// How many times each analysis was computed for this version, by name: what a test of the cache reads.
    computed: parking_lot::Mutex<HashMap<&'static str, usize>>,
    /// The rows a stored run kept of this version's analyses, read before computing one (decision 0333 §4).
    stored: parking_lot::Mutex<Option<StoredRows>>,
    /// How many analyses were read from `stored`, by name.
    loaded: parking_lot::Mutex<HashMap<&'static str, usize>>,
    /// The version this one is an edit of, and the edit (decision 0393): an analysis asked for here that the other
    /// holds or kept as rows is maintained across the edit (`Analysis::maintain`) before it is computed.
    lineage: parking_lot::Mutex<Option<(Arc<Version>, Delta)>>,
    /// The sha256 of this version's image (or overlay delta) as a store holds it, where it was opened from one: a
    /// run kept again names it, and copies no bytes (decision 0393).
    kept_image: parking_lot::Mutex<Option<[u8; 32]>>,
}

impl Store {
    /// How many times analysis `name` was computed for this version (not carried to it).
    pub fn computed(&self, name: &str) -> usize {
        self.computed.lock().get(name).copied().unwrap_or(0)
    }
    /// How many times analysis `name` was read from a stored run's rows (decision 0333 §4).
    pub fn loaded(&self, name: &str) -> usize {
        self.loaded.lock().get(name).copied().unwrap_or(0)
    }
    /// `from`'s analyses held and kept as rows, as this store's: `from` is a version of the same rules under the same
    /// ids (`Run::shallow`, decision 0385).
    pub fn adopt(&self, from: &Store) {
        let held: Vec<(TypeId, Entry)> = from.of.lock().iter().map(|(t, e)| (*t, e.clone())).collect();
        self.of.lock().extend(held);
        *self.stored.lock() = from.stored.lock().clone();
        *self.lineage.lock() = from.lineage.lock().clone();
    }
    /// Records that this store's version was opened from a store's image of sha256 `digest` (`kept_image`).
    pub fn set_kept_image(&self, digest: [u8; 32]) {
        *self.kept_image.lock() = Some(digest);
    }
    /// The sha256 of the image a store holds of this version, where it was opened from one.
    pub fn kept_image(&self) -> Option<[u8; 32]> {
        *self.kept_image.lock()
    }
    /// Records that this store's version is `delta`, an edit of `from` (`lineage`), where it is no rebuild.
    fn set_lineage(&self, from: &Arc<Version>, delta: &Delta) {
        if std::ptr::eq(self, from.analyses()) || delta.rebuilt {
            return;
        }
        let mut l = self.lineage.lock();
        if l.is_none() {
            *l = Some((from.clone(), delta.clone()));
        }
    }
    /// Analysis `A` of `v` (this store's version) where it is held or a stored run kept its rows: never computed.
    fn held_or_kept<A: Analysis>(&self, v: &Version) -> Option<Arc<A>> {
        if let Some(e) = self.of.lock().get(&TypeId::of::<A>()) {
            return e.value.clone().downcast::<A>().ok();
        }
        let stored = self.stored.lock().clone()?;
        let a = Arc::new(A::of_rows(v, &stored(A::NAME, A::VERSION)?)?);
        *self.loaded.lock().entry(A::NAME).or_default() += 1;
        let e = Entry { value: a.clone(), reads: A::READS, name: A::NAME, maintain: maintain_as::<A>, agrees: agrees_as::<A>, version: A::VERSION, rows: rows_as::<A> };
        let kept = self.of.lock().entry(TypeId::of::<A>()).or_insert(e).value.clone();
        kept.downcast::<A>().ok()
    }
    /// The rows a stored run kept of these analyses: read before computing one.
    pub fn read_stored(&self, rows: StoredRows) {
        *self.stored.lock() = Some(rows);
    }
    /// The analyses held that can be stored, as rows of `v` (theirs): name, version and rows.
    pub fn rows(&self, v: &Version) -> Vec<(&'static str, u32, Vec<Row>)> {
        let held: Vec<Entry> = self.of.lock().values().cloned().collect();
        let mut out: Vec<_> = held.iter().filter_map(|e| (e.rows)(&e.value, v).map(|rows| (e.name, e.version, rows))).collect();
        out.sort_by_key(|(name, ..)| *name);
        out
    }
    /// `a`, analysis `A` of this version read from a store apart from a stored run's rows (the kept plans' entry,
    /// decision 0361 step 5), held as if computed and counted as loaded: the one held already, if any.
    pub fn hold<A: Analysis>(&self, a: Arc<A>) -> Arc<A> {
        *self.loaded.lock().entry(A::NAME).or_default() += 1;
        let e = Entry { value: a, reads: A::READS, name: A::NAME, maintain: maintain_as::<A>, agrees: agrees_as::<A>, version: A::VERSION, rows: rows_as::<A> };
        let kept = self.of.lock().entry(TypeId::of::<A>()).or_insert(e).value.clone();
        kept.downcast::<A>().expect("an entry of its type")
    }
    /// Whether analysis `A` is held.
    pub fn holds<A: Analysis>(&self) -> bool {
        self.of.lock().contains_key(&TypeId::of::<A>())
    }
    /// `from`'s analyses (an edit's input's) carried into this store (its output's, whose context is `cx`) across
    /// `delta`: those whose `READS` it misses as they are, then those it touches that `maintain` gives; the rest are
    /// left to be computed when asked for. A debug build checks each against a fresh computation in a scratch store.
    fn carry(&self, from: &Store, cx: &Cx<'_>, delta: &Delta) -> Result<()> {
        if std::ptr::eq(self, from) {
            return Ok(());
        }
        let old: Vec<(TypeId, Entry)> = from.of.lock().iter().map(|(t, e)| (*t, e.clone())).collect();
        let mut carried = Vec::new();
        for (t, e) in old {
            if self.of.lock().contains_key(&t) {
                continue;
            }
            let value = if !e.reads.meets(delta.touched) { Some(e.value.clone()) } else { (e.maintain)(&e.value, cx, delta) };
            if let Some(value) = value {
                carried.push((t, Entry { value, ..e }));
            }
        }
        if cfg!(debug_assertions) {
            let scratch = Store::default();
            let fresh = Cx { v: cx.v, store: &scratch, pass: cx.pass, decided: cx.decided, asked: None };
            for (_, e) in &carried {
                if !(e.agrees)(&e.value, &fresh)? {
                    return Err(format!("analysis {} carried over {}'s edit ({:?}) is not what it computes", e.name, cx.pass, delta.touched));
                }
            }
        }
        let mut of = self.of.lock();
        for (t, e) in carried {
            of.entry(t).or_insert(e);
        }
        Ok(())
    }
}

/// Analysis `A` of `v`, held by `v` (`Version::analyses`): for a reader outside a pipeline (a run's `Placed`).
/// `v`'s twin (`Version::twin`) holding `v`'s analyses: what an analysis of `v` makes over `v` it makes over this,
/// so the analysis, held by `v`, does not hold `v` (a cycle, which no drop ends: Q377). The analyses are shared, as
/// they are of the same rules under the same ids (`Store::adopt`), so none is computed again; none adopted is a
/// version made over `v` (each is made over a twin), so the twin holds none of `v` either.
pub fn twin(v: &Arc<Version>) -> Arc<Version> {
    let t = v.twin();
    t.analyses().adopt(v.analyses());
    t
}

pub fn analysis<A: Analysis>(v: &Arc<Version>) -> Result<Arc<A>> {
    let decided = RefCell::new(Vec::new());
    Cx { v, store: v.analyses(), pass: "reader", decided: &decided, asked: None }.get::<A>()
}

/// Analysis `A` of `to`, an edit of `from` (an overlay or view of it): `from`'s analyses carried across the edit
/// (`Delta::of`), maintained where they can be, as a pipeline's step carries them. A pass that tries a program of
/// its own (a round's candidate) reads its analyses so at the round's cost.
pub fn carried<A: Analysis>(from: &Arc<Version>, to: &Arc<Version>) -> Result<Arc<A>> {
    let d = Delta::of(from, to);
    let decided = RefCell::new(Vec::new());
    let cx = Cx { v: to, store: to.analyses(), pass: "carried", decided: &decided, asked: None };
    to.analyses().carry(from.analyses(), &cx, &d)?;
    cx.get::<A>()
}

/// A decision a pass made, as a row of its step (0269): the pass, what, and its detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decided {
    pub pass: std::borrow::Cow<'static, str>,
    pub what: std::borrow::Cow<'static, str>,
    pub detail: String,
}

/// What a pass reads: its input version, the analyses of it, and where its decisions go.
pub struct Cx<'a> {
    v: &'a Arc<Version>,
    store: &'a Store,
    pass: &'static str,
    decided: &'a RefCell<Vec<Decided>>,
    /// Where a step records the analyses its pass asks for, when it is kept to run again.
    asked: Option<&'a RefCell<Vec<Asked>>>,
}

impl<'a> Cx<'a> {
    /// The version the pass reads.
    pub fn v(&self) -> &'a Arc<Version> {
        self.v
    }
    /// Analysis `A` of the version: the one cached, else computed now and cached.
    pub fn get<A: Analysis>(&self) -> Result<Arc<A>> {
        if let Some(asked) = self.asked {
            let t = TypeId::of::<A>();
            let mut asked = asked.borrow_mut();
            if !asked.iter().any(|a| a.t == t) {
                asked.push(Asked { t, name: A::NAME, get: get_as::<A>, same: same_as::<A> });
            }
        }
        if let Some(e) = self.store.of.lock().get(&TypeId::of::<A>()) {
            return Ok(e.value.clone().downcast::<A>().expect("an entry of its type"));
        }
        // A stored run's rows of it, for a version reopened from that run (decision 0333 §4).
        let stored = self.store.stored.lock().clone();
        let read = stored.and_then(|rows| rows(A::NAME, A::VERSION)).and_then(|rows| A::of_rows(self.v, &rows));
        if let Some(a) = &read {
            *self.store.loaded.lock().entry(A::NAME).or_default() += 1;
            if cfg!(debug_assertions) && A::compute(self)? != *a {
                return Err(format!("analysis {} read from a stored run is not what it computes", A::NAME));
            }
        }
        // Maintained from the version this one is an edit of, where that one holds it (decision 0393).
        let mut read = read.map(Arc::new);
        if read.is_none() {
            let lineage = self.store.lineage.lock().clone();
            if let Some((from, delta)) = lineage
                && let Some(was) = from.analyses().held_or_kept::<A>(&from)
            {
                read = if A::READS.meets(delta.touched) { was.maintain(self, &delta).map(Arc::new) } else { Some(was) };
                if read.is_some() {
                    *self.store.loaded.lock().entry(A::NAME).or_default() += 1;
                }
            }
        }
        // Computed with no lock held: it may ask for other analyses. Two threads asking at once both compute it; the
        // first one stored is kept.
        let a = match read {
            Some(a) => a,
            None => {
                let a = Arc::new(A::compute(self)?);
                *self.store.computed.lock().entry(A::NAME).or_default() += 1;
                a
            }
        };
        let e = Entry { value: a.clone(), reads: A::READS, name: A::NAME, maintain: maintain_as::<A>, agrees: agrees_as::<A>, version: A::VERSION, rows: rows_as::<A> };
        let kept = self.store.of.lock().entry(TypeId::of::<A>()).or_insert(e).value.clone();
        Ok(kept.downcast::<A>().expect("an entry of its type"))
    }
    /// Records a decision of the pass.
    pub fn decide(&self, what: &'static str, detail: impl FnOnce() -> String) {
        self.decided.borrow_mut().push(Decided { pass: self.pass.into(), what: what.into(), detail: detail() });
    }
}

// ---- rule passes ----

/// The rules a rule pass visits (0281 §3), found through the version's indexes: all of them, the rules of some
/// heads, or the readers of some relations, or named rules. A pass costs what it selects.
#[derive(Clone, Debug, PartialEq)]
pub enum Select {
    All,
    Heads(RelSet),
    Readers(RelSet),
    Rules(RuleSet),
}

impl Select {
    /// The rules selected, by id, ascending; `None` for all.
    fn ids(&self, v: &Version) -> Option<RuleSet> {
        match self {
            Select::All => None,
            Select::Heads(rels) => Some(rels.iter().flat_map(|r| v.heads(r).map(|x| x.id()).collect::<Vec<_>>()).collect()),
            Select::Readers(rels) => Some(rels.iter().flat_map(|r| v.readers(r).map(|(x, _)| x.id()).collect::<Vec<_>>()).collect()),
            Select::Rules(ids) => Some(ids.iter().filter(|&id| v.contains(id)).collect()),
        }
    }
    /// Whether it selects rule `r`: what `ids` gives, for one rule.
    fn picks(&self, r: RuleRef<'_>) -> bool {
        match self {
            Select::All => true,
            Select::Heads(rels) => rels.has(r.head().rel()),
            Select::Readers(rels) => r.reads().any(|a| rels.has(a.rel())),
            Select::Rules(ids) => ids.has(r.id()),
        }
    }
}

/// What became of a rule a rule pass visited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// As it was: the manager keeps it, by its range or a copy.
    Keep,
    /// Gone, nothing in its place.
    Drop,
    /// Replaced by what the pass wrote through its `RuleOut` (one rule or more, each recording it as its source).
    Written,
}

/// A pass local to a rule (0173 §9): what it does to one rule, read alone.
pub trait RulePass {
    const NAME: &'static str;
    /// Bumped when what the pass makes of an input changes (a keyed step's key holds it, 0281 §6).
    const VERSION: u32 = 1;
    /// What the pass reads besides its input and analyses (its options), as bytes for a keyed step's key.
    fn settings(&self) -> Vec<u8> {
        Vec::new()
    }
    /// The kinds of change to its input that make a rerun visit every rule again (`Pipeline::again`): what it reads
    /// of the version directly, beyond the rule it visits and the analyses it asks for (those a rerun compares).
    /// `ALL`, the default, reruns it whole on any edit. A pass that sets less keeps no state across rules (its
    /// `rule` reads the rule and its `begin`'s analyses alone, and its `finish` writes only what `begin` decides), and
    /// returns its results in its output, not in itself: a rerun visits the edit's added rules alone.
    const WHOLE: Reads = Reads::ALL;
    /// Before the first rule: the rules it visits. Analyses a pass reads for every rule are asked for here.
    fn begin(&mut self, _cx: &Cx<'_>) -> Result<Select> {
        Ok(Select::All)
    }
    /// What becomes of rule `r`; what replaces it is written through `out`.
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step>;
    /// After the last rule: the declarations, facts and rules the pass adds once.
    fn finish(&mut self, _cx: &Cx<'_>, _b: &mut Builder<'_>) -> Result<()> {
        Ok(())
    }
}

/// The writer bound to the rule a rule pass is visiting: each rule written through it records that rule as its
/// source (§9).
pub struct RuleOut<'o, 'p> {
    b: &'o mut Builder<'p>,
    v: &'o Version,
    r: RuleRef<'o>,
    wrote: usize,
}

impl<'o, 'p> RuleOut<'o, 'p> {
    /// The rule through `rw`, written whole.
    pub fn rewrite(&mut self, rw: &mut impl Rewrite) -> Result<Step> {
        self.b.rewrite(self.v, self.r, rw).finish()?;
        self.wrote += 1;
        Ok(Step::Written)
    }
    /// The rule through `rw`, its head and kept literals written; more may follow before it is finished.
    pub fn rewriting(&mut self, rw: &mut impl Rewrite) -> RuleBuilder<'_, 'p> {
        self.wrote += 1;
        self.b.rewrite(self.v, self.r, rw)
    }
    /// A rule written term by term, from this one.
    pub fn rule(&mut self) -> RuleBuilder<'_, 'p> {
        self.wrote += 1;
        let id = self.r.id();
        self.b.rule().source(id)
    }
    /// The writer, for its names (`rel`, `atom`) and its fields.
    pub fn builder(&mut self) -> &mut Builder<'p> {
        self.b
    }
}

/// Runs rule pass `pass` over `v` into `b`: in a derivation (`derive`), every rule of `v` in order, the selected
/// visited and the rest kept; in an edit of `v`, the selected alone, each kept left in place. Returns how many rules
/// it visited.
fn drive<P: RulePass>(pass: &mut P, cx: &Cx<'_>, v: &Version, b: &mut Builder<'_>, select: &Select, derive: bool) -> Result<usize> {
    let picked = select.ids(v);
    let mut visited = 0;
    if derive {
        // The kept rules are copied in runs (`Builder::keep_run`): a run ends where a visited rule writes.
        let mut kept: Vec<RuleRef<'_>> = Vec::new();
        for r in v.rules() {
            let step = match &picked {
                Some(p) if !p.has(r.id()) => Step::Keep,
                _ => {
                    b.keep_run(v, &std::mem::take(&mut kept));
                    visited += 1;
                    visit(pass, cx, v, r, b)?
                }
            };
            if step == Step::Keep {
                kept.push(r);
            }
        }
        b.keep_run(v, &kept);
    } else {
        let ids: Vec<RuleId> = match &picked {
            Some(p) => p.iter().collect(),
            None => v.rules().map(|r| r.id()).collect(),
        };
        for id in ids {
            let r = v.rule(id).expect("a selected rule of the version");
            visited += 1;
            if visit(pass, cx, v, r, b)? != Step::Keep {
                b.remove(id);
            }
        }
    }
    Ok(visited)
}

/// One rule through `pass`: what it says became of it, checked against what it wrote.
fn visit<P: RulePass>(pass: &mut P, cx: &Cx<'_>, v: &Version, r: RuleRef<'_>, b: &mut Builder<'_>) -> Result<Step> {
    let mut out = RuleOut { b, v, r, wrote: 0 };
    let step = pass.rule(cx, r, &mut out)?;
    match (step, out.wrote) {
        (Step::Keep, n) if n > 0 => Err(format!("{}: wrote {n} rules for a rule it kept", P::NAME)),
        (Step::Drop, n) if n > 0 => Err(format!("{}: wrote {n} rules for a rule it dropped", P::NAME)),
        _ => Ok(step),
    }
}

// ---- program passes ----

/// A pass over the whole program: it reads analyses and states an edit through `w`.
pub trait ProgramPass {
    const NAME: &'static str;
    /// As `RulePass::VERSION`.
    const VERSION: u32 = 1;
    /// As `RulePass::settings`.
    fn settings(&self) -> Vec<u8> {
        Vec::new()
    }
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()>;
    /// Over an edit of its last run's input of rules alone (`Pipeline::again`, decisions 0393, 0385): its output
    /// written onto its last output (`w`'s input) from the edit alone, where the decisions it made over the whole are the edit's input's
    /// too. `Ok(false)`, the default, runs it whole. A pass that writes here leaves in its last output every rule but
    /// what came from the edit's removed rules (`Version::derived`), and writes what its added rules make.
    fn again(&mut self, _cx: &Cx<'_>, _last: &Last<'_>, _w: &mut ProgramW<'_>) -> Result<bool> {
        Ok(false)
    }
    /// What it decided over its whole input that its `again` reads back (`Last::plan`), as rows kept with its step
    /// (decision 0393): none by default.
    fn plan(&self) -> Vec<Row> {
        Vec::new()
    }
}

/// A program pass's rule part over an edit (`ProgramPass::again`, decision 0393), written onto its last output (`w`'s
/// input): what came from each of the edit's removed rules dropped (`Version::derived`), and each added rule visited
/// by `pass`, kept (copied with itself as its source) or replaced by what it wrote. The rules visited.
pub fn rules_again<P: RulePass>(pass: &mut P, cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<usize> {
    let v = cx.v;
    let b = w.on()?;
    for id in last.delta.removed.iter() {
        let came: Vec<RuleId> = last.output.derived(id).map(|r| r.id()).collect();
        for c in came {
            b.remove(c);
        }
    }
    let mut visited = 0;
    for &id in &last.delta.added {
        let r = v.rule(id).ok_or_else(|| format!("rule {} of the edit is not its output's", id.0))?;
        visited += 1;
        if let Step::Keep = visit(pass, cx, v, r, b)? {
            b.keep_into(v, r);
        }
    }
    w.visited += visited;
    Ok(visited)
}

/// Rule `r` of `of` (a pass's last input) as `out` (its last output) holds it, as itself: by its id where `out`
/// numbers it as `of` does, else the rule of its head with its text (an output written onto an earlier output
/// numbers its rules its own way).
pub fn held_as(out: &Version, of: &Version, r: RuleRef<'_>) -> Option<RuleId> {
    let same = |o: RuleRef<'_>| o.hash() == r.hash() && out.show_rule(o) == of.show_rule(r);
    out.rule(r.id()).filter(|&o| same(o)).or_else(|| out.rel(of.rel_name(r.head().rel())).and_then(|h| out.heads(h).find(|&o| same(o)))).map(|o| o.id())
}

/// The edit itself written onto the last output (`ProgramPass::again`, decision 0393), for a pass whose output holds
/// each rule of its input as itself and adds what its decisions make, where the edit changes none of them: each
/// removed rule taken out, each added one copied in. `Ok(false)` where the last output does not hold a removed rule.
pub fn edit_as_is(cx: &Cx<'_>, last: &Last<'_>, w: &mut ProgramW<'_>) -> Result<bool> {
    let v = cx.v;
    let mut gone = Vec::new();
    for id in last.delta.removed.iter() {
        let Some(r) = last.input.rule(id) else { return Ok(false) };
        match held_as(last.output, last.input, r) {
            Some(o) => gone.push(o),
            None => return Ok(false),
        }
    }
    let b = w.on()?;
    for id in gone {
        b.remove(id);
    }
    for &id in &last.delta.added {
        let r = v.rule(id).ok_or_else(|| format!("rule {} of the edit is not its output's", id.0))?;
        b.copy(v, r);
    }
    Ok(true)
}

/// A program pass's last run, for `ProgramPass::again`: its input and output, and the edit from that input to the
/// one it runs over now (`cx`'s).
pub struct Last<'a> {
    pub input: &'a Arc<Version>,
    pub output: &'a Arc<Version>,
    pub delta: &'a Delta,
    /// What it kept of its decisions (`ProgramPass::plan`).
    pub plan: &'a [Row],
}

/// The forms a program pass's writer takes (`ProgramW`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// An edit: a view, an overlay, or folded, as its size says.
    Edit,
    /// An overlay, never folded: a version whose cost must stay its own size (a part's fields, decision 0244).
    Overlay,
    /// A derivation: a new base under the input's ids.
    Derive,
}

/// The writer a program pass states its edit through (0276 §3): an edit of its input (`on`), an overlay of it never
/// folded (`overlay`), or a derivation of it (`derive`), one of the three. Nothing stated, the input is the output.
pub struct ProgramW<'p> {
    input: &'p Arc<Version>,
    b: Option<(Builder<'p>, Mode)>,
    visited: usize,
}

impl<'p> ProgramW<'p> {
    fn new(input: &'p Arc<Version>) -> Self {
        ProgramW { input, b: None, visited: 0 }
    }
    fn builder(&mut self, mode: Mode) -> Result<&mut Builder<'p>> {
        let input = self.input;
        let (b, m) = self.b.get_or_insert_with(|| (if mode == Mode::Derive { Builder::derive(input) } else { Builder::on(input) }, mode));
        if *m != mode {
            return Err(format!("an edit of {} rules stated in two forms ({m:?}, {mode:?})", input.size()));
        }
        Ok(b)
    }
    /// The input.
    pub fn input(&self) -> &'p Arc<Version> {
        self.input
    }
    /// An edit of the input: removals make a view; additions and fields an overlay, folded when it reaches half.
    pub fn on(&mut self) -> Result<&mut Builder<'p>> {
        self.builder(Mode::Edit)
    }
    /// An overlay of the input, never folded.
    pub fn overlay(&mut self) -> Result<&mut Builder<'p>> {
        self.builder(Mode::Overlay)
    }
    /// A derivation of the input: a new base under its ids, holding its fields and none of its rules.
    pub fn derive(&mut self) -> Result<&mut Builder<'p>> {
        self.builder(Mode::Derive)
    }
    /// The input's rules `keep` refuses removed.
    pub fn keep(&mut self, keep: impl FnMut(RuleRef<'_>) -> bool) -> Result<()> {
        self.on()?.retain(keep);
        Ok(())
    }
    /// Rule pass `pass` over the rules `select` picks, in this writer's form (a derivation keeps the rest in order).
    pub fn apply<P: RulePass>(&mut self, cx: &Cx<'_>, pass: &mut P, select: &Select) -> Result<()> {
        let mode = self.b.as_ref().map_or(Mode::Edit, |(_, m)| *m);
        let input = self.input;
        let b = self.builder(mode)?;
        let n = drive(pass, cx, input, b, select, mode == Mode::Derive)?;
        pass.finish(cx, b)?;
        self.visited += n;
        Ok(())
    }
    fn finish(self) -> Result<(Arc<Version>, Form, usize)> {
        // Nothing stated (or an edit or overlay that states nothing): the input is the output.
        let Some((b, mode)) = self.b.filter(|(b, m)| *m == Mode::Derive || !b.is_empty()) else { return Ok((self.input.clone(), Form::Unchanged, self.visited)) };
        let (v, form) = match mode {
            Mode::Overlay => (b.finish_overlay()?, Form::Overlay),
            Mode::Derive => (b.finish()?.0, Form::Derived),
            Mode::Edit => {
                let (v, made) = b.finish()?;
                (v, match made {
                    Made::View => Form::View,
                    Made::Overlay => Form::Overlay,
                    Made::Folded => Form::Folded,
                })
            }
        };
        // A derivation that changed nothing is its input: its analyses carry, and no new base is kept. The digest is
        // a sum over the rules, blind to their order, so the rules are compared in order too (a derivation that
        // reorders them, `RuleOrder`, changes the program).
        if mode == Mode::Derive && v.digest() == self.input.digest() && v.size() == self.input.size() && v.rules().map(|r| r.hash()).eq(self.input.rules().map(|r| r.hash())) {
            return Ok((self.input.clone(), Form::Unchanged, self.visited));
        }
        Ok((v, form, self.visited))
    }
}

// ---- the pipeline ----

/// The form a step's writer chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Nothing changed: the input is the output.
    Unchanged,
    View,
    Overlay,
    Folded,
    Derived,
}

/// How a step ran (`Pipeline::again`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ran {
    /// Over its whole input (a program pass's analyses carried from its last input when it has one).
    Whole,
    /// A rule pass over its input's edit since its last run alone: the added rules visited, what came from the
    /// removed ones dropped through `from`, onto its last output.
    Delta,
    /// Not at all: its input is its last run's, so its output is too.
    Reused,
    /// Not at all: a keyed pipeline's store held its key (0281 §6), so its output is the stored one.
    Stored,
}

/// A pass's step, as the pipeline traces it: the pass, how it ran, the form, the rules it visited, the versions'
/// sizes and digests, and the size of the edit its writer wrote.
#[derive(Clone, Debug)]
pub struct Stage {
    pub pass: &'static str,
    pub ran: Ran,
    pub form: Form,
    pub visited: usize,
    pub input: (usize, u64),
    pub output: (usize, u64),
    pub delta: usize,
    pub touched: Reads,
}

/// A step as a kept run holds it: its input and output, and what a rerun of a rule pass compares.
struct Kept {
    pass: std::borrow::Cow<'static, str>,
    input: Arc<Version>,
    output: Arc<Version>,
    form: Form,
    /// A rule pass's, when its output can be run again onto (`None` reruns it whole).
    rule: Option<RuleKept>,
    /// A program pass's plan (`ProgramPass::plan`).
    plan: Vec<Row>,
    /// The profile's decisions a program pass made when it ran whole, stated again where it runs over an edit
    /// (decision 0385).
    said: Vec<crate::code_rule_dsl::logic::profile::Decided>,
    /// The request record as the pipeline carried it after the step (`AskedByRule`, by the output's rule ids), where
    /// it carried one: a step run again over an edit onto this output carries it on (`AskedByRule::onto`).
    asked: Option<AskedByRule>,
}

struct RuleKept {
    select: Select,
    /// The analyses it asked for, compared on the next input.
    asked: Vec<Asked>,
    /// Those of a run read from the store, by name (decision 0333 §3): matched to what the pass asks for again.
    named: Vec<String>,
    /// The input's rules below this id that the output holds as themselves (an edit's output); every other rule of
    /// the output records the input's rule it came from (`from`).
    own_end: u32,
}

/// A pipeline's run, kept to run again over an edit of its input (`Pipeline::again`, 0281 §3): each step's input
/// and output, and what a rerun compares. It holds versions, not a cache (0103): the caller keeps it, as a session
/// keeps its last program, and drops it with them.
pub struct Run {
    name: &'static str,
    input: Arc<Version>,
    output: Arc<Version>,
    stages: Vec<Stage>,
    steps: Vec<Kept>,
    decided: Vec<Decided>,
}

impl Run {
    /// The version it ran over.
    pub fn input(&self) -> &Arc<Version> {
        &self.input
    }
    pub fn output(&self) -> &Arc<Version> {
        &self.output
    }
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }
    pub fn decisions(&self) -> &[Decided] {
        &self.decided
    }

    /// The run with each of its versions `REBASE_DEPTH` deltas deep or more made flat (`build::flat`, decision
    /// 0385): the same rules under the same ids, a view of one base. The next edit is written onto it a layer deep,
    /// and run again over the edit, where before it ran whole (`rebase` refuses a version that deep). Its analyses are
    /// the deep one's. A version that cannot be made flat is left as it is.
    pub fn shallow(self) -> Run {
        if self.steps.iter().all(|k| k.input.depth() < REBASE_DEPTH && k.output.depth() < REBASE_DEPTH) && self.input.depth() < REBASE_DEPTH && self.output.depth() < REBASE_DEPTH {
            return self;
        }
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("a run made shallow ({} steps; decision 0385)", self.steps.len()));
        let mut made: HashMap<*const Version, Arc<Version>> = HashMap::new();
        let mut flat = |v: &Arc<Version>| -> Arc<Version> {
            if v.depth() < REBASE_DEPTH {
                return v.clone();
            }
            made.entry(Arc::as_ptr(v))
                .or_insert_with(|| match super::build::Builder::flat(v) {
                    Ok(Some(f)) => {
                        f.analyses().adopt(v.analyses());
                        f
                    }
                    _ => v.clone(),
                })
                .clone()
        };
        let Run { name, input, output, stages, steps, decided } = self;
        let steps = steps.into_iter().map(|k| Kept { input: flat(&k.input), output: flat(&k.output), ..k }).collect();
        Run { name, input: flat(&input), output: flat(&output), stages, steps, decided }
    }
}

/// The last run's step at a step's place, against the step's input.
enum Prior {
    None,
    /// Its input is the step's.
    Same(Kept),
    /// The step's input is an edit of its input (`Delta::of` reads it; not a new base).
    Edit(Kept, Delta),
}

/// Passes run in order over a version (0281 §5). Each step's error stops the pipeline, named by its pass; `finish`
/// returns it. A pipeline made to be kept (`keeping`, `again`) holds each step's versions to the end, for `done`; one
/// made `keyed` stores each step under its key (0281 §6).
pub struct Pipeline<'s> {
    name: &'static str,
    input: Arc<Version>,
    v: Arc<Version>,
    decided: RefCell<Vec<Decided>>,
    stages: Vec<Stage>,
    failed: Option<String>,
    keep: bool,
    /// The last run's steps, each taken by the step at its place that runs the same pass.
    last: Vec<Option<Kept>>,
    kept: Vec<Kept>,
    /// A keyed pipeline's store and chain (0281 §6).
    keyed: Option<Keyed<'s>>,
    /// The request record by rule, carried through each step (`AskedByRule`); `None` once a step is read from a
    /// keyed store (its delta unknown: the record is left as the output states it).
    asked: Option<AskedByRule>,
    /// A step read from the store left the record to be read again from its output (`load`).
    asked_again: bool,
}

/// The request record (`Version::asked`, decision 0174's `asked` rows) by rule id, carried through a pipeline's
/// steps (decision 0331): a rule kept serves its readers; a rule a pass rewrote, its source's (`RuleRef::source`);
/// a rule equal to rules the step removed (a repeat dropped, a rule made again alike), theirs too; any other rule
/// the readers the step's own record states for it, else its own relation, which `prune` keeps wherever the relation
/// is needed. The record is by place among a relation's
/// rules, which a pass that adds, drops or reorders rules shifts: the pipeline states it again at its end.
///
/// A pass that renames readers states the record itself: one that merges or folds relations (`MergeIdentical`,
/// hiding's `FoldOwn`) has a request of a merged reader serve the relation it was merged into, and one that inlines
/// relations into their readers has a request of an inlined relation serve those readers. A copy of a request rule
/// under another name is no request rule, but reads what its source reads: where its source is a reader, so is it (a
/// guest's copy of a reader, read in its place, asks what the reader asked).
#[derive(Clone, Debug)]
struct AskedByRule {
    by: HashMap<RuleId, BTreeSet<String>>,
    reqs: BTreeSet<String>,
}

impl AskedByRule {
    fn of(v: &Version) -> Option<AskedByRule> {
        let asked = v.asked();
        if asked.is_empty() {
            return None;
        }
        // A record whose length is not its relation's rule count gates nothing (every reader of it checks: `prune`,
        // the embed slice, lowering): it is carried as no record.
        let (mut by, mut reqs) = (HashMap::new(), BTreeSet::new());
        for (req, readers) in &asked {
            let Some(rel) = v.rel(req) else { continue };
            let ids: Vec<RuleId> = v.heads(rel).map(|r| r.id()).collect();
            if ids.len() == readers.len() {
                by.extend(ids.into_iter().zip(readers.iter().cloned()));
                reqs.insert(req.clone());
            }
        }
        Some(AskedByRule { by, reqs })
    }
    fn step(&mut self, from: &Version, out: &Version, delta: &Delta) {
        let (was, stated) = (from.asked(), out.asked());
        // A request relation whose record the step dropped is no longer one (`prune`'s `revived`: its readers all
        // went, and it is an input again, its rules kept): the record does not state it again. A step may drop it
        // and change no rule.
        self.reqs.retain(|req| !was.contains_key(req) || stated.contains_key(req));
        if delta.is_empty() {
            return;
        }
        let mut next = HashMap::new();
        // A pass that states the record itself (the merge renames readers) is taken at its word where its record
        // holds an entry a rule.
        // A request the step states first (a later round's, `SourcesOnRequest`) is carried from here on.
        for (req, by) in &stated {
            if !self.reqs.contains(req) && out.rel(req).is_some_and(|rel| out.heads(rel).count() == by.len()) {
                self.reqs.insert(req.clone());
            }
        }
        for req in &self.reqs {
            let Some(rel) = out.rel(req) else { continue };
            if let Some(by) = stated.get(req).filter(|by| was.get(req) != Some(*by) && by.len() == out.heads(rel).count()) {
                next.extend(out.heads(rel).map(|r| r.id()).zip(by.iter().cloned()));
                continue;
            }
            // A new base whose rules of this relation are the input's, in order (a fold: its rules renumbered, each
            // keeping the `source` of the derivation it came from, not an id of this step's input): each place keeps
            // its readers.
            if delta.rebuilt {
                let old: Vec<(RuleId, u64)> = from.rel(req).map(|o| from.heads(o).map(|r| (r.id(), r.hash())).collect()).unwrap_or_default();
                let new: Vec<(RuleId, u64)> = out.heads(rel).map(|r| (r.id(), r.hash())).collect();
                if old.len() == new.len() && old.iter().zip(&new).all(|(a, b)| a.1 == b.1) {
                    for ((was, _), (now, _)) in old.iter().zip(&new) {
                        next.insert(*now, self.by.get(was).cloned().unwrap_or_else(|| BTreeSet::from([req.clone()])));
                    }
                    continue;
                }
            }
            // The input's rules of this relation the step removed (in a derivation, those no output rule came from),
            // by content: a rule equal to one (a repeat dropped) serves its readers too.
            let sourced: std::collections::HashSet<RuleId> = if delta.rebuilt { out.heads(rel).filter_map(|r| r.source()).collect() } else { Default::default() };
            let mut gone: HashMap<u64, Vec<BTreeSet<String>>> = HashMap::new();
            if let Some(old) = from.rel(req) {
                for r in from.heads(old).filter(|r| if delta.rebuilt { !sourced.contains(&r.id()) } else { delta.removed.has(r.id()) }) {
                    if let Some(b) = self.by.get(&r.id()) {
                        gone.entry(r.hash()).or_default().push(b.clone());
                    }
                }
            }
            // As many rules made alike without a source's readers as were removed: each takes one's readers, in order
            // (a rule made again, as the tree keeps each its entry); otherwise each takes them all (repeats dropped
            // into one).
            let mut alike: HashMap<u64, usize> = HashMap::new();
            for r in out.heads(rel) {
                let added = delta.rebuilt || delta.added.contains(&r.id());
                let at = if added { r.source() } else { Some(r.id()) };
                if at.and_then(|id| self.by.get(&id)).is_none_or(|b| b.is_empty()) {
                    *alike.entry(r.hash()).or_default() += 1;
                }
            }
            let mut seen: HashMap<u64, usize> = HashMap::new();
            // What the step states of this relation, an entry a rule: where the carry knows nothing of a rule (made
            // from a rule that served no reader, as a request from its reader), the step's own word for it.
            let said = stated.get(req).filter(|by| by.len() == out.heads(rel).count());
            for (k, r) in out.heads(rel).enumerate() {
                let mut readers = BTreeSet::new();
                // A rule the step kept keeps its readers; one it made, its source's (an id of this step's input: a
                // kept rule's own `source` names an earlier derivation's).
                let added = delta.rebuilt || delta.added.contains(&r.id());
                let at = if added { r.source() } else { Some(r.id()) };
                if let Some(b) = at.and_then(|id| self.by.get(&id)) {
                    readers.extend(b.iter().cloned());
                }
                // A rule made again without a source (or from one that served no reader) takes the readers of a
                // removed rule alike; one whose source serves readers is that source's, not its look-alike's.
                if let Some(b) = gone.get(&r.hash()).filter(|_| readers.is_empty()) {
                    let j = seen.entry(r.hash()).or_default();
                    if b.len() == alike[&r.hash()] {
                        readers.extend(b[*j].iter().cloned());
                    } else {
                        readers.extend(b.iter().flatten().cloned());
                    }
                    *j += 1;
                }
                if readers.is_empty() {
                    match said {
                        Some(by) => readers.extend(by[k].iter().cloned()),
                        None => {
                            readers.insert(req.clone());
                        }
                    }
                }
                next.insert(r.id(), readers);
            }
        }
        self.by = next;
    }
    /// The record after a step run again over an edit onto its last output `from` (decision 0393): a rule the step
    /// kept of `from` serves the readers the last run's record (`kept`, after the step) gave it; one it added, its
    /// source's in this record (an id of the step's input); any other its own relation. The edit added and removed
    /// no rule of a request (`spares_record`), so each request's rules are the last output's, as the last run's
    /// decisions. Never over a fold (`rule_step` runs it whole).
    fn onto(&self, kept: &AskedByRule, out: &Version, delta: &Delta) -> AskedByRule {
        let mut by = HashMap::new();
        for req in &self.reqs {
            let Some(rel) = out.rel(req) else { continue };
            for r in out.heads(rel) {
                let readers = if delta.added.contains(&r.id()) { r.source().and_then(|s| self.by.get(&s)) } else { kept.by.get(&r.id()) };
                by.insert(r.id(), readers.cloned().unwrap_or_else(|| BTreeSet::from([req.clone()])));
            }
        }
        AskedByRule { by, reqs: self.reqs.clone() }
    }
    /// The record of `of`'s rules for `to`'s, place by place: `to` the same program (a rebase of `of`, decision 0393),
    /// each request's rules in the same order. `None` where a request's rules differ.
    fn by_place(&self, of: &Version, to: &Version) -> Option<AskedByRule> {
        let mut by = HashMap::new();
        for req in &self.reqs {
            let (a, b): (Vec<RuleRef<'_>>, Vec<RuleRef<'_>>) = (of.rel(req).map(|r| of.heads(r).collect()).unwrap_or_default(), to.rel(req).map(|r| to.heads(r).collect()).unwrap_or_default());
            if a.len() != b.len() || a.iter().zip(&b).any(|(x, y)| x.hash() != y.hash()) {
                return None;
            }
            for (x, y) in a.iter().zip(&b) {
                if let Some(readers) = self.by.get(&x.id()) {
                    by.insert(y.id(), readers.clone());
                }
            }
        }
        Some(AskedByRule { by, reqs: self.reqs.clone() })
    }

    /// `v`'s record with the carried relations' entries by rule, where they differ from what `v` states (a step
    /// changed their rules and did not state it); `None` where they agree. A request relation the pipeline does not
    /// carry (a step made it) keeps its own.
    fn restated(&self, v: &Version) -> Option<super::about::Asked> {
        let stated = v.asked();
        let carried = self.asked(v);
        if self.reqs.iter().all(|req| carried.get(req) == stated.get(req)) {
            return None;
        }
        let mut out = stated;
        for req in &self.reqs {
            match carried.get(req) {
                Some(by) => {
                    out.insert(req.clone(), by.clone());
                }
                None => {
                    out.remove(req);
                }
            }
        }
        Some(out)
    }

    /// The record as `Version::asked` states it, for `v`'s rules.
    fn asked(&self, v: &Version) -> super::about::Asked {
        let mut asked = super::about::Asked::new();
        for req in &self.reqs {
            let Some(rel) = v.rel(req) else { continue };
            let by: Vec<BTreeSet<String>> = v.heads(rel).map(|r| self.by.get(&r.id()).cloned().unwrap_or_else(|| BTreeSet::from([req.clone()]))).collect();
            if !by.is_empty() {
                asked.insert(req.clone(), by);
            }
        }
        asked
    }
}

/// How `Pipeline::record` carries the request record over a step.
enum Carry {
    /// From the step's input to its output (`AskedByRule::step`).
    Step,
    /// Onto the last run's output of the step, from the last run's record after it (`AskedByRule::onto`).
    Onto(AskedByRule),
    /// As stated.
    Is(AskedByRule),
}

impl<'s> Pipeline<'s> {
    /// Whether a step may run again over the edit `d` of its last input onto its last output with the request record
    /// carried (decision 0393): no record, or the last run's after the step with the same requests, and the edit adds
    /// and removes no request's rule. Else the step runs whole, and the record is carried from its input.
    fn spares_record(&self, k: &Kept, d: &Delta) -> Option<Carry> {
        let Some(now) = &self.asked else { return Some(Carry::Step) };
        let kept = k.asked.as_ref().filter(|a| a.reqs == now.reqs)?;
        let req = |of: &Version, r: RuleRef<'_>| now.reqs.contains(of.rel_name(r.head().rel()));
        if d.rebuilt || d.removed.iter().any(|id| k.input.rule(id).is_none_or(|r| req(&k.input, r))) || d.added.iter().any(|&id| self.v.rule(id).is_none_or(|r| req(&self.v, r))) {
            return None;
        }
        Some(Carry::Onto(kept.clone()))
    }

    pub fn new(name: &'static str, v: Arc<Version>) -> Pipeline<'s> {
        let asked = AskedByRule::of(&v);
        Pipeline { name, input: v.clone(), v, decided: RefCell::new(Vec::new()), stages: Vec::new(), failed: None, keep: false, last: Vec::new(), kept: Vec::new(), keyed: None, asked, asked_again: false }
    }
    /// A pipeline whose last step states the request record itself (a part's fields, `passes::PartOf`): the record is
    /// not carried by rule, nor stated again after each step (Q180). Restated after a step, the record is a layer of
    /// its own that the last step's states again (a part made a view, the record's overlay and its fields' overlay).
    pub fn stating_record(name: &'static str, v: Arc<Version>) -> Pipeline<'s> {
        Pipeline { asked: None, ..Pipeline::new(name, v) }
    }
    /// A pipeline whose steps are keyed and kept in `steps` (0281 §6). A step's key is a chain: the input version's
    /// identity (`identity`), the pipeline's name and the engine build, then for each step so far its pass's `NAME`,
    /// `VERSION` and `settings`, and the key parts the caller added before it (`key_part`). A step's input is the step
    /// before's output, so the chain names it as the input's digest would, and every key is known before anything
    /// runs. A step whose key `steps` holds is not run: its decisions are the stored ones, and its output is opened
    /// from the store only when a later step runs, or at the end; a pipeline whose every step is stored opens one
    /// image. A step that runs stores its output under its key. A pass that is skipped so has not run: what it
    /// returns in itself, beside its output and decisions, is not there (`ran`).
    pub fn keyed(name: &'static str, v: Arc<Version>, steps: &'s dyn Steps) -> Pipeline<'s> {
        let id = identity(&v);
        Pipeline::keyed_by(name, v, id, steps)
    }
    /// `keyed`, with the input's `identity` given: a caller keying many pipelines over one input reads its rules once.
    pub fn keyed_by(name: &'static str, v: Arc<Version>, identity: [u8; 32], steps: &'s dyn Steps) -> Pipeline<'s> {
        let chain = chain_next(&identity, &[b"pipeline", name.as_bytes(), crate::code_rule_dsl::build_ids::engine().as_bytes()]);
        Pipeline { keyed: Some(Keyed { steps, chain, pending: None }), ..Pipeline::new(name, v) }
    }
    /// Adds a part to the key of each later step (a keyed pipeline's, 0281 §6): a value the passes read that is not in
    /// the input version (a caller's state, as 0298's general forms read the kept part's old rows). A step keyed with
    /// it is found again only by a pipeline that adds the same part at the same place.
    pub fn key_part(&mut self, label: &str, bytes: &[u8]) -> &mut Self {
        if let Some(k) = &mut self.keyed {
            k.chain = chain_next(&k.chain, &[b"part", label.as_bytes(), bytes]);
        }
        self
    }
    /// Whether the step at `at` (its place in `stages`) ran, rather than being read from a keyed store.
    pub fn ran(&self, at: usize) -> bool {
        self.stages.get(at).is_some_and(|s| s.ran != Ran::Stored)
    }
    /// The current version, opened from the store first when the last steps were stored ones (`keyed`).
    pub fn current(&mut self) -> Result<&Arc<Version>> {
        self.load()?;
        Ok(&self.v)
    }
    /// A pipeline whose run is kept (`done`), to run again over an edit of `v` (`again`).
    pub fn keeping(name: &'static str, v: Arc<Version>) -> Pipeline<'s> {
        Pipeline { keep: true, ..Pipeline::new(name, v) }
    }
    /// The same passes, run again over `v`, an edit of `last`'s input (0281 §3). Each step whose input is its last
    /// one's reuses its output. Otherwise, over the edit (`Delta::of`) from its last input: the analyses are carried
    /// to the new input across it (maintained, or computed again where it touches what they read); a rule pass that
    /// reads nothing else the edit touched (`RulePass::WHOLE`), and whose selection and analyses come out the same,
    /// visits the added rules alone and writes onto its last output; every other step runs whole. The caller passes
    /// the same passes, in the same order: a step is matched to the last run's by its place and its pass's name.
    pub fn again(last: Run, v: Arc<Version>) -> Pipeline<'s> {
        Pipeline { last: last.steps.into_iter().map(Some).collect(), ..Pipeline::keeping(last.name, v) }
    }
    /// The current version: in a keyed pipeline, the last one run or opened (`current` opens a stored one).
    pub fn version(&self) -> &Arc<Version> {
        &self.v
    }
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }
    pub fn decisions(&self) -> Vec<Decided> {
        self.decided.borrow().clone()
    }
    /// The current version's analyses.
    pub fn store(&self) -> &Store {
        self.v.analyses()
    }
    /// Analysis `A` of the current version.
    pub fn get<A: Analysis>(&self) -> Result<Arc<A>> {
        let decided = RefCell::new(Vec::new());
        Cx { v: &self.v, store: self.v.analyses(), pass: "pipeline", decided: &decided, asked: None }.get::<A>()
    }

    /// The last run's step at this one's place, if it ran `pass`, against the current input. Over an edit, the
    /// last input's analyses are carried to the current one first, so a pass reads them at the edit's cost.
    fn prior(&mut self, pass: &'static str) -> Result<Prior> {
        let Some(k) = self.last.get_mut(self.kept.len()).and_then(|k| k.take_if(|k| k.pass == pass)) else { return Ok(Prior::None) };
        if Arc::ptr_eq(&k.input, &self.v) {
            return Ok(Prior::Same(k));
        }
        // An input not made as an edit of the last one (a step after one that ran whole): a rebuilt delta, of the
        // whole program, is never read, so it is not made.
        if !Delta::read_off(&k.input, &self.v) {
            return Ok(Prior::None);
        }
        let d = Delta::of(&k.input, &self.v);
        if d.rebuilt {
            return Ok(Prior::None);
        }
        let cx = Cx { v: &self.v, store: self.v.analyses(), pass, decided: &self.decided, asked: None };
        self.v.analyses().carry(k.input.analyses(), &cx, &d)?;
        self.v.analyses().set_lineage(&k.input, &d);
        Ok(Prior::Edit(k, d))
    }

    /// Records a step that made `out` by an edit of `from` (the input, or the last output a rerun wrote onto): its
    /// stage, its analyses carried from `from`'s, and, when kept, its versions.
    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, pass: &'static str, out: Arc<Version>, ran: Ran, form: Form, visited: usize, from: &Arc<Version>, rule: Option<RuleKept>, carry: Carry) -> Result<()> {
        let mut delta = Delta::of(from, &out);
        let mut out = out;
        match (&mut self.asked, carry) {
            (Some(asked), Carry::Step) => {
                asked.step(from, &out, &delta);
                // A step that added, dropped, replaced or reordered a request relation's rules and did not state the
                // record left it by place over other rules (Q226: `inline filters` made three, and `request rules
                // another subsumes` read the record misaligned and kept it whole; a text pass replaced one, the count
                // the same). Stated again now, by rule, as the tree's passes state it, so the next pass reads it
                // aligned.
                if let Some(restated) = (!delta.is_empty()).then(|| asked.restated(&out)).flatten() {
                    let mut b = out.builder();
                    b.fields().set_asked(&restated);
                    out = b.finish_overlay()?;
                    delta = Delta::of(from, &out);
                }
            }
            (Some(asked), Carry::Onto(kept)) => *asked = asked.onto(&kept, &out, &delta),
            (Some(asked), Carry::Is(now)) => *asked = now,
            // A pipeline that began with no record carries the one its first step states (the lowering's tail: the
            // requests made on request, `SourcesOnRequest`), unless a stored step left it to be read on `load`.
            (None, _) if !self.asked_again && from.asked().is_empty() => self.asked = AskedByRule::of(&out),
            (None, _) => {}
        }
        let cx = Cx { v: &out, store: out.analyses(), pass, decided: &self.decided, asked: None };
        out.analyses().carry(from.analyses(), &cx, &delta)?;
        out.analyses().set_lineage(from, &delta);
        self.stages.push(Stage { pass, ran, form, visited, input: (self.v.size(), self.v.digest()), output: (out.size(), out.digest()), delta: delta.size(), touched: delta.touched });
        if self.keep {
            self.kept.push(Kept { pass: pass.into(), input: self.v.clone(), output: out.clone(), form, rule, plan: Vec::new(), said: Vec::new(), asked: self.asked.clone() });
        }
        self.v = out;
        Ok(())
    }

    fn fail(&mut self, pass: &'static str, r: Result<()>) -> &mut Self {
        if let Err(e) = r {
            self.failed = Some(format!("{}: {pass}: {e}", self.name));
        }
        self
    }

    /// Runs rule pass `pass` over the rules it selects: in a derivation when it selects all (the program's order
    /// kept), else as an edit of the version; over the edit alone when it runs again (`again`).
    pub fn rules<P: RulePass>(&mut self, pass: &mut P) -> &mut Self {
        if self.failed.is_some() {
            return self;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("pass {}: {}", self.name, P::NAME));
        let r = match self.stored(P::NAME, P::VERSION, &pass.settings()) {
            Ok(Some(key)) => self.load().and_then(|()| self.keep_step(key, P::NAME, P::VERSION, |p| p.rule_step(pass))),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        self.fail(P::NAME, r)
    }

    fn rule_step<P: RulePass>(&mut self, pass: &mut P) -> Result<()> {
        let local = P::WHOLE != Reads::ALL;
        let prior = self.prior(P::NAME)?;
        let prior = match prior {
            Prior::Same(k) if local && k.rule.is_some() => {
                let out = k.output.clone();
                // The last run's step on this input: its record after the step is the record (decision 0393).
                let carry = match (&self.asked, k.asked) {
                    (None, _) => Carry::Step,
                    (Some(now), Some(kept)) if kept.reqs == now.reqs => Carry::Is(kept),
                    (Some(_), _) => return self.rule_whole(pass),
                };
                return self.record(P::NAME, out.clone(), Ran::Reused, k.form, 0, &out, k.rule, carry);
            }
            p => p,
        };
        let v = self.v.clone();
        let asked = RefCell::new(Vec::new());
        let spared = match &prior {
            Prior::Edit(k, d) => self.spares_record(k, d),
            _ => None,
        };
        let made = (|| -> Result<Option<(Arc<Version>, Ran, Form, usize, Arc<Version>, Option<RuleKept>, Carry)>> {
            let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: self.keep.then_some(&asked) };
            let select = pass.begin(&cx)?;
            // What `begin` asked for, apart from the list the comparisons below add to.
            let asked_now: Vec<Asked> = asked.borrow().clone();
            if let Prior::Edit(k, d) = &prior
                && let Some(rk) = &k.rule
                && local
                && !P::WHOLE.meets(d.touched)
                && rk.select == select
                && same_analyses(&rk.asked, &k.input, &cx)?
                && same_named(&rk.named, &asked_now, &k.input, &cx)?
                && let Some(carry) = spared
            {
                let (out, form, visited, base, own_end) = rerun(pass, &cx, k, d, &select, rk.own_end)?;
                // Onto the new input itself (the last run changed nothing), the record is carried from it; onto a
                // fold, whose rules are numbered anew, the step runs whole (decision 0393).
                let carry = if Arc::ptr_eq(&base, &v) { Carry::Step } else { carry };
                if form == Form::Folded && self.asked.is_some() && !matches!(carry, Carry::Step) {
                    return Ok(None);
                }
                let mut all = rk.asked.clone();
                for a in asked.borrow().iter() {
                    if !all.iter().any(|b| b.t == a.t) {
                        all.push(*a);
                    }
                }
                let keep = (form != Form::Folded || own_end == 0).then_some(RuleKept { select, asked: all, named: Vec::new(), own_end });
                return Ok(Some((out, Ran::Delta, form, visited, base, keep, carry)));
            }
            let mut w = ProgramW::new(&v);
            if matches!(select, Select::All) {
                w.derive()?;
            }
            w.apply(&cx, pass, &select)?;
            let (out, form, visited) = w.finish()?;
            let own_end = if form == Form::Derived { 0 } else { v.rule_end() };
            let keep = (form != Form::Folded).then(|| RuleKept { select, asked: asked.take(), named: Vec::new(), own_end });
            Ok(Some((out, Ran::Whole, form, visited, v.clone(), keep, Carry::Step)))
        })();
        let Some((out, ran, form, visited, from, keep, carry)) = made? else { return self.rule_whole(pass) };
        self.record(P::NAME, out, ran, form, visited, &from, keep, carry)
    }

    /// A rule pass run whole on the current version, its record carried from it.
    fn rule_whole<P: RulePass>(&mut self, pass: &mut P) -> Result<()> {
        let v = self.v.clone();
        let asked = RefCell::new(Vec::new());
        let (out, form, visited, keep) = {
            let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: self.keep.then_some(&asked) };
            let select = pass.begin(&cx)?;
            let mut w = ProgramW::new(&v);
            if matches!(select, Select::All) {
                w.derive()?;
            }
            w.apply(&cx, pass, &select)?;
            let (out, form, visited) = w.finish()?;
            let own_end = if form == Form::Derived { 0 } else { v.rule_end() };
            (out, form, visited, (form != Form::Folded).then(|| RuleKept { select, asked: asked.take(), named: Vec::new(), own_end }))
        };
        self.record(P::NAME, out, Ran::Whole, form, visited, &v, keep, Carry::Step)
    }

    /// Runs program pass `pass`, over the whole input (its analyses carried from the last input when run again).
    pub fn program<P: ProgramPass>(&mut self, pass: &mut P) -> &mut Self {
        if self.failed.is_some() {
            return self;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("pass {}: {}", self.name, P::NAME));
        let r = match self.stored(P::NAME, P::VERSION, &pass.settings()) {
            Ok(Some(key)) => self.load().and_then(|()| self.keep_step(key, P::NAME, P::VERSION, |p| p.program_step(pass))),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        self.fail(P::NAME, r)
    }

    fn program_step<P: ProgramPass>(&mut self, pass: &mut P) -> Result<()> {
        let prior = self.prior(P::NAME)?;
        let v = self.v.clone();
        // Over an edit of the last run's input: the pass writes the edit onto its last output where it can. Only an
        // edit of rules alone: the last output holds the last input's declarations, facts, inputs and outputs, which
        // a pass writing onto it does not state again (decision 0385).
        if let Prior::Edit(k, d) = &prior
            && d.touched == Reads::RULES
            && let Some(carry) = self.spares_record(k, d)
        {
            let made = {
                let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: None };
                let mut w = ProgramW::new(&k.output);
                match pass.again(&cx, &Last { input: &k.input, output: &k.output, delta: d, plan: &k.plan }, &mut w)? {
                    true => Some(w.finish()?),
                    false => None,
                }
            };
            // Onto a fold, whose rules are numbered anew, a carried record is not carried on: the step runs whole.
            if let Some((out, form, visited)) = made
                && !(form == Form::Folded && matches!(carry, Carry::Onto(_)))
            {
                // What it said when it ran whole stands with its decisions (decision 0385).
                crate::code_rule_dsl::logic::profile::restate(&k.said);
                self.record(P::NAME, out, Ran::Delta, form, visited, &k.output, None, carry)?;
                self.planned(pass.plan(), k.said.clone());
                return Ok(());
            }
        }
        let deciding = crate::code_rule_dsl::logic::profile::Deciding::open();
        let made = {
            let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: None };
            let mut w = ProgramW::new(&v);
            pass.run(&cx, &mut w).and_then(|()| w.finish())
        };
        let said = deciding.close();
        let (out, form, visited) = made?;
        // Over an edit, its output written as an edit of its last output (`rebase`, decision 0393): the steps after
        // it are run again over an edit too, not whole.
        if let Prior::Edit(k, _) = &prior
            && !Delta::read_off(&k.output, &out)
            && let Some(r) = rebase_output(&k.output, &out)?
        {
            // The record carried to the whole output, then place by place to its rebase (decision 0393); where a
            // request's rules are not in the same places, the whole output is kept.
            let carry = match &self.asked {
                None => Some(Carry::Step),
                Some(a) => {
                    let mut a = a.clone();
                    a.step(&v, &out, &Delta::of(&v, &out));
                    a.by_place(&out, &r).map(Carry::Is)
                }
            };
            if let Some(carry) = carry {
                self.record(P::NAME, r, Ran::Whole, Form::Overlay, visited, &k.output, None, carry)?;
                self.planned(pass.plan(), said);
                return Ok(());
            }
        }
        self.record(P::NAME, out, Ran::Whole, form, visited, &v, None, Carry::Step)?;
        self.planned(pass.plan(), said);
        Ok(())
    }

    /// The plan of the step just recorded, and the profile's decisions it made, kept with it.
    fn planned(&mut self, plan: Vec<Row>, said: Vec<crate::code_rule_dsl::logic::profile::Decided>) {
        if self.keep
            && let Some(k) = self.kept.last_mut()
        {
            k.plan = plan;
            k.said = said;
        }
    }

    /// A step whose output `make` writes from the input by a writer of its own, not through `ProgramW` (a stratum's
    /// general form, decision 0298): keyed and stored as any step, its decisions those `make` returns. Its output is
    /// not an edit of its input, so no analysis is carried to it and no delta is compared.
    pub fn made(&mut self, pass: &'static str, version: u32, settings: &[u8], make: impl FnOnce(&Arc<Version>) -> Result<(Arc<Version>, Vec<Decided>)>) -> &mut Self {
        if self.failed.is_some() {
            return self;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("pass {}: {pass}", self.name));
        let r = match self.stored(pass, version, settings) {
            Ok(Some(key)) => self.load().and_then(|()| {
                self.keep_step(key, pass, version, |p| {
                    let (out, decided) = make(&p.v)?;
                    p.decided.borrow_mut().extend(decided);
                    p.stages.push(Stage { pass, ran: Ran::Whole, form: Form::Derived, visited: 0, input: (p.v.size(), p.v.digest()), output: (out.size(), out.digest()), delta: out.size(), touched: Reads::ALL });
                    // A made version is built whole, its rules numbered anew, from a program that states its request
                    // record (`Program::asked_by`): the record is carried from it, by its rules' ids, not from the
                    // input's ids, which a later step would no longer find (each rule then read as serving only its
                    // own relation: the deferred joins' step, decision 0500).
                    if p.asked.is_some() {
                        p.asked = AskedByRule::of(&out);
                    }
                    p.v = out;
                    Ok(())
                })
            }),
            Ok(None) => Ok(()),
            Err(e) => Err(e),
        };
        self.fail(pass, r)
    }

    /// A keyed pipeline's next step (0281 §6): its key, chained; `Ok(None)` when the store holds it (the step is
    /// recorded as stored, its decisions read back, its output opened later by `load`), else `Ok(Some(key))` (always
    /// `Some` for a pipeline not keyed, whose key is never read).
    fn stored(&mut self, pass: &'static str, version: u32, settings: &[u8]) -> Result<Option<StepKey>> {
        let Some(k) = &mut self.keyed else { return Ok(Some([0; 32])) };
        k.chain = chain_next(&k.chain, &[b"step", pass.as_bytes(), &version.to_le_bytes(), settings]);
        let key = k.chain;
        let Some(found) = k.steps.find(&key)? else { return Ok(Some(key)) };
        if found.pass != pass || found.version != version {
            return Ok(Some(key));
        }
        // Its delta unknown, the record is read again from its output once it is opened (`load`): each step's output
        // states its record (`record`).
        self.asked = None;
        self.asked_again = true;
        let input = self.stages.last().map_or((self.v.size(), self.v.digest()), |s| s.output);
        self.stages.push(Stage { pass, ran: Ran::Stored, form: Form::Unchanged, visited: 0, input, output: found.output, delta: 0, touched: Reads::default() });
        self.decided.borrow_mut().extend(found.decided.iter().cloned());
        k.pending = Some(found);
        Ok(None)
    }

    /// Opens the output of the last stored step, when the pipeline has not opened it yet (`keyed`).
    fn load(&mut self) -> Result<()> {
        let Some(found) = self.keyed.as_mut().and_then(|k| k.pending.take()) else { return Ok(()) };
        let steps = self.keyed.as_ref().expect("keyed").steps;
        let v = steps.open(&found.image)?.ok_or_else(|| format!("{}: {}: its stored output is gone from the store", self.name, found.pass))?;
        if (v.size(), v.digest()) != found.output {
            return Err(format!("{}: {}: its stored output is not the program its step recorded", self.name, found.pass));
        }
        self.v = v;
        if std::mem::take(&mut self.asked_again) {
            self.asked = AskedByRule::of(&self.v);
        }
        Ok(())
    }

    /// Runs `step`, and stores its output and decisions under `key` when the pipeline is keyed.
    fn keep_step(&mut self, key: StepKey, pass: &'static str, version: u32, step: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let from = self.decided.borrow().len();
        step(self)?;
        if self.keyed.is_none() {
            return Ok(());
        }
        // A step stored holds its record (as the pipeline's end states it, `state_asked`): a run that reads the step
        // from the store goes on from the record a run that made it had (`load` reads it again).
        self.state_asked()?;
        let Some(k) = &self.keyed else { return Ok(()) };
        let decided = self.decided.borrow()[from..].to_vec();
        k.steps.keep(&key, &StepRecord { pass: pass.to_owned(), version, image: [0; 32], output: (self.v.size(), self.v.digest()), decided }, &self.v)
    }

    /// Runs `f`'s passes where `on`.
    pub fn run_if(&mut self, on: bool, f: impl FnOnce(&mut Self) -> &mut Self) -> &mut Self {
        if on { f(self) } else { self }
    }
    /// Runs `f`'s passes until a round changes nothing (the digest is the round's start's), at most `rounds` times.
    pub fn until_unchanged(&mut self, rounds: usize, mut f: impl FnMut(&mut Self)) -> &mut Self {
        for _ in 0..rounds {
            if let Err(e) = self.load() {
                self.failed.get_or_insert(e);
                return self;
            }
            let before = self.v.digest();
            f(self);
            if let Err(e) = self.load() {
                self.failed.get_or_insert(e);
            }
            if self.failed.is_some() || self.v.digest() == before {
                return self;
            }
        }
        self.failed.get_or_insert_with(|| format!("{}: still changing after {rounds} rounds", self.name));
        self
    }
    /// States the carried request record in the current version (`AskedByRule`), where the steps shifted it.
    fn state_asked(&mut self) -> Result<()> {
        let Some(carried) = &self.asked else { return Ok(()) };
        let asked = carried.asked(&self.v);
        if asked == self.v.asked() {
            return Ok(());
        }
        let v = self.v.clone();
        let mut b = v.builder();
        b.fields().set_asked(&asked);
        let stated = b.finish_overlay()?;
        let cx = Cx { v: &stated, store: stated.analyses(), pass: "the request record stated", decided: &self.decided, asked: None };
        stated.analyses().carry(v.analyses(), &cx, &Delta::of(&v, &stated))?;
        self.v = stated;
        Ok(())
    }
    /// The last version, or the first step's error.
    pub fn finish(mut self) -> Result<(Arc<Version>, Vec<Stage>)> {
        if self.failed.is_none()
            && let Err(e) = self.load().and_then(|()| self.state_asked())
        {
            self.failed = Some(e);
        }
        match self.failed {
            Some(e) => Err(e),
            None => Ok((self.v, self.stages)),
        }
    }
    /// The run, kept to run again (`again`), or the first step's error.
    pub fn done(mut self) -> Result<Run> {
        if self.failed.is_none()
            && let Err(e) = self.load().and_then(|()| self.state_asked())
        {
            self.failed = Some(e);
        }
        if let Some(e) = self.failed {
            return Err(e);
        }
        Ok(Run { name: self.name, input: self.input, output: self.v, stages: self.stages, steps: self.kept, decided: self.decided.into_inner() })
    }
}

// ---- stored runs (decision 0333) ----

/// A version of a stored run, over the versions before it in the run's table (decision 0333 §2).
pub enum StoredVersion<'r> {
    /// A base: its image, as the version holds it (a record's piece takes it so, uncopied: decision 0464).
    Base(&'r Arc<super::Image>),
    /// An overlay of version `parent`: its delta's image.
    Overlay { parent: usize, delta: &'r Arc<super::Image> },
    /// A view of version `parent`: the parent's rules it leaves out, ascending.
    View { parent: usize, removed: Vector<u32> },
}

/// What a store keeps of a run's step: its pass, its input and output (in the run's table), its writer's form, and
/// for a rule pass what a rerun compares (decision 0333 §2, §3).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredStep {
    pub pass: String,
    pub input: usize,
    pub output: usize,
    pub form: Form,
    pub rule: Option<StoredRule>,
    /// A program pass's plan (`ProgramPass::plan`).
    pub plan: Vec<Row>,
    /// What it said (`Kept::said`).
    pub said: Vec<crate::code_rule_dsl::logic::profile::Decided>,
    /// The request record after it (`Kept::asked`): the requests, and each request rule's readers by its id in the
    /// step's output. `None` where the run carried none.
    pub asked: Option<StoredAsked>,
}

/// A step's request record, as a stored run keeps it (`StoredStep::asked`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StoredAsked {
    pub reqs: Vec<String>,
    pub by: Vec<(u32, Vec<String>)>,
}

/// A rule pass's selection (its kind, 0–3 for `All`, `Heads`, `Readers` and `Rules`, and its ids), the names of the
/// analyses it asked for, and its `own_end`.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredRule {
    pub select: (u8, Vector<u32>),
    pub asked: Vec<String>,
    pub own_end: u32,
}

/// A run as a store keeps it: its versions, each over those before it, the run's input and output among them, its
/// steps, its decisions, and the analyses of each step's input that can be stored (by version: name, version, rows).
pub struct RunParts<'r> {
    pub versions: Vec<StoredVersion<'r>>,
    pub input: usize,
    pub output: usize,
    pub steps: Vec<StoredStep>,
    pub decided: &'r [Decided],
    pub analyses: Vec<(usize, &'static str, u32, Vec<Row>)>,
    /// By version: the sha256 of its image as a store holds it, where it was opened from one (`Store::kept_image`).
    pub kept: Vec<Option<[u8; 32]>>,
}

impl Form {
    pub fn code(self) -> u8 {
        match self {
            Form::Unchanged => 0,
            Form::View => 1,
            Form::Overlay => 2,
            Form::Folded => 3,
            Form::Derived => 4,
        }
    }
    pub fn of_code(code: u8) -> Option<Form> {
        Some(match code {
            0 => Form::Unchanged,
            1 => Form::View,
            2 => Form::Overlay,
            3 => Form::Folded,
            4 => Form::Derived,
            _ => return None,
        })
    }
}

impl Select {
    fn stored(&self) -> (u8, Vector<u32>) {
        match self {
            Select::All => (0, rerail_blocks::vector::Vector::new()),
            Select::Heads(s) => (1, s.iter().map(|r| r.0).collect()),
            Select::Readers(s) => (2, s.iter().map(|r| r.0).collect()),
            Select::Rules(s) => (3, s.iter().map(|r| r.0).collect()),
        }
    }
    fn of_stored((kind, ids): &(u8, Vector<u32>)) -> Option<Select> {
        Some(match kind {
            0 => Select::All,
            1 => Select::Heads(ids.iter().map(|&k| RelId(k)).collect()),
            2 => Select::Readers(ids.iter().map(|&k| RelId(k)).collect()),
            3 => Select::Rules(ids.iter().map(|&k| RuleId(k)).collect()),
            _ => return None,
        })
    }
}

impl Run {
    /// The run as a store keeps it (decision 0333 §2): every version it holds, from its bases up, once each.
    pub fn parts(&self) -> RunParts<'_> {
        fn add<'r>(v: &'r Arc<Version>, table: &mut Vec<&'r Arc<Version>>, out: &mut Vec<StoredVersion<'r>>) -> usize {
            if let Some(i) = table.iter().position(|t| Arc::ptr_eq(t, v)) {
                return i;
            }
            let stored = match &**v {
                Version::Base { image, .. } => StoredVersion::Base(image),
                Version::Overlay { parent, delta, .. } => StoredVersion::Overlay { parent: add(parent, table, out), delta },
                Version::View { parent, .. } => {
                    let removed = parent.rules().map(|r| r.id()).filter(|&id| !v.contains(id)).map(|id| id.0).collect();
                    StoredVersion::View { parent: add(parent, table, out), removed }
                }
            };
            table.push(v);
            out.push(stored);
            table.len() - 1
        }
        let (mut table, mut versions) = (Vec::new(), Vec::new());
        let input = add(&self.input, &mut table, &mut versions);
        let mut steps = Vec::new();
        let mut inputs = vec![input];
        for k in &self.steps {
            let i = add(&k.input, &mut table, &mut versions);
            let o = add(&k.output, &mut table, &mut versions);
            inputs.push(i);
            let rule = k.rule.as_ref().map(|rk| StoredRule {
                select: rk.select.stored(),
                asked: rk.asked.iter().map(|a| a.name.to_owned()).chain(rk.named.iter().cloned()).collect(),
                own_end: rk.own_end,
            });
            let asked = k.asked.as_ref().map(|a| {
                let mut by: Vec<(u32, Vec<String>)> = a.by.iter().map(|(id, rs)| (id.0, rs.iter().cloned().collect())).collect();
                by.sort();
                StoredAsked { reqs: a.reqs.iter().cloned().collect(), by }
            });
            steps.push(StoredStep { pass: k.pass.to_string(), input: i, output: o, form: k.form, rule, plan: k.plan.clone(), said: k.said.clone(), asked });
        }
        let output = add(&self.output, &mut table, &mut versions);
        inputs.sort_unstable();
        inputs.dedup();
        let analyses = inputs.iter().flat_map(|&i| table[i].analyses().rows(table[i]).into_iter().map(move |(name, version, rows)| (i, name, version, rows))).collect();
        let kept = table.iter().map(|v| v.analyses().kept_image()).collect();
        RunParts { versions, input, output, steps, decided: &self.decided, analyses, kept }
    }

    /// A run read back from a store (decision 0333 §2): `versions` its table reopened, in order, with the other
    /// parts `parts` gives. Its stages are not kept: a pipeline run `again` from it makes its own.
    pub fn assemble(name: &'static str, versions: &[Arc<Version>], input: usize, output: usize, steps: Vec<StoredStep>, decided: Vec<Decided>) -> Result<Run> {
        let at = |i: usize| versions.get(i).cloned().ok_or_else(|| format!("a stored run names version {i} of {}", versions.len()));
        let mut kept = Vec::new();
        for s in steps {
            let rule = match s.rule {
                Some(r) => Some(RuleKept { select: Select::of_stored(&r.select).ok_or("a stored step's selection is not one")?, asked: Vec::new(), named: r.asked, own_end: r.own_end }),
                None => None,
            };
            let asked = s.asked.map(|a| AskedByRule { by: a.by.into_iter().map(|(id, rs)| (RuleId(id), rs.into_iter().collect())).collect(), reqs: a.reqs.into_iter().collect() });
            kept.push(Kept { pass: s.pass.into(), input: at(s.input)?, output: at(s.output)?, form: s.form, rule, plan: s.plan, said: s.said, asked });
        }
        Ok(Run { name, input: at(input)?, output: at(output)?, stages: Vec::new(), steps: kept, decided })
    }
}

/// Where a pipeline's last run is kept (decision 0333 §2): by its key (the pipeline's name and what found it).
pub trait Runs {
    fn keep(&self, key: &[u8; 32], run: &Run) -> Result<()>;
    fn open(&self, key: &[u8; 32], name: &'static str) -> Result<Option<Run>>;
}

// ---- keyed steps (0281 §6) ----

/// A step's key: the chain `Pipeline::keyed` describes.
pub type StepKey = [u8; 32];

/// What a store keeps of a step (0281 §6): its pass and version, its output's image (by the digest of its bytes) with
/// the output's size and digest, and its decisions (0269).
#[derive(Clone, Debug, PartialEq)]
pub struct StepRecord {
    pub pass: String,
    pub version: u32,
    /// The sha256 of the output's image bytes; `keep` is handed zeros and stores the image it is given.
    pub image: [u8; 32],
    pub output: (usize, u64),
    pub decided: Vec<Decided>,
}

/// Where a keyed pipeline keeps its steps (0281 §6): facts of the store, by key, beside the program images they name
/// (`program_text::StoredSteps`). No cache store: 0103's two rules, the key and the step it names.
pub trait Steps {
    /// The step stored under `key`, when the store holds it and its output's image.
    fn find(&self, key: &StepKey) -> Result<Option<StepRecord>>;
    /// The version whose image's bytes have sha256 `image`.
    fn open(&self, image: &[u8; 32]) -> Result<Option<Arc<Version>>>;
    /// Stores `record` under `key`, and `output`'s image (folded to a base when it is a view or an overlay).
    fn keep(&self, key: &StepKey, record: &StepRecord, output: &Version) -> Result<()>;
}

struct Keyed<'s> {
    steps: &'s dyn Steps,
    chain: StepKey,
    /// The last step read from the store, its output not opened yet.
    pending: Option<StepRecord>,
}

/// How many overlays deep a last input may be and still be rebased onto (`rebase`).
pub const REBASE_DEPTH: usize = 8;

/// `new` written as an edit of `last` (decision 0333 §1), so `Delta::of(last, _)` reads the edit and a pipeline run
/// `again` from `last` reruns on it: the rules both hold (by hash, as many times as each holds them) kept as `last`'s,
/// `last`'s others removed, `new`'s others copied in, and the header set where it differs. `None` when the edit would
/// remove or add more than half of `last`'s rules: that is a new program, run from scratch.
pub fn rebase(last: &Arc<Version>, new: &Arc<Version>) -> Result<Option<Arc<Version>>> {
    if Arc::ptr_eq(last, new) {
        return Ok(Some(last.clone()));
    }
    // A run is stored with every version down to its bases (decision 0333 §2): past a few edits in a row, run from
    // scratch, which makes new bases, so a stored run stays a few layers deep.
    if last.depth() >= REBASE_DEPTH {
        return Ok(None);
    }
    let mut wanted: HashMap<u64, usize> = HashMap::new();
    for r in new.rules() {
        *wanted.entry(r.hash()).or_default() += 1;
    }
    let mut removed = Vec::new();
    for r in last.rules() {
        match wanted.get_mut(&r.hash()) {
            Some(n) if *n > 0 => *n -= 1,
            _ => removed.push(r.id()),
        }
    }
    let mut added = Vec::new();
    for r in new.rules() {
        if let Some(n) = wanted.get_mut(&r.hash())
            && *n > 0
        {
            *n -= 1;
            added.push(r);
        }
    }
    if 2 * (removed.len() + added.len()) > last.size().max(1) {
        return Ok(None);
    }
    // The rebased rules' order: `last`'s kept, then `new`'s others, each by its head and hash.
    let gone: HashSet<_> = removed.iter().copied().collect();
    let order: Vec<(&str, u64)> = last.rules().filter(|r| !gone.contains(&r.id())).map(|r| (last.rel_name(r.head().rel()), r.hash())).chain(added.iter().map(|r| (new.rel_name(r.head().rel()), r.hash()))).collect();
    let asked = rebased_record(new, &order);
    let mut b = Builder::on(last);
    for id in removed {
        b.remove(id);
    }
    for r in added {
        b.copy(new, r);
    }
    b.fields().state_fields_as(last, new);
    if let Some(asked) = asked {
        b.fields().set_asked(&asked);
    }
    b.finish_overlay().map(Some)
}

/// `rebase` for a pass's output (decision 0393): `new`, the pass's output over its input, written as an edit of
/// `last`, its last output. A rule is kept as `last`'s only where both hold it with the same source (the rule of the
/// pass's input it came from, `RuleRef::source`), and copied in with `new`'s source otherwise, so a later run's
/// `again` finds what came from each removed rule (`Version::derived`).
pub fn rebase_output(last: &Arc<Version>, new: &Arc<Version>) -> Result<Option<Arc<Version>>> {
    if last.depth() >= REBASE_DEPTH {
        return Ok(None);
    }
    let key = |r: RuleRef<'_>| (r.hash(), r.source());
    let mut wanted: HashMap<(u64, Option<RuleId>), usize> = HashMap::new();
    for r in new.rules() {
        *wanted.entry(key(r)).or_default() += 1;
    }
    let mut removed = Vec::new();
    for r in last.rules() {
        match wanted.get_mut(&key(r)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => removed.push(r.id()),
        }
    }
    let mut added = Vec::new();
    for r in new.rules() {
        if let Some(n) = wanted.get_mut(&key(r))
            && *n > 0
        {
            *n -= 1;
            added.push(r);
        }
    }
    if 2 * (removed.len() + added.len()) > last.size().max(1) {
        return Ok(None);
    }
    let mut b = Builder::on(last);
    for id in removed {
        b.remove(id);
    }
    for r in added {
        b.copy_as_is(new, r);
    }
    b.fields().state_fields_as(last, new);
    b.finish_overlay().map(Some)
}

/// `new`'s request record restated over the rebased rules (`rebase`), `order` their heads and hashes: the record
/// names each request rule's readers by its place among its relation's rules (decision 0331), and the rebased version
/// holds `last`'s rules in `last`'s order and `new`'s others after them. Taken by place from `new`, the readers of a
/// request relation whose rules lie in another order sat on other rules (`rr_value_req`'s `__all__` rule serving
/// `plain_str`), which `same_program` refused; a run found its last run only when the store had written it. `None`
/// where `new` has no record.
fn rebased_record(new: &Version, order: &[(&str, u64)]) -> Option<super::about::Asked> {
    let asked = new.asked();
    if asked.is_empty() {
        return None;
    }
    // Each request rule's readers by its head and hash, in `new`'s order (a rule held twice, its places in turn).
    let mut by_rule: HashMap<(&str, u64), std::collections::VecDeque<&BTreeSet<String>>> = HashMap::new();
    for (req, by) in &asked {
        for (r, readers) in new.rel(req).into_iter().flat_map(|rel| new.heads(rel)).zip(by) {
            by_rule.entry((req.as_str(), r.hash())).or_default().push_back(readers);
        }
    }
    let mut out = super::about::Asked::new();
    for &(head, hash) in order {
        if asked.contains_key(head) {
            let readers = by_rule.get_mut(&(head, hash)).and_then(|q| q.pop_front()).cloned().unwrap_or_default();
            out.entry(head.to_owned()).or_default().push(readers);
        }
    }
    Some(out)
}

/// A version's identity for a key: its digest (a sum, blind to the rules' order), its size, and its rules' hashes in
/// order, so a reordering (`RuleOrder`) is a different input.
pub fn identity(v: &Version) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.version-identity/v1\0");
    h.update(v.digest().to_le_bytes());
    h.update((v.size() as u64).to_le_bytes());
    for r in v.rules() {
        h.update(r.hash().to_le_bytes());
    }
    h.finalize().into()
}

/// The chain `prev` extended by `parts`, each length-prefixed.
fn chain_next(prev: &StepKey, parts: &[&[u8]]) -> StepKey {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.step/v1\0");
    h.update(prev);
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().into()
}

/// Whether each analysis a stored run's step named (`named`) is one the pass asked for again (`asked`), and the same
/// on `cx`'s version as on `before` (decision 0333 §3).
fn same_named(named: &[String], asked: &[Asked], before: &Arc<Version>, cx: &Cx<'_>) -> Result<bool> {
    let mut of = Vec::new();
    for n in named {
        let Some(a) = asked.iter().find(|a| a.name == n.as_str()) else { return Ok(false) };
        of.push(*a);
    }
    same_analyses(&of, before, cx)
}

/// Whether each analysis in `asked` is the same on `cx`'s version as on `before`.
fn same_analyses(asked: &[Asked], before: &Arc<Version>, cx: &Cx<'_>) -> Result<bool> {
    let decided = RefCell::new(Vec::new());
    let was = Cx { v: before, store: before.analyses(), pass: cx.pass, decided: &decided, asked: None };
    for a in asked {
        if !(a.same)(&(a.get)(&was)?, &(a.get)(cx)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Rule pass `pass` run again over `d`, the edit of its last input `k.input` that made `cx`'s version, onto its last
/// output: what came from the removed rules dropped (through `from`, or as themselves below `own_end`), and the added
/// rules visited (the selected) or kept (the rest). When the last run changed nothing, onto the new input itself.
/// Returns the output, its form, the rules visited, the version it is an edit of, and its `own_end`.
fn rerun<P: RulePass>(pass: &mut P, cx: &Cx<'_>, k: &Kept, d: &Delta, select: &Select, own_end: u32) -> Result<(Arc<Version>, Form, usize, Arc<Version>, u32)> {
    let v = cx.v;
    let on_input = Arc::ptr_eq(&k.output, &k.input);
    let base = if on_input { v.clone() } else { k.output.clone() };
    let mut visited = 0;
    let made = {
        let mut b = Builder::on(&base);
        if !on_input {
            for id in d.removed.iter() {
                let came: Vec<RuleId> = base.derived(id).map(|r| r.id()).collect();
                for c in came {
                    b.remove(c);
                }
                if id.0 < own_end && base.contains(id) {
                    b.remove(id);
                }
            }
        }
        for &id in &d.added {
            let r = v.rule(id).ok_or_else(|| format!("rule {} of the edit is not its output's", id.0))?;
            let step = if select.picks(r) {
                visited += 1;
                visit(pass, cx, v, r, &mut b)?
            } else {
                Step::Keep
            };
            match (on_input, step) {
                (true, Step::Keep) | (false, Step::Drop | Step::Written) => {}
                (true, _) => b.remove(id),
                (false, Step::Keep) => {
                    b.keep_into(v, r);
                }
            }
        }
        if b.is_empty() { None } else { Some(b.finish()?) }
    };
    let (out, form) = match made {
        None => (base.clone(), Form::Unchanged),
        Some((o, Made::View)) => (o, Form::View),
        Some((o, Made::Overlay)) => (o, Form::Overlay),
        Some((o, Made::Folded)) => (o, Form::Folded),
    };
    Ok((out, form, visited, base, if on_input { v.rule_end() } else { own_end }))
}
