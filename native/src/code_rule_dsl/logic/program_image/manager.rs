//! The pass manager (decision 0281): a pass states what it changes, and the manager does the rest.
//!
//! - A **rule pass** (`RulePass`, 0173 §9's local pass) says what becomes of one rule, read alone: keep it, drop it,
//!   or write what replaces it through its `RuleOut`.
//! - A **program pass** (`ProgramPass`) reads analyses of the whole program and states an edit through its
//!   `ProgramW`: a filter, rule passes over a selection, or a derivation written rule by rule.
//! - An **analysis** (`Analysis`) is asked for by type (`cx.get::<Strata>()`), computed once per version for as long
//!   as a pipeline runs, and carried to the next version when the edit's `Delta` misses what it reads.
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
        fields_delta(input, output, &mut d);
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
fn fields_delta(input: &Version, output: &Version, d: &mut Delta) {
    use std::collections::HashMap;
    let decls = |v: &Version| -> HashMap<String, u64> { v.decls().into_iter().map(|x| (v.text(x.name()).to_owned(), x.hash())).collect() };
    let (a, b) = (decls(input), decls(output));
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
    if facts(input.all_facts(), input) != facts(output.all_facts(), output) {
        d.touched = d.touched | Reads::FACTS;
    }
    if facts(input.about_facts(), input) != facts(output.about_facts(), output) {
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
fn agrees_as<A: Analysis>(v: &Value, cx: &Cx<'_>) -> Result<bool> {
    Ok(v.downcast_ref::<A>().is_some_and(|a| A::compute(cx).is_ok_and(|fresh| fresh == *a)))
}

/// The analyses of the pipeline's current version. Not a process cache (0103): it ends with its pipeline.
#[derive(Default)]
pub struct Store {
    of: RefCell<HashMap<TypeId, Entry>>,
    /// How many times each analysis was computed, by name: what a test of the cache reads.
    computed: RefCell<HashMap<&'static str, usize>>,
}

impl Store {
    /// How many times analysis `name` was computed in this pipeline.
    pub fn computed(&self, name: &str) -> usize {
        self.computed.borrow().get(name).copied().unwrap_or(0)
    }
    /// The analyses carried from `input` to `output` (whose context is `cx`, over this store) across `delta`: those
    /// whose `READS` it misses as they are, then those it touches that `maintain` gives; the rest are dropped, to be
    /// computed again when asked for. A debug build checks each against a fresh computation in a scratch store.
    fn step(&self, cx: &Cx<'_>, delta: &Delta) -> Result<()> {
        let old = std::mem::take(&mut *self.of.borrow_mut());
        let (carried, touched): (Vec<_>, Vec<_>) = old.into_iter().partition(|(_, e)| !e.reads.meets(delta.touched));
        self.of.borrow_mut().extend(carried);
        for (t, e) in touched {
            if let Some(value) = (e.maintain)(&e.value, cx, delta) {
                self.of.borrow_mut().insert(t, Entry { value, ..e });
            }
        }
        if cfg!(debug_assertions) {
            let scratch = Store::default();
            let fresh = Cx { v: cx.v, store: &scratch, pass: cx.pass, decided: cx.decided };
            let of = self.of.borrow();
            for e in of.values() {
                if !(e.agrees)(&e.value, &fresh)? {
                    return Err(format!("analysis {} carried over {}'s edit ({:?}) is not what it computes", e.name, cx.pass, delta.touched));
                }
            }
        }
        Ok(())
    }
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
}

impl<'a> Cx<'a> {
    /// The version the pass reads.
    pub fn v(&self) -> &'a Arc<Version> {
        self.v
    }
    /// Analysis `A` of the version: the one cached, else computed now and cached.
    pub fn get<A: Analysis>(&self) -> Result<Arc<A>> {
        if let Some(e) = self.store.of.borrow().get(&TypeId::of::<A>()) {
            return Ok(e.value.clone().downcast::<A>().expect("an entry of its type"));
        }
        let a = Arc::new(A::compute(self)?);
        *self.store.computed.borrow_mut().entry(A::NAME).or_default() += 1;
        let e = Entry { value: a.clone(), reads: A::READS, name: A::NAME, maintain: maintain_as::<A>, agrees: agrees_as::<A> };
        self.store.of.borrow_mut().insert(TypeId::of::<A>(), e);
        Ok(a)
    }
    /// Records a decision of the pass.
    pub fn decide(&self, what: &'static str, detail: impl FnOnce() -> String) {
        self.decided.borrow_mut().push(Decided { pass: self.pass, what, detail: detail() });
    }
}

// ---- rule passes ----

/// The rules a rule pass visits (0281 §3), found through the version's indexes: all of them, the rules of some
/// heads, or the readers of some relations, or named rules. A pass costs what it selects.
#[derive(Clone, Debug)]
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

/// The writer a program pass states its edit through (0276 §3): an edit of its input (`on`), or a derivation of it
/// (`derive`), never both. Nothing stated, the input is the output.
pub struct ProgramW<'p> {
    input: &'p Arc<Version>,
    b: Option<(Builder<'p>, bool)>,
    visited: usize,
}

impl<'p> ProgramW<'p> {
    fn new(input: &'p Arc<Version>) -> Self {
        ProgramW { input, b: None, visited: 0 }
    }
    fn builder(&mut self, derive: bool) -> Result<&mut Builder<'p>> {
        let input = self.input;
        let (b, d) = self.b.get_or_insert_with(|| (if derive { Builder::derive(input) } else { Builder::on(input) }, derive));
        if *d != derive {
            return Err(format!("an edit of {} rules stated both as an edit and as a derivation", input.size()));
        }
        Ok(b)
    }
    /// The input.
    pub fn input(&self) -> &'p Arc<Version> {
        self.input
    }
    /// An edit of the input: removals make a view; additions and fields an overlay, folded when it reaches half.
    pub fn on(&mut self) -> Result<&mut Builder<'p>> {
        self.builder(false)
    }
    /// A derivation of the input: a new base under its ids, holding its fields and none of its rules.
    pub fn derive(&mut self) -> Result<&mut Builder<'p>> {
        self.builder(true)
    }
    /// The input's rules `keep` refuses removed.
    pub fn keep(&mut self, keep: impl FnMut(RuleRef<'_>) -> bool) -> Result<()> {
        self.on()?.retain(keep);
        Ok(())
    }
    /// Rule pass `pass` over the rules `select` picks, in this writer's form (a derivation keeps the rest in order).
    pub fn apply<P: RulePass>(&mut self, cx: &Cx<'_>, pass: &mut P, select: &Select) -> Result<()> {
        let derive = self.b.as_ref().is_some_and(|(_, d)| *d);
        let input = self.input;
        let b = self.builder(derive)?;
        let n = drive(pass, cx, input, b, select, derive)?;
        pass.finish(cx, b)?;
        self.visited += n;
        Ok(())
    }
    fn finish(self) -> Result<(Arc<Version>, Form, usize)> {
        match self.b {
            None => Ok((self.input.clone(), Form::Unchanged, self.visited)),
            Some((b, derive)) => {
                let (v, made) = b.finish()?;
                // A derivation that changed nothing is its input: its analyses carry, and no new base is kept.
                if derive && v.digest() == self.input.digest() && v.size() == self.input.size() {
                    return Ok((self.input.clone(), Form::Unchanged, self.visited));
                }
                let form = match (derive, made) {
                    (true, _) => Form::Derived,
                    (false, Made::View) => Form::View,
                    (false, Made::Overlay) => Form::Overlay,
                    (false, Made::Folded) => Form::Folded,
                };
                Ok((v, form, self.visited))
            }
        }
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

/// A pass's step, as the pipeline traces it: the pass, the form, the rules it visited, the versions' sizes and
/// digests, and the delta's size.
#[derive(Clone, Debug)]
pub struct Stage {
    pub pass: &'static str,
    pub form: Form,
    pub visited: usize,
    pub input: (usize, u64),
    pub output: (usize, u64),
    pub delta: usize,
    pub touched: Reads,
}

/// Passes run in order over a version (0281 §5). Each step's error stops the pipeline, named by its pass; `finish`
/// returns it.
pub struct Pipeline {
    name: &'static str,
    v: Arc<Version>,
    store: Store,
    decided: RefCell<Vec<Decided>>,
    stages: Vec<Stage>,
    failed: Option<String>,
}

impl Pipeline {
    pub fn new(name: &'static str, v: Arc<Version>) -> Pipeline {
        Pipeline { name, v, store: Store::default(), decided: RefCell::new(Vec::new()), stages: Vec::new(), failed: None }
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
    pub fn store(&self) -> &Store {
        &self.store
    }
    /// Analysis `A` of the current version.
    pub fn get<A: Analysis>(&self) -> Result<Arc<A>> {
        self.cx("pipeline").get::<A>()
    }
    fn cx(&self, pass: &'static str) -> Cx<'_> {
        Cx { v: &self.v, store: &self.store, pass, decided: &self.decided }
    }

    /// One step: `f` writes the pass's edit; the pipeline records it and carries the analyses to its output.
    fn step(&mut self, pass: &'static str, f: impl FnOnce(&Cx<'_>, &mut ProgramW<'_>) -> Result<()>) -> &mut Self {
        if self.failed.is_some() {
            return self;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("pass {}: {pass}", self.name));
        let made = {
            let cx = self.cx(pass);
            let mut w = ProgramW::new(&self.v);
            f(&cx, &mut w).and_then(|()| w.finish())
        };
        let (out, form, visited) = match made {
            Ok(m) => m,
            Err(e) => {
                self.failed = Some(format!("{}: {pass}: {e}", self.name));
                return self;
            }
        };
        let delta = Delta::of(&self.v, &out);
        let stage = Stage { pass, form, visited, input: (self.v.size(), self.v.digest()), output: (out.size(), out.digest()), delta: delta.size(), touched: delta.touched };
        let carried = Cx { v: &out, store: &self.store, pass, decided: &self.decided };
        if let Err(e) = self.store.step(&carried, &delta) {
            self.failed = Some(format!("{}: {e}", self.name));
            return self;
        }
        self.stages.push(stage);
        self.v = out;
        self
    }
    /// Runs rule pass `pass` over the rules it selects: in a derivation when it selects all (the program's order
    /// kept), else as an edit of the version.
    pub fn rules<P: RulePass>(&mut self, pass: &mut P) -> &mut Self {
        self.step(P::NAME, |cx, w| {
            let select = pass.begin(cx)?;
            if matches!(select, Select::All) {
                w.derive()?;
            }
            w.apply(cx, pass, &select)
        })
    }
    /// Runs program pass `pass`.
    pub fn program<P: ProgramPass>(&mut self, pass: &mut P) -> &mut Self {
        self.step(P::NAME, |cx, w| pass.run(cx, w))
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
}
