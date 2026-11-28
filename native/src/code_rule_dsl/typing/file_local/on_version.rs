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
//! The tree's split still runs where a pass reads the tree (`split_located`, `locate`); nothing here checks that the
//! two agree.

use super::{CHUNK_VALUES, Setting, document_column, option_input, path_key_column};
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
    /// Each derived relation's path keys (decision 0665): columns every rule fills from a file's path (`doc`'s:
    /// `path_key_column`) or from a location, beyond its `locations`. A path places a rule's
    /// document as a location does, so the split joins through it; a memo reads a row's document off a location's
    /// value, which a path is not, so `locations` (and `same_doc`, `row_documents`) leave it out, and `locate` names
    /// the document a path key's row is of through `doc` (`passes::Locate`).
    pub path_keys: BTreeMap<String, Vec<usize>>,
    /// The path keys that hold a file's path as it is, not a name built from one (`cat(p, "::", q)`, `name_built`):
    /// those every rule fills from `doc`'s path, or from such a key, unchanged. `locate` finds a row's document by
    /// one, through `doc(d, p, …)`, which a built name is not (decision 0665): a key built alone places a row in one
    /// document, but names no document `doc` can find.
    pub paths: BTreeMap<String, Vec<usize>>,
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
            && self.path_keys == o.path_keys
            && self.paths == o.paths
            && self.no_document == o.no_document
            && self.keys == o.keys
    }
}

impl FileLocal {
    /// A derived relation's columns that place its row's document (decision 0665): its locations and its path keys.
    pub fn placing(&self, rel: &str) -> Vec<usize> {
        let mut c: Vec<usize> = self.locations.get(rel).into_iter().chain(self.path_keys.get(rel)).flatten().copied().collect();
        c.sort_unstable();
        c.dedup();
        c
    }

    /// What the parts are made of (`programs_of`): the file-local relations and those of constants; none of either for
    /// a program that reads no document, whose relations are all cross-file.
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
            p.rels.is_derived(r) && !self.nullable.contains_key(rel) && (self.covered.contains(rel) || !self.local.contains(rel) || self.placing(rel).is_empty())
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
        option_input(rel) || Setting::of(self.f.qualifiers_of(rel)).is_some() || (crate::code_rule_dsl::logic::plan::corpus::phases::is_phased(rel) && self.f.is_input(rel))
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

    /// A base relation's columns that place a rule's document (decision 0665): its locations (`base_locations`) and its
    /// path key (`path_key_column`).
    fn base_placing(&self, rel: &str) -> Vec<usize> {
        let mut columns = self.base_locations(rel);
        if let Some(c) = path_key_column(rel).filter(|c| !columns.contains(c)) {
            columns.push(c);
        }
        columns
    }

