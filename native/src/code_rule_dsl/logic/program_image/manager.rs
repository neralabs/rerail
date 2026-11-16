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
//! The manager writes through `Builder` (0173 §6, 0276 §3): a pass never opens one, never copies the rules it
//! keeps, and never picks a view, an overlay or a derivation.
use super::build::{Builder, Rewrite, RuleBuilder};
use super::handles::RuleRef;
use super::version::{Made, Version};
use super::{RelId, RuleId};
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
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
    bits: Vec<u64>,
    len: usize,
    of: PhantomData<I>,
}
pub type RelSet = IdSet<RelId>;
pub type RuleSet = IdSet<RuleId>;

impl<I: Dense> IdSet<I> {
    pub fn new() -> Self {
        IdSet { bits: Vec::new(), len: 0, of: PhantomData }
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
}

impl Delta {
    pub fn is_empty(&self) -> bool {
        self.touched == Reads::NONE
    }
    /// The edit's size, in rules removed and added and declarations changed.
    pub fn size(&self) -> usize {
        self.removed.len() + self.added.len() + self.decls.len()
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
}

type Value = Arc<dyn Any + Send + Sync>;

/// A cached analysis, with what the store needs of its type to carry it.
#[derive(Clone)]
struct Entry {
    value: Value,
    reads: Reads,
    name: &'static str,
    maintain: fn(&Value, &Cx<'_>, &Delta) -> Option<Value>,
    agrees: fn(&Value, &Cx<'_>) -> Result<bool>,
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

/// A version's analyses (`Version::analyses`), held by the version beside its indexes and dropped with it: not a
/// process cache (0103), and never keyed by anything but the version that holds it. A pipeline carries them from a
/// pass's input to its output (`carry`).
#[derive(Default)]
pub struct Store {
    of: parking_lot::Mutex<HashMap<TypeId, Entry>>,
    /// How many times each analysis was computed for this version, by name: what a test of the cache reads.
    computed: parking_lot::Mutex<HashMap<&'static str, usize>>,
}

impl Store {
    /// How many times analysis `name` was computed for this version (not carried to it).
    pub fn computed(&self, name: &str) -> usize {
        self.computed.lock().get(name).copied().unwrap_or(0)
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
pub fn analysis<A: Analysis>(v: &Arc<Version>) -> Result<Arc<A>> {
    let decided = RefCell::new(Vec::new());
    Cx { v, store: v.analyses(), pass: "reader", decided: &decided, asked: None }.get::<A>()
}

/// A decision a pass made, as a row of its step (0269): the pass, what, and its detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decided {
    pub pass: &'static str,
    pub what: &'static str,
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
        // Computed with no lock held: it may ask for other analyses. Two threads asking at once both compute it; the
        // first one stored is kept.
        let a = Arc::new(A::compute(self)?);
        *self.store.computed.lock().entry(A::NAME).or_default() += 1;
        let e = Entry { value: a.clone(), reads: A::READS, name: A::NAME, maintain: maintain_as::<A>, agrees: agrees_as::<A> };
        let kept = self.store.of.lock().entry(TypeId::of::<A>()).or_insert(e).value.clone();
        Ok(kept.downcast::<A>().expect("an entry of its type"))
    }
    /// Records a decision of the pass.
    pub fn decide(&self, what: &'static str, detail: impl FnOnce() -> String) {
        self.decided.borrow_mut().push(Decided { pass: self.pass, what, detail: detail() });
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
        for r in v.rules() {
            let step = match &picked {
                Some(p) if !p.has(r.id()) => Step::Keep,
                _ => {
                    visited += 1;
                    visit(pass, cx, v, r, b)?
                }
            };
            if step == Step::Keep {
                b.keep(v, r);
            }
        }
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
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()>;
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
        // A derivation that changed nothing is its input: its analyses carry, and no new base is kept.
        if mode == Mode::Derive && v.digest() == self.input.digest() && v.size() == self.input.size() {
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
    pass: &'static str,
    input: Arc<Version>,
    output: Arc<Version>,
    form: Form,
    /// A rule pass's, when its output can be run again onto (`None` reruns it whole).
    rule: Option<RuleKept>,
}

struct RuleKept {
    select: Select,
    /// The analyses it asked for, compared on the next input.
    asked: Vec<Asked>,
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
/// returns it. A pipeline made to be kept (`keeping`, `again`) holds each step's versions to the end, for `done`.
pub struct Pipeline {
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
}

impl Pipeline {
    pub fn new(name: &'static str, v: Arc<Version>) -> Pipeline {
        Pipeline { name, input: v.clone(), v, decided: RefCell::new(Vec::new()), stages: Vec::new(), failed: None, keep: false, last: Vec::new(), kept: Vec::new() }
    }
    /// A pipeline whose run is kept (`done`), to run again over an edit of `v` (`again`).
    pub fn keeping(name: &'static str, v: Arc<Version>) -> Pipeline {
        Pipeline { keep: true, ..Pipeline::new(name, v) }
    }
    /// The same passes, run again over `v`, an edit of `last`'s input (0281 §3). Each step whose input is its last
    /// one's reuses its output. Otherwise, over the edit (`Delta::of`) from its last input: the analyses are carried
    /// to the new input across it (maintained, or computed again where it touches what they read); a rule pass that
    /// reads nothing else the edit touched (`RulePass::WHOLE`), and whose selection and analyses come out the same,
    /// visits the added rules alone and writes onto its last output; every other step runs whole. The caller passes
    /// the same passes, in the same order: a step is matched to the last run's by its place and its pass's name.
    pub fn again(last: Run, v: Arc<Version>) -> Pipeline {
        Pipeline { last: last.steps.into_iter().map(Some).collect(), ..Pipeline::keeping(last.name, v) }
    }
    /// The current version.
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
        let d = Delta::of(&k.input, &self.v);
        if d.rebuilt {
            return Ok(Prior::None);
        }
        let cx = Cx { v: &self.v, store: self.v.analyses(), pass, decided: &self.decided, asked: None };
        self.v.analyses().carry(k.input.analyses(), &cx, &d)?;
        Ok(Prior::Edit(k, d))
    }

    /// Records a step that made `out` by an edit of `from` (the input, or the last output a rerun wrote onto): its
    /// stage, its analyses carried from `from`'s, and, when kept, its versions.
    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, pass: &'static str, out: Arc<Version>, ran: Ran, form: Form, visited: usize, from: &Arc<Version>, rule: Option<RuleKept>) -> Result<()> {
        let delta = Delta::of(from, &out);
        let cx = Cx { v: &out, store: out.analyses(), pass, decided: &self.decided, asked: None };
        out.analyses().carry(from.analyses(), &cx, &delta)?;
        self.stages.push(Stage { pass, ran, form, visited, input: (self.v.size(), self.v.digest()), output: (out.size(), out.digest()), delta: delta.size(), touched: delta.touched });
        if self.keep {
            self.kept.push(Kept { pass, input: self.v.clone(), output: out.clone(), form, rule });
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
        let r = self.rule_step(pass);
        self.fail(P::NAME, r)
    }

    fn rule_step<P: RulePass>(&mut self, pass: &mut P) -> Result<()> {
        let local = P::WHOLE != Reads::ALL;
        let prior = self.prior(P::NAME)?;
        let prior = match prior {
            Prior::Same(k) if local && k.rule.is_some() => {
                let out = k.output.clone();
                return self.record(P::NAME, out.clone(), Ran::Reused, k.form, 0, &out, k.rule);
            }
            p => p,
        };
        let v = self.v.clone();
        let asked = RefCell::new(Vec::new());
        let made = (|| -> Result<(Arc<Version>, Ran, Form, usize, Arc<Version>, Option<RuleKept>)> {
            let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: self.keep.then_some(&asked) };
            let select = pass.begin(&cx)?;
            if let Prior::Edit(k, d) = &prior
                && let Some(rk) = &k.rule
                && local
                && !P::WHOLE.meets(d.touched)
                && rk.select == select
                && same_analyses(&rk.asked, &k.input, &cx)?
            {
                let (out, form, visited, base, own_end) = rerun(pass, &cx, k, d, &select, rk.own_end)?;
                let mut all = rk.asked.clone();
                for a in asked.borrow().iter() {
                    if !all.iter().any(|b| b.t == a.t) {
                        all.push(*a);
                    }
                }
                let keep = (form != Form::Folded || own_end == 0).then_some(RuleKept { select, asked: all, own_end });
                return Ok((out, Ran::Delta, form, visited, base, keep));
            }
            let mut w = ProgramW::new(&v);
            if matches!(select, Select::All) {
                w.derive()?;
            }
            w.apply(&cx, pass, &select)?;
            let (out, form, visited) = w.finish()?;
            let own_end = if form == Form::Derived { 0 } else { v.rule_end() };
            let keep = (form != Form::Folded).then(|| RuleKept { select, asked: asked.take(), own_end });
            Ok((out, Ran::Whole, form, visited, v.clone(), keep))
        })();
        let (out, ran, form, visited, from, keep) = made?;
        self.record(P::NAME, out, ran, form, visited, &from, keep)
    }

    /// Runs program pass `pass`, over the whole input (its analyses carried from the last input when run again).
    pub fn program<P: ProgramPass>(&mut self, pass: &mut P) -> &mut Self {
        if self.failed.is_some() {
            return self;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("pass {}: {}", self.name, P::NAME));
        let r = self.program_step(pass);
        self.fail(P::NAME, r)
    }

    fn program_step<P: ProgramPass>(&mut self, pass: &mut P) -> Result<()> {
        self.prior(P::NAME)?;
        let v = self.v.clone();
        let made = {
            let cx = Cx { v: &v, store: v.analyses(), pass: P::NAME, decided: &self.decided, asked: None };
            let mut w = ProgramW::new(&v);
            pass.run(&cx, &mut w).and_then(|()| w.finish())
        };
        let (out, form, visited) = made?;
        self.record(P::NAME, out, Ran::Whole, form, visited, &v, None)
    }

    /// Runs `f`'s passes where `on`.
    pub fn run_if(&mut self, on: bool, f: impl FnOnce(&mut Self) -> &mut Self) -> &mut Self {
        if on { f(self) } else { self }
    }
    /// Runs `f`'s passes until a round changes nothing (the digest is the round's start's), at most `rounds` times.
    pub fn until_unchanged(&mut self, rounds: usize, mut f: impl FnMut(&mut Self)) -> &mut Self {
        for _ in 0..rounds {
            let before = self.v.digest();
            f(self);
            if self.failed.is_some() || self.v.digest() == before {
                return self;
            }
        }
        self.failed.get_or_insert_with(|| format!("{}: still changing after {rounds} rounds", self.name));
        self
    }
    /// The last version, or the first step's error.
    pub fn finish(self) -> Result<(Arc<Version>, Vec<Stage>)> {
        match self.failed {
            Some(e) => Err(e),
            None => Ok((self.v, self.stages)),
        }
    }
    /// The run, kept to run again (`again`), or the first step's error.
    pub fn done(self) -> Result<Run> {
        if let Some(e) = self.failed {
            return Err(e);
        }
        Ok(Run { name: self.name, input: self.input, output: self.v, stages: self.stages, steps: self.kept, decided: self.decided.into_inner() })
    }
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
