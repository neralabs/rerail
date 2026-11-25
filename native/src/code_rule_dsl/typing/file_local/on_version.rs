//! The file-local split read from the program's version (decision 0281's path, step 5): what `split_in` finds of
//! the tree, as an analysis of the version (`FileLocal`), maintained across an edit (`Analysis::maintain`).
//!
//! The tree's split numbers its relations by their names' addresses (`shape::Rels`) and each body's variables by
//! first appearance (`shape::Form`), and keeps the run's last splits on its thread (`split_memo`) so a split of a
//! program a few rules apart computes only what the difference reaches. Here the numbers are the image's own: a
//! relation is its `RelId`, a variable its `Slot`, and the last split is the analysis of the edit's input, carried to
//! its output by the manager, which hands `maintain` the edit. The fixpoints, and the questions they ask of a rule,
//! are the tree's, one for one.
//!
//! While the tree's split runs too (its passes are not yet on versions), a debug build checks the two agree
//! (`file_local::programs_over`).

use super::{CHUNK_VALUES, document_column, option_input};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Delta, Reads};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, NativeSpec, RelId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

/// A program's split, read from its version: the file-local relations with each cross-file one's cause, the
/// constant relations (derived in both parts), each derived relation's location columns, the string filters
/// (decision 0037), the same-document columns of each base input and file-local relation, and whether the program
/// reads no document (`reads_no_document`). With the digests it was made under, which `maintain` compares.
#[derive(Clone, Debug, Default)]
pub struct FileLocal {
    pub local: BTreeSet<String>,
    pub constants: BTreeSet<String>,
    pub locations: BTreeMap<String, Vec<usize>>,
    pub filters: BTreeSet<String>,
    pub causes: BTreeMap<String, String>,
    pub same_doc: BTreeMap<String, Vec<usize>>,
    /// The undeclared crossings (decision 0652): each relation in the cross-file part, not declared `cross_file`, that
    /// is there for a reason of its own: one of its rules places rows of two documents, or reads a document's rows with
    /// no location, or it is made natively. Its value is the cause and the rule. Not one: a relation whose rules read a
    /// cross-file relation (it follows that relation), or read no document's rows, or one document's rows plainly (a
    /// union of each file's rows).
    pub crossings: BTreeMap<String, String>,
    /// Each derived relation's nullable location columns (decision 0649): not its `locations`, but filled in every
    /// rule by a location or by `unset` (-1, no node). A rule joins through one only where the same rule places its
    /// variable by a location column too, which -1 never is; a row's document is never read off one.
    pub nullable: BTreeMap<String, Vec<usize>>,
    /// The views its readers cover (decision 0652, `covers`): file-local though a rule of each joins documents, because
    /// each reader holds what makes the rows it reads. A covered view of values alone is a filter too.
    pub covered: BTreeSet<String>,
    /// What placed each covered view (`covers`): its readers, and theirs through each located reader whose head anchors
    /// it. Not in its rules' digests, so `maintain` asks a covered view again when one of these changed or went.
    cover_deps: BTreeMap<String, BTreeSet<String>>,
    /// Each derived relation one document's in every row, file-local or not, with its location columns (two or more):
    /// its rules place them in one document through reads of such relations alone (`one_document`, a cross-file read
    /// taken where its own rows are each one document's). A demand that drops a read of one keeps its tie (0652).
    pub row_documents: BTreeMap<String, Vec<usize>>,
    pub no_document: bool,
    keys: Keys,
}

/// Two splits are the same where they place every relation alike; a cause's words are the first failure the split
/// found, which depends on the order it met them (a split maintained from an earlier one meets a relation read
/// already cross-file where a whole split meets the same rule failing on its location first), so only which
/// relations have one is compared.
impl PartialEq for FileLocal {
    fn eq(&self, o: &FileLocal) -> bool {
        self.local == o.local
            && self.constants == o.constants
            && self.locations == o.locations
            && self.filters == o.filters
            && self.causes.keys().eq(o.causes.keys())
            && self.same_doc == o.same_doc
            && self.crossings.keys().eq(o.crossings.keys())
            && self.nullable == o.nullable
            && self.covered == o.covered
            && self.cover_deps == o.cover_deps
            && self.row_documents == o.row_documents
            && self.no_document == o.no_document
            && self.keys == o.keys
    }
}

impl FileLocal {
    /// What the parts are made of (`file_local::found`): the file-local relations and those of constants; none of
    /// either for a program that reads no document, whose relations are all cross-file.
    pub fn found(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        match self.no_document {
            true => Default::default(),
            false => (self.local.clone(), self.constants.clone()),
        }
    }
}

impl Analysis for FileLocal {
    const NAME: &'static str = "file-local split";
    const READS: Reads = Reads::RULES.or(Reads::DECLS).or(Reads::FACTS).or(Reads::INPUTS).or(Reads::ABOUT);
    fn compute(cx: &Cx<'_>) -> Result<FileLocal, String> {
        let d = cx.get::<FieldsData>()?;
        let p = Program::of(cx.v(), Fields::over(cx.v(), &d));
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("split: all {} relations", p.rels.derived_count));
        Ok(p.split(None, None))
    }
    /// The split of the edit's output from the input's: the relations whose rules or declaration changed and every
    /// relation reading one, transitively (the cone), computed again, every other relation keeping its place. Exact
    /// (outside the cone no relation reads one inside); `None`, a whole split, where what every relation's place
    /// reads besides rules changed (the inputs, facts and `.about` rows), or past half the relations.
    fn maintain(&self, cx: &Cx<'_>, delta: &Delta) -> Option<FileLocal> {
        if delta.is_empty() {
            return Some(self.clone());
        }
        // An edit of half the rules or more (a part, a new base) reaches past half the relations: split whole, with no
        // digest of every rule first.
        if delta.rebuilt || 2 * (delta.removed.len() + delta.added.len()) >= cx.v().size() {
            return None;
        }
        let d = cx.get::<FieldsData>().ok()?;
        let p = Program::of(cx.v(), Fields::over(cx.v(), &d));
        let keys = p.keys();
        if keys.global != self.keys.global {
            return None;
        }
        let mut changed: Vec<&str> = keys.heads.iter().filter(|(rel, k)| self.keys.heads.get(*rel) != Some(*k)).map(|(rel, _)| rel.as_str()).collect();
        changed.extend(self.keys.heads.keys().filter(|rel| !keys.heads.contains_key(*rel)).map(String::as_str));
        if changed.is_empty() {
            return Some(p.finish(self.clone(), keys));
        }
        let mut cone: BTreeSet<u32> = BTreeSet::new();
        let mut work: Vec<u32> = changed.iter().filter_map(|r| p.v.rel(r)).map(|r| r.0).collect();
        // A covered view whose readers changed, or one of the readers that placed it went (`cover_deps`), is asked again.
        work.extend(self.cover_deps.iter().filter(|(_, d)| changed.iter().any(|c| d.contains(*c))).filter_map(|(v, _)| p.v.rel(v)).map(|r| r.0));
        cone.extend(work.iter().copied().filter(|&r| p.rels.is_derived(r)));
        // A view its readers may cover (decision 0652) is placed by its readers as well: one read by a rule of the cone,
        // covered before or not file-local, is in it, with its readers.
        let valueless = |r: u32| {
            let rel = p.name(r);
            p.rels.is_derived(r) && !self.nullable.contains_key(rel) && (self.covered.contains(rel) || !self.local.contains(rel) || self.locations.get(rel).is_none_or(|c| c.is_empty()))
        };
        while let Some(r) = work.pop() {
            for &k in p.rels.reading(r) {
                let h = p.rels.head[k];
                if cone.insert(h) {
                    work.push(h);
                }
            }
            for &k in &p.rels.rules[r as usize] {
                for a in p.rules[k].reads() {
                    let x = a.rel().0;
                    if valueless(x) && cone.insert(x) {
                        work.push(x);
                    }
                }
            }
        }
        if cone.len() * 2 > p.rels.derived_count {
            return None;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("split: {} of {} relations again", cone.len(), p.rels.derived_count));
        let split = p.split(Some(&cone), Some(self));
        if std::env::var_os("RERAIL_SPLIT_CHECK").is_some() {
            let whole = p.split(None, None);
            assert_eq!((&split.constants, &split.locations, &split.filters, &split.local), (&whole.constants, &whole.locations, &whole.filters, &whole.local), "a split of its cone differs from a whole split");
        }
        Some(split)
    }
}

impl Program<'_> {
    /// `split` with what it reads of every input, made again whatever the edit (the same-document columns, and
    /// whether the program reads a document: the keys digest only the inputs the fixpoints ask about), and `keys`.
    fn finish(&self, mut split: FileLocal, keys: Keys) -> FileLocal {
        let mut same_doc: BTreeMap<String, Vec<usize>> = self.f.inputs().filter(|i| !self.f.derives(i)).map(|i| (i.to_owned(), self.base_locations(i))).collect();
        same_doc.extend(split.local.iter().map(|rel| (rel.clone(), split.locations[rel].clone())));
        split.same_doc = same_doc;
        split.no_document = self.reads_no_document();
        split.keys = keys;
        split
    }
}