    /// Whether no rule reads a document: every relation a rule reads is derived, a fact, or an input with one value for
    /// the run (`run_constant`), and no rule matches an AST pattern or asks for the nearest of a node. Such a program
    /// reads an earlier phase's exports and the run's options at most, and reads at least one of them.
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
        // A derived relation's placing columns: those every rule fills from a location or a file's path (decision
        // 0665). The split places by these; what it publishes as locations is those filled from a location (`real`).
        let mut locs: Vec<Vec<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vec::new();
                }
                let rel = self.name(r as u32);
                let kept = was(r).filter(|p| p.locations.contains_key(rel)).map(|p| p.placing(rel));
                kept.unwrap_or_else(|| (0..self.f.width(rel)).collect())
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
        // The location columns among them: those every rule fills from a location alone (a node, a document), which a
        // row's document is read off. A column filled from a path somewhere is a path key (`FileLocal::path_keys`).
        let mut real: Vec<Vec<usize>> = (0..n)
            .map(|r| match was(r) {
                Some(p) => p.locations.get(self.name(r as u32)).cloned().unwrap_or_default(),
                None => locs[r].clone(),
            })
            .collect();
        let mut round: Vec<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && !real[r as usize].is_empty()).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vec<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes_over(rels, &real, &rels.base_real)).collect();
                    let kept: Vec<usize> = real[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed(self.rules[k], c, p))).collect();
                    (kept != real[r as usize]).then_some((r, kept))
                })
                .collect();
            let next = rels.heads_reading(changed.iter().map(|(r, _)| *r), |h| in_scope[h as usize]);
            for (r, kept) in changed {
                real[r as usize] = kept;
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
        // The split places by `placing`; it publishes `locations` and `path_keys` (decision 0665).
        let placing: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        let locations: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), real[r as usize].clone())).collect();
        let path_keys: BTreeMap<String, Vec<usize>> = derived()
            .map(|r| (self.name(r).to_owned(), locs[r as usize].iter().copied().filter(|c| !real[r as usize].contains(c)).collect::<Vec<usize>>()))
            .filter(|(_, c)| !c.is_empty())
            .collect();
        // Which path keys are paths as they are (`FileLocal::paths`): the fixpoint `real`'s is, from the path keys down,
        // over `doc`'s path and the path columns alone, a built name (`placed_classes`' `built`) not counted.
        let base_paths: Vec<Vec<usize>> = (0..n).map(|r| rels.base_locs[r].iter().copied().filter(|c| !rels.base_real[r].contains(c)).collect()).collect();
        let mut plain: Vec<Vec<usize>> = (0..n)
            .map(|r| match was(r) {
                Some(p) => p.paths.get(self.name(r as u32)).cloned().unwrap_or_default(),
                None => locs[r].iter().copied().filter(|c| !real[r].contains(c)).collect(),
            })
            .collect();
        let mut round: Vec<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && !plain[r as usize].is_empty()).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vec<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes_over(rels, &plain, &base_paths)).collect();
                    let kept: Vec<usize> = plain[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed(self.rules[k], c, p))).collect();
                    (kept != plain[r as usize]).then_some((r, kept))
                })
                .collect();
            let next = rels.heads_reading(changed.iter().map(|(r, _)| *r), |h| in_scope[h as usize]);
            for (r, kept) in changed {
                plain[r as usize] = kept;
            }
            round = next;
        }
        let paths: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), plain[r as usize].clone())).filter(|(_, c)| !c.is_empty()).collect();
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
        // A relation `locate_all` moved (`CROSS_FILE`): its rows name no document, so it is derived once over every file
        // (decision 0290). One the cross part reads back from its located form (`READ_LOCATED`, decision 0665).
        let moved = |rel: &str| self.f.qualified(rel, super::CROSS_FILE) || self.f.qualified(rel, super::READ_LOCATED);
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
                    None if self.f.qualified(rel, super::READ_LOCATED) => Some((rel.to_owned(), "read back from its located form (`locate`)".to_owned())),
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
            let keyed = self.keyed(&local_now, &valueless, &placeless, &locs);
            while let Some(r) = asked.pop() {
                if !covered_now[r as usize] {
                    continue;
                }
                anchoring.memo.borrow_mut().clear();
                let got = match keyed[r as usize] {
                    Some(root) if root != r => self.covers_keyed(r, root, &keyed, &local_now, &placeless, &locs),
                    _ => self.covers(r, &local_now, &covered_now, &valueless, &placeless, &locs, &anchoring),
                };
                if got.is_err() {
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
            let explain = |k: usize, local_now: &[bool]| self.why_not_local(self.rules[k], local_now, &constants, &read_as_tests, &placing, &nullable);
            let check = |k: usize| -> Option<(u32, String)> {
                let head = rels.head[k];
                if (covered_now[head as usize] && local_now[head as usize]) || rels.never[k] {
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
            if left {
                // Anchoring can depend on a reader through several located views, not only a direct read.
                // Reconsider those covers against the new local set before keeping any of their readers local.
                for r in (0..n).filter(|&r| in_scope[r] && covered_now[r]) {
                    next.extend(rels.rules[r].iter().copied());
                }
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
        // Its reads of documents' rows. None: the run's inputs alone, the run's rows. One plain positive read: each row
        // is one file's, a union (as the cross part reads the chunks' findings back). Neither is a crossing; documents
        // meet only where a rule joins two reads of them, or aggregates one.
        let union = |k: usize| {
            let documents: Vec<&Lit> = forms.get(k).lits.iter().filter(|l| match l {
                Lit::Read { rel: x, .. } => !placeless_or_input[*x as usize],
                Lit::Ast | Lit::Nearest(..) => true,
                Lit::Cmp | Lit::Same(..) => false,
            }).collect();
            matches!(documents.as_slice(), [] | [Lit::Read { neg: false, plain: true, .. }])
        };
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
                if reads_cross || union(k) || form.one_document(r, rels, &placeless_or_input, &local_now, &filtering, &skip, &locs, &nulls) {
                    continue;
                }
                let why = self.why_not_local(self.rules[k], &local_now, &constants, &filters, &placing, &nullable).unwrap_or_else(|| "places more than one document".to_owned());
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
        // Their location columns alone (decision 0665): a demand ties two by `@doc`, which a path is not.
        let row_documents: BTreeMap<String, Vec<usize>> = derived().filter(|&r| one_row[r as usize] && real[r as usize].len() > 1).map(|r| (self.name(r).to_owned(), real[r as usize].clone())).collect();
        // The same-document columns and the rest: `finish`.
        self.finish(FileLocal { local, constants, locations, filters, causes, nullable, crossings, covered, cover_deps, row_documents, path_keys, paths, ..Default::default() }, self.keys())
    }

    /// The keyed families of relations of values: by relation, its family's root. A root is a file-local relation of
    /// one column and values alone (`brace_value(v) :- format_call(_, v)`, `num_source(t)`), its own root. A member is a
    /// candidate view of values (`valueless`) each rule of which fills its first column with a variable `x` and reads
    /// only its family at `x` (in its first column; positive, negated or aggregated: `brace_rest(v, r, i)`,
    /// `!brace_bad(v)`, `count : { brace_arg(v, k, "") }`), placeless relations and comparisons, the family having one
    /// root. A member's rows at a key follow from its root's row at that key alone, so they are the same in a chunk as
    /// over every file wherever the chunk holds that root row (`covers_keyed`). The greatest such assignment.
    fn keyed(&self, local_now: &[bool], valueless: &[bool], placeless: &[bool], locs: &[Vec<usize>]) -> Vec<Option<u32>> {
        let rels = &self.rels;
        let rootable = |r: usize| self.rootable(r, local_now, placeless, locs);
        // `None`: not in a family; `Some(None)`: in one, its root not yet known; `Some(Some(d))`: of root `d`.
        let mut at: Vec<Option<Option<u32>>> = (0..rels.n).map(|r| (valueless[r] || rootable(r)).then_some(None)).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for r in 0..rels.n {
                if at[r].is_none() || at[r] == Some(Some(r as u32)) {
                    continue;
                }
                let next = family_next(r, self.family_roots(r, &at, placeless), rootable(r));
                if next != at[r] && !(next == Some(None) && at[r].is_some()) {
                    at[r] = next;
                    changed = true;
                }
            }
        }
        at.into_iter().map(|x| x.flatten()).collect()
    }

    /// A relation that may be a keyed family's root: one that is no member is one, where it is file-local by its own
    /// rules, of one column and no location.
    fn rootable(&self, r: usize, local_now: &[bool], placeless: &[bool], locs: &[Vec<usize>]) -> bool {
        let rels = &self.rels;
        let local_values = rels.is_derived(r as u32) && local_now[r] && locs[r].is_empty() && !placeless[r];
        local_values && !rels.rules[r].is_empty() && self.f.width(self.name(r as u32)) == 1
    }

    /// The roots of the family reads of `r`'s rules, each of which reads only its family at its key (`keyed`); `None`
    /// where one does not.
    fn family_roots(&self, r: usize, at: &[Option<Option<u32>>], placeless: &[bool]) -> Option<BTreeSet<u32>> {
        let mut roots = BTreeSet::new();
        for &k in &self.rels.rules[r] {
            let rule = self.rules[k];
            let classes = value_classes(rule);
            let Some(TermRef::Var(x)) = rule.head().args().next() else { return None };
            let key = classes[x.0 as usize];
            for l in rule.body() {
                let a = match l.view() {
                    L::Atom(a) | L::Agg { atom: a, .. } => a,
                    L::Cmp { .. } => continue,
                    _ => return None,
                };
                let s = a.rel().0 as usize;
                let at_key = matches!(a.args().next(), Some(TermRef::Var(y)) if classes[y.0 as usize] == key);
                match (placeless[s], at[s]) {
                    (true, _) => {}
                    (false, Some(root)) if at_key => roots.extend(root),
                    _ => return None,
                }
            }
        }
        Some(roots)
    }

    /// Whether every reader of keyed member `r` (`keyed`, its root `root`) holds its root's row at the key it reads `r`
    /// at (`key_held`), so the chunk derives the root's row, and `r`'s rows at that key, as over every file.
    fn covers_keyed(&self, r: u32, root: u32, keyed: &[Option<u32>], local_now: &[bool], placeless: &[bool], locs: &[Vec<usize>]) -> Result<(), String> {
        let hold = KeyHold { root, keyed, local_now, placeless, locs };
        self.key_held(&hold, r, 0, &mut BTreeSet::new())
    }

    /// Whether every reader of `rel` holds `root`'s row at the value it reads in `rel`'s column `col`. Each is
    /// file-local; one in the root's family reads `rel` at its own key (`col` 0). Any other reads `rel` (plainly or
    /// aggregated) with a variable there, and either one of the root's rules maps into it exactly at that variable
    /// (`maps_at`: its own document holds the root's row), or its head passes the variable on (an aggregate's helper,
    /// `rr_h_agg_…(c, v, k) :- brace_name(v, k, _, _), …`) and every reader of the head holds the root's row there in
    /// turn. A relation met again is assumed to (its readers outside the cycle are asked); an output never does.
    fn key_held(&self, hold: &KeyHold<'_>, rel: u32, col: usize, seen: &mut BTreeSet<(u32, usize)>) -> Result<(), String> {
        let rels = &self.rels;
        if !seen.insert((rel, col)) {
            return Ok(());
        }
        if rels.output[rel as usize] {
            return Err(format!("{} is an output: its key is held by no reader", self.name(rel)));
        }
        let readers = rels.reading(rel);
        if readers.is_empty() {
            return Err(format!("{} has no readers", self.name(rel)));
        }
        readers.iter().try_for_each(|&j| self.reader_holds(hold, j, rel, col, seen))
    }

    /// `key_held` for one reader of `rel`, rule `j`.
    fn reader_holds(&self, hold: &KeyHold<'_>, j: usize, rel: u32, col: usize, seen: &mut BTreeSet<(u32, usize)>) -> Result<(), String> {
        let rels = &self.rels;
        let h = rels.head[j];
        if !hold.local_now[h as usize] {
            return Err(format!("reader {} not local", self.name(h)));
        }
        // A file-local member of the family reads it at its own key.
        if col == 0 && hold.keyed[h as usize] == Some(hold.root) {
            return Ok(());
        }
        let reader = self.rules[j];
        let qc = value_classes(reader);
        let columns = |x: u32| -> &[usize] { if rels.is_derived(x) { &hold.locs[x as usize] } else { &rels.base_locs[x as usize] } };
        let reads = reader.body().filter_map(|l| match l.view() {
            L::Atom(a) | L::Agg { atom: a, .. } if a.rel().0 == rel => Some(a),
            _ => None,
        });
        for a in reads {
            let Some(TermRef::Var(x)) = a.args().nth(col) else { return Err(format!("reader reads it at no variable: {}", self.v.show_rule(reader))) };
            let key = [Some(MapT::Var(qc[x.0 as usize]))];
            let placeless = |x: u32| hold.placeless[x as usize];
            if rels.rules[hold.root as usize].iter().any(|&k| maps_at(self.v, self.rules[k], reader, None, &key, &[], &[], &placeless, &columns, true)) {
                continue;
            }
            let mut passed = reader.head().args().enumerate().filter(|(_, t)| matches!(t, TermRef::Var(y) if qc[y.0 as usize] == qc[x.0 as usize])).map(|(c, _)| c);
            if !passed.any(|c| self.key_held(hold, h, c, seen).is_ok()) {
                return Err(format!("reader holds no row of {} at its key: {}", self.name(hold.root), self.v.show_rule(reader)));
            }
        }
        Ok(())
    }

    /// Whether every reader of `r` covers it (decision 0652). Each reader is a file-local rule reading `r` in a plain
    /// literal (positive or negated, not aggregated); each of `r`'s location columns it reads is a variable another of
    /// its positive reads places, or a location column of its head when its head is a located view covered in turn
    /// (whose readers place it). Each rule of `r` then finds its rows in the reader's (`maps`). Per chunk every atom
    /// of a rule's instance is the chunk's, so `r` derived per chunk holds at the reader's values exactly where it
    /// holds over every file: a rule's rows per chunk are among its rows over every file, and a row over every file,
    /// at the reader's values, has its body in the reader's own document's rows, so the chunk derives it too.
    fn covers(&self, r: u32, local_now: &[bool], covered: &[bool], valueless: &[bool], placeless: &[bool], locs: &[Vec<usize>], anchoring: &Anchoring<'_>) -> Result<(), String> {
        let cx = Cover { local_now, covered, valueless, placeless, locs, anchoring };
        let own = self.plain_own_rules(r, &cx)?;
        let readers = self.rels.reading(r);
        if readers.is_empty() {
            return Err("no readers".into());
        }
        readers.iter().try_for_each(|&j| self.reader_covers(r, j, &own, &cx))
    }

    /// `r`'s own rules, each reading file-local or placeless relations other than `r` alone, plainly, with
    /// comparisons and nearests.
    fn plain_own_rules(&self, r: u32, cx: &Cover<'_>) -> Result<Vec<RuleRef<'v>>, String> {
        let rels = &self.rels;
        let own: Vec<RuleRef<'v>> = rels.rules[r as usize].iter().map(|&k| self.rules[k]).collect();
        for l in own.iter().flat_map(|rule| rule.body()) {
            match l.view() {
                L::Atom(a) if a.rel().0 == r || (rels.is_derived(a.rel().0) && !cx.local_now[a.rel().0 as usize] && !cx.placeless[a.rel().0 as usize]) => {
                    return Err(format!("own rule reads {}", self.name(a.rel().0)));
                }
                L::Atom(_) | L::Cmp { .. } | L::Nearest { .. } => {}
                _ => return Err("own rule has an aggregate or pattern".into()),
            }
        }
        Ok(own)
    }

    /// `covers` for one reader of `r`, rule `j`.
    fn reader_covers(&self, r: u32, j: usize, own: &[RuleRef<'v>], cx: &Cover<'_>) -> Result<(), String> {
        let h = self.rels.head[j];
        // A cross-file reader covers a located view too, where its own reads place the view's location columns in
        // one document: the rows it joins are that document's, which the view per chunk derives. A view of values
        // maps into its reader's rows, which are one document's only in a file-local rule.
        let cross_reader = !cx.local_now[h as usize];
        if h == r || (cross_reader && cx.valueless[r as usize]) {
            return Err(format!("reader {} not local", self.name(h)));
        }
        let reader = self.rules[j];
        let placed = self.placed_classes(r, reader, h, cross_reader, cx);
        for (at, l) in reader.body().enumerate() {
            match l.view() {
                L::Atom(a) if a.rel().0 == r => self.read_covered(r, reader, at, a, &placed, own, cx)?,
                L::Agg { atom, .. } if atom.rel().0 == r => return Err("aggregated by a reader".into()),
                _ => {}
            }
        }
        Ok(())
    }

    /// The classes `reader`'s positive reads other than of `r` place, and its head's location columns where its
    /// readers place them (a file-local located view, `anchors`).
    fn placed_classes(&self, r: u32, reader: RuleRef<'v>, h: u32, cross_reader: bool, cx: &Cover<'_>) -> BTreeSet<u32> {
        let qc = value_classes(reader);
        let mut placed: BTreeSet<u32> = BTreeSet::new();
        let mut place = |args: Vec<TermRef<'_>>, cols: &[usize]| placed.extend(cols.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| qc[s.0 as usize]));
        if !cross_reader && !cx.valueless[h as usize] && self.anchors(h, cx) {
            place(reader.head().args().collect(), &cx.locs[h as usize]);
        }
        for l in reader.body() {
            if let L::Atom(a) = l.view()
                && !a.neg()
                && a.rel().0 != r
                && !cx.placeless[a.rel().0 as usize]
            {
                place(a.args().collect(), cx.columns(&self.rels, a.rel().0));
            }
        }
        placed
    }

    /// Whether file-local located view `h`'s readers place its head: covered already, or covered when asked here, once
    /// a round (a cycle back to a view being asked does not).
    fn anchors(&self, h: u32, cx: &Cover<'_>) -> bool {
        if cx.covered[h as usize] {
            return true;
        }
        if !cx.anchoring.candidate[h as usize] || !cx.local_now[h as usize] || cx.locs[h as usize].is_empty() {
            return false;
        }
        if let Some(&known) = cx.anchoring.memo.borrow().get(&h) {
            return known;
        }
        cx.anchoring.memo.borrow_mut().insert(h, false);
        let got = self.covers(h, cx.local_now, cx.covered, cx.valueless, cx.placeless, cx.locs, cx.anchoring).is_ok();
        cx.anchoring.memo.borrow_mut().insert(h, got);
        got
    }

    /// Whether `reader`'s read `a` of `r` (its literal `at`) is covered: each of `r`'s location columns placed, in one
    /// document for a cross-file reader, and each of `r`'s rules found in the reader (`maps`). A rule found nowhere in
    /// the reader is covered by one found there with the same head and the same literals beside its located reads
    /// (`same_rest`): a disjunction's branches over two reads (`negate(a, b) :- (number(_, a, _) ; number(_, _, a)),
    /// …`). Their tests are of the head's values alone, so where one's row is over every file the other's is in the
    /// reader's chunk.
    #[allow(clippy::too_many_arguments)]
    fn read_covered(&self, r: u32, reader: RuleRef<'v>, at: usize, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>, placed: &BTreeSet<u32>, own: &[RuleRef<'v>], cx: &Cover<'_>) -> Result<(), String> {
        let qc = value_classes(reader);
        let columns = |x: u32| cx.columns(&self.rels, x);
        let placeless = |x: u32| cx.placeless[x as usize];
        let args: Vec<TermRef<'_>> = a.args().collect();
        let at_cols = || columns(r).iter().filter_map(|&c| args.get(c).and_then(|t| t.var()));
        if !columns(r).iter().all(|&c| args.get(c).and_then(|t| t.var()).is_some_and(|s| placed.contains(&qc[s.0 as usize]))) {
            return Err(format!("reader leaves a location open: {}", self.v.show_rule(reader)));
        }
        // An anchored file-local reader's head is placed in one document by its own readers (`anchors`): its location
        // columns are tied together, as a read's are.
        let h = reader.head().rel().0;
        let anchored = cx.local_now[h as usize] && !cx.valueless[h as usize] && self.anchors(h, cx);
        let tied: Vec<Slot> = if anchored { cx.locs[h as usize].iter().filter_map(|&c| reader.head().args().nth(c).and_then(|t| t.var())).collect() } else { Vec::new() };
        if !one_place(self.v, reader, r, &placeless, &columns, &tied, at_cols()) {
            return Err(format!("reader does not tie the view's documents: {}", self.v.show_rule(reader)));
        }
        let found: Vec<bool> = own.iter().map(|&rule| maps(self.v, rule, reader, at, columns(r), &tied, &placeless, &columns)).collect();
        let covered_by_found = |rule: &RuleRef<'v>| own.iter().zip(&found).any(|(other, ok)| *ok && same_rest(self.v, *rule, *other, &placeless));
        match own.iter().zip(&found).find(|(rule, ok)| !**ok && !covered_by_found(rule)) {
            Some((rule, _)) => Err(format!("rule {} not found in reader {}", self.v.show_rule(*rule), self.v.show_rule(reader))),
            None => Ok(()),
        }
    }

    /// Why `rule` places more than one document, from its text (`split_in`'s `rule_local`): `None` where it does
    /// not. Asked only of a rule the numbered check fails (and, in a debug build, of each it passes).
    fn why_not_local(&self, rule: RuleRef<'v>, local_now: &[bool], constants: &BTreeSet<String>, filters: &BTreeSet<String>, locations: &BTreeMap<String, Vec<usize>>, nullable: &BTreeMap<String, Vec<usize>>) -> Option<String> {
        let v = self.v;
        if never_holds(&rule, |r| if self.rels.is_derived(r.0) { Vec::new() } else { unset_columns(v.rel_name(r), self.base_placing(v.rel_name(r))) }) {
            return None;
        }
        let strict = self.strict_vars(rule, locations);
        let text = RuleText { v, local_now, constants, filters, locations, nullable, nullable_row: self.nullable_rows(rule, locations, nullable, &strict), strict, placed: ValueTapes::of(v, rule) };
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        let mut groups: Vec<BTreeSet<String>> = text.placed.splices.iter().map(|&(n, t)| BTreeSet::from([var(n), text.tape(rule, t)])).collect();
        // `@doc(a) = @doc(b)` and `d = @doc(n)`: one document's (`Lit::Same`).
        let mut equal: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (var(a), var(b))).collect();
        for (index, l) in rule.body().enumerate() {
            match self.text_step(rule, index, l.view(), &text) {
                Step::Skip => {}
                Step::Group(g) => groups.push(g),
                Step::Same(a, b) => equal.push((a, b)),
                Step::Why(why) => return Some(why),
                Step::Never => return None,
            }
        }
        for g in &mut groups {
            super::close_under(g, &equal);
        }
        let head = v.rel_name(rule.head().rel());
        documents_verdict(merged_groups(groups), filters.contains(head))
    }

    /// The variables a location column of a positive read places (`one_document`'s `strict`): a nullable column's
    /// variable among them is no -1.
    fn strict_vars(&self, rule: RuleRef<'v>, locations: &BTreeMap<String, Vec<usize>>) -> BTreeSet<String> {
        let v = self.v;
        let mut strict: BTreeSet<String> = BTreeSet::new();
        for l in rule.body() {
            if let L::Atom(a) = l.view()
                && !a.neg()
            {
                let rel = v.rel_name(a.rel());
                let columns = if self.f.derives(rel) { locations.get(rel).cloned().unwrap_or_default() } else { self.base_placing(rel) };
                let args: Vec<TermRef<'_>> = a.args().collect();
                strict.extend(columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| v.text(rule.var(s)).to_owned()));
            }
        }
        let equal_names: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (v.text(rule.var(a)).to_owned(), v.text(rule.var(b)).to_owned())).collect();
        super::close_under(&mut strict, &equal_names);
        strict
    }

    /// `one_document`'s `nullable_row`: a nullable column's variable no location column places, to its positive
    /// row's first location variable.
    fn nullable_rows(&self, rule: RuleRef<'v>, locations: &BTreeMap<String, Vec<usize>>, nullable: &BTreeMap<String, Vec<usize>>, strict: &BTreeSet<String>) -> BTreeMap<String, String> {
        let v = self.v;
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        let mut nullable_row: BTreeMap<String, String> = BTreeMap::new();
        for l in rule.body() {
            let L::Atom(a) = l.view() else { continue };
            let rel = v.rel_name(a.rel());
            let Some(nc) = nullable.get(rel).filter(|_| !a.neg() && self.f.derives(rel)) else { continue };
            let args: Vec<TermRef<'_>> = a.args().collect();
            let Some(anchor) = locations.get(rel).and_then(|cs| cs.iter().find_map(|&c| args.get(c).and_then(|t| t.var()))) else { continue };
            for x in nc.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).filter(|x| !strict.contains(x)) {
                nullable_row.entry(x).or_insert_with(|| var(anchor));
            }
        }
        nullable_row
    }

    /// What literal `l` (the rule's `index`th) adds to `why_not_local`'s documents.
    fn text_step(&self, rule: RuleRef<'v>, index: usize, l: L<'_>, text: &RuleText<'_>) -> Step {
        let v = self.v;
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        match l {
            L::Atom(a) => self.read_step(rule, index, a, true, text),
            L::Agg { atom, .. } => self.read_step(rule, index, atom, false, text),
            L::Ast { .. } => Step::Why("an AST pattern".to_owned()),
            L::Nearest { n, a, .. } => Step::Group(BTreeSet::from([var(n), var(a)])),
            L::Cmp { l, op: CmpOp::Eq, r } => match (doc_side(v, l), doc_side(v, r)) {
                (Some((a, x)), Some((b, y))) if x || y => Step::Same(var(a), var(b)),
                _ => Step::Skip,
            },
            L::Cmp { .. } => Step::Skip,
        }
    }

    /// Whether a read of `rel` places nothing: a constant, a chunk's value, a function of values, a string relation, a
    /// filter (or a read by one).
    fn placeless_read(&self, rel: &str, head: &str, text: &RuleText<'_>) -> bool {
        let strings = rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
        self.value_read(rel, text) || strings || text.filters.contains(rel) || text.filters.contains(head)
    }

    /// Whether `rel` is a value wherever it is read: a constant, a chunk's value, a per-file function of strings.
    fn value_read(&self, rel: &str, text: &RuleText<'_>) -> bool {
        let base = !self.f.derives(rel);
        let fact = base && self.fact(rel) && !self.f.is_input(rel);
        let functional = text.local(self.v, rel) && self.f.native(rel).is_some_and(|nat| matches!(nat.spec, NativeSpec::MatchAll { .. }));
        fact || text.constants.contains(rel) || self.run_constant(rel) || (base && CHUNK_VALUES.contains(&rel)) || functional
    }

    /// What read `atom` (plain, or an aggregate's when not `plain`) adds: the variables its location columns place.
    fn read_step(&self, rule: RuleRef<'v>, index: usize, atom: crate::code_rule_dsl::logic::program_image::AtomRef<'_>, plain: bool, text: &RuleText<'_>) -> Step {
        let v = self.v;
        let rel = v.rel_name(atom.rel());
        if self.placeless_read(rel, v.rel_name(rule.head().rel()), text) {
            return Step::Skip;
        }
        let derived = self.f.derives(rel);
        if derived && !text.local(v, rel) {
            return Step::Why(format!("reads cross-file {rel}"));
        }
        let columns = if derived { text.locations[rel].clone() } else { self.base_placing(rel) };
        let args: Vec<TermRef<'_>> = atom.args().collect();
        // The empty value -1 (`unset(x)`) in a location column no rule fills with it (not `nullable`): no document's
        // row holds it. A positive read never holds, so the rule makes nothing; a negated one always holds, and an
        // aggregate's reads no row: they place nothing.
        if columns.iter().any(|&c| empty_at(rel, derived, c, &args, text.nullable)) {
            return if atom.neg() || !plain { Step::Skip } else { Step::Never };
        }
        let vars = self.read_vars(rule, index, atom, plain, &columns, &args, text);
        if vars.is_empty() {
            return Step::Why(format!("reads {rel} with no location"));
        }
        Step::Group(vars)
    }

    /// The variables read `atom`'s location `columns` place: their variables and paths written out, a negation's
    /// nullable ones at their rows, a binder's tapes.
    #[allow(clippy::too_many_arguments)]
    fn read_vars(&self, rule: RuleRef<'v>, index: usize, atom: crate::code_rule_dsl::logic::program_image::AtomRef<'_>, plain: bool, columns: &[usize], args: &[TermRef<'_>], text: &RuleText<'_>) -> BTreeSet<String> {
        let v = self.v;
        let rel = v.rel_name(atom.rel());
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        let mut vars: BTreeSet<String> = columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).collect();
        // `one_document`'s path written out (decision 0665): each string one document's.
        vars.extend(columns.iter().filter_map(|&c| match args.get(c) {
            Some(TermRef::Str(x)) => Some(format!("rr_value_path:{}", x.0)),
            _ => None,
        }));
        if atom.neg() {
            vars = vars.into_iter().map(|x| text.nullable_row.get(&x).cloned().unwrap_or(x)).collect();
        }
        if let Some(nc) = text.nullable.get(rel).filter(|_| self.f.derives(rel)) {
            vars.extend(nc.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).filter(|x| text.strict.contains(x)));
        }
        let binder = plain && text.placed.by_binder.values().any(|b| *b == index);
        if vars.is_empty() && binder && !columns.is_empty() {
            vars.insert(format!("rr_value_binder:{index}"));
        }
        if !vars.is_empty() {
            vars.extend(text.placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| text.tape(rule, *t)));
            let tape_of = args.get(1).and_then(|t| t.var()).and_then(|t| text.placed.canonical(t)).filter(|_| rel == "tape_of");
            vars.extend(tape_of.map(|t| text.tape(rule, t)));
        }
        vars
    }
}

