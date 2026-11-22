//! The file-local split, read from the program's version (decision 0281's path, step 5), as an analysis of it
//! (`FileLocal`), maintained across an edit (`Analysis::maintain`).
//!
//! A split of a program a few rules apart computes only what the difference reaches. The numbers are the image's
//! own: a relation is its `RelId`, a variable its `Slot`, and the last split is the analysis of the edit's input,
//! carried to its output by the manager, which hands `maintain` the edit.

use rerail_blocks::vector::Vector;
use super::{CHUNK_VALUES, document_column, option_input};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Delta, Reads};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, ArchivedNativeSpec, NativeSpec, RelId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

/// The relations a program derives within one document (list step 29's split, R0): a relation is file-local when
/// each of its rules joins only one document's rows. A rule is when every relation it reads (positive, negated or
/// aggregated) is a base relation, a constant one, or file-local, each with a location (a node, or a document) of the
/// rule's one component: locations are joined within an atom of a base or file-local relation (`node(d, n, …)`,
/// `child(p, c, _)`, `nearest_<set>(n, a)`), and nothing else joins them. A symbol shared by two atoms (`text($a,
/// $x), text($b, $x)`) does not: those may be two files. The relations are the greatest set closed under this, so a
/// recursion within a document is file-local. Evaluated per chunk of documents, a file-local relation's rows are the
/// union of the chunks'.
///
/// Each part is a greatest fixpoint over the derived relations, read from the version: the file-local relations
/// (`local`); the constant relations (`constants`: derived from the program's constants alone, as `la_side_queue(x)
/// :- la_set("…", x)`, every rule reading only facts, the run's options, other such relations and comparisons; a rule
/// reading one joins no document by it, and both parts derive them); each derived relation's location columns
/// (`locations`: those every rule fills from a location, a node or a document of `node`, `doc`, …); the string
/// filters (decision 0037); the same-document columns of each base input and file-local relation (`same_doc`: its
/// location columns, whose values in one row are all of one document; a row's document is read off any of them,
/// decision 0052); and whether the program reads no document (`reads_no_document`).
///
/// `causes` says why each derived relation is cross-file, by the first rule the split found not to join one
/// document: a root cause ("joins 2 documents: {n} {m}", "reads field with no location", "made natively") or "reads
/// cross-file r". Read through the second kind to the first to see what makes a check run over the whole corpus.
/// With the digests it was made under, which `maintain` compares.
#[derive(Clone, Debug, Default)]
pub struct FileLocal {
    pub local: BTreeSet<String>,
    pub constants: BTreeSet<String>,
    pub locations: BTreeMap<String, Vector<usize>>,
    pub filters: BTreeSet<String>,
    pub causes: BTreeMap<String, String>,
    pub same_doc: BTreeMap<String, Vector<usize>>,
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
            && self.no_document == o.no_document
            && self.keys == o.keys
    }
}