/// The digests a split was made under: one of what every relation's place reads besides rules, and one per relation
/// of its rules and declaration (a base relation's of its declaration), by name.
#[derive(Clone, Debug, Default, PartialEq)]
struct Keys {
    global: u64,
    heads: BTreeMap<String, u64>,
}

/// The keys' hasher: foldhash's quality hasher with a fixed seed (`split_memo`'s).
fn hasher() -> impl Hasher {
    use std::hash::BuildHasher;
    foldhash::quality::FixedState::with_seed(0x5eed_5011_7000_0001).build_hasher()
}

/// A version as the split reads it: its rules in order, its relations numbered by id (`Rels`), and its fields.
struct Program<'v> {
    v: &'v Version,
    f: Fields<'v>,
    rules: Vec<RuleRef<'v>>,
    rels: Rels<'v>,
}

impl<'v> Program<'v> {
    fn of(v: &'v Version, f: Fields<'v>) -> Program<'v> {
        let rules: Vec<RuleRef<'v>> = v.rules().collect();
        let rels = Rels::of(v, f, &rules);
        Program { v, f, rules, rels }
    }

    fn name(&self, r: u32) -> &'v str {
        self.v.rel_name(RelId(r))
    }

    /// Whether `rel` has an entry of stated facts (`Program::facts`), not an `.about` row.
    fn fact(&self, rel: &str) -> bool {
        self.f.facts(rel).is_some_and(|f| !f.about())
    }

    /// An input with one value for the whole run (`file_local::run_constant`).
    fn run_constant(&self, rel: &str) -> bool {
        option_input(rel) || (crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(rel) && self.f.is_input(rel))
    }

    fn keys(&self) -> Keys {
        let mut g = hasher();
        // The inputs a split asks about (`split_memo::keys`): derived, facts, or a phase's exports.
        let mut inputs: Vec<&str> = self.f.inputs().filter(|i| self.f.derives(i) || self.fact(i) || crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(i)).collect();
        inputs.sort_unstable();
        inputs.hash(&mut g);
        let mut facts: Vec<&str> = self.v.all_facts().into_iter().filter(|f| !f.about()).map(|f| self.v.text(f.name())).collect();
        facts.sort_unstable();
        facts.hash(&mut g);
        // The `.about` rows: natives and lattices, and the rest, which the split does not read (a change there
        // splits whole, as a change of a native does).
        let mut about: Vec<u64> = self.v.about_facts().into_iter().map(|f| f.hash()).collect();
        about.sort_unstable();
        about.hash(&mut g);
        let mut heads: BTreeMap<String, u64> = BTreeMap::new();
        for r in 0..self.rels.n as u32 {
            if self.rels.is_derived(r) {
                let mut h = hasher();
                self.f.columns(self.name(r)).hash(&mut h);
                for &k in &self.rels.rules[r as usize] {
                    h.write_u64(self.rules[k].hash());
                }
                heads.insert(self.name(r).to_owned(), h.finish());
            }
        }
        // A base relation's declaration is its own key (`split_memo::keys`).
        for d in self.v.decls() {
            let rel = self.v.text(d.name());
            if !self.f.derives(rel) {
                let mut h = hasher();
                b"base".hash(&mut h);
                d.hash().hash(&mut h);
                heads.insert(rel.to_owned(), h.finish());
            }
        }
        Keys { global: g.finish(), heads }
    }

    /// A base relation's location columns (`file_local::base_locations`).
    fn base_locations(&self, rel: &str) -> Vec<usize> {
        if matches!(rel, "reference_edge" | "reference_open") {
            return Vec::new();
        }
        let mut columns: Vec<usize> = self.f.types(rel).iter().enumerate().filter(|(_, t)| **t == "node").map(|(i, _)| i).collect();
        if let Some(column) = document_column(rel) {
            columns.push(column);
        }
        if rel == "docof" && !columns.contains(&1) {
            columns.push(1);
        }
        // The tape's `tape(n, par, size, depth)` (the climbs' parent column, lower.rs `need_tape`) is declared over
        // numbers, but a node's parent is a node of its document: both are locations (decision 0267,
        // O-cold-seed-join).
        if rel == "tape" {
            for column in [0, 1] {
                if !columns.contains(&column) {
                    columns.push(column);
                }
            }
        }
        columns
    }

    /// Whether no rule reads a document (`file_local::reads_no_document`).
    fn reads_no_document(&self) -> bool {
        let mut phased = false;
        let none = self.f.natives().is_empty()
            && self.rules.iter().flat_map(|r| r.body()).all(|l| match l.view() {
                L::Atom(a) | L::Agg { atom: a, .. } => {
                    let rel = self.v.rel_name(a.rel());
                    let constant = self.run_constant(rel);
                    phased |= constant && !option_input(rel);
                    self.f.derives(rel) || constant || (self.fact(rel) && !self.f.is_input(rel))
                }
                L::Cmp { .. } => true,
                _ => false,
            });
        none && phased
    }

    /// The constant relations in `scope` (by flag): a greatest fixpoint, any other derived relation a constant where
    /// `prev` holds it (`file_local::split_constants`).
    fn constants(&self, forms: &Forms<'_, 'v>, scope: &[bool], prev: Option<&BTreeSet<String>>) -> Vec<bool> {
        let rels = &self.rels;
        let mut constant: Vec<bool> = (0..rels.n)
            .map(|r| {
                rels.is_derived(r as u32)
                    && match prev.filter(|_| !scope[r]) {
                        Some(p) => p.contains(self.name(r as u32)),
                        None => rels.candidate[r],
                    }
            })
            .collect();
        let mut round: Vec<u32> = (0..rels.n as u32).filter(|&r| scope[r as usize] && constant[r as usize]).collect();
        while !round.is_empty() {
            let leaving: Vec<u32> = round.iter().copied().filter(|&r| !rels.rules[r as usize].iter().all(|&k| forms.get(k).reads_constants(rels, &constant))).collect();
            for &r in &leaving {
                constant[r as usize] = false;
            }
            round = rels.heads_reading(leaving.iter().copied(), |reader| constant[reader as usize]);
        }
        constant
    }

    /// The split, computed for the derived relations in `scope` only (every one where `None`), every other keeping
    /// its place in `prev` (`file_local::split_in`).
    fn split(&self, scope: Option<&BTreeSet<u32>>, prev: Option<&FileLocal>) -> FileLocal {
        let rels = &self.rels;
        let n = rels.n;
        let in_scope: Vec<bool> = (0..n as u32).map(|r| rels.is_derived(r) && scope.is_none_or(|s| s.contains(&r))).collect();
        let was = |r: usize| prev.filter(|_| !in_scope[r]);
        let forms = Forms::new(self);
        let constant = self.constants(&forms, &in_scope, prev.map(|p| &p.constants));
        let placeless: Vec<bool> = (0..n).map(|r| rels.fact[r] || constant[r] || rels.run_constant[r] || rels.chunk_value[r] || rels.test[r]).collect();
        // A derived relation's location columns: those every rule fills from a location.
        let mut locs: Vec<Vec<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vec::new();
                }
                let kept = was(r).and_then(|p| p.locations.get(self.name(r as u32)).cloned());
                kept.unwrap_or_else(|| (0..self.f.width(self.name(r as u32))).collect())
            })
            .collect();
        let mut round: Vec<u32> = (0..n as u32).filter(|&r| in_scope[r as usize]).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vec<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes(rels, &locs)).collect();
                    let kept: Vec<usize> = locs[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed(self.rules[k], c, p))).collect();
                    (kept != locs[r as usize]).then_some((r, kept))
                })
                .collect();
            let next = rels.heads_reading(changed.iter().map(|(r, _)| *r), |_| true);
            for (r, kept) in changed {
                locs[r as usize] = kept;
            }
            round = next;
        }
        // Nullable location columns (decision 0649): those every rule fills from a location (a location column, or a
        // nullable one read) or with `unset`, past the location columns.
        let mut nulls: Vec<Vec<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vec::new();
                }
                match was(r) {
                    Some(p) => p.nullable.get(self.name(r as u32)).cloned().unwrap_or_default(),
                    None => (0..self.f.width(self.name(r as u32))).filter(|c| !locs[r].contains(c)).collect(),
                }
            })
            .collect();
        let mut round: Vec<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && !nulls[r as usize].is_empty()).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vec<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_or_null(rels, &locs, &nulls)).collect();
                    let kept: Vec<usize> = nulls[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed_or_unset(self.rules[k], c, p))).collect();
                    (kept != nulls[r as usize]).then_some((r, kept))
                })
                .collect();
            let next = rels.heads_reading(changed.iter().map(|(r, _)| *r), |_| true);
            for (r, kept) in changed {
                nulls[r as usize] = kept;
            }
            round = next;
        }
        let derived = || (0..n as u32).filter(|&r| rels.is_derived(r));
        let locations: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        let nullable: BTreeMap<String, Vec<usize>> = derived().filter(|&r| !nulls[r as usize].is_empty()).map(|r| (self.name(r).to_owned(), nulls[r as usize].clone())).collect();
        // String filters (decision 0037).
        // A covered view of values is in `filters` too (read as a test, below), but it is no string filter: outside the
        // scope a relation is one where it was before that.
        let mut filtering: Vec<bool> = (0..n)
            .map(|r| {
                let rel = self.name(r as u32);
                rels.is_derived(r as u32) && was(r).is_none_or(|p| p.filters.contains(rel) && !p.covered.contains(rel))
            })
            .collect();
        let mut round: Vec<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && filtering[r as usize]).collect();
        while !round.is_empty() {
            let leaving: Vec<u32> = round.iter().copied().filter(|&r| !rels.rules[r as usize].iter().all(|&k| forms.get(k).filters(self.rules[k], rels, &filtering))).collect();
            for &r in &leaving {
                filtering[r as usize] = false;
            }
            round = rels.heads_reading(leaving.iter().copied(), |reader| filtering[reader as usize]);
        }
        let constants: BTreeSet<String> = derived().filter(|&r| constant[r as usize]).map(|r| self.name(r).to_owned()).collect();
        let filters: BTreeSet<String> = derived().filter(|&r| filtering[r as usize]).map(|r| self.name(r).to_owned()).collect();
        let fact_not_input = |r: RelId| {
            let rel = self.v.rel_name(r);
            self.fact(rel) && !self.f.is_input(rel)
        };
        // A native relation is judged by its rules, which the lowering keeps beside it with its rows' shape (decision
        // 0652): a walk (`dfs_first`, `reach`, `components`) over edges of one document each stays in it, and one over
        // edges between documents is cross-file by its rules as any rule is. A function of strings (`match_all`, a
        // text test) is judged by its input (`functional`). Python's natives over a cross-file target are made in the
        // cross-file part.
        let cross_native = |rel: &str| {
            self.f.native(rel).is_some_and(|nat| match &nat.spec {
                NativeSpec::TextTest { .. } => false,
                NativeSpec::PythonCandidates { targets, .. } => !constants.contains(self.v.rel_name(*targets)) && !fact_not_input(*targets),
                NativeSpec::Python { targets, .. } => targets.is_some_and(|t| !constants.contains(self.v.rel_name(t)) && !fact_not_input(t)),
                _ => false,
            })
        };
        // A relation `locate` moved (`CROSS_FILE`), its rows naming no document (decision 0290), or one declared
        // `cross_file` (decision 0652): derived once over every file.
        let moved = |rel: &str| self.f.qualified(rel, super::CROSS_FILE);
        let mut local_now: Vec<bool> = (0..n)
            .map(|r| {
                rels.is_derived(r as u32)
                    && match was(r) {
                        Some(p) => p.local.contains(self.name(r as u32)),
                        None => !cross_native(self.name(r as u32)) && !moved(self.name(r as u32)),
                    }
            })
            .collect();
        let mut causes: BTreeMap<String, String> = derived()
            .filter_map(|r| {
                let rel = self.name(r);
                match was(r as usize) {
                    Some(p) => p.causes.get(rel).map(|c| (rel.to_owned(), c.clone())),
                    None if cross_native(rel) => Some((rel.to_owned(), "made natively".to_owned())),
                    None if self.f.qualified(rel, super::MOVED_CROSS_FILE) => Some((rel.to_owned(), "its rows name no document (moved by `locate`)".to_owned())),
                    None => moved(rel).then(|| (rel.to_owned(), "declared `cross_file`".to_owned())),
                }
            })
            .collect();
        // The views its readers cover (decision 0652): a relation of values alone (no location column), read only by
        // file-local rules, each of whose rules finds its rows in every reader's own rows (`covers`). Derived in each
        // chunk, it answers each reader as it would over every file, and its readers read it as a test. Assumed of
        // each candidate, as file-local is, and given up where `covers` fails.
        let cover_candidate: Vec<bool> = (0..n)
            .map(|r| {
                let rel = self.name(r as u32);
                in_scope[r]
                    && rels.candidate[r]
                    && !rels.output[r]
                    && !rels.functional[r]
                    && !rels.rules[r].is_empty()
                    && nulls[r].is_empty()
                    && !placeless[r]
                    && !filtering[r]
                    && !moved(rel)
            })
            .collect();
        // A located candidate is asked when a rule of it places more than one document; one whose rules are each on one
        // document is file-local as any relation, and asked only as a reader whose head places a view's location
        // columns (`covers`, `Anchoring`). A candidate of values alone is asked always: its readers read it as a test.
        let valueless: Vec<bool> = (0..n).map(|r| cover_candidate[r] && locs[r].is_empty()).collect();
        // Outside the scope a view keeps its place, covered or not.
        let mut covered_now: Vec<bool> = (0..n)
            .map(|r| {
                if let Some(p) = was(r) {
                    return p.covered.contains(self.name(r as u32));
                }
                cover_candidate[r]
                    && (valueless[r] || rels.rules[r].iter().any(|&k| !forms.get(k).one_document(r as u32, rels, &placeless, &local_now, &filtering, &valueless, &locs, &nulls)))
            })
            .collect();
        let mut dirty: Option<Vec<usize>> = None;
        loop {
            // A covered view whose readers changed is asked again; one no longer covered is read as any relation, by
            // its own rules and its readers.
            let asked: Vec<u32> = match &dirty {
                None => (0..n as u32).filter(|&r| in_scope[r as usize] && covered_now[r as usize]).collect(),
                Some(rules) => {
                    let mut heads: Vec<u32> = rules.iter().map(|&k| rels.head[k]).filter(|&h| in_scope[h as usize] && covered_now[h as usize]).collect();
                    heads.sort_unstable();
                    heads.dedup();
                    heads
                }
            };
            let mut also: Vec<usize> = Vec::new();
            let mut asked = asked;
            let anchoring = Anchoring { candidate: &cover_candidate, memo: Default::default() };
            while let Some(r) = asked.pop() {
                if !covered_now[r as usize] {
                    continue;
                }
                anchoring.memo.borrow_mut().clear();
                if self.covers(r, &local_now, &covered_now, &valueless, &placeless, &locs, &anchoring).is_err() {
                    covered_now[r as usize] = false;
                    also.extend(rels.rules[r as usize].iter().copied());
                    also.extend(rels.reading(r).iter().copied());
                    // The views its rules read are covered through its head no more: asked again.
                    for &k in &rels.rules[r as usize] {
                        for a in self.rules[k].reads() {
                            if covered_now[a.rel().0 as usize] && in_scope[a.rel().0 as usize] {
                                asked.push(a.rel().0);
                            }
                        }
                    }
                }
            }
            // The covered views of values, in the scope and out of it (where `valueless`, a candidate's, is not asked).
            let skip: Vec<bool> = (0..n).map(|r| covered_now[r] && local_now[r] && locs[r].is_empty()).collect();
            let read_as_tests: BTreeSet<String> = filters.iter().cloned().chain((0..n as u32).filter(|&r| skip[r as usize]).map(|r| self.name(r).to_owned())).collect();
            let explain = |k: usize, local_now: &[bool]| self.why_not_local(self.rules[k], local_now, &constants, &read_as_tests, &locations, &nullable);
            let check = |k: usize| -> Option<(u32, String)> {
                let head = rels.head[k];
                if covered_now[head as usize] && local_now[head as usize] {
                    return None;
                }
                // A function of strings is where its input is: its rows are its input strings' values.
                if rels.functional[head as usize] {
                    let crossed = forms.get(k).lits.iter().find_map(|l| match l {
                        Lit::Read { rel: x, .. } if rels.is_derived(*x) && !local_now[*x as usize] && !placeless[*x as usize] => Some(*x),
                        _ => None,
                    });
                    return crossed.map(|x| (head, format!("reads cross-file {}", self.name(x))));
                }
                if forms.get(k).one_document(head, rels, &placeless, &local_now, &filtering, &skip, &locs, &nulls) {
                    debug_assert!(explain(k, &local_now).is_none(), "the numbered check and the text's agree on {}", self.name(head));
                    return None;
                }
                let why = explain(k, &local_now);
                debug_assert!(why.is_some(), "the numbered check and the text's agree on {}", self.name(head));
                Some((head, why.unwrap_or_else(|| "places more than one document".to_owned())))
            };
            let failing: Vec<(u32, String)> = match &dirty {
                None => (0..self.rules.len()).filter(|&k| in_scope[rels.head[k] as usize] && local_now[rels.head[k] as usize]).filter_map(check).collect(),
                Some(rules) => {
                    let mut rules: Vec<usize> = rules.iter().copied().chain(also).collect();
                    rules.sort_unstable();
                    rules.dedup();
                    rules.into_iter().filter(|&k| local_now[rels.head[k] as usize]).filter_map(check).collect()
                }
            };
            let mut next: Vec<usize> = Vec::new();
            let mut left = false;
            for (r, why) in failing {
                if std::mem::replace(&mut local_now[r as usize], false) {
                    next.extend(rels.reading(r));
                    // The covered views it read lose a reader: asked again.
                    for &k in &rels.rules[r as usize] {
                        for a in self.rules[k].reads() {
                            if covered_now[a.rel().0 as usize] {
                                next.extend(rels.rules[a.rel().0 as usize].iter().copied());
                            }
                        }
                    }
                    left = true;
                }
                causes.entry(self.name(r).to_owned()).or_insert(why);
            }
            next.sort_unstable();
            next.dedup();
            dirty = Some(next);
            if !left {
                break;
            }
        }
        let local: BTreeSet<String> = derived().filter(|&r| local_now[r as usize]).map(|r| self.name(r).to_owned()).collect();
        let covered: BTreeSet<String> = derived().filter(|&r| covered_now[r as usize] && local_now[r as usize]).map(|r| self.name(r).to_owned()).collect();
        // What placed each: its readers, and theirs through every located reader (one may anchor it, `Anchoring`).
        let cover_deps: BTreeMap<String, BTreeSet<String>> = derived()
            .filter(|&r| covered_now[r as usize] && local_now[r as usize])
            .map(|r| {
                let (mut deps, mut seen, mut work) = (BTreeSet::new(), BTreeSet::from([r]), vec![r]);
                while let Some(x) = work.pop() {
                    for &k in rels.reading(x) {
                        let h = rels.head[k];
                        deps.insert(self.name(h).to_owned());
                        if !locs[h as usize].is_empty() && seen.insert(h) {
                            work.push(h);
                        }
                    }
                }
                (self.name(r).to_owned(), deps)
            })
            .collect();
        let skip: Vec<bool> = (0..n).map(|r| covered_now[r] && local_now[r] && locs[r].is_empty()).collect();
        // A covered view of values is read as a filter is: a part given it reads it so (`part:filter`).
        let filters: BTreeSet<String> = filters.into_iter().chain(derived().filter(|&r| skip[r as usize]).map(|r| self.name(r).to_owned())).collect();
        // The undeclared crossings (decision 0652): a cross-file relation's rule that reads no cross-file relation and,
        // with every input naming no document taken as placeless, still does not place one document. Recorded only:
        // where a relation is derived is the split's above, and preparation refuses these (`crossings_checked`).
        let placeless_or_input: Vec<bool> = (0..n).map(|r| placeless[r] || (!rels.is_derived(r as u32) && rels.base_locs[r].is_empty())).collect();
        let mut crossings: BTreeMap<String, String> = BTreeMap::new();
        for r in derived() {
            let (ru, rel) = (r as usize, self.name(r));
            if local_now[ru] || moved(rel) {
                continue;
            }
            if let Some(p) = was(ru) {
                if let Some(c) = p.crossings.get(rel) {
                    crossings.insert(rel.to_owned(), c.clone());
                }
                continue;
            }
            if cross_native(rel) {
                crossings.insert(rel.to_owned(), "made natively".to_owned());
                continue;
            }
            for &k in &rels.rules[ru] {
                let form = forms.get(k);
                let reads_cross = form.lits.iter().any(|l| matches!(l, Lit::Read { rel: x, .. } if rels.is_derived(*x) && !local_now[*x as usize] && !placeless[*x as usize]));
                // Its reads of documents' rows. None: the run's inputs alone. One plain positive read: each row is one
                // file's, a union (as the cross part reads the chunks' findings back). Neither is a crossing; documents
                // meet only where a rule joins two reads of them, or aggregates one.
                let documents: Vec<&Lit> = form
                    .lits
                    .iter()
                    .filter(|l| match l {
                        Lit::Read { rel: x, .. } => !placeless_or_input[*x as usize],
                        Lit::Ast | Lit::Nearest(..) => true,
                        Lit::Cmp | Lit::Same(..) => false,
                    })
                    .collect();
                let union = matches!(documents.as_slice(), [] | [Lit::Read { neg: false, plain: true, .. }]);
                if reads_cross || union || form.one_document(r, rels, &placeless_or_input, &local_now, &filtering, &skip, &locs, &nulls) {
                    continue;
                }
                let why = self.why_not_local(self.rules[k], &local_now, &constants, &filters, &locations, &nullable).unwrap_or_else(|| "places more than one document".to_owned());
                crossings.insert(rel.to_owned(), format!("{why}\n      {}", self.v.show_rule(self.rules[k]).chars().take(400).collect::<String>()));
                break;
            }
        }
        // The relations one document's in every row (`row_documents`): assumed of each, and given up where a rule does not
        // place one document through reads of such relations, as `local_now` is but for where a relation is derived. One
        // with a single location column (a native's too, `sym_at(n, q)`) is so whatever makes it. A covered view of values
        // is read as a test, as the crossings read it (`skip`).
        let single = |r: u32| locs[r as usize].len() <= 1;
        let mut one_row: Vec<bool> = (0..n).map(|r| rels.is_derived(r as u32) && (single(r as u32) || !cross_native(self.name(r as u32)))).collect();
        let mut round: Vec<u32> = derived().collect();
        while !round.is_empty() {
            let leaving: Vec<u32> = round
                .iter()
                .copied()
                .filter(|&r| one_row[r as usize] && !single(r) && !rels.rules[r as usize].iter().all(|&k| forms.get(k).one_document(r, rels, &placeless_or_input, &one_row, &filtering, &skip, &locs, &nulls)))
                .collect();
            for &r in &leaving {
                one_row[r as usize] = false;
            }
            round = rels.heads_reading(leaving.into_iter(), |h| one_row[h as usize]);
        }
        let row_documents: BTreeMap<String, Vec<usize>> = derived().filter(|&r| one_row[r as usize] && locs[r as usize].len() > 1).map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        // The same-document columns and the rest: `finish`.
        self.finish(FileLocal { local, constants, locations, filters, causes, nullable, crossings, covered, cover_deps, row_documents, ..Default::default() }, self.keys())
    }

    /// Whether every reader of `r` covers it (decision 0652). Each reader is a file-local rule reading `r` in a plain
    /// literal (positive or negated, not aggregated); each of `r`'s location columns it reads is a variable another of
    /// its positive reads places, or a location column of its head when its head is a located view covered in turn
    /// (whose readers place it). Each rule of `r` then finds its rows in the reader's (`maps`). Per chunk every atom
    /// of a rule's instance is the chunk's, so `r` derived per chunk holds at the reader's values exactly where it
    /// holds over every file: a rule's rows per chunk are among its rows over every file, and a row over every file,
    /// at the reader's values, has its body in the reader's own document's rows, so the chunk derives it too.
    fn covers(&self, r: u32, local_now: &[bool], covered: &[bool], valueless: &[bool], placeless: &[bool], locs: &[Vec<usize>], anchoring: &Anchoring<'_>) -> Result<(), String> {
        let rels = &self.rels;
        let columns = |x: u32| -> &[usize] { if rels.is_derived(x) { &locs[x as usize] } else { &rels.base_locs[x as usize] } };
        let own: Vec<RuleRef<'v>> = rels.rules[r as usize].iter().map(|&k| self.rules[k]).collect();
        for rule in &own {
            for l in rule.body() {
                match l.view() {
                    L::Atom(a) => {
                        let x = a.rel().0;
                        if x == r || (rels.is_derived(x) && !local_now[x as usize] && !placeless[x as usize]) {
                            return Err(format!("own rule reads {}", self.name(x)));
                        }
                    }
                    L::Cmp { .. } | L::Nearest { .. } => {}
                    _ => return Err("own rule has an aggregate or pattern".into()),
                }
            }
        }
        let readers = rels.reading(r);
        if readers.is_empty() {
            return Err("no readers".into());
        }
        for &j in readers {
            let h = rels.head[j];
            // A cross-file reader covers a located view too, where its own reads place the view's location columns in
            // one document: the rows it joins are that document's, which the view per chunk derives. A view of values
            // maps into its reader's rows, which are one document's only in a file-local rule.
            let cross_reader = !local_now[h as usize];
            if h == r || (cross_reader && valueless[r as usize]) {
                return Err(format!("reader {} not local", self.name(h)));
            }
            let reader = self.rules[j];
            let qc = value_classes(reader);
            // The classes the reader's other positive reads place, and its head's location columns where its readers
            // place them.
            let mut placed: BTreeSet<u32> = BTreeSet::new();
            // A file-local located view whose rules are each on one document is asked here, once a round: its readers
            // place its head (a cycle back to a view being asked does not).
            let anchors = |h: u32| -> bool {
                if covered[h as usize] {
                    return true;
                }
                if !anchoring.candidate[h as usize] || !local_now[h as usize] || locs[h as usize].is_empty() {
                    return false;
                }
                if let Some(&known) = anchoring.memo.borrow().get(&h) {
                    return known;
                }
                anchoring.memo.borrow_mut().insert(h, false);
                let got = self.covers(h, local_now, covered, valueless, placeless, locs, anchoring).is_ok();
                anchoring.memo.borrow_mut().insert(h, got);
                got
            };
            if !cross_reader && !valueless[h as usize] && anchors(h) {
                let args: Vec<TermRef<'_>> = reader.head().args().collect();
                placed.extend(locs[h as usize].iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| qc[s.0 as usize]));
            }
            for l in reader.body() {
                if let L::Atom(a) = l.view()
                    && !a.neg()
                    && a.rel().0 != r
                    && !placeless[a.rel().0 as usize]
                {
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    placed.extend(columns(a.rel().0).iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| qc[s.0 as usize]));
                }
            }
            for (at, l) in reader.body().enumerate() {
                match l.view() {
                    L::Atom(a) if a.rel().0 == r => {
                        let args: Vec<TermRef<'_>> = a.args().collect();
                        if !columns(r).iter().all(|&c| args.get(c).and_then(|t| t.var()).is_some_and(|s| placed.contains(&qc[s.0 as usize]))) {
                            return Err(format!("reader leaves a location open: {}", self.v.show_rule(reader)));
                        }
                        if cross_reader && !one_place(self.v, reader, r, &|x| placeless[x as usize], &columns, columns(r).iter().filter_map(|&c| args.get(c).and_then(|t| t.var()))) {
                            return Err(format!("cross-file reader places the view in two documents: {}", self.v.show_rule(reader)));
                        }
                        if let Some(rule) = own.iter().find(|&&rule| !maps(self.v, rule, reader, at, columns(r), &|x| placeless[x as usize], &columns)) {
                            return Err(format!("rule {} not found in reader {}", self.v.show_rule(*rule), self.v.show_rule(reader)));
                        }
                    }
                    L::Agg { atom, .. } if atom.rel().0 == r => return Err("aggregated by a reader".into()),
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Why `rule` places more than one document, from its text (`split_in`'s `rule_local`): `None` where it does
    /// not. Asked only of a rule the numbered check fails (and, in a debug build, of each it passes).
    fn why_not_local(&self, rule: RuleRef<'v>, local_now: &[bool], constants: &BTreeSet<String>, filters: &BTreeSet<String>, locations: &BTreeMap<String, Vec<usize>>, nullable: &BTreeMap<String, Vec<usize>>) -> Option<String> {
        let v = self.v;
        let equal_names: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (v.text(rule.var(a)).to_owned(), v.text(rule.var(b)).to_owned())).collect();
        // The variables a location column of a positive read places (`one_document`'s `strict`): a nullable
        // column's variable among them is no -1.
        let mut strict: BTreeSet<String> = BTreeSet::new();
        for l in rule.body() {
            if let L::Atom(a) | L::Agg { atom: a, .. } = l.view()
                && !a.neg()
            {
                let rel = v.rel_name(a.rel());
                let columns = if self.f.derives(rel) { locations.get(rel).cloned().unwrap_or_default() } else { self.base_locations(rel) };
                let args: Vec<TermRef<'_>> = a.args().collect();
                strict.extend(columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| v.text(rule.var(s)).to_owned()));
            }
        }
        super::close_under(&mut strict, &equal_names);
        // `one_document`'s `nullable_row`: a nullable column's variable no location column places, to its positive
        // row's first location variable.
        let mut nullable_row: BTreeMap<String, String> = BTreeMap::new();
        for l in rule.body() {
            if let L::Atom(a) = l.view()
                && !a.neg()
                && self.f.derives(v.rel_name(a.rel()))
                && let Some(nc) = nullable.get(v.rel_name(a.rel()))
            {
                let args: Vec<TermRef<'_>> = a.args().collect();
                let Some(anchor) = locations.get(v.rel_name(a.rel())).and_then(|cs| cs.iter().find_map(|&c| args.get(c).and_then(|t| t.var()))) else { continue };
                for &c in nc {
                    if let Some(x) = args.get(c).and_then(|t| t.var()).map(|s| v.text(rule.var(s)).to_owned())
                        && !strict.contains(&x)
                    {
                        nullable_row.entry(x).or_insert_with(|| v.text(rule.var(anchor)).to_owned());
                    }
                }
            }
        }
        let head = v.rel_name(rule.head().rel());
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        let local = |rel: &str| v.rel(rel).is_some_and(|r| local_now[r.0 as usize]);
        let placed = ValueTapes::of(v, rule);
        let tape = |c: u32| format!("rr_value_tape:{}", var(Slot(c)));
        let mut groups: Vec<BTreeSet<String>> = Vec::new();
        // `@doc(a) = @doc(b)` and `d = @doc(n)`: one document's (`Lit::Same`).
        let mut same_doc: Vec<(String, String)> = Vec::new();
        for &(n, t) in &placed.splices {
            groups.push(BTreeSet::from([var(n), tape(t)]));
        }
        for (index, l) in rule.body().enumerate() {
            let (atom, plain) = match l.view() {
                L::Atom(a) => (a, true),
                L::Agg { atom, .. } => (atom, false),
                L::Ast { .. } => return Some("an AST pattern".to_owned()),
                L::Nearest { n, a, .. } => {
                    groups.push(BTreeSet::from([var(n), var(a)]));
                    continue;
                }
                L::Cmp { l, op: CmpOp::Eq, r } => {
                    if let (Some((a, x)), Some((b, y))) = (doc_side(v, l), doc_side(v, r))
                        && (x || y)
                    {
                        same_doc.push((var(a), var(b)));
                    }
                    continue;
                }
                L::Cmp { .. } => continue,
            };
            let rel = v.rel_name(atom.rel());
            let derived = self.f.derives(rel);
            let constant = (!derived && self.fact(rel) && !self.f.is_input(rel)) || constants.contains(rel) || self.run_constant(rel);
            let chunk_value = !derived && CHUNK_VALUES.contains(&rel);
            let functional = local(rel) && (self.f.native(rel).is_some_and(|nat| matches!(nat.spec, NativeSpec::MatchAll { .. })));
            if constant || chunk_value || functional || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) || filters.contains(rel) || filters.contains(head) {
                continue;
            }
            if derived && !local(rel) {
                return Some(format!("reads cross-file {rel}"));
            }
            let columns = if derived { locations[rel].clone() } else { self.base_locations(rel) };
            let args: Vec<TermRef<'_>> = atom.args().collect();
            let mut vars: BTreeSet<String> = columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).collect();
            if atom.neg() {
                vars = vars.into_iter().map(|x| nullable_row.get(&x).cloned().unwrap_or(x)).collect();
            }
            if derived && let Some(nc) = nullable.get(rel) {
                vars.extend(nc.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).filter(|x| strict.contains(x)));
            }
            let binder = plain && placed.by_binder.values().any(|b| *b == index);
            if vars.is_empty() && binder && !columns.is_empty() {
                vars.insert(format!("rr_value_binder:{index}"));
            }
            if !vars.is_empty() {
                vars.extend(placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| tape(*t)));
                if rel == "tape_of"
                    && let Some(t) = args.get(1).and_then(|t| t.var())
                    && let Some(t) = placed.canonical(t)
                {
                    vars.insert(tape(t));
                }
            }
            if vars.is_empty() {
                return Some(format!("reads {rel} with no location"));
            }
            groups.push(vars);
        }
        let mut equal: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (var(a), var(b))).collect();
        equal.extend(same_doc);
        for g in &mut groups {
            super::close_under(g, &equal);
        }
        let mut merged: Vec<BTreeSet<String>> = Vec::new();
        for group in groups {
            let (joined, rest): (Vec<_>, Vec<_>) = merged.into_iter().partition(|m| !m.is_disjoint(&group));
            let mut all = group;
            for j in joined {
                all.extend(j);
            }
            merged = rest;
            merged.push(all);
        }
        if merged.len() == 1 || (merged.is_empty() && filters.contains(head)) {
            None
        } else if merged.is_empty() {
            Some("reads no location".to_owned())
        } else {
            let parts: Vec<String> = merged.iter().map(|m| m.iter().filter(|v| !v.starts_with("rr_value")).cloned().collect::<Vec<_>>().join(" ")).collect();
            Some(format!("joins {} documents: {{{}}}", merged.len(), parts.join("} {")))
        }
    }
}