/// Whether read `args` of `rel` holds the empty value -1 (`unset(x)`) at location column `c`, which no rule fills with
/// it (not `nullable`), and a base relation's row never holds there (`unset_columns`).
fn empty_at(rel: &str, derived: bool, c: usize, args: &[TermRef<'_>], nullable: &BTreeMap<String, Vec<usize>>) -> bool {
    let unset = matches!(args.get(c), Some(TermRef::Int(crate::code_rule_dsl::logic::rr::lower::UNSET)));
    unset && !nullable.get(rel).is_some_and(|nc| nc.contains(&c)) && (derived || unset_columns(rel, vec![c]) == [c])
}

/// What a literal adds to `why_not_local`'s documents.
enum Step {
    /// Nothing.
    Skip,
    /// A group of names one document's.
    Group(BTreeSet<String>),
    /// Two variables one document's (`@doc(a) = @doc(b)`).
    Same(String, String),
    /// The rule places more than one document, or none: why.
    Why(String),
    /// The rule never holds.
    Never,
}

/// `why_not_local`'s rule-wide state: the split's so far, and the rule's own placing.
struct RuleText<'a> {
    v: &'a Version,
    local_now: &'a [bool],
    constants: &'a BTreeSet<String>,
    filters: &'a BTreeSet<String>,
    locations: &'a BTreeMap<String, Vec<usize>>,
    nullable: &'a BTreeMap<String, Vec<usize>>,
    strict: BTreeSet<String>,
    nullable_row: BTreeMap<String, String>,
    placed: ValueTapes,
}