impl FileLocal {
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
        // A part's split (decision 0345 stage 2): its own relations, the ones another part derives read as given.
        match p.given_split() {
            Some((scope, given)) => Ok(p.split(Some(&scope), Some(&given))),
            None => Ok(p.split(None, None)),
        }
    }
    /// Its fields as rows, each tagged by its field (decision 0333 §4): `(0, rel)` file-local, `(1, rel)` constant,
    /// `(2, rel, columns)` locations, `(3, rel)` filters, `(4, rel, cause)`, `(5, rel, columns)` same-document,
    /// `(6, no document)`, and its keys, `(7, global)` and `(8, head, key)`.
    fn rows(&self, _v: &Version) -> Option<Vec<crate::code_rule_dsl::logic::program_image::manager::Row>> {
        use crate::code_rule_dsl::logic::front::Value;
        let str = |s: &str| Value::Str(s.to_owned().into());
        let cols = |c: &[usize]| str(&c.iter().map(usize::to_string).collect::<Vec<_>>().join(","));
        let tag = |k: i64| Value::Int(k);
        let mut out = Vec::new();
        out.extend(self.local.iter().map(|r| vec![tag(0), str(r)]));
        out.extend(self.constants.iter().map(|r| vec![tag(1), str(r)]));
        out.extend(self.locations.iter().map(|(r, c)| vec![tag(2), str(r), cols(c)]));
        out.extend(self.filters.iter().map(|r| vec![tag(3), str(r)]));
        out.extend(self.causes.iter().map(|(r, c)| vec![tag(4), str(r), str(c)]));
        out.extend(self.same_doc.iter().map(|(r, c)| vec![tag(5), str(r), cols(c)]));
        out.push(vec![tag(6), Value::Int(i64::from(self.no_document))]);
        out.push(vec![tag(7), Value::Int(self.keys.global as i64)]);
        out.extend(self.keys.heads.iter().map(|(h, k)| vec![tag(8), str(h), Value::Int(*k as i64)]));
        Some(out)
    }
    fn of_rows(_v: &Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<FileLocal> {
        use crate::code_rule_dsl::logic::front::Value;
        let cols = |s: &str| -> Option<rerail_blocks::vector::Vector<usize>> { if s.is_empty() { Some(rerail_blocks::vector::Vector::new()) } else { s.split(',').map(|x| x.parse::<usize>().ok()).collect() } };
        let mut f = FileLocal::default();
        for r in rows {
            match r.as_slice() {
                [Value::Int(0), Value::Str(n)] => drop(f.local.insert(n.to_string())),
                [Value::Int(1), Value::Str(n)] => drop(f.constants.insert(n.to_string())),
                [Value::Int(2), Value::Str(n), Value::Str(c)] => drop(f.locations.insert(n.to_string(), cols(c)?)),
                [Value::Int(3), Value::Str(n)] => drop(f.filters.insert(n.to_string())),
                [Value::Int(4), Value::Str(n), Value::Str(c)] => drop(f.causes.insert(n.to_string(), c.to_string())),
                [Value::Int(5), Value::Str(n), Value::Str(c)] => drop(f.same_doc.insert(n.to_string(), cols(c)?)),
                [Value::Int(6), Value::Int(b)] => f.no_document = *b != 0,
                [Value::Int(7), Value::Int(k)] => f.keys.global = *k as u64,
                [Value::Int(8), Value::Str(h), Value::Int(k)] => drop(f.keys.heads.insert(h.to_string(), *k as u64)),
                _ => return None,
            }
        }
        Some(f)
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
        // An edit of rules alone whose heads are derived before and after: only their keys are made again (decision
        // 0385); the global key reads no rule but which relations are derived.
        let keys = match &delta.heads {
            Some(heads) if delta.touched == Reads::RULES => p.keys_over(&self.keys, heads),
            _ => None,
        }
        .unwrap_or_else(|| p.keys());
        if keys.global != self.keys.global {
            return None;
        }
        let mut changed: Vec<&str> = keys.heads.iter().filter(|(rel, k)| self.keys.heads.get(*rel) != Some(*k)).map(|(rel, _)| rel.as_str()).collect();
        changed.extend(self.keys.heads.keys().filter(|rel| !keys.heads.contains_key(*rel)).map(String::as_str));
        if changed.is_empty() {
            return Some(p.finish(self.clone(), keys));
        }
        let mut cone: BTreeSet<u32> = BTreeSet::new();
        let mut work: Vector<u32> = changed.iter().filter_map(|r| p.v.rel(r)).map(|r| r.0).collect();
        cone.extend(work.iter().copied().filter(|&r| p.rels.is_derived(r)));
        while let Some(r) = work.pop() {
            for &k in p.rels.reading(r) {
                let h = p.rels.head[k];
                if cone.insert(h) {
                    work.push(h);
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
        let mut same_doc: BTreeMap<String, Vector<usize>> = self.f.inputs().filter(|i| !self.f.derives(i)).map(|i| (i.to_owned(), self.base_locations(i))).collect();
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

/// The keys' hasher: foldhash's quality hasher with a fixed seed.
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

    /// A part's own relations (those its rules derive), and the places its relations another part derives were
    /// given (`fields::GIVEN`, and the words of `given_words`): `None` for a program that is no part.
    fn given_split(&self) -> Option<(BTreeSet<u32>, FileLocal)> {
        let mut given = FileLocal::default();
        let mut any = false;
        for (r, _) in self.rels.derived.iter().enumerate().filter(|(_, d)| **d) {
            let rel = self.name(r as u32);
            if !self.f.given(rel) {
                continue;
            }
            any = true;
            for w in self.f.qualifiers_of(rel) {
                let cols = |c: &str| c.split(',').filter(|x| !x.is_empty()).filter_map(|x| x.parse::<usize>().ok()).collect::<Vector<usize>>();
                match w.split_once('=') {
                    Some(("part:loc", c)) => drop(given.locations.insert(rel.to_owned(), cols(c))),
                    Some(("part:same", c)) => drop(given.same_doc.insert(rel.to_owned(), cols(c))),
                    _ => match w {
                        "part:local" => {
                            given.local.insert(rel.to_owned());
                        }
                        "part:constant" => {
                            given.constants.insert(rel.to_owned());
                        }
                        "part:filter" => {
                            given.filters.insert(rel.to_owned());
                        }
                        _ => {}
                    },
                }
            }
            if !given.local.contains(rel) {
                given.causes.insert(rel.to_owned(), "cross-file where it is derived, another part".to_owned());
            }
        }
        any.then(|| ((0..self.rels.n as u32).filter(|&r| !self.rels.rules[r as usize].is_empty()).collect(), given))
    }

    /// Whether `rel` has an entry of stated facts (`Program::facts`), not an `.about` row.
    fn fact(&self, rel: &str) -> bool {
        self.f.facts(rel).is_some_and(|f| !f.about())
    }

    /// An input with one value for the whole run (`file_local::run_constant`).
    fn run_constant(&self, rel: &str) -> bool {
        option_input(rel) || (crate::code_rule_dsl::logic::plan::phased::is_phased(rel) && self.f.is_input(rel))
    }

    /// An input a file-local rule may join and stay file-local: the run's options alone. An earlier phase's export is
    /// a run constant, but not placeless: it is every file's rows together, and an edit's delta of it (a function
    /// added) is the reader's input delta (decision 0083 §2.3). A file-local rule that joined it made every file's
    /// memo entry a function of all its rows, so each such edit missed every file (the 50-file data-layer pack: 5.3 s
    /// an edit that adds a definition, 1.6 s one that does not). A rule reading one reads an input with no location:
    /// cross-file, maintained there by the export's delta.
    fn placeless_input(&self, rel: &str) -> bool {
        option_input(rel)
    }

    /// A derived relation's key: its columns and its rules' hashes, in the program's order.
    fn head_key(&self, r: u32) -> u64 {
        let mut h = hasher();
        self.f.columns(self.name(r)).hash(&mut h);
        for &k in &self.rels.rules[r as usize] {
            h.write_u64(self.rules[k].hash());
        }
        h.finish()
    }

    /// `keys` over an edit of rules alone (decision 0385): `was` with the keys of `heads` (the edit's) made again,
    /// where each is derived before and after (so the derived relations, which the global key reads, are as they
    /// were). `None` otherwise.
    fn keys_over(&self, was: &Keys, heads: &BTreeSet<String>) -> Option<Keys> {
        let mut keys = was.clone();
        for rel in heads {
            let r = self.v.rel(rel)?.0;
            // Derived before: keyed, and not by its declaration (a base relation's key).
            let before = was.heads.get(rel).is_some_and(|&k| self.base_key(rel) != Some(k));
            if !self.rels.is_derived(r) || self.rels.rules[r as usize].is_empty() || !before {
                return None;
            }
            keys.heads.insert(rel.clone(), self.head_key(r));
        }
        Some(keys)
    }

    fn keys(&self) -> Keys {
        let mut g = hasher();
        // The inputs a split asks about: derived, facts, or a phase's exports.
        let mut inputs: Vec<&str> = self.f.inputs().filter(|i| self.f.derives(i) || self.fact(i) || crate::code_rule_dsl::logic::plan::phased::is_phased(i)).collect();
        inputs.sort_unstable();
        inputs.hash(&mut g);
        let mut facts: Vec<&str> = self.v.all_facts().into_iter().filter(|f| !f.about()).map(|f| self.v.text(f.name())).collect();
        facts.sort_unstable();
        facts.hash(&mut g);
        // The `.about` rows: natives and lattices, and the rest, which the split does not read (a change there
        // splits whole, as a change of a native does).
        let mut about: Vector<u64> = self.v.about_facts().into_iter().map(|f| f.hash()).collect();
        about.sort_unstable();
        about.hash(&mut g);
        let mut heads: BTreeMap<String, u64> = BTreeMap::new();
        for r in 0..self.rels.n as u32 {
            if self.rels.is_derived(r) {
                heads.insert(self.name(r).to_owned(), self.head_key(r));
            }
        }
        // A base relation's declaration is its own key.
        for d in self.v.decls() {
            let rel = self.v.text(d.name());
            if !self.f.derives(rel) {
                heads.insert(rel.to_owned(), Self::decl_key(d.hash()));
            }
        }
        Keys { global: g.finish(), heads }
    }

    /// A base relation's key: its declaration's.
    fn decl_key(decl: u64) -> u64 {
        let mut h = hasher();
        b"base".hash(&mut h);
        decl.hash(&mut h);
        h.finish()
    }

    /// The key `rel` would have as a base relation, where it is declared.
    fn base_key(&self, rel: &str) -> Option<u64> {
        self.v.decls().into_iter().find(|d| self.v.text(d.name()) == rel).map(|d| Self::decl_key(d.hash()))
    }

    /// A base relation's location columns: node columns, the document column (`document_column`: `node`, `doc`,
    /// `pkg`, `syntax_error`, `cpython_syntax_error`, the source text's, the path's, the guests') and `docof(n, d)`'s
    /// document.
    fn base_locations(&self, rel: &str) -> Vector<usize> {
        // An undeclared one (a tree axis before lowering declares it) has none of its own, as the tree's.
        let mut columns: Vector<usize> = self.f.columns(rel).unwrap_or_default().iter().enumerate().filter(|(_, (_, t))| *t == "node").map(|(i, _)| i).collect();
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
    /// `prev` holds it (`split`).
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
        let mut round: Vector<u32> = (0..rels.n as u32).filter(|&r| scope[r as usize] && constant[r as usize]).collect();
        while !round.is_empty() {
            let leaving: Vector<u32> = round.iter().copied().filter(|&r| !rels.rules[r as usize].iter().all(|&k| forms.get(k).reads_constants(rels, &constant))).collect();
            for &r in &leaving {
                constant[r as usize] = false;
            }
            round = rels.heads_reading(leaving.iter().copied(), |reader| constant[reader as usize]);
        }
        constant
    }

    /// The split, computed for the derived relations in `scope` only (every one where `None`: the whole split), every
    /// other keeping its place in `prev`. That is exact when `scope` holds every relation whose rules changed since
    /// `prev` and every relation that reads one, transitively (`maintain`): a relation outside reads none inside, so its
    /// place in each fixpoint is what it was, and each fixpoint inside starts from the top with that boundary fixed.
    fn split(&self, scope: Option<&BTreeSet<u32>>, prev: Option<&FileLocal>) -> FileLocal {
        let rels = &self.rels;
        let n = rels.n;
        let in_scope: Vec<bool> = (0..n as u32).map(|r| rels.is_derived(r) && scope.is_none_or(|s| s.contains(&r))).collect();
        let was = |r: usize| prev.filter(|_| !in_scope[r]);
        let forms = Forms::new(self);
        let constant = self.constants(&forms, &in_scope, prev.map(|p| &p.constants));
        let placeless: Vec<bool> = (0..n).map(|r| rels.fact[r] || constant[r] || rels.placeless_input[r] || rels.chunk_value[r] || rels.test[r]).collect();
        // A derived relation's location columns: those every rule fills from a location.
        let mut locs: Vec<Vector<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vector::new();
                }
                let kept = was(r).and_then(|p| p.locations.get(self.name(r as u32)).cloned());
                kept.unwrap_or_else(|| (0..self.f.width(self.name(r as u32))).collect())
            })
            .collect();
        let mut round: Vector<u32> = (0..n as u32).filter(|&r| in_scope[r as usize]).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vector<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes(rels, &locs)).collect();
                    let kept: Vector<usize> = locs[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed(self.rules[k], c, p))).collect();
                    (kept != locs[r as usize]).then_some((r, kept))
                })
                .collect();
            let next = rels.heads_reading(changed.iter().map(|(r, _)| *r), |_| true);
            for (r, kept) in changed {
                locs[r as usize] = kept;
            }
            round = next;
        }
        let derived = || (0..n as u32).filter(|&r| rels.is_derived(r));
        let locations: BTreeMap<String, Vector<usize>> = derived().map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        // String filters (decision 0037).
        let mut filtering: Vec<bool> = (0..n).map(|r| rels.is_derived(r as u32) && was(r).is_none_or(|p| p.filters.contains(self.name(r as u32)))).collect();
        let mut round: Vector<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && filtering[r as usize]).collect();
        while !round.is_empty() {
            let leaving: Vector<u32> = round.iter().copied().filter(|&r| !rels.rules[r as usize].iter().all(|&k| forms.get(k).filters(self.rules[k], rels, &filtering))).collect();
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
        let cross_native = |rel: &str| {
            self.f.native(rel).is_some_and(|nat| match &nat.spec {
                ArchivedNativeSpec::TextTest { .. } => false,
                // A rank's rows are its groups' (`rank_form`): its rule's join on the group is judged as any rule's.
                ArchivedNativeSpec::Rank { .. } => false,
                ArchivedNativeSpec::PythonCandidates { targets, .. } => !constants.contains(self.v.rel_name(*targets)) && !fact_not_input(*targets),
                ArchivedNativeSpec::Python { targets, .. } => targets.is_some_and(|t| !constants.contains(self.v.rel_name(t)) && !fact_not_input(t)),
                _ => true,
            })
        };
        // A relation `locate_all` moved (`CROSS_FILE`): its rows name no document, so it is derived once over every file
        // (decision 0290).
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
                    None => moved(rel).then(|| (rel.to_owned(), "its rows name no document (moved by `locate`)".to_owned())),
                }
            })
            .collect();
        let mut dirty: Option<rerail_blocks::vector::Vector<usize>> = None;
        loop {
            let explain = |k: usize, local_now: &[bool]| self.why_not_local(self.rules[k], local_now, &constants, &filters, &locations);
            let check = |k: usize| -> Option<(u32, String)> {
                let head = rels.head[k];
                if forms.get(k).one_document(head, rels, &placeless, &local_now, &filtering, &locs) {
                    debug_assert!(explain(k, &local_now).is_none(), "the numbered check and the text's agree on {}", self.name(head));
                    return None;
                }
                let why = explain(k, &local_now);
                debug_assert!(why.is_some(), "the numbered check and the text's agree on {}", self.name(head));
                Some((head, why.unwrap_or_else(|| "places more than one document".to_owned())))
            };
            let failing: Vec<(u32, String)> = match &dirty {
                None => (0..self.rules.len()).filter(|&k| in_scope[rels.head[k] as usize] && local_now[rels.head[k] as usize]).filter_map(check).collect(),
                Some(rules) => rules.iter().copied().filter(|&k| local_now[rels.head[k] as usize]).filter_map(check).collect(),
            };
            let mut next: rerail_blocks::vector::Vector<usize> = rerail_blocks::vector::Vector::new();
            let mut left = false;
            for (r, why) in failing {
                if std::mem::replace(&mut local_now[r as usize], false) {
                    next.extend(rels.reading(r));
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
        // The same-document columns and the rest: `finish`.
        self.finish(FileLocal { local, constants, locations, filters, causes, ..Default::default() }, self.keys())
    }

    /// Why `rule` places more than one document, from its text: `None` where it does
    /// not. Asked only of a rule the numbered check fails (and, in a debug build, of each it passes).
    fn why_not_local(&self, rule: RuleRef<'v>, local_now: &[bool], constants: &BTreeSet<String>, filters: &BTreeSet<String>, locations: &BTreeMap<String, Vector<usize>>) -> Option<String> {
        let v = self.v;
        let head = v.rel_name(rule.head().rel());
        let var = |s: Slot| v.text(rule.var(s)).to_owned();
        let local = |rel: &str| v.rel(rel).is_some_and(|r| local_now[r.0 as usize]);
        let placed = ValueTapes::of(v, rule);
        let tape = |c: u32| format!("rr_value_tape:{}", var(Slot(c)));
        let mut groups: Vec<BTreeSet<String>> = Vec::new();
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
                L::Cmp { .. } => continue,
            };
            let rel = v.rel_name(atom.rel());
            let derived = self.f.derives(rel);
            let constant = (!derived && self.fact(rel) && !self.f.is_input(rel)) || constants.contains(rel) || self.placeless_input(rel);
            let chunk_value = !derived && CHUNK_VALUES.contains(&rel);
            if constant || chunk_value || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) || filters.contains(rel) || filters.contains(head) {
                continue;
            }
            if derived && !local(rel) {
                return Some(format!("reads cross-file {rel}"));
            }
            let columns = if derived { locations[rel].clone() } else { self.base_locations(rel) };
            let args: Vec<TermRef<'_>> = atom.args().collect();
            let mut vars: BTreeSet<String> = columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).collect();
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
        let equal: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (var(a), var(b))).collect();
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