/// The candidates a split may cover (decision 0652), and the file-local located views `covers` asked as anchoring
/// readers this round: true where its readers place its head.
struct Anchoring<'a> {
    candidate: &'a [bool],
    memo: std::cell::RefCell<BTreeMap<u32, bool>>,
}

/// Whether `reader`'s positive located reads, other than of `r`, place `vars` in one document: joined through their
/// location columns, `a = b`, `@doc` and `nearest` (decision 0652, a cross-file reader of a covered view).
fn one_place<'c>(v: &Version, reader: RuleRef<'_>, r: u32, placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], vars: impl Iterator<Item = Slot>) -> bool {
    let mut doc: Vec<u32> = (0..reader.var_count() as u32).collect();
    for (a, b) in super::var_equalities_in(v, reader) {
        join(&mut doc, a.0, b.0);
    }
    for l in reader.body() {
        match l.view() {
            L::Atom(a) if !a.neg() && a.rel().0 != r && !placeless(a.rel().0) => {
                let args: Vec<TermRef<'_>> = a.args().collect();
                let vs: Vec<u32> = columns(a.rel().0).iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| s.0).collect();
                for w in vs.windows(2) {
                    join(&mut doc, w[0], w[1]);
                }
            }
            L::Nearest { n, a, .. } => join(&mut doc, n.0, a.0),
            _ => {}
        }
    }
    let mut roots: Vec<u32> = vars.map(|s| find(&mut doc, s.0)).collect();
    roots.sort_unstable();
    roots.dedup();
    roots.len() <= 1
}