impl RuleText<'_> {
    fn local(&self, v: &Version, rel: &str) -> bool {
        v.rel(rel).is_some_and(|r| self.local_now[r.0 as usize])
    }

    fn tape(&self, rule: RuleRef<'_>, c: u32) -> String {
        format!("rr_value_tape:{}", self.v.text(rule.var(Slot(c))))
    }
}

/// `groups` merged where they meet.
fn merged_groups(groups: Vec<BTreeSet<String>>) -> Vec<BTreeSet<String>> {
    let mut merged: Vec<BTreeSet<String>> = Vec::new();
    for group in groups {
        let (joined, rest): (Vec<_>, Vec<_>) = merged.into_iter().partition(|m| !m.is_disjoint(&group));
        merged = rest;
        merged.push(joined.into_iter().fold(group, |mut all, j| {
            all.extend(j);
            all
        }));
    }
    merged
}

/// `why_not_local`'s verdict on the documents a rule's reads `merged` make: one, or none for a filter's (`filtered`).
fn documents_verdict(merged: Vec<BTreeSet<String>>, filtered: bool) -> Option<String> {
    match merged.len() {
        1 => None,
        0 if filtered => None,
        0 => Some("reads no location".to_owned()),
        n => {
            let parts: Vec<String> = merged.iter().map(|m| m.iter().filter(|v| !v.starts_with("rr_value")).cloned().collect::<Vec<_>>().join(" ")).collect();
            Some(format!("joins {n} documents: {{{}}}", parts.join("} {")))
        }
    }
}