/// What passes between a split's parts (`assemble`), read from the version, `local` the file-local relations: the
/// exports, the base relations the cross-file part reads (and those it decodes), and the file-local relations it
/// derives itself from a stored input (`projected`). Names alone: no part is made.
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
    // A part's relation a cross-file rule of another part reads (decision 0345 stage 2) is read cross-file here too.
    let elsewhere = |rel: &str| f.qualified(rel, crate::code_rule_dsl::logic::plan::fields::PART_CROSS_READ);
    let projected: BTreeSet<String> = cross_reads.iter().map(String::as_str).chain(local.iter().map(String::as_str).filter(|r| elsewhere(r))).filter(|r| is_local(r) && projection(r)).map(str::to_owned).collect();
    let in_cross = |rel: &str| !is_local(rel) || projected.contains(rel);
    let mut cross_reads = read(&|h| in_cross(h));
    cross_reads.extend(f.natives().iter().filter(|n| in_cross(v.rel_name(n.rel))).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r).to_owned()));
    cross_reads.extend(local.iter().filter(|r| elsewhere(r)).cloned());
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

/// A split's relations by `RelId`: what the split asks of each, asked once.
struct Rels<'v> {
    n: usize,
    derived_count: usize,
    derived: Vec<bool>,
    fact: Vec<bool>,
    placeless_input: Vec<bool>,
    chunk_value: Vec<bool>,
    test: Vec<bool>,
    text: Vec<Option<&'static [usize]>>,
    base_locs: Vec<Vector<usize>>,
    candidate: Vec<bool>,
    /// A derived relation's rules, by index in the program's order.
    rules: Vec<Vector<usize>>,
    /// The rules reading each relation, by index, each once, in order.
    reading: Vec<Vector<usize>>,
    /// Each rule's head.
    head: Vector<u32>,
    _v: std::marker::PhantomData<&'v ()>,
}