/// A rule's variables by value: each slot's class under the body's `a = b` between two variables.
fn value_classes(rule: RuleRef<'_>) -> Vec<u32> {
    let mut root: Vec<u32> = (0..rule.var_count() as u32).collect();
    for l in rule.body() {
        if let L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
            join(&mut root, a.0, b.0);
        }
    }
    (0..rule.var_count() as u32).map(|s| find(&mut root, s)).collect()
}

/// Whether `rule`'s rows are found in `reader`'s at its literal `at`, a read of `rule`'s head (decision 0652).
///
/// The head's location columns (`anchors`) are in the reader's document (`covers`), and so is each positive read of a
/// located relation that a location of the rule ties to one of them (through the reads' location columns, `a = b` and
/// `@doc`): anchored. Every other positive located read maps to one of the reader's positive reads of the same
/// relation, argument by argument, under a map of the rule's variables to the reader's terms that takes the head to
/// the read's arguments. A row over every file is then derived from the reader's document alone: its anchored reads
/// are there already, and the others are the reader's own rows, at the variables they share with the rest of the body
/// the head fixes. The rule's other literals (comparisons, placeless reads, negated reads, which must be anchored)
/// read no mapped variable but those the head fixes. A head argument the read leaves `_` fixes nothing. A class of
/// the rule's variables may stand for several of the reader's terms: its head columns' arguments, equal in any row.
fn maps<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: usize, anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize]) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum T {
        Var(u32),
        Int(i64),
        Str(u32),
    }
    let n = rule.var_count();
    let (rc, qc) = (value_classes(rule), value_classes(reader));
    let target = |t: TermRef<'_>| match t {
        TermRef::Var(s) => Some(T::Var(qc[s.0 as usize])),
        TermRef::Int(i) => Some(T::Int(i)),
        TermRef::Str(a) => Some(T::Str(a.0)),
        _ => None,
    };
    let Some(L::Atom(read)) = reader.body().nth(at).map(|l| l.view()) else { return false };
    let mut bind: Vec<Vec<T>> = vec![Vec::new(); n];
    let mut anchor_vars: Vec<u32> = Vec::new();
    for (c, (h, u)) in rule.head().args().zip(read.args()).enumerate() {
        match h {
            TermRef::Var(s) => {
                if let Some(t) = target(u) {
                    bind[rc[s.0 as usize] as usize].push(t);
                }
                if anchors.contains(&c) {
                    anchor_vars.push(s.0);
                }
            }
            TermRef::Int(_) | TermRef::Str(_) => {}
            _ => return false,
        }
    }
    let given: Vec<bool> = bind.iter().map(|b| !b.is_empty()).collect();
    // The rule's documents: its positive located reads' location columns, joined by `a = b`, `@doc` and `nearest`.
    let mut doc: Vec<u32> = (0..n as u32).collect();
    for (a, b) in super::var_equalities_in(v, rule) {
        join(&mut doc, a.0, b.0);
    }
    let located_vars = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| -> Vec<u32> {
        let args: Vec<TermRef<'_>> = a.args().collect();
        columns(a.rel().0).iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| s.0).collect()
    };
    for l in rule.body() {
        match l.view() {
            L::Atom(a) if !a.neg() && !placeless(a.rel().0) => {
                let vs = located_vars(a);
                for w in vs.windows(2) {
                    join(&mut doc, w[0], w[1]);
                }
            }
            L::Nearest { n: x, a: y, .. } => join(&mut doc, x.0, y.0),
            _ => {}
        }
    }
    let anchored_roots: BTreeSet<u32> = anchor_vars.iter().map(|&v| find(&mut doc, v)).collect();
    let mut anchored = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| located_vars(a).into_iter().any(|v| anchored_roots.contains(&find(&mut doc, v)));
    fn vars(t: TermRef<'_>, out: &mut Vec<u32>) {
        match t {
            TermRef::Var(s) => out.push(s.0),
            TermRef::Call(_, xs) => xs.for_each(|x| vars(x, out)),
            _ => {}
        }
    }
    let mut mapped = Vec::new();
    let mut in_mapped = vec![false; n];
    let mut others: Vec<Vec<u32>> = Vec::new();
    for l in rule.body() {
        let mut vs = Vec::new();
        match l.view() {
            L::Atom(a) if !a.neg() && !placeless(a.rel().0) => {
                if anchored(a) {
                    a.args().for_each(|t| vars(t, &mut vs));
                    others.push(vs);
                    continue;
                }
                for t in a.args() {
                    match t {
                        TermRef::Var(s) => in_mapped[rc[s.0 as usize] as usize] = true,
                        TermRef::Wild | TermRef::Int(_) | TermRef::Str(_) => {}
                        _ => return false,
                    }
                }
                mapped.push(a);
            }
            L::Atom(a) => {
                if a.neg() && !placeless(a.rel().0) && !anchored(a) {
                    return false;
                }
                a.args().for_each(|t| vars(t, &mut vs));
                others.push(vs);
            }
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } if rc[a.0 as usize] == rc[b.0 as usize] => {}
            L::Cmp { l, r, .. } => {
                vars(l, &mut vs);
                vars(r, &mut vs);
                others.push(vs);
            }
            L::Nearest { .. } => {}
            _ => return false,
        }
    }
    if others.iter().flatten().any(|&s| {
        let c = rc[s as usize] as usize;
        in_mapped[c] && !given[c]
    }) {
        return false;
    }
    let targets: Vec<_> = reader
        .body()
        .enumerate()
        .filter_map(|(i, l)| match l.view() {
            L::Atom(a) if !a.neg() && i != at => Some(a),
            _ => None,
        })
        .collect();
    // Backtracking over the mapped reads, each to a target of its relation; bounded, a failure past the bound.
    fn go(k: usize, mapped: &[crate::code_rule_dsl::logic::program_image::AtomRef<'_>], targets: &[crate::code_rule_dsl::logic::program_image::AtomRef<'_>], bind: &mut Vec<Vec<T>>, rc: &[u32], target: &dyn Fn(TermRef<'_>) -> Option<T>, steps: &mut usize) -> bool {
        let Some(a) = mapped.get(k) else { return true };
        for b in targets.iter().filter(|b| b.rel().0 == a.rel().0) {
            *steps += 1;
            if *steps > 4096 {
                return false;
            }
            let saved = bind.clone();
            let fits = a.args().zip(b.args()).all(|(x, y)| match x {
                TermRef::Wild => true,
                TermRef::Int(i) => target(y) == Some(T::Int(i)),
                TermRef::Str(s) => target(y) == Some(T::Str(s.0)),
                TermRef::Var(s) => {
                    let c = rc[s.0 as usize] as usize;
                    match target(y) {
                        None => false,
                        Some(u) if bind[c].is_empty() => {
                            bind[c].push(u);
                            true
                        }
                        Some(u) => bind[c].contains(&u),
                    }
                }
                _ => false,
            });
            if fits && go(k + 1, mapped, targets, bind, rc, target, steps) {
                return true;
            }
            *bind = saved;
        }
        false
    }
    let mut steps = 0usize;
    go(0, &mapped, &targets, &mut bind, &rc, &target, &mut steps)
}