/// What `covers` reads of the split so far.
struct Cover<'a> {
    local_now: &'a [bool],
    covered: &'a [bool],
    valueless: &'a [bool],
    placeless: &'a [bool],
    locs: &'a [Vec<usize>],
    anchoring: &'a Anchoring<'a>,
}

impl<'a> Cover<'a> {
    /// Relation `x`'s location columns: a derived one's as found so far, a base one's as declared.
    fn columns<'s>(&'s self, rels: &'s Rels<'_>, x: u32) -> &'s [usize] {
        if rels.is_derived(x) { &self.locs[x as usize] } else { &rels.base_locs[x as usize] }
    }
}

/// The candidates a split may cover (decision 0652), and the file-local located views `covers` asked as anchoring
/// readers this round: true where its readers place its head.
struct Anchoring<'a> {
    candidate: &'a [bool],
    memo: std::cell::RefCell<BTreeMap<u32, bool>>,
}

/// Whether `reader`'s positive located reads, other than of `r`, place `vars` in one document: joined through their
/// location columns, `a = b`, `@doc`, `nearest` and `tied` (an anchored head's location variables, decision 0652).
fn one_place<'c>(v: &Version, reader: RuleRef<'_>, r: u32, placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], tied: &[Slot], vars: impl Iterator<Item = Slot>) -> bool {
    let mut doc: Vec<u32> = (0..reader.var_count() as u32).collect();
    for (a, b) in super::var_equalities_in(v, reader) {
        join(&mut doc, a.0, b.0);
    }
    for w in tied.windows(2) {
        join(&mut doc, w[0].0, w[1].0);
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

/// Whether rules `a` and `b` have the same head and the same literals beside their positive located reads, term by
/// term with variables by name (`covers`): their tests then hold at the same head values.
fn same_rest(v: &Version, a: RuleRef<'_>, b: RuleRef<'_>, placeless: &dyn Fn(u32) -> bool) -> bool {
    fn term(v: &Version, ra: RuleRef<'_>, x: TermRef<'_>, rb: RuleRef<'_>, y: TermRef<'_>) -> bool {
        match (x, y) {
            (TermRef::Var(s), TermRef::Var(t)) => v.text(ra.var(s)) == v.text(rb.var(t)),
            (TermRef::Wild, TermRef::Wild) => true,
            (TermRef::Int(i), TermRef::Int(j)) => i == j,
            (TermRef::Str(s), TermRef::Str(t)) => v.text(s) == v.text(t),
            (TermRef::Call(f, xs), TermRef::Call(g, ys)) => {
                let (xs, ys): (Vec<_>, Vec<_>) = (xs.collect(), ys.collect());
                v.text(f) == v.text(g) && xs.len() == ys.len() && xs.into_iter().zip(ys).all(|(x, y)| term(v, ra, x, rb, y))
            }
            _ => false,
        }
    }
    let rest = |r: RuleRef<'_>| -> Vec<Lit2> {
        r.body()
            .filter_map(|l| match l.view() {
                L::Atom(x) if !x.neg() && !placeless(x.rel().0) => None,
                L::Atom(x) => Some(Lit2::Atom(x.neg(), x.rel().0, x.args().count())),
                L::Cmp { op, .. } => Some(Lit2::Cmp(op)),
                _ => Some(Lit2::Other),
            })
            .collect()
    };
    #[derive(PartialEq)]
    enum Lit2 {
        Atom(bool, u32, usize),
        Cmp(CmpOp),
        Other,
    }
    fn args(x: crate::code_rule_dsl::logic::program_image::AtomRef<'_>) -> Vec<TermRef<'_>> {
        x.args().collect()
    }
    if a.head().rel() != b.head().rel() || !args(a.head()).into_iter().zip(args(b.head())).all(|(x, y)| term(v, a, x, b, y)) {
        return false;
    }
    let (ra, rb) = (rest(a), rest(b));
    if ra != rb || ra.contains(&Lit2::Other) {
        return false;
    }
    fn kept<'r>(r: RuleRef<'r>, placeless: &dyn Fn(u32) -> bool) -> Vec<crate::code_rule_dsl::logic::program_image::LitRef<'r>> {
        r.body().filter(|l| !matches!(l.view(), L::Atom(x) if !x.neg() && !placeless(x.rel().0))).collect()
    }
    kept(a, placeless).into_iter().zip(kept(b, placeless)).all(|(x, y)| match (x.view(), y.view()) {
        (L::Atom(x), L::Atom(y)) => args(x).into_iter().zip(args(y)).all(|(s, t)| term(v, a, s, b, t)),
        (L::Cmp { l, r, .. }, L::Cmp { l: l2, r: r2, .. }) => term(v, a, l, b, l2) && term(v, a, r, b, r2),
        _ => false,
    })
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
#[allow(clippy::too_many_arguments)]
fn maps<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: usize, anchors: &[usize], tied: &[Slot], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize]) -> bool {
    let qc = value_classes(reader);
    let Some(L::Atom(read)) = reader.body().nth(at).map(|l| l.view()) else { return false };
    let head: Vec<Option<MapT>> = read.args().map(|u| MapT::of(&qc, u)).collect();
    maps_at(v, rule, reader, Some(at), &head, anchors, tied, placeless, columns, false)
}

/// A term of a reader as `maps` binds a rule's variables to it: a class of its variables, or a constant.
#[derive(Clone, Copy, PartialEq)]
enum MapT {
    Var(u32),
    Int(i64),
    Str(u32),
}

impl MapT {
    fn of(qc: &[u32], t: TermRef<'_>) -> Option<MapT> {
        match t {
            TermRef::Var(s) => Some(MapT::Var(qc[s.0 as usize])),
            TermRef::Int(i) => Some(MapT::Int(i)),
            TermRef::Str(a) => Some(MapT::Str(a.0)),
            _ => None,
        }
    }
}

/// `maps` with the head's arguments given (`head`), the reader's literal `at` a read of it where there is one, and
/// `tied` the reader's variables placed in one document by its anchored head. Where
/// `exact`, `rule` is a keyed family's root's (`covers_keyed`): each of its literals is found in the reader as it is,
/// every atom (negated, placeless) a reader's atom of its relation and sign, each comparison one of the reader's, since
/// the rule's whole body must hold in the reader's rows, not only its reads.
#[allow(clippy::too_many_arguments)]
fn maps_at<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: Option<usize>, head: &[Option<MapT>], anchors: &[usize], tied: &[Slot], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], exact: bool) -> bool {
    let (rc, qc) = (value_classes(rule), value_classes(reader));
    let target = |t: TermRef<'_>| MapT::of(&qc, t);
    let Some(HeadMap { mut bind, anchor_vars, head_calls }) = head_map(rule, head, anchors, &rc, exact) else { return false };
    let given: Vec<bool> = bind.iter().map(|b| !b.is_empty()).collect();
    let mut doc = rule_documents(v, rule, placeless, columns);
    let anchored_roots: BTreeSet<u32> = anchor_vars.iter().map(|&v| find(&mut doc, v)).collect();
    let mut anchored = |a: AtomRef<'_>| located_vars(a, columns).into_iter().any(|v| anchored_roots.contains(&find(&mut doc, v)));
    let Some(body) = MapBody::of(rule, &rc, exact, placeless, &mut anchored) else { return false };
    if !body.fixed_by_head(&head_calls, &given, &rc) {
        return false;
    }
    let targets: Vec<_> = reader
        .body()
        .enumerate()
        .filter_map(|(i, l)| match l.view() {
            L::Atom(a) if (exact || !a.neg()) && Some(i) != at => Some(a),
            _ => None,
        })
        .collect();
    let theirs: Vec<(TermRef<'_>, CmpOp, TermRef<'_>)> = reader
        .body()
        .filter_map(|l| match l.view() {
            L::Cmp { l, op, r } => Some((l, op, r)),
            _ => None,
        })
        .collect();
    let m = Mapping { v, rc: &rc, target: &target };
    let done = |bind: &mut Vec<Vec<MapT>>| {
        if !body.cmps.iter().all(|&cmp| theirs.iter().any(|&other| m.same_cmp(cmp, other, bind))) {
            return false;
        }
        if exact || anchors.is_empty() {
            return true;
        }
        // A mapped read must be in the anchored document, not merely present somewhere in a cross-file reader.
        let Some(places) = mapped_places(&anchor_vars, &body.mapped, bind, &rc, columns) else { return false };
        let Some(L::Atom(read)) = at.and_then(|at| reader.body().nth(at)).map(|l| l.view()) else { return false };
        one_place(v, reader, read.rel().0, placeless, columns, tied, places.into_iter())
    };
    m.go(0, &body.mapped, &targets, &mut bind, &mut 0, &done)
}