impl<'v> Rels<'v> {
    fn of(v: &'v Version, f: Fields<'v>, rules: &[RuleRef<'v>]) -> Rels<'v> {
        let n = v.rel_end() as usize;
        let mut derived = vec![false; n];
        let mut by_head: Vec<Vector<usize>> = vec![rerail_blocks::vector::Vector::new(); n];
        let mut reading: Vec<Vector<usize>> = vec![rerail_blocks::vector::Vector::new(); n];
        let mut read = vec![false; n];
        let head: Vector<u32> = rules.iter().map(|r| r.head().rel().0).collect();
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
        // A part's relation another part derives is derived (`fields::GIVEN`), by no rule here.
        for (r, d) in derived.iter_mut().enumerate() {
            if !*d && read[r] && f.given(name(r)) {
                *d = true;
            }
        }
        let base = |r: usize| !derived[r] && read[r];
        let test = |r: &str| r.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
        Rels {
            n,
            derived_count: derived.iter().filter(|d| **d).count(),
            fact: (0..n).map(|r| base(r) && p.fact(name(r)) && !f.is_input(name(r))).collect(),
            placeless_input: (0..n).map(|r| (derived[r] || read[r]) && p.placeless_input(name(r))).collect(),
            chunk_value: (0..n).map(|r| base(r) && CHUNK_VALUES.contains(&name(r))).collect(),
            test: (0..n).map(|r| (derived[r] || read[r]) && test(name(r))).collect(),
            text: (0..n).map(|r| if derived[r] || read[r] { crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(t, _)| *t == name(r)).map(|(_, c)| *c) } else { None }).collect(),
            base_locs: (0..n).map(|r| if base(r) { p.base_locations(name(r)) } else { rerail_blocks::vector::Vector::new() }).collect(),
            candidate: (0..n).map(|r| derived[r] && f.native(name(r)).is_none() && !f.lattice(name(r)) && !f.is_input(name(r))).collect(),
            rules: by_head,
            reading,
            head,
            derived,
            _v: std::marker::PhantomData,
        }
    }

    fn empty() -> Rels<'v> {
        Rels { n: 0, derived_count: 0, derived: Vec::new(), fact: Vec::new(), placeless_input: Vec::new(), chunk_value: Vec::new(), test: Vec::new(), text: Vec::new(), base_locs: Vec::new(), candidate: Vec::new(), rules: Vec::new(), reading: Vec::new(), head: rerail_blocks::vector::Vector::new(), _v: std::marker::PhantomData }
    }