/// What passes between a split's parts (`file_local::crossing`), read from the version: the exports, the base
/// relations the cross-file part reads (and those it decodes), and the file-local relations it derives itself from a
/// stored input.
pub(super) struct Crossing {
    pub(super) exports: Vec<String>,
    pub(super) base_read: Vec<String>,
    pub(super) base: Vec<String>,
    pub(super) projected: BTreeSet<String>,
}

pub(super) fn crossing(v: &Version, f: Fields<'_>, local: &BTreeSet<String>) -> Crossing {
    let is_local = |rel: &str| local.contains(rel);
    let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
    let read = |keep: &dyn Fn(&str) -> bool| -> BTreeSet<String> {
        v.rules().filter(|r| keep(head(r))).flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>()).collect()
    };
    let cross_reads = read(&|h| !is_local(h));
    let projection = |rel: &str| {
        let Some(id) = v.rel(rel) else { return false };
        let mut rules = v.heads(id).peekable();
        rules.peek().is_some()
            && !f.lattice(rel)
            && rules.all(|r| {
                let mut body = r.body();
                match (body.next().map(|l| l.view()), body.next()) {
                    (Some(L::Atom(a)), None) => !a.neg() && f.is_input(v.rel_name(a.rel())),
                    _ => false,
                }
            })
    };
    let projected: BTreeSet<String> = cross_reads.iter().filter(|r| is_local(r) && projection(r)).cloned().collect();
    let in_cross = |rel: &str| !is_local(rel) || projected.contains(rel);
    let mut cross_reads = read(&|h| in_cross(h));
    cross_reads.extend(f.natives().iter().filter(|n| in_cross(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r).to_owned()));
    let exports: Vec<String> = cross_reads.iter().filter(|r| is_local(r) && !projected.contains(*r)).cloned().collect();
    let base_read: Vec<String> = f.inputs().filter(|i| cross_reads.contains(*i)).map(str::to_owned).collect();
    let mut base = base_read.clone();
    for decode in ["node", "doc"] {
        if f.is_input(decode) && !base.iter().any(|b| b == decode) {
            base.push(decode.to_owned());
        }
    }
    Crossing { exports, base_read, base, projected }
}