/// The reader's variables that the head's anchored variables `anchor_vars`, and the mapped reads' located variables,
/// are bound to (`bind`, by value class `rc`): `None` where one is bound to nothing or to no variable, or where a
/// mapped read leaves a location column unnamed.
fn mapped_places<'c>(anchor_vars: &[u32], mapped: &[AtomRef<'_>], bind: &[Vec<MapT>], rc: &[u32], columns: &dyn Fn(u32) -> &'c [usize]) -> Option<Vec<Slot>> {
    let mut places = Vec::new();
    for s in anchor_vars.iter().copied().chain(mapped.iter().flat_map(|a| located_vars(*a, columns))) {
        let terms = &bind[rc[s as usize] as usize];
        if terms.is_empty() {
            return None;
        }
        for term in terms {
            let MapT::Var(slot) = term else { return None };
            places.push(Slot(*slot));
        }
    }
    let unnamed = mapped.iter().any(|a| columns(a.rel().0).iter().any(|&c| a.args().nth(c).and_then(|t| t.var()).is_none()));
    (!unnamed).then_some(places)
}

type AtomRef<'r> = crate::code_rule_dsl::logic::program_image::AtomRef<'r>;

/// The variables of term `t`, a call's arguments' too.
fn term_slots(t: TermRef<'_>, out: &mut Vec<u32>) {
    match t {
        TermRef::Var(s) => out.push(s.0),
        TermRef::Call(_, xs) => xs.for_each(|x| term_slots(x, out)),
        _ => {}
    }
}

/// The variables of read `a`'s location columns.
fn located_vars<'c>(a: AtomRef<'_>, columns: &dyn Fn(u32) -> &'c [usize]) -> Vec<u32> {
    let args: Vec<TermRef<'_>> = a.args().collect();
    columns(a.rel().0).iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| s.0).collect()
}

/// `maps_at`'s head, bound: each class of the rule's variables to the reader's terms its head columns take, the
/// variables of the head's location columns (`anchors`), and those of a head column a call fills (`negate(a,
/// @concat("-", a))`: a function of its variables, which, as a comparison's, the head must fix).
struct HeadMap {
    bind: Vec<Vec<MapT>>,
    anchor_vars: Vec<u32>,
    head_calls: Vec<u32>,
}

/// `rule`'s head bound to `head`; `None` where it cannot be: another width, an exact rule's constant other than the
/// value asked, a call in an exact rule's head.
fn head_map(rule: RuleRef<'_>, head: &[Option<MapT>], anchors: &[usize], rc: &[u32], exact: bool) -> Option<HeadMap> {
    if rule.head().args().count() != head.len() {
        return None;
    }
    let mut m = HeadMap { bind: vec![Vec::new(); rule.var_count()], anchor_vars: Vec::new(), head_calls: Vec::new() };
    for (c, (h, u)) in rule.head().args().zip(head.iter().copied()).enumerate() {
        match h {
            TermRef::Var(s) => {
                m.bind[rc[s.0 as usize] as usize].extend(u);
                if anchors.contains(&c) {
                    m.anchor_vars.push(s.0);
                }
            }
            // An exact rule's constant head must be the value asked.
            TermRef::Int(_) | TermRef::Str(_) if exact && u.is_some_and(|u| Some(u) != MapT::of(&[], h)) => return None,
            TermRef::Int(_) | TermRef::Str(_) => {}
            TermRef::Call(..) if !exact => term_slots(h, &mut m.head_calls),
            _ => return None,
        }
    }
    Some(m)
}

/// The rule's documents, as a union-find over its variables: its positive located reads' location columns, joined
/// by `a = b`, `@doc` and `nearest`.
fn rule_documents<'c>(v: &Version, rule: RuleRef<'_>, placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize]) -> Vec<u32> {
    let mut doc: Vec<u32> = (0..rule.var_count() as u32).collect();
    for (a, b) in super::var_equalities_in(v, rule) {
        join(&mut doc, a.0, b.0);
    }
    for l in rule.body() {
        match l.view() {
            L::Atom(a) if !a.neg() && !placeless(a.rel().0) => {
                for w in located_vars(a, columns).windows(2) {
                    join(&mut doc, w[0], w[1]);
                }
            }
            L::Nearest { n: x, a: y, .. } => join(&mut doc, x.0, y.0),
            _ => {}
        }
    }
    doc
}

/// `maps_at`'s rule body, sorted: the reads to map, an exact rule's comparisons (each found in the reader once its
/// reads are, `same_term`), the classes the mapped reads hold, and every other literal's variables.
struct MapBody<'r> {
    mapped: Vec<AtomRef<'r>>,
    cmps: Vec<(TermRef<'r>, CmpOp, TermRef<'r>)>,
    in_mapped: Vec<bool>,
    others: Vec<Vec<u32>>,
}

impl<'r> MapBody<'r> {
    /// Whether the head fixes what the body needs: each variable a head call reads (`head_calls`) is given (`given`, by
    /// value class `rc`), and the rule's other literals read no mapped variable but those the head fixes.
    fn fixed_by_head(&self, head_calls: &[u32], given: &[bool], rc: &[u32]) -> bool {
        let given_at = |s: u32| given[rc[s as usize] as usize];
        head_calls.iter().all(|&s| given_at(s)) && !self.others.iter().flatten().any(|&s| self.in_mapped[rc[s as usize] as usize] && !given_at(s))
    }
    /// `rule`'s body sorted; `None` where a literal cannot be mapped.
    fn of(rule: RuleRef<'r>, rc: &[u32], exact: bool, placeless: &dyn Fn(u32) -> bool, anchored: &mut dyn FnMut(AtomRef<'_>) -> bool) -> Option<Self> {
        let mut body = MapBody { mapped: Vec::new(), cmps: Vec::new(), in_mapped: vec![false; rule.var_count()], others: Vec::new() };
        for l in rule.body() {
            let ok = if exact { body.exact_literal(l.view(), rc) } else { body.literal(l.view(), rc, placeless, anchored) };
            if !ok {
                return None;
            }
        }
        Some(body)
    }

    /// A read to map: its variables' classes held; any other term than a variable, `_` or a constant fails.
    fn map(&mut self, a: AtomRef<'r>, rc: &[u32]) -> bool {
        for t in a.args() {
            match t {
                TermRef::Var(s) => self.in_mapped[rc[s.0 as usize] as usize] = true,
                TermRef::Wild | TermRef::Int(_) | TermRef::Str(_) => {}
                _ => return false,
            }
        }
        self.mapped.push(a);
        true
    }

    /// An exact rule's literal: every atom mapped, each comparison found (but a variable's with its own class).
    fn exact_literal(&mut self, l: L<'r>, rc: &[u32]) -> bool {
        match l {
            L::Atom(a) => self.map(a, rc),
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } if rc[a.0 as usize] == rc[b.0 as usize] => true,
            L::Cmp { l, op, r } => {
                self.cmps.push((l, op, r));
                true
            }
            _ => false,
        }
    }

    /// Another rule's literal: a positive located read mapped unless anchored; a negated located read only anchored;
    /// the rest's variables kept.
    fn literal(&mut self, l: L<'r>, rc: &[u32], placeless: &dyn Fn(u32) -> bool, anchored: &mut dyn FnMut(AtomRef<'_>) -> bool) -> bool {
        let mut vs = Vec::new();
        match l {
            L::Atom(a) if !a.neg() && !placeless(a.rel().0) && !anchored(a) => return self.map(a, rc),
            L::Atom(a) if a.neg() && !placeless(a.rel().0) && !anchored(a) => return false,
            L::Atom(a) => a.args().for_each(|t| term_slots(t, &mut vs)),
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } if rc[a.0 as usize] == rc[b.0 as usize] => return true,
            L::Cmp { l, r, .. } => {
                term_slots(l, &mut vs);
                term_slots(r, &mut vs);
            }
            L::Nearest { .. } => return false,
            _ => return false,
        }
        self.others.push(vs);
        true
    }
}

/// A map of a rule's classes to a reader's terms in the making (`maps_at`).
struct Mapping<'a> {
    v: &'a Version,
    rc: &'a [u32],
    target: &'a dyn Fn(TermRef<'_>) -> Option<MapT>,
}