    fn is_derived(&self, r: u32) -> bool {
        self.derived[r as usize]
    }

    fn reading(&self, r: u32) -> &[usize] {
        &self.reading[r as usize]
    }

    /// The derived relations whose rules read any of `rels`, each once, in order.
    fn heads_reading(&self, rels: impl Iterator<Item = u32>, keep: impl Fn(u32) -> bool) -> Vector<u32> {
        let mut out: Vector<u32> = rels.flat_map(|r| self.reading[r as usize].iter().map(|&k| self.head[k])).filter(|&h| keep(h)).collect();
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

/// The value tapes of a rule (decision 0058 §4, O96) and where they stand, by slot: each class under the rule's `a =
/// b` named by its least slot. A value is made in the chunk of the document whose row gave its text, and is that
/// document's: `parse_of(x, g, t)`'s t stands with the one atom that binds x (two atoms binding x join two documents
/// on a string, which is no one document's), and `splice_of(_, n, _, t3)`'s t3 with node n.
struct ValueTapes {
    /// Each value tape's class and the body index of the atom that gave its text.
    by_binder: BTreeMap<u32, usize>,
    /// Each spliced tape's node and tape class.
    splices: Vec<(Slot, u32)>,
    class: Vector<u32>,
}

impl ValueTapes {
    fn of(v: &Version, rule: RuleRef<'_>) -> ValueTapes {
        let n = rule.var_count();
        let mut root: Vector<u32> = (0..n as u32).collect();
        for l in rule.body() {
            if let L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } = l.view() {
                join(&mut root, a.0, b.0);
            }
        }
        let class: Vector<u32> = (0..n as u32).map(|s| find(&mut root, s)).collect();
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
                    let binders: Vector<usize> = body
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

/// A body literal as the split reads it.
enum Lit {
    Read { neg: bool, plain: bool, rel: u32, args: Vec<Option<u32>>, binder: bool, tapes: Vector<u32> },
    Ast,
    Nearest(u32, u32),
    Cmp,
}

/// A rule body numbered for the split: its slots, each with its class under the body's equalities;
/// then its value tapes; then one id per body literal.
struct Form {
    /// Whether a term of the body names the slot (a head's variable is found only then, as the tree's `Form` finds it
    /// by name).
    named: Vec<bool>,
    class: Vector<u32>,
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
        let mut root: Vector<u32> = (0..n as u32).collect();
        for (a, b) in super::var_equalities_in(v, rule) {
            join(&mut root, a.0, b.0);
        }
        let class: Vector<u32> = (0..n as u32).map(|s| find(&mut root, s)).collect();
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
                    let mut stand: Vector<u32> = placed.by_binder.iter().filter(|(_, b)| **b == index).map(|(t, _)| tape(*t)).collect();
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
                constant[r] || rels.placeless_input[r] || rels.fact[r]
            }
            Lit::Cmp => true,
            _ => false,
        })
    }

    fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vector<usize>]) -> Bits {
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
            Lit::Cmp => true,
            _ => false,
        });
        reads_only_text && rule.head().args().all(|t| self.var(t).is_some_and(|v| strings.has(v)))
    }

    fn one_document(&self, head: u32, rels: &Rels<'_>, placeless: &[bool], local: &[bool], filters: &[bool], locs: &[Vector<usize>]) -> bool {
        let head_filter = filters[head as usize];
        let mut root: Vector<u32> = (0..self.ids as u32).collect();
        let mut firsts: Vector<u32> = rerail_blocks::vector::Vector::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        let mut group: Vector<u32> = rerail_blocks::vector::Vector::new();
        for (index, l) in self.lits.iter().enumerate() {
            match l {
                Lit::Read { plain, rel, args, binder, tapes, .. } => {
                    let r = *rel;
                    let derived = rels.is_derived(r);
                    if placeless[r as usize] || (derived && filters[r as usize]) || head_filter {
                        continue;
                    }
                    if derived && !local[r as usize] {
                        return false;
                    }
                    let columns = if derived { &locs[r as usize] } else { &rels.base_locs[r as usize] };
                    group.clear();
                    group.extend(columns.iter().filter_map(|&c| args.get(c).copied().flatten().map(|v| self.class[v as usize])));
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
            }
        }
        let mut roots: Vector<u32> = firsts.iter().map(|&f| find(&mut root, f)).collect();
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
struct Bits(Vector<u64>);

impl Bits {
    fn new(n: usize) -> Bits {
        Bits(rerail_blocks::vector![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        self.0[i as usize / 64] |= 1 << (i % 64);
    }
    fn has(&self, i: u32) -> bool {
        self.0.get(i as usize / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
}