/// A split's relations by `RelId` (`shape::Rels`): what the split asks of each, asked once.
struct Rels<'v> {
    n: usize,
    derived_count: usize,
    derived: Vec<bool>,
    fact: Vec<bool>,
    run_constant: Vec<bool>,
    chunk_value: Vec<bool>,
    test: Vec<bool>,
    /// Placed where its inputs are, and read as a test is where it is per-file: a native function of strings
    /// (`match_all`).
    functional: Vec<bool>,
    text: Vec<Option<&'static [usize]>>,
    base_locs: Vec<Vec<usize>>,
    candidate: Vec<bool>,
    output: Vec<bool>,
    /// A derived relation's rules, by index in the program's order.
    rules: Vec<Vec<usize>>,
    /// The rules reading each relation, by index, each once, in order.
    reading: Vec<Vec<usize>>,
    /// Each rule's head.
    head: Vec<u32>,
    _v: std::marker::PhantomData<&'v ()>,
}

impl<'v> Rels<'v> {
    fn of(v: &'v Version, f: Fields<'v>, rules: &[RuleRef<'v>]) -> Rels<'v> {
        let n = v.rel_end() as usize;
        let mut derived = vec![false; n];
        let mut by_head: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut reading: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut read = vec![false; n];
        let head: Vec<u32> = rules.iter().map(|r| r.head().rel().0).collect();
        for (k, r) in rules.iter().enumerate() {
            let h = head[k] as usize;
            derived[h] = true;
            by_head[h].push(k);
            for a in r.reads() {
                let at = a.rel().0 as usize;
                read[at] = true;
                if reading[at].last() != Some(&k) {
                    reading[at].push(k);
                }
            }
        }
        let p = Program { v, f, rules: Vec::new(), rels: Rels::empty() };
        let name = |r: usize| v.rel_name(RelId(r as u32));
        let base = |r: usize| !derived[r] && read[r];
        let test = |r: &str| r.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
        let functional = |r: &str| f.native(r).is_some_and(|nat| matches!(nat.spec, NativeSpec::MatchAll { .. }));
        Rels {
            n,
            derived_count: derived.iter().filter(|d| **d).count(),
            fact: (0..n).map(|r| base(r) && p.fact(name(r)) && !f.is_input(name(r))).collect(),
            run_constant: (0..n).map(|r| (derived[r] || read[r]) && p.run_constant(name(r))).collect(),
            chunk_value: (0..n).map(|r| base(r) && CHUNK_VALUES.contains(&name(r))).collect(),
            test: (0..n).map(|r| (derived[r] || read[r]) && test(name(r))).collect(),
            functional: (0..n).map(|r| derived[r] && functional(name(r))).collect(),
            text: (0..n).map(|r| if derived[r] || read[r] { crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(t, _)| *t == name(r)).map(|(_, c)| *c) } else { None }).collect(),
            base_locs: (0..n).map(|r| if base(r) { p.base_locations(name(r)) } else { Vec::new() }).collect(),
            candidate: (0..n).map(|r| derived[r] && f.native(name(r)).is_none() && !f.lattice(name(r)) && !f.is_input(name(r))).collect(),
            output: (0..n).map(|r| derived[r] && f.is_output(name(r))).collect(),
            rules: by_head,
            reading,
            head,
            derived,
            _v: std::marker::PhantomData,
        }
    }

    fn empty() -> Rels<'v> {
        Rels { n: 0, derived_count: 0, derived: Vec::new(), fact: Vec::new(), run_constant: Vec::new(), chunk_value: Vec::new(), test: Vec::new(), functional: Vec::new(), text: Vec::new(), base_locs: Vec::new(), candidate: Vec::new(), output: Vec::new(), rules: Vec::new(), reading: Vec::new(), head: Vec::new(), _v: std::marker::PhantomData }
    }