impl Mapping<'_> {
    /// Class `c` bound to `u`: where it is unbound, now; else whether it is.
    fn bind_to(bind: &mut [Vec<MapT>], c: usize, u: Option<MapT>) -> bool {
        match u {
            None => false,
            Some(u) if bind[c].is_empty() => {
                bind[c].push(u);
                true
            }
            Some(u) => bind[c].contains(&u),
        }
    }

    /// Backtracking over the mapped reads from the `k`th, each to a target of its relation; bounded, a failure past the
    /// bound.
    fn go(&self, k: usize, mapped: &[AtomRef<'_>], targets: &[AtomRef<'_>], bind: &mut Vec<Vec<MapT>>, steps: &mut usize, done: &dyn Fn(&mut Vec<Vec<MapT>>) -> bool) -> bool {
        let Some(a) = mapped.get(k) else { return done(bind) };
        // Of its sign: a negated read is found only as that negated read, its `_` as `_` (`!r(e, _)` holds less often
        // than `!r(e, y)`).
        for b in targets.iter().filter(|b| b.rel().0 == a.rel().0 && b.neg() == a.neg()) {
            *steps += 1;
            if *steps > 4096 {
                return false;
            }
            let saved = bind.clone();
            if a.args().zip(b.args()).all(|(x, y)| self.arg_fits(a.neg(), x, y, bind)) && self.go(k + 1, mapped, targets, bind, steps, done) {
                return true;
            }
            *bind = saved;
        }
        false
    }

    /// Whether a mapped read's argument `x` fits a target's `y`.
    fn arg_fits(&self, neg: bool, x: TermRef<'_>, y: TermRef<'_>, bind: &mut [Vec<MapT>]) -> bool {
        match x {
            TermRef::Wild => !neg || matches!(y, TermRef::Wild),
            TermRef::Int(i) => (self.target)(y) == Some(MapT::Int(i)),
            TermRef::Str(s) => (self.target)(y) == Some(MapT::Str(s.0)),
            TermRef::Var(s) => Self::bind_to(bind, self.rc[s.0 as usize] as usize, (self.target)(y)),
            _ => false,
        }
    }

    /// A term of the rule as the reader's under the map: a variable bound to it (or, unbound, now), a constant, or a
    /// call of the same function on such terms.
    fn same_term(&self, x: TermRef<'_>, y: TermRef<'_>, bind: &mut [Vec<MapT>]) -> bool {
        let v = self.v;
        match (x, y) {
            (TermRef::Var(s), _) => Self::bind_to(bind, self.rc[s.0 as usize] as usize, (self.target)(y)),
            (TermRef::Int(a), TermRef::Int(b)) => a == b,
            (TermRef::Str(a), TermRef::Str(b)) => v.text(a) == v.text(b),
            (TermRef::Call(f, xs), TermRef::Call(g, ys)) => {
                let (xs, ys): (Vec<_>, Vec<_>) = (xs.collect(), ys.collect());
                v.text(f) == v.text(g) && xs.len() == ys.len() && xs.into_iter().zip(ys).all(|(x, y)| self.same_term(x, y, bind))
            }
            _ => false,
        }
    }

    /// Whether comparison `(l, op, r)` is the reader's `(l2, op2, r2)` under the map, `=` and `!=` either way round;
    /// the binding of the first found is kept.
    fn same_cmp(&self, (l, op, r): (TermRef<'_>, CmpOp, TermRef<'_>), (l2, op2, r2): (TermRef<'_>, CmpOp, TermRef<'_>), bind: &mut Vec<Vec<MapT>>) -> bool {
        if op != op2 {
            return false;
        }
        let saved = bind.clone();
        if self.same_term(l, l2, bind) && self.same_term(r, r2, bind) {
            return true;
        }
        *bind = saved.clone();
        if matches!(op, CmpOp::Eq | CmpOp::Ne) && self.same_term(l, r2, bind) && self.same_term(r, l2, bind) {
            return true;
        }
        *bind = saved;
        false
    }
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

/// A relation's family in `keyed`'s next round, from its rules' family roots (`None`: a rule reads outside its
/// family): a member of the root now known, else a root of its own (where `rootable`), or out of every family.
fn family_next(r: usize, roots: Option<BTreeSet<u32>>, rootable: bool) -> Option<Option<u32>> {
    match roots {
        Some(roots) if roots.len() <= 1 => Some(roots.into_iter().next()),
        Some(_) => None,
        None if rootable => Some(Some(r as u32)),
        None => None,
    }
}

/// What `key_held` asks of a keyed family's root: the root, each relation's family root (`keyed`), and the split's
/// state so far.
struct KeyHold<'a> {
    root: u32,
    keyed: &'a [Option<u32>],
    local_now: &'a [bool],
    placeless: &'a [bool],
    locs: &'a [Vec<usize>],
}

/// The placing columns of base relation `rel` at which -1 is the empty value (`unset`), no row's: all of them but
/// the tape's parent, which a root's row holds as -1 (`facts/vtab/syntax.rs`; review F1).
fn unset_columns(rel: &str, placing: Vec<usize>) -> Vec<usize> {
    if rel == "tape" { placing.into_iter().filter(|&c| c != 1).collect() } else { placing }
}

/// Whether `rule` never holds, whatever the facts: a comparison of two constants that is false (`-1 != -1`, where
/// `unset(x)` made `x` the empty value in a rule whose other branch tests `!unset(x)`), or a positive read of a base
/// relation at the empty value -1 in a column that places a document (`kind(-1, "none")`): no document's row holds
/// it. `placing` is a base relation's placing columns (empty for a derived one). Such a rule makes no row, so it places
/// nothing: it is per-file, and its relation's places are its other rules' (decision 0652; before decision 0677's
/// fix 2 such a rule's reading of `sym_of` made it cross-file, which hid it).
fn never_holds(rule: &RuleRef<'_>, placing: impl Fn(RelId) -> Vec<usize>) -> bool {
    const UNSET: i64 = crate::code_rule_dsl::logic::rr::lower::UNSET;
    rule.body().any(|l| match l.view() {
        L::Cmp { l: TermRef::Int(a), op, r: TermRef::Int(b) } => !match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        },
        L::Atom(a) if !a.neg() => placing(a.rel()).iter().any(|&c| matches!(a.args().nth(c), Some(TermRef::Int(UNSET)))),
        _ => false,
    })
}