    fn is_derived(&self, r: u32) -> bool {
        self.derived[r as usize]
    }

    fn reading(&self, r: u32) -> &[usize] {
        &self.reading[r as usize]
    }

    /// The derived relations whose rules read any of `rels`, each once, in order.
    fn heads_reading(&self, rels: impl Iterator<Item = u32>, keep: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut out: Vec<u32> = rels.flat_map(|r| self.reading[r as usize].iter().map(|&k| self.head[k])).filter(|&h| keep(h)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Each rule's form (`Form`), made the first time a split asks it.
struct Forms<'p, 'v> {
    p: &'p Program<'v>,
    made: Vec<std::sync::OnceLock<Form>>,
}

impl<'p, 'v> Forms<'p, 'v> {
    fn new(p: &'p Program<'v>) -> Forms<'p, 'v> {
        Forms { p, made: (0..p.rules.len()).map(|_| std::sync::OnceLock::new()).collect() }
    }
    fn get(&self, k: usize) -> &Form {
        self.made[k].get_or_init(|| Form::of(self.p.v, self.p.rules[k]))
    }
}

/// The value tapes of a rule (`file_local::ValueTapes`), by slot: each class under the rule's `a = b` named by its
/// least slot.
struct ValueTapes {
    /// Each value tape's class and the body index of the atom that gave its text.
    by_binder: BTreeMap<u32, usize>,
    /// Each spliced tape's node and tape class.
    splices: Vec<(Slot, u32)>,
    class: Vec<u32>,
}

impl ValueTapes {
    fn of(v: &Version, rule: RuleRef<'_>) -> ValueTapes {
        let n = rule.var_count();
        let mut root: Vec<u32> = (0..n as u32).collect();
        for l in rule.body() {
            if let L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
                join(&mut root, a.0, b.0);
            }
        }
        let class: Vec<u32> = (0..n as u32).map(|s| find(&mut root, s)).collect();
        let mut out = ValueTapes { by_binder: BTreeMap::new(), splices: Vec::new(), class };
        let body: Vec<L<'_>> = rule.body().map(|l| l.view()).collect();
        for l in &body {
            let L::Atom(a) = l else { continue };
            if a.neg() {
                continue;
            }
            let args: Vec<TermRef<'_>> = a.args().collect();
            match (v.rel_name(a.rel()), args.as_slice()) {
                ("parse_of", [TermRef::Var(x), _, TermRef::Var(t)]) => {
                    let x = out.class[x.0 as usize];
                    let binders: Vec<usize> = body
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| match l {
                            L::Atom(b) => {
                                let rel = v.rel_name(b.rel());
                                !b.neg() && !CHUNK_VALUES.contains(&rel) && rel != "tape_of" && b.args().any(|t| t.var().is_some_and(|s| out.class[s.0 as usize] == x))
                            }
                            _ => false,
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if let [b] = binders[..] {
                        out.by_binder.insert(out.class[t.0 as usize], b);
                    }
                }
                ("splice_of", [_, TermRef::Var(n), _, TermRef::Var(t3)]) => out.splices.push((*n, out.class[t3.0 as usize])),
                // A guest's tape stands with its node: a guest and its host are one file (decision 0055), as a splice's
                // tape with its node (decision 0652: the lowering's finding for a guest that does not parse).
                ("sub_tape", [TermRef::Var(n), _, TermRef::Var(t)]) => out.splices.push((*n, out.class[t.0 as usize])),
                _ => {}
            }
        }
        out
    }

    /// `s`'s value tape (its class), when it is one.
    fn canonical(&self, s: Slot) -> Option<u32> {
        let c = self.class[s.0 as usize];
        (self.by_binder.contains_key(&c) || self.splices.iter().any(|(_, t)| *t == c)).then_some(c)
    }
}

/// A body literal as the split reads it (`shape::Lit`).
enum Lit {
    Read { neg: bool, plain: bool, rel: u32, args: Vec<Option<u32>>, binder: bool, tapes: Vec<u32> },
    Ast,
    Nearest(u32, u32),
    Cmp,
    /// `@doc(a) = @doc(b)`, or `d = @doc(n)`: the two classes are of one document (decision 0652), as `docof` says.
    Same(u32, u32),
}

/// A side of `@doc(a) = @doc(b)` or `d = @doc(n)`: its variable, and whether it is the `@doc` call.
fn doc_side(v: &Version, t: TermRef<'_>) -> Option<(Slot, bool)> {
    match t {
        TermRef::Var(s) => Some((s, false)),
        TermRef::Call(f, mut args) if v.text(f) == "doc" => match (args.next(), args.next()) {
            (Some(TermRef::Var(s)), None) => Some((s, true)),
            _ => None,
        },
        _ => None,
    }
}

/// A rule body numbered for the split (`shape::Form`): its slots, each with its class under the body's equalities;
/// then its value tapes; then one id per body literal.
struct Form {
    /// Whether a term of the body names the slot (a head's variable is found only then, as the tree's `Form` finds it
    /// by name).
    named: Vec<bool>,
    class: Vec<u32>,
    lits: Vec<Lit>,
    splices: Vec<(u32, u32)>,
    ids: usize,
    binders: u32,
}

impl Form {
    fn of(v: &Version, rule: RuleRef<'_>) -> Form {
        let n = rule.var_count();
        let mut named = vec![false; n];
        fn terms(t: TermRef<'_>, named: &mut [bool]) {
            match t {
                TermRef::Var(s) => named[s.0 as usize] = true,
                TermRef::Call(_, xs) => xs.for_each(|x| terms(x, named)),
                _ => {}
            }
        }
        for l in rule.body() {
            match l.view() {
                L::Atom(a) | L::Agg { atom: a, .. } => a.args().for_each(|t| terms(t, &mut named)),
                L::Cmp { l, r, .. } => {
                    terms(l, &mut named);
                    terms(r, &mut named);
                }
                _ => {}
            }
        }
        let mut root: Vec<u32> = (0..n as u32).collect();
        for (a, b) in super::var_equalities_in(v, rule) {
            join(&mut root, a.0, b.0);
        }
        let class: Vec<u32> = (0..n as u32).map(|s| find(&mut root, s)).collect();
        let placed = ValueTapes::of(v, rule);
        // The value tapes, numbered after the slots.
        let mut tapes: BTreeMap<u32, u32> = BTreeMap::new();
        let mut tape = |t: u32| -> u32 {
            let k = n as u32 + tapes.len() as u32;
            *tapes.entry(t).or_insert(k)
        };
        let splices: Vec<(u32, u32)> = placed.splices.iter().map(|&(s, t)| (class[s.0 as usize], tape(t))).collect();
        let mut lits = Vec::new();
        for (index, l) in rule.body().enumerate() {
            lits.push(match l.view() {
                lv @ (L::Atom(_) | L::Agg { .. }) => {
                    let (a, plain) = match lv {
                        L::Atom(a) => (a, true),
                        L::Agg { atom, .. } => (atom, false),
                        _ => unreachable!(),
                    };
                    let args: Vec<TermRef<'_>> = a.args().collect();
                    let mut stand: Vec<u32> = placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| tape(*t)).collect();
                    if v.rel_name(a.rel()) == "tape_of"
                        && let Some(t) = args.get(1).and_then(|t| t.var())
                        && let Some(t) = placed.canonical(t)
                    {
                        stand.push(tape(t));
                    }
                    Lit::Read {
                        neg: a.neg(),
                        plain,
                        rel: a.rel().0,
                        args: args.iter().map(|t| t.var().map(|s| s.0)).collect(),
                        binder: placed.by_binder.values().any(|b| *b == index),
                        tapes: stand,
                    }
                }
                L::Ast { .. } => Lit::Ast,
                L::Nearest { n, a, .. } => Lit::Nearest(class[n.0 as usize], class[a.0 as usize]),
                L::Cmp { l, op: CmpOp::Eq, r } => match (doc_side(v, l), doc_side(v, r)) {
                    (Some((a, x)), Some((b, y))) if x || y => Lit::Same(class[a.0 as usize], class[b.0 as usize]),
                    _ => Lit::Cmp,
                },
                L::Cmp { .. } => Lit::Cmp,
            });
        }
        let binders = (n + tapes.len()) as u32;
        let ids = binders as usize + lits.len();
        Form { named, class, lits, splices, ids, binders }
    }

    /// The slot a head argument is, when the body names it.
    fn var(&self, t: TermRef<'_>) -> Option<u32> {
        t.var().map(|s| s.0).filter(|&s| self.named.get(s as usize).copied().unwrap_or(false))
    }

    fn reads_constants(&self, rels: &Rels<'_>, constant: &[bool]) -> bool {
        self.lits.iter().all(|l| match l {
            Lit::Read { rel, .. } => {
                let r = *rel as usize;
                constant[r] || rels.run_constant[r] || rels.fact[r]
            }
            Lit::Cmp | Lit::Same(..) => true,
            _ => false,
        })
    }

    fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vec<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l {
                let columns = if rels.is_derived(*rel) { &locs[*rel as usize] } else { &rels.base_locs[*rel as usize] };
                for &c in columns {
                    if let Some(Some(v)) = args.get(c) {
                        out.set(self.class[*v as usize]);
                    }
                }
            }
        }
        out
    }