/// A split's relations by `RelId`: what the split asks of each, asked once.
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
    /// A base relation's columns that place its document: its locations and its path key (`Program::base_placing`).
    base_locs: Vec<Vec<usize>>,
    /// Its locations alone (`Program::base_locations`): what a row's document is read off.
    base_real: Vec<Vec<usize>>,
    candidate: Vec<bool>,
    output: Vec<bool>,
    /// A derived relation's rules, by index in the program's order: those that can hold (`never_holds`), which alone say where
    /// its rows are.
    rules: Vec<Vec<usize>>,
    /// Each rule that never holds (`never_holds`): it makes no row, so it is per-file.
    never: Vec<bool>,
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
        let never: Vec<bool> = rules.iter().map(|r| never_holds(r, |rel| if base(rel.0 as usize) { unset_columns(name(rel.0 as usize), p.base_placing(name(rel.0 as usize))) } else { Vec::new() })).collect();
        for ks in &mut by_head {
            ks.retain(|&k| !never[k]);
        }
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
            base_locs: (0..n).map(|r| if base(r) { p.base_placing(name(r)) } else { Vec::new() }).collect(),
            base_real: (0..n).map(|r| if base(r) { p.base_locations(name(r)) } else { Vec::new() }).collect(),
            candidate: (0..n).map(|r| derived[r] && f.native(name(r)).is_none() && !f.lattice(name(r)) && !f.is_input(name(r))).collect(),
            output: (0..n).map(|r| derived[r] && f.is_output(name(r))).collect(),
            rules: by_head,
            never,
            reading,
            head,
            derived,
            _v: std::marker::PhantomData,
        }
    }

    fn empty() -> Rels<'v> {
        Rels { n: 0, derived_count: 0, derived: Vec::new(), fact: Vec::new(), run_constant: Vec::new(), chunk_value: Vec::new(), test: Vec::new(), functional: Vec::new(), text: Vec::new(), base_locs: Vec::new(), base_real: Vec::new(), candidate: Vec::new(), output: Vec::new(), rules: Vec::new(), never: Vec::new(), reading: Vec::new(), head: Vec::new(), _v: std::marker::PhantomData }
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
    /// A read: its variables by argument, and its string constants by argument (decision 0665: a path written out,
    /// `bcgp_line("a.py", l, sp)`), each numbered among the rule's (`Form::paths`).
    /// `unset`: the arguments that are the empty value -1 (`unset(x)`).
    Read { neg: bool, plain: bool, rel: u32, args: Vec<Option<u32>>, consts: Vec<Option<u32>>, unset: Vec<bool>, binder: bool, tapes: Vec<u32> },
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
    /// The first id of the rule's string constants (`Lit::Read`'s `consts`), numbered after the literals: a string in
    /// a column that places a document is a file's path (a location is a node or a number), so each names one
    /// document, the same for every atom that writes it (decision 0665).
    paths: u32,
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
                // A node and its nearest ancestor in a set: both tree nodes (`placed_classes_over`).
                L::Nearest { n: x, a: y, .. } => {
                    named[x.0 as usize] = true;
                    named[y.0 as usize] = true;
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
        let mut strings: BTreeMap<u32, u32> = BTreeMap::new();
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
                        consts: args
                            .iter()
                            .map(|t| match t {
                                TermRef::Str(x) => {
                                    let k = strings.len() as u32;
                                    Some(*strings.entry(x.0).or_insert(k))
                                }
                                _ => None,
                            })
                            .collect(),
                        // The tape's parent holds -1 at a root: no empty value there (`unset_columns`).
                        unset: args
                            .iter()
                            .enumerate()
                            .map(|(c, t)| matches!(t, TermRef::Int(crate::code_rule_dsl::logic::rr::lower::UNSET)) && unset_columns(v.rel_name(a.rel()), vec![c]) == [c])
                            .collect(),
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
        let paths = binders + lits.len() as u32;
        let ids = paths as usize + strings.len();
        Form { named, class, lits, splices, ids, binders, paths }
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

    /// Only values read unchanged from placing columns retain their document identity. An injective string
    /// transformation is not the original path: `cat(p, "i")` can equal another document's path.
    fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vec<usize>]) -> Bits {
        self.placed_classes_over(rels, locs, &rels.base_locs)
    }

    /// `placed_classes` with the base relations' columns `base`: their placing columns, or their locations alone.
    fn placed_classes_over(&self, rels: &Rels<'_>, locs: &[Vec<usize>], base: &[Vec<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            // `nearest(n, a, set)`: both are tree nodes, `a` an ancestor of `n` in `n`'s document. A head filled by
            // `a` (`rdefn(s, x, f) :- …, nearest(f, s, scope)`) is a location, as one filled by `n` is.
            if let Lit::Nearest(n, a) = l {
                out.set(*n);
                out.set(*a);
                continue;
            }
            if let Lit::Read { neg: false, plain: true, rel, args, .. } = l {
                let columns = if rels.is_derived(*rel) { &locs[*rel as usize] } else { &base[*rel as usize] };
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

    #[allow(clippy::too_many_arguments)]
    fn one_document(&self, head: u32, rels: &Rels<'_>, placeless: &[bool], local: &[bool], filters: &[bool], covered: &[bool], locs: &[Vec<usize>], nulls: &[Vec<usize>]) -> bool {
        // A nullable column joins its row's group only where a location column of this rule places its variable
        // (decision 0649): then it is no -1.
        let strict = self.placed_classes(rels, locs);
        let cx = DocCx { rels, placeless, local, filters, covered, locs, nulls, head_filter: filters[head as usize], nullable_row: self.nullable_row(rels, locs, nulls, &strict), strict };
        let mut root: Vec<u32> = (0..self.ids as u32).collect();
        let mut firsts: Vec<u32> = Vec::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        for (index, l) in self.lits.iter().enumerate() {
            match self.doc_step(index, l, &cx) {
                DocStep::Skip => {}
                DocStep::Join(group) => {
                    for &g in &group[1..] {
                        join(&mut root, group[0], g);
                    }
                    firsts.push(group[0]);
                }
                DocStep::Same(a, b) => join(&mut root, a, b),
                DocStep::Decided(one) => return one,
            }
        }
        let mut roots: Vec<u32> = firsts.iter().map(|&f| find(&mut root, f)).collect();
        roots.sort_unstable();
        roots.dedup();
        roots.len() == 1 || (roots.is_empty() && cx.head_filter)
    }

    /// A negated read of a nullable column's variable that no location column places (decision 0652): where it is -1
    /// the negated read matches no row and reads no document, and otherwise it is a node of the row's document, so the
    /// negated read is placed with the positive row that holds the column: (its class, the row's anchor's).
    fn nullable_row(&self, rels: &Rels<'_>, locs: &[Vec<usize>], nulls: &[Vec<usize>], strict: &Bits) -> Vec<(u32, u32)> {
        let mut nullable_row: Vec<(u32, u32)> = Vec::new();
        for l in &self.lits {
            let Lit::Read { neg: false, plain: true, rel, args, .. } = l else { continue };
            let anchor = locs[*rel as usize].iter().find_map(|&c| args.get(c).copied().flatten()).filter(|_| rels.is_derived(*rel));
            let Some(anchor) = anchor else { continue };
            let vars = nulls[*rel as usize].iter().filter_map(|&c| args.get(c).copied().flatten());
            nullable_row.extend(vars.filter(|&v| !strict.has(self.class[v as usize])).map(|v| (self.class[v as usize], self.class[anchor as usize])));
        }
        nullable_row
    }

    /// What literal `l` (the rule's `index`th) adds to `one_document`'s documents.
    fn doc_step(&self, index: usize, l: &Lit, cx: &DocCx<'_>) -> DocStep {
        match l {
            Lit::Read { .. } => self.read_doc_step(index, l, cx),
            Lit::Ast => DocStep::Decided(false),
            Lit::Nearest(n, a) => DocStep::Join(vec![*n, *a]),
            Lit::Cmp => DocStep::Skip,
            Lit::Same(a, b) => DocStep::Same(*a, *b),
        }
    }

    /// `doc_step` for a read: the classes its location columns place.
    fn read_doc_step(&self, index: usize, l: &Lit, cx: &DocCx<'_>) -> DocStep {
        let Lit::Read { neg, plain, rel, unset, .. } = l else { return DocStep::Skip };
        let (r, rels) = (*rel as usize, cx.rels);
        let derived = rels.is_derived(*rel);
        if cx.reads_no_place(r, derived) {
            return DocStep::Skip;
        }
        if derived && !cx.local[r] {
            return DocStep::Decided(false);
        }
        let columns = if derived { &cx.locs[r] } else { &rels.base_locs[r] };
        // The empty value in a location column no rule fills with it: a positive read never holds, and the rule makes
        // nothing; a negated read, or an aggregate's, reads no row (`why_not_local`).
        if cx.empty_located(r, derived, columns, unset) {
            return if *neg || !*plain { DocStep::Skip } else { DocStep::Decided(true) };
        }
        match self.read_group(index, l, columns, derived, cx) {
            group if group.is_empty() => DocStep::Decided(false),
            group => DocStep::Join(group),
        }
    }

    /// The classes read `l`'s location `columns` place: their variables' and paths', a negation's nullable ones at
    /// their rows, a binder's, its tapes.
    fn read_group(&self, index: usize, l: &Lit, columns: &[usize], derived: bool, cx: &DocCx<'_>) -> Vec<u32> {
        let Lit::Read { neg, plain, rel, args, consts, binder, tapes, .. } = l else { return Vec::new() };
        let class = |v: u32| self.class[v as usize];
        let mut group: Vec<u32> = columns.iter().filter_map(|&c| args.get(c).copied().flatten().map(class)).collect();
        // A path written out places the one document of that path (decision 0665).
        group.extend(columns.iter().filter_map(|&c| consts.get(c).copied().flatten().map(|k| self.paths + k)));
        if *neg {
            for k in group.iter_mut() {
                *k = cx.nullable_row.iter().find(|(x, _)| x == k).map_or(*k, |&(_, anchor)| anchor);
            }
        }
        if derived {
            group.extend(cx.nulls[*rel as usize].iter().filter_map(|&c| args.get(c).copied().flatten().map(class)).filter(|&k| cx.strict.has(k)));
        }
        if group.is_empty() && *plain && *binder && !columns.is_empty() {
            group.push(self.binders + index as u32);
        }
        if !group.is_empty() {
            group.extend(tapes.iter().copied());
        }
        group
    }
}

/// What `one_document` reads of the split so far, and of the rule's nullable columns.
struct DocCx<'a> {
    rels: &'a Rels<'a>,
    placeless: &'a [bool],
    local: &'a [bool],
    filters: &'a [bool],
    covered: &'a [bool],
    locs: &'a [Vec<usize>],
    nulls: &'a [Vec<usize>],
    head_filter: bool,
    strict: Bits,
    nullable_row: Vec<(u32, u32)>,
}

impl DocCx<'_> {
    /// Whether a read of relation `r` holds the empty value (`unset`) at one of its location `columns` no rule fills
    /// with it (not a derived relation's nullable column).
    fn empty_located(&self, r: usize, derived: bool, columns: &[usize], unset: &[bool]) -> bool {
        columns.iter().any(|&c| unset.get(c).copied().unwrap_or(false) && !(derived && self.nulls[r].contains(&c)))
    }

    /// Whether a read of relation `r` places nothing: placeless, a filter's (or read by one), covered, a function of
    /// strings where it is per-file (read as a test is).
    fn reads_no_place(&self, r: usize, derived: bool) -> bool {
        let filter = derived && self.filters[r];
        self.placeless[r] || filter || self.covered[r] || self.head_filter || (self.rels.functional[r] && self.local[r])
    }
}

/// What a literal adds to `one_document`'s documents.
enum DocStep {
    Skip,
    /// Classes of one document, the first standing for them.
    Join(Vec<u32>),
    /// Two classes of one document, standing for none.
    Same(u32, u32),
    /// The answer, whatever the rest: one document (the rule never holds), or not.
    Decided(bool),
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