    /// `placed_classes` with the classes nullable location columns place too (decision 0649).
    fn placed_or_null(&self, rels: &Rels<'_>, locs: &[Vec<usize>], nulls: &[Vec<usize>]) -> Bits {
        let mut out = self.placed_classes(rels, locs);
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l
                && rels.is_derived(*rel)
            {
                for &c in &nulls[*rel as usize] {
                    if let Some(Some(v)) = args.get(c) {
                        out.set(self.class[*v as usize]);
                    }
                }
            }
        }
        out
    }

    /// Head column `c` is a placed variable, or `unset`'s -1.
    fn head_placed_or_unset(&self, rule: RuleRef<'_>, c: usize, placed: &Bits) -> bool {
        matches!(rule.head().args().nth(c), Some(TermRef::Int(crate::code_rule_dsl::logic::rr::lower::UNSET))) || self.head_placed(rule, c, placed)
    }

    fn head_placed(&self, rule: RuleRef<'_>, c: usize, placed: &Bits) -> bool {
        rule.head().args().nth(c).and_then(|t| self.var(t)).is_some_and(|v| placed.has(self.class[v as usize]))
    }

    fn filters(&self, rule: RuleRef<'_>, rels: &Rels<'_>, filters: &[bool]) -> bool {
        let mut strings = Bits::new(self.ids);
        let reads_only_text = self.lits.iter().all(|l| match l {
            Lit::Read { plain: true, neg, rel, args, .. } => {
                let r = *rel;
                if rels.test[r as usize] || (rels.is_derived(r) && filters[r as usize]) {
                    true
                } else if let Some(cols) = rels.text[r as usize].filter(|_| !neg) {
                    cols.iter().filter_map(|&c| args.get(c).copied().flatten()).for_each(|v| strings.set(v));
                    true
                } else {
                    false
                }
            }
            Lit::Cmp | Lit::Same(..) => true,
            _ => false,
        });
        reads_only_text && rule.head().args().all(|t| self.var(t).is_some_and(|v| strings.has(v)))
    }

    fn one_document(&self, head: u32, rels: &Rels<'_>, placeless: &[bool], local: &[bool], filters: &[bool], covered: &[bool], locs: &[Vec<usize>], nulls: &[Vec<usize>]) -> bool {
        let head_filter = filters[head as usize];
        // A nullable column joins its row's group only where a location column of this rule places its variable
        // (decision 0649): then it is no -1.
        let strict = self.placed_classes(rels, locs);
        // A negated read of a nullable column's variable that no location column places (decision 0652): where it
        // is -1 the negated read matches no row and reads no document, and otherwise it is a node of the row's
        // document, so the negated read is placed with the positive row that holds the column.
        let mut nullable_row: Vec<(u32, u32)> = Vec::new();
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l
                && rels.is_derived(*rel)
                && let Some(&anchor) = locs[*rel as usize].iter().find_map(|&c| args.get(c).copied().flatten()).as_ref()
            {
                for &c in &nulls[*rel as usize] {
                    if let Some(v) = args.get(c).copied().flatten()
                        && !strict.has(self.class[v as usize])
                    {
                        nullable_row.push((self.class[v as usize], self.class[anchor as usize]));
                    }
                }
            }
        }
        let mut root: Vec<u32> = (0..self.ids as u32).collect();
        let mut firsts: Vec<u32> = Vec::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        let mut group: Vec<u32> = Vec::new();
        for (index, l) in self.lits.iter().enumerate() {
            match l {
                Lit::Read { neg, plain, rel, args, binder, tapes } => {
                    let r = *rel;
                    let derived = rels.is_derived(r);
                    // A function of strings where it is per-file is read as a test is.
                    if placeless[r as usize] || (derived && filters[r as usize]) || covered[r as usize] || head_filter || (rels.functional[r as usize] && local[r as usize]) {
                        continue;
                    }
                    if derived && !local[r as usize] {
                        return false;
                    }
                    let columns = if derived { &locs[r as usize] } else { &rels.base_locs[r as usize] };
                    group.clear();
                    group.extend(columns.iter().filter_map(|&c| args.get(c).copied().flatten().map(|v| self.class[v as usize])));
                    if *neg {
                        for k in group.iter_mut() {
                            if let Some(&(_, anchor)) = nullable_row.iter().find(|(x, _)| x == k) {
                                *k = anchor;
                            }
                        }
                    }
                    if derived {
                        group.extend(nulls[r as usize].iter().filter_map(|&c| args.get(c).copied().flatten().map(|v| self.class[v as usize])).filter(|&k| strict.has(k)));
                    }
                    if group.is_empty() && *plain && *binder && !columns.is_empty() {
                        group.push(self.binders + index as u32);
                    }
                    if !group.is_empty() {
                        group.extend(tapes.iter().copied());
                    }
                    let Some(&first) = group.first() else { return false };
                    for &g in &group[1..] {
                        join(&mut root, first, g);
                    }
                    firsts.push(first);
                }
                Lit::Ast => return false,
                Lit::Nearest(n, a) => {
                    join(&mut root, *n, *a);
                    firsts.push(*n);
                }
                Lit::Cmp => {}
                Lit::Same(a, b) => join(&mut root, *a, *b),
            }
        }
        let mut roots: Vec<u32> = firsts.iter().map(|&f| find(&mut root, f)).collect();
        roots.sort_unstable();
        roots.dedup();
        roots.len() == 1 || (roots.is_empty() && head_filter)
    }
}

fn find(root: &mut [u32], mut x: u32) -> u32 {
    while root[x as usize] != x {
        root[x as usize] = root[root[x as usize] as usize];
        x = root[x as usize];
    }
    x
}

fn join(root: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find(root, a), find(root, b));
    if a != b {
        root[a.max(b) as usize] = a.min(b);
    }
}

/// A set of a body's ids.
struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        self.0[i as usize / 64] |= 1 << (i % 64);
    }
    fn has(&self, i: u32) -> bool {
        self.0.get(i as usize / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
}
