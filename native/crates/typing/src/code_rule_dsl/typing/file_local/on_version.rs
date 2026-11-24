//! The file-local split, read from the program's version (decision 0281's path, step 5), as an analysis of it
//! (`FileLocal`), maintained across an edit (`Analysis::maintain`).
//!
//! A split of a program a few rules apart computes only what the difference reaches. The numbers are the image's
//! own: a relation is its `RelId`, a variable its `Slot`, and the last split is the analysis of the edit's input,
//! carried to its output by the manager, which hands `maintain` the edit.

use rerail_blocks::vector::Vector;
use super::{CHUNK_VALUES, document_column, option_input, path_key_column, setting_input};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Delta, Reads};
use crate::code_rule_dsl::logic::program_image::{ArchivedNativeSpec, CmpOp, Lit as L, NativeSpec, RelId, RuleRef, Slot, TermRef, Version};
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
    /// Each derived relation's nullable location columns (decision 0649): not its `locations`, but filled in every
    /// rule by a location or by `unset` (-1, no node). A rule joins through one only where the same rule places its
    /// variable by a location column too, which -1 never is; a row's document is never read off one.
    pub nullable: BTreeMap<String, Vector<usize>>,
    /// The undeclared crossings (decision 0652): each relation in the cross-file part that is not declared
    /// `cross_file`, whose own rule places rows of two documents, or which is made natively. Its value is the cause and
    /// the rule. Not one: a relation whose rules read a cross-file relation (it follows it), or are on one document
    /// but for an input naming no document (the run's settings, a phase's export, the files beyond the corpus).
    pub crossings: BTreeMap<String, String>,
    /// The relations `locate` moved (`MOVED_CROSS_FILE`) each rule of which reads one document's rows, plain and
    /// positive: a union, per-file by decision 0680 but with no document its rows can be named by (no `docof`, no
    /// located read). The cross-file part holds it as it did before 0680; no crossing (0652's "What is a crossing").
    pub moved_unions: BTreeSet<String>,
    /// The views its readers cover (decision 0652, `covers`): file-local though a rule of each joins documents, because
    /// each reader holds what makes the rows it reads. A covered view of values alone is a filter too.
    pub covered: BTreeSet<String>,
    /// What placed each covered view (`covers`): its readers, and theirs through each located reader whose head anchors
    /// it. Not in its rules' digests, so `maintain` asks a covered view again when one of these changed or went. A
    /// file-local view of values not covered has its readers here too (decision 0680): one going (a part keeping its
    /// file-local rules) may cover it.
    cover_deps: BTreeMap<String, BTreeSet<String>>,
    /// Each derived relation one document's in every row, file-local or not, with its location columns (two or more):
    /// its rules place them in one document through reads of such relations alone (`one_document`, a cross-file read
    /// taken where its own rows are each one document's). A demand that drops a read of one keeps its tie (0652).
    pub row_documents: BTreeMap<String, Vector<usize>>,
    /// Each derived relation's path keys (decision 0665): columns every rule fills from a file's path (`doc`'s:
    /// `path_key_column`) or from a location, beyond its `locations`. A path places a rule's
    /// document as a location does, so the split joins through it; a memo reads a row's document off a location's
    /// value, which a path is not, so `locations` (and `same_doc`, `row_documents`) leave it out, and `locate` names
    /// the document a path key's row is of through `doc` (`passes::Locate`).
    pub path_keys: BTreeMap<String, Vector<usize>>,
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
            && self.nullable == o.nullable
            && self.crossings == o.crossings
            && self.moved_unions == o.moved_unions
            && self.covered == o.covered
            && self.cover_deps == o.cover_deps
            && self.row_documents == o.row_documents
            && self.path_keys == o.path_keys
            && self.no_document == o.no_document
            && self.keys == o.keys
    }
}

impl FileLocal {
    /// A derived relation's columns that place its row's document (decision 0665): its locations and its path keys.
    pub fn placing(&self, rel: &str) -> Vector<usize> {
        let mut c: Vector<usize> = self.locations.get(rel).into_iter().flat_map(|c| c.iter()).chain(self.path_keys.get(rel).into_iter().flatten()).copied().collect();
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
        // A part's split (decision 0345 stage 2): its own relations, the ones another part derives read as given.
        match p.given_split() {
            Some((scope, given)) => Ok(p.split(Some(&scope), Some(&given))),
            None => Ok(p.split(None, None)),
        }
    }
    /// Its fields as rows, each tagged by its field (decision 0333 §4): `(0, rel)` file-local, `(1, rel)` constant,
    /// `(2, rel, columns)` locations, `(3, rel)` filters, `(4, rel, cause)`, `(5, rel, columns)` same-document,
    /// `(6, no document)`, its keys, `(7, global)` and `(8, head, key)`, and `(9, rel, columns)` nullable locations, `(10, rel, cause)` crossings
    /// and `(11, rel)` covered views, `(12, rel, dep)` what placed each, `(13, rel, columns)` one document's rows, and
    /// `(14, rel, columns)` path keys.
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
        out.extend(self.nullable.iter().map(|(r, c)| vec![tag(9), str(r), cols(c)]));
        out.extend(self.crossings.iter().map(|(r, c)| vec![tag(10), str(r), str(c)]));
        out.extend(self.covered.iter().map(|r| vec![tag(11), str(r)]));
        out.extend(self.cover_deps.iter().flat_map(|(r, d)| d.iter().map(move |x| (r, x))).map(|(r, x)| vec![tag(12), str(r), str(x)]));
        out.extend(self.row_documents.iter().map(|(r, c)| vec![tag(13), str(r), cols(c)]));
        out.extend(self.path_keys.iter().map(|(r, c)| vec![tag(14), str(r), cols(c)]));
        out.extend(self.moved_unions.iter().map(|r| vec![tag(15), str(r)]));
        Some(out)
    }
    fn of_rows(_v: &Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<FileLocal> {
        use crate::code_rule_dsl::logic::front::Value;
        let cols = |s: &str| -> Option<Vector<usize>> { if s.is_empty() { Some(Vector::new()) } else { s.split(',').map(|x| x.parse::<usize>().ok()).collect() } };
        let mut f = FileLocal::default();
        for r in rows {
            match r.as_slice() {
                [Value::Int(0), Value::Str(n)] => drop(f.local.insert(n.to_string())),
                [Value::Int(1), Value::Str(n)] => drop(f.constants.insert(n.to_string())),
                [Value::Int(2), Value::Str(n), Value::Str(c)] => drop(f.locations.insert(n.to_string(), cols(c)?.into_iter().collect())),
                [Value::Int(3), Value::Str(n)] => drop(f.filters.insert(n.to_string())),
                [Value::Int(4), Value::Str(n), Value::Str(c)] => drop(f.causes.insert(n.to_string(), c.to_string())),
                [Value::Int(5), Value::Str(n), Value::Str(c)] => drop(f.same_doc.insert(n.to_string(), cols(c)?.into_iter().collect())),
                [Value::Int(6), Value::Int(b)] => f.no_document = *b != 0,
                [Value::Int(7), Value::Int(k)] => f.keys.global = *k as u64,
                [Value::Int(8), Value::Str(h), Value::Int(k)] => drop(f.keys.heads.insert(h.to_string(), *k as u64)),
                [Value::Int(9), Value::Str(n), Value::Str(c)] => drop(f.nullable.insert(n.to_string(), cols(c)?)),
                [Value::Int(10), Value::Str(n), Value::Str(c)] => drop(f.crossings.insert(n.to_string(), c.to_string())),
                [Value::Int(11), Value::Str(n)] => drop(f.covered.insert(n.to_string())),
                [Value::Int(12), Value::Str(n), Value::Str(x)] => drop(f.cover_deps.entry(n.to_string()).or_default().insert(x.to_string())),
                [Value::Int(13), Value::Str(n), Value::Str(c)] => drop(f.row_documents.insert(n.to_string(), cols(c)?)),
                [Value::Int(14), Value::Str(n), Value::Str(c)] => drop(f.path_keys.insert(n.to_string(), cols(c)?)),
                [Value::Int(15), Value::Str(n)] => drop(f.moved_unions.insert(n.to_string())),
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
    /// `holds_root`'s answers, by relation, column and root.
    holds: std::cell::RefCell<std::collections::HashMap<(u32, usize, u32), bool>>,
}

impl<'v> Program<'v> {
    fn of(v: &'v Version, f: Fields<'v>) -> Program<'v> {
        let rules: Vec<RuleRef<'v>> = v.rules().collect();
        let rels = Rels::of(v, f, &rules);
        Program { v, f, rules, rels, holds: Default::default() }
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
                    Some(("part:loc", c)) => drop(given.locations.insert(rel.to_owned(), cols(c).into_iter().collect())),
                    Some(("part:key", c)) => drop(given.path_keys.insert(rel.to_owned(), cols(c))),
                    Some(("part:same", c)) => drop(given.same_doc.insert(rel.to_owned(), cols(c).into_iter().collect())),
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
        option_input(rel) || setting_input(rel) || (crate::code_rule_dsl::logic::plan::phased::is_phased(rel) && self.f.is_input(rel))
    }

    /// An input a file-local rule may join and stay file-local: the run's options and its settings (decision 0665:
    /// the configuration files' rows, which a source file's edit does not change). An earlier phase's export is
    /// a run constant, but not placeless: it is every file's rows together, and an edit's delta of it (a function
    /// added) is the reader's input delta (decision 0083 §2.3). A file-local rule that joined it made every file's
    /// memo entry a function of all its rows, so each such edit missed every file (the 50-file data-layer pack: 5.3 s
    /// an edit that adds a definition, 1.6 s one that does not). A rule reading one reads an input with no location:
    /// cross-file, maintained there by the export's delta.
    fn placeless_input(&self, rel: &str) -> bool {
        option_input(rel) || setting_input(rel)
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

    /// A base relation's columns that place a rule's document (decision 0665): its locations (`base_locations`) and its
    /// path key (`path_key_column`).
    fn base_placing(&self, rel: &str) -> Vector<usize> {
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
        // A derived relation's placing columns: those every rule fills from a location or a file's path (decision
        // 0665). The split places by these; what it publishes as locations is those filled from a location (`real`).
        let mut locs: Vec<Vector<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vector::new();
                }
                let rel = self.name(r as u32);
                let kept = was(r).filter(|p| p.locations.contains_key(rel)).map(|p| p.placing(rel));
                kept.unwrap_or_else(|| (0..self.f.width(rel)).collect())
            })
            .collect();
        let mut round: Vector<u32> = (0..n as u32).filter(|&r| in_scope[r as usize]).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vector<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes(rels, &locs)).collect();
                    let kept: Vector<usize> = locs[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed_built(self.v, self.rules[k], c, p))).collect();
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
        let mut real: Vec<Vector<usize>> = (0..n)
            .map(|r| match was(r) {
                Some(p) => p.locations.get(self.name(r as u32)).cloned().unwrap_or_default(),
                None => locs[r].clone(),
            })
            .collect();
        let mut round: Vector<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && !real[r as usize].is_empty()).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vector<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes_over(rels, &real, &rels.base_real)).collect();
                    let kept: Vector<usize> = real[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed(self.rules[k], c, p))).collect();
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
        let mut nulls: Vec<Vector<usize>> = (0..n)
            .map(|r| {
                if !rels.is_derived(r as u32) {
                    return Vector::new();
                }
                match was(r) {
                    Some(p) => p.nullable.get(self.name(r as u32)).cloned().unwrap_or_default(),
                    None => (0..self.f.width(self.name(r as u32))).filter(|c| !locs[r].contains(c)).collect(),
                }
            })
            .collect();
        let mut round: Vector<u32> = (0..n as u32).filter(|&r| in_scope[r as usize] && !nulls[r as usize].is_empty()).collect();
        while !round.is_empty() {
            let changed: Vec<(u32, Vector<usize>)> = round
                .iter()
                .filter_map(|&r| {
                    let rules = &rels.rules[r as usize];
                    let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_or_null(rels, &locs, &nulls)).collect();
                    let kept: Vector<usize> = nulls[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed_or_unset(self.rules[k], c, p))).collect();
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
        let placing: BTreeMap<String, Vector<usize>> = derived().map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        let locations: BTreeMap<String, Vector<usize>> = derived().map(|r| (self.name(r).to_owned(), real[r as usize].clone())).collect();
        let path_keys: BTreeMap<String, Vector<usize>> = derived()
            .map(|r| (self.name(r).to_owned(), locs[r as usize].iter().copied().filter(|c| !real[r as usize].contains(c)).collect::<Vector<usize>>()))
            .filter(|(_, c)| !c.is_empty())
            .collect();
        let nullable: BTreeMap<String, Vector<usize>> = derived().filter(|&r| !nulls[r as usize].is_empty()).map(|r| (self.name(r).to_owned(), nulls[r as usize].clone())).collect();
        // String filters (decision 0037).
        // A covered view of values is in `filters` too (read as a test, below), but it is no string filter: outside the
        // scope a relation is one where it was before that.
        let mut filtering: Vec<bool> = (0..n)
            .map(|r| {
                let rel = self.name(r as u32);
                rels.is_derived(r as u32) && was(r).is_none_or(|p| p.filters.contains(rel) && !p.covered.contains(rel))
            })
            .collect();
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
        // A native relation is judged by its rules, which the lowering keeps beside it with its rows' shape (decision
        // 0652): a walk (`dfs_first`, `reach`, `components`) over edges of one document each stays in it, and one over
        // edges between documents is cross-file by its rules as any rule is. A function of strings (`match_all`, a
        // text test) is judged by its input (`functional`). Python's natives over a cross-file target are made in the
        // cross-file part.
        let cross_native = |rel: &str| {
            self.f.native(rel).is_some_and(|nat| match &nat.spec {
                ArchivedNativeSpec::PythonCandidates { targets, .. } => !constants.contains(self.v.rel_name(*targets)) && !fact_not_input(*targets),
                ArchivedNativeSpec::Python { targets, .. } => targets.is_some_and(|t| !constants.contains(self.v.rel_name(t)) && !fact_not_input(t)),
                _ => false,
            })
        };
        // A relation `locate_all` moved (`CROSS_FILE`): its rows name no document, so it is derived once over every file
        // (decision 0290). One the cross part reads back from its located form (`READ_LOCATED`, decision 0665).
        let moved = |rel: &str| self.f.qualified(rel, super::CROSS_FILE) || self.f.qualified(rel, super::READ_LOCATED);
        // A derived relation of values with stated facts besides its rules (`py_want("builtins")`): those rows are no
        // document's, so it is no union of one file's rows each (decision 0680), and no located form could carry them.
        let stated = |r: usize| locs[r].is_empty() && self.fact(self.name(r as u32));
        let mut local_now: Vec<bool> = (0..n)
            .map(|r| {
                rels.is_derived(r as u32)
                    && match was(r) {
                        Some(p) => p.local.contains(self.name(r as u32)),
                        None => !cross_native(self.name(r as u32)) && !moved(self.name(r as u32)) && !stated(r),
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
                    None if stated(r as usize) => Some((rel.to_owned(), "its stated facts name no document".to_owned())),
                    None => moved(rel).then(|| (rel.to_owned(), "declared `cross_file`".to_owned())),
                }
            })
            .collect();
        // The views its readers cover (decision 0652): a relation of values alone (no location column), read only by
        // file-local rules, each of whose rules finds its rows in every reader's own rows (`covers`). Derived in each
        // chunk, it answers each reader as it would over every file, and its readers read it as a test. Assumed of
        // each candidate, as file-local is, and given up where `covers` fails.
        let family_candidate: Vec<bool> = (0..n)
            .map(|r| {
                let rel = self.name(r as u32);
                rels.candidate[r]
                    && !rels.output[r]
                    && !rels.functional[r]
                    && !rels.rules[r].is_empty()
                    && nulls[r].is_empty()
                    && !placeless[r]
                    && !filtering[r]
                    && !moved(rel)
                    && !self.f.qualified(rel, crate::code_rule_dsl::logic::plan::fields::PART_CROSS_READ)
            })
            .collect();
        let cover_candidate: Vec<bool> = (0..n).map(|r| in_scope[r] && family_candidate[r]).collect();
        // A keyed family is judged over every relation, out of the scope too (decision 0680): its members' places
        // there are as before, and a member's readers in the scope read it as a whole split does.
        let family_values: Vec<bool> = (0..n).map(|r| family_candidate[r] && locs[r].is_empty()).collect();
        // A located candidate is asked when a rule of it places more than one document; one whose rules are each on one
        // document is file-local as any relation, and asked only as a reader whose head places a view's location
        // columns (`covers`, `Anchoring`). A candidate of values alone is asked always: its readers read it as a test.
        let valueless: Vec<bool> = (0..n).map(|r| cover_candidate[r] && locs[r].is_empty()).collect();
        // The relations each row of which is one document's (decision 0680): file-local, and no view its readers cover
        // (one may join documents). A candidate is assumed covered here.
        let whole_assumed: Vec<bool> = (0..n).map(|r| local_now[r] && !cover_candidate[r]).collect();
        let family_assumed = self.keyed(&local_now, &family_values, &placeless, &locs);
        // Outside the scope a view keeps its place, covered or not.
        let mut covered_now: Vec<bool> = (0..n)
            .map(|r| {
                if let Some(p) = was(r) {
                    return p.covered.contains(self.name(r as u32));
                }
                cover_candidate[r]
                    && (valueless[r] || rels.rules[r].iter().any(|&k| !forms.get(k).one_document(r as u32, rels, &placeless, &local_now, &filtering, &valueless, &locs, &nulls, &whole_assumed, &family_assumed, &|root, x| self.root_held(self.rules[k], root, x, &placeless, &locs))))
            })
            .collect();
        let mut dirty: Option<Vector<usize>> = None;
        loop {
            // A covered view whose readers changed is asked again; one no longer covered is read as any relation, by
            // its own rules and its readers.
            let asked: Vector<u32> = match &dirty {
                None => (0..n as u32).filter(|&r| in_scope[r as usize] && covered_now[r as usize]).collect(),
                Some(rules) => {
                    let mut heads: Vector<u32> = rules.iter().map(|&k| rels.head[k]).filter(|&h| in_scope[h as usize] && covered_now[h as usize]).collect();
                    heads.sort_unstable();
                    heads.dedup();
                    heads
                }
            };
            let mut also: Vector<usize> = Vector::new();
            let mut asked = asked;
            let anchoring = Anchoring { candidate: &cover_candidate, memo: Default::default() };
            let keyed = self.keyed(&local_now, &family_values, &placeless, &locs);
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
            let whole: Vec<bool> = (0..n).map(|r| local_now[r] && !covered_now[r]).collect();
            let explain = |k: usize, local_now: &[bool]| self.why_not_local(k, local_now, &constants, &read_as_tests, &placing, &nullable, &whole, &keyed, &placeless, &locs);
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
                if forms.get(k).one_document(head, rels, &placeless, &local_now, &filtering, &skip, &locs, &nulls, &whole, &keyed, &|root, x| self.root_held(self.rules[k], root, x, &placeless, &locs)) {
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
                    let mut rules: Vector<usize> = rules.iter().copied().chain(also).collect();
                    rules.sort_unstable();
                    rules.dedup();
                    rules.into_iter().filter(|&k| local_now[rels.head[k] as usize]).filter_map(check).collect()
                }
            };
            let mut next: Vector<usize> = Vector::new();
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
            .filter(|&r| local_now[r as usize] && (covered_now[r as usize] || family_values[r as usize]))
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
        let whole: Vec<bool> = (0..n).map(|r| local_now[r] && !covered_now[r]).collect();
        let keyed = self.keyed(&local_now, &family_values, &placeless, &locs);
        // A covered view of values is read as a filter is: a part given it reads it so (`part:filter`).
        let filters: BTreeSet<String> = filters.into_iter().chain(derived().filter(|&r| skip[r as usize]).map(|r| self.name(r).to_owned())).collect();
        // The undeclared crossings (decision 0652): a cross-file relation's rule that reads no cross-file relation and,
        // with every input naming no document taken as placeless, still does not place one document.
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
        let mut moved_unions: BTreeSet<String> = BTreeSet::new();
        for r in derived() {
            let (ru, rel) = (r as usize, self.name(r));
            if self.f.qualified(rel, super::MOVED_CROSS_FILE) {
                let union_only = match was(ru) {
                    Some(p) => p.moved_unions.contains(rel),
                    None => !rels.rules[ru].is_empty() && rels.rules[ru].iter().all(|&k| union(k)),
                };
                if union_only {
                    moved_unions.insert(rel.to_owned());
                }
            }
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
                // A read of a located form at no document (`want_at(_, q)`, decision 0680) reads the chunks' rows, as a
                // read of the relation read back from it did (`READ_LOCATED`): it follows them, as of a cross-file one.
                let reads_cross = form.lits.iter().any(|l| match l {
                    Lit::Read { rel: x, args, .. } => (rels.is_derived(*x) && !local_now[*x as usize] && !placeless[*x as usize]) || (rels.twin_of[*x as usize].is_some() && args.first().copied().flatten().is_none()),
                    _ => false,
                });
                if reads_cross || union(k) || form.one_document(r, rels, &placeless_or_input, &local_now, &filtering, &skip, &locs, &nulls, &whole, &keyed, &|root, x| self.root_held(self.rules[k], root, x, &placeless, &locs)) {
                    continue;
                }
                let why = self.why_not_local(k, &local_now, &constants, &filters, &placing, &nullable, &whole, &keyed, &placeless, &locs).unwrap_or_else(|| "places more than one document".to_owned());
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
        let mut round: Vector<u32> = derived().collect();
        while !round.is_empty() {
            let leaving: Vector<u32> = round
                .iter()
                .copied()
                .filter(|&r| one_row[r as usize] && !single(r) && !rels.rules[r as usize].iter().all(|&k| forms.get(k).one_document(r, rels, &placeless_or_input, &one_row, &filtering, &skip, &locs, &nulls, &whole, &keyed, &|root, x| self.root_held(self.rules[k], root, x, &placeless, &locs))))
                .collect();
            for &r in &leaving {
                one_row[r as usize] = false;
            }
            round = rels.heads_reading(leaving.into_iter(), |h| one_row[h as usize]);
        }
        // Their location columns alone (decision 0665): a demand ties two by `@doc`, which a path is not.
        let row_documents: BTreeMap<String, Vector<usize>> = derived().filter(|&r| one_row[r as usize] && real[r as usize].len() > 1).map(|r| (self.name(r).to_owned(), real[r as usize].clone())).collect();
        // The same-document columns and the rest: `finish`.
        self.finish(FileLocal { local, constants, locations, filters, causes, nullable, crossings, moved_unions, covered, cover_deps, row_documents, path_keys, ..Default::default() }, self.keys())
    }

    /// The keyed families of relations of values: by relation, its family's root. A root is a file-local relation of
    /// one column and values alone (`brace_value(v) :- format_call(_, v)`, `num_source(t)`), its own root. A member is a
    /// candidate view of values (`valueless`) each rule of which fills its first column with a variable `x` and reads
    /// only its family at `x` (in its first column; positive, negated or aggregated: `brace_rest(v, r, i)`,
    /// `!brace_bad(v)`, `count : { brace_arg(v, k, "") }`), placeless relations and comparisons, the family having one
    /// root. A member's rows at a key follow from its root's row at that key alone, so they are the same in a chunk as
    /// over every file wherever the chunk holds that root row (`covers_keyed`). The greatest such assignment.
    /// Whether `reader` holds family root `root`'s row at its variable `slot` through its own reads (decision 0680), so
    /// the chunk derives that row: one of the root's rules maps into it exactly there (`maps_at`, as `key_held` asks),
    /// or it reads positively a relation each of whose rules holds the root at the column it reads there
    /// (`holds_root`: `p121_open_call(c, v, n) :- format_call(c, v), …`).
    fn root_held(&self, reader: RuleRef<'v>, root: u32, slot: u32, placeless: &[bool], locs: &[Vector<usize>]) -> bool {
        let rels = &self.rels;
        let columns = |x: u32| -> &[usize] { if rels.is_derived(x) { &locs[x as usize] } else { &rels.base_locs[x as usize] } };
        let qc = value_classes(reader);
        let key = [Some(MapT::Var(qc[slot as usize]))];
        if rels.rules[root as usize].iter().any(|&k| maps_at(self.v, self.rules[k], reader, None, &key, &[], &|x| placeless[x as usize], &columns, &|x| rels.twin_of[x as usize], true)) {
            return true;
        }
        reader.body().any(|l| match l.view() {
            L::Atom(a) if !a.neg() && rels.is_derived(a.rel().0) && a.rel().0 != root => {
                a.args().enumerate().any(|(c, t)| matches!(t, TermRef::Var(y) if qc[y.0 as usize] == qc[slot as usize]) && self.holds_root(a.rel().0, c, root, placeless, locs))
            }
            _ => false,
        })
    }

    /// Whether every rule of `rel` holds family root `root`'s row at its head's column `col` (`root_held`). A relation
    /// met again is taken to hold it: each row has a derivation from rules that do, by induction.
    fn holds_root(&self, rel: u32, col: usize, root: u32, placeless: &[bool], locs: &[Vector<usize>]) -> bool {
        let key = (rel, col, root);
        if let Some(&known) = self.holds.borrow().get(&key) {
            return known;
        }
        self.holds.borrow_mut().insert(key, true);
        let rules = &self.rels.rules[rel as usize];
        let holds = !rules.is_empty()
            && rules.iter().all(|&k| {
                let rule = self.rules[k];
                matches!(rule.head().args().nth(col), Some(TermRef::Var(x)) if self.root_held(rule, root, x.0, placeless, locs))
            });
        self.holds.borrow_mut().insert(key, holds);
        holds
    }

    fn keyed(&self, local_now: &[bool], valueless: &[bool], placeless: &[bool], locs: &[Vector<usize>]) -> Vec<Option<u32>> {
        let rels = &self.rels;
        let n = rels.n;
        // A relation that may be a root: one that is no member is one, where it is file-local by its own rules.
        let rootable = |r: usize| rels.is_derived(r as u32) && local_now[r] && locs[r].is_empty() && !placeless[r] && !rels.rules[r].is_empty() && self.f.width(self.name(r as u32)) == 1;
        // `None`: not in a family; `Some(None)`: in one, its root not yet known; `Some(Some(d))`: of root `d`.
        let mut at: Vec<Option<Option<u32>>> = (0..n).map(|r| (valueless[r] || rootable(r)).then_some(None)).collect();
        loop {
            let mut changed = false;
            for r in 0..n {
                if at[r].is_none() || at[r] == Some(Some(r as u32)) {
                    continue;
                }
                // Each rule's family reads at its key, and their roots.
                let mut roots: BTreeSet<u32> = BTreeSet::new();
                let mut fits = true;
                'rules: for &k in &rels.rules[r] {
                    let rule = self.rules[k];
                    let classes = value_classes(rule);
                    let Some(TermRef::Var(x)) = rule.head().args().next() else {
                        fits = false;
                        break;
                    };
                    let key = classes[x.0 as usize];
                    for l in rule.body() {
                        let a = match l.view() {
                            L::Atom(a) | L::Agg { atom: a, .. } => a,
                            L::Cmp { .. } => continue,
                            _ => {
                                fits = false;
                                break 'rules;
                            }
                        };
                        let s = a.rel().0 as usize;
                        if placeless[s] {
                            continue;
                        }
                        let in_family = matches!(at[s], Some(_));
                        let at_key = matches!(a.args().next(), Some(TermRef::Var(y)) if classes[y.0 as usize] == key);
                        if !in_family || !at_key {
                            fits = false;
                            break 'rules;
                        }
                        if let Some(Some(d)) = at[s] {
                            roots.insert(d);
                        }
                    }
                }
                // A member of the root now known (a known root is not forgotten: the reads that gave it stay); else a
                // root of its own, or out of every family.
                let next = if fits && roots.len() <= 1 {
                    Some(roots.into_iter().next())
                } else if fits || !rootable(r) {
                    None
                } else {
                    Some(Some(r as u32))
                };
                if next != at[r] && !(next == Some(None) && at[r].is_some()) {
                    at[r] = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        at.into_iter().map(|x| x.flatten()).collect()
    }

    /// Whether every reader of keyed member `r` (`keyed`, its root `root`) holds its root's row at the key it reads `r`
    /// at (`key_held`), so the chunk derives the root's row, and `r`'s rows at that key, as over every file.
    fn covers_keyed(&self, r: u32, root: u32, keyed: &[Option<u32>], local_now: &[bool], placeless: &[bool], locs: &[Vector<usize>]) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        self.key_held(r, 0, root, keyed, local_now, placeless, locs, &mut seen)
    }

    /// Whether every reader of `rel` holds `root`'s row at the value it reads in `rel`'s column `col`. Each is
    /// file-local; one in the root's family reads `rel` at its own key (`col` 0). Any other reads `rel` (plainly or
    /// aggregated) with a variable there, and either one of the root's rules maps into it exactly at that variable
    /// (`maps_at`: its own document holds the root's row), or its head passes the variable on (an aggregate's helper,
    /// `rr_h_agg_…(c, v, k) :- brace_name(v, k, _, _), …`) and every reader of the head holds the root's row there in
    /// turn. A relation met again is assumed to (its readers outside the cycle are asked); an output never does.
    #[allow(clippy::too_many_arguments)]
    fn key_held(&self, rel: u32, col: usize, root: u32, keyed: &[Option<u32>], local_now: &[bool], placeless: &[bool], locs: &[Vector<usize>], seen: &mut BTreeSet<(u32, usize)>) -> Result<(), String> {
        let rels = &self.rels;
        if !seen.insert((rel, col)) {
            return Ok(());
        }
        if rels.output[rel as usize] {
            return Err(format!("{} is an output: its key is held by no reader", self.name(rel)));
        }
        let columns = |x: u32| -> &[usize] { if rels.is_derived(x) { &locs[x as usize] } else { &rels.base_locs[x as usize] } };
        let readers = rels.reading(rel);
        if readers.is_empty() {
            return Err(format!("{} has no readers", self.name(rel)));
        }
        for &j in readers {
            let h = rels.head[j];
            if !local_now[h as usize] {
                return Err(format!("reader {} not local", self.name(h)));
            }
            // A file-local member of the family reads it at its own key.
            if col == 0 && keyed[h as usize] == Some(root) {
                continue;
            }
            let reader = self.rules[j];
            let qc = value_classes(reader);
            for l in reader.body() {
                let a = match l.view() {
                    L::Atom(a) | L::Agg { atom: a, .. } if a.rel().0 == rel => a,
                    _ => continue,
                };
                let Some(TermRef::Var(x)) = a.args().nth(col) else { return Err(format!("reader reads it at no variable: {}", self.v.show_rule(reader))) };
                let key = [Some(MapT::Var(qc[x.0 as usize]))];
                if rels.rules[root as usize].iter().any(|&k| maps_at(self.v, self.rules[k], reader, None, &key, &[], &|x| placeless[x as usize], &columns, &|x| rels.twin_of[x as usize], true)) {
                    continue;
                }
                let passed: Vec<usize> = reader.head().args().enumerate().filter(|(_, t)| matches!(t, TermRef::Var(y) if qc[y.0 as usize] == qc[x.0 as usize])).map(|(c, _)| c).collect();
                if !passed.iter().any(|&c| self.key_held(h, c, root, keyed, local_now, placeless, locs, seen).is_ok()) {
                    return Err(format!("reader holds no row of {} at its key: {}", self.name(root), self.v.show_rule(reader)));
                }
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
    fn covers(&self, r: u32, local_now: &[bool], covered: &[bool], valueless: &[bool], placeless: &[bool], locs: &[Vector<usize>], anchoring: &Anchoring<'_>) -> Result<(), String> {
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
                        // A rule found nowhere in the reader is covered by one found there with the same head and the same
                        // literals beside its located reads (`same_rest`): a disjunction's branches over two reads
                        // (`negate(a, b) :- (number(_, a, _) ; number(_, _, a)), …`). Their tests are of the head's
                        // values alone, so where one's row is over every file the other's is in the reader's chunk.
                        let found: Vec<bool> = own.iter().map(|&rule| maps(self.v, rule, reader, at, columns(r), &|x| placeless[x as usize], &columns, &|x| rels.twin_of[x as usize])).collect();
                        let lost = own.iter().zip(&found).find(|(rule, ok)| !**ok && !own.iter().zip(&found).any(|(other, ok)| *ok && same_rest(self.v, **rule, *other, &|x| placeless[x as usize])));
                        if let Some((rule, _)) = lost {
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

    /// Why `rule` places more than one document, from its text: `None` where it does
    /// not. Asked only of a rule the numbered check fails (and, in a debug build, of each it passes).
    fn why_not_local(&self, k: usize, local_now: &[bool], constants: &BTreeSet<String>, filters: &BTreeSet<String>, locations: &BTreeMap<String, Vector<usize>>, nullable: &BTreeMap<String, Vector<usize>>, whole: &[bool], family: &[Option<u32>], placeless: &[bool], locs: &[Vector<usize>]) -> Option<String> {
        let v = self.v;
        let rule = self.rules[k];
        let equal_names: Vec<(String, String)> = super::var_equalities_in(v, rule).into_iter().map(|(a, b)| (v.text(rule.var(a)).to_owned(), v.text(rule.var(b)).to_owned())).collect();
        // The variables a location column of a positive read places (`one_document`'s `strict`): a nullable
        // column's variable among them is no -1.
        let mut strict: BTreeSet<String> = BTreeSet::new();
        for l in rule.body() {
            if let L::Atom(a) | L::Agg { atom: a, .. } = l.view()
                && !a.neg()
            {
                let rel = v.rel_name(a.rel());
                let columns = if self.f.derives(rel) { locations.get(rel).cloned().unwrap_or_default() } else { self.base_placing(rel) };
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
        // `one_document`'s keyed family reads (decision 0680): by family root and key variable (the least name equal to
        // it); held by the first positive read's name where no located read holds the root.
        let family_read = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| -> Option<(u32, String, Slot, bool)> {
            let r = a.rel().0 as usize;
            let (q, at, twin) = match self.rels.twin_of.get(r).copied().flatten() {
                Some(q) => (q, 1, true),
                None => (r as u32, 0, false),
            };
            if !self.rels.is_derived(q) || !local_now[q as usize] {
                return None;
            }
            let x = a.args().nth(at)?.var()?;
            let mut names = BTreeSet::from([var(x)]);
            super::close_under(&mut names, &equal_names);
            Some((family[q as usize]?, names.into_iter().next()?, x, twin))
        };
        let root_held = |root: u32, x: Slot| self.root_held(rule, root, x.0, placeless, locs);
        let mut held: Vec<((u32, String), String)> = Vec::new();
        let mut tested: Vec<((u32, String), bool)> = Vec::new();
        for (index, l) in rule.body().enumerate() {
            if let L::Atom(a) = l.view()
                && !a.neg()
                && let Some((root, key, x, _)) = family_read(a)
                && !root_held(root, x)
                && !held.iter().any(|(h, _)| *h == (root, key.clone()))
            {
                held.push(((root, key), format!("rr_value_key:{index}")));
            }
        }
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
            let constant = (!derived && self.fact(rel) && !self.f.is_input(rel)) || constants.contains(rel) || self.placeless_input(rel);
            let chunk_value = !derived && CHUNK_VALUES.contains(&rel);
            let functional = local(rel) && (self.f.native(rel).is_some_and(|nat| matches!(nat.spec, ArchivedNativeSpec::MatchAll { .. })) || self.f.qualified(rel, super::STRATEGY));
            if constant || chunk_value || functional || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) || filters.contains(head) {
                continue;
            }
            if filters.contains(rel) {
                tested.extend(family_read(atom).map(|(root, key, ..)| ((root, key), plain && !atom.neg())));
                continue;
            }
            if derived && !local(rel) {
                return Some(format!("reads cross-file {rel}"));
            }
            let family = if derived { family_read(atom) } else { None };
            let at_key = family.as_ref().and_then(|(root, key, ..)| held.iter().find(|(h, _)| h.0 == *root && h.1 == *key).map(|(_, at)| at.clone()));
            if let Some((root, _, x, twin)) = &family {
                let named = *twin && atom.args().next().and_then(|t| t.var()).is_some();
                if root_held(*root, *x) && !named {
                    continue;
                }
                if let (Some(at), false) = (&at_key, *twin) {
                    groups.push(BTreeSet::from([at.clone()]));
                    continue;
                }
            }
            let columns = if derived { locations[rel].clone() } else { self.base_placing(rel) };
            let args: Vec<TermRef<'_>> = atom.args().collect();
            let mut vars: BTreeSet<String> = columns.iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(var).collect();
            // `one_document`'s path written out (decision 0665): each string one document's.
            vars.extend(columns.iter().filter_map(|&c| match args.get(c) {
                Some(TermRef::Str(x)) => Some(format!("rr_value_path:{}", x.0)),
                _ => None,
            }));
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
            // `one_document`'s read of rows each one document's, naming no variable of it (decision 0680).
            if vars.is_empty() && plain && !atom.neg() && if derived { v.rel(rel).is_some_and(|r| whole[r.0 as usize]) } else { !columns.is_empty() } {
                vars.insert(format!("rr_value_row:{index}"));
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
            vars.extend(at_key);
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
        let one_key = tested.iter().any(|(_, positive)| *positive) && tested.iter().all(|(k, _)| *k == tested[0].0);
        if merged.len() == 1 || (merged.is_empty() && (filters.contains(head) || one_key)) {
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
    let mut doc: Vector<u32> = (0..reader.var_count() as u32).collect();
    for (a, b) in super::var_equalities_in(v, reader) {
        join(&mut doc, a.0, b.0);
    }
    for l in reader.body() {
        match l.view() {
            L::Atom(a) if !a.neg() && a.rel().0 != r && !placeless(a.rel().0) => {
                let args: Vec<TermRef<'_>> = a.args().collect();
                let vs: Vector<u32> = columns(a.rel().0).iter().filter_map(|&c| args.get(c).and_then(|t| t.var())).map(|s| s.0).collect();
                for w in vs.windows(2) {
                    join(&mut doc, w[0], w[1]);
                }
            }
            L::Nearest { n, a, .. } => join(&mut doc, n.0, a.0),
            _ => {}
        }
    }
    let mut roots: Vector<u32> = vars.map(|s| find(&mut doc, s.0)).collect();
    roots.sort_unstable();
    roots.dedup();
    roots.len() <= 1
}

/// A rule's variables by value: each slot's class under the body's `a = b` between two variables.
fn value_classes(rule: RuleRef<'_>) -> Vector<u32> {
    let mut root: Vector<u32> = (0..rule.var_count() as u32).collect();
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
fn maps<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: usize, anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], twin: &dyn Fn(u32) -> Option<u32>) -> bool {
    let qc = value_classes(reader);
    let Some(L::Atom(read)) = reader.body().nth(at).map(|l| l.view()) else { return false };
    let head: Vec<Option<MapT>> = read.args().map(|u| MapT::of(&qc, u)).collect();
    maps_at(v, rule, reader, Some(at), &head, anchors, placeless, columns, twin, false)
}

/// A reader's atom as `maps_at` finds a rule's read in it: its relation, a located form's original's (decision 0680).
struct Target<'v> {
    rel: u32,
    neg: bool,
    args: Vec<TermRef<'v>>,
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

/// `maps` with the head's arguments given (`head`), the reader's literal `at` a read of it where there is one. Where
/// `exact`, `rule` is a keyed family's root's (`covers_keyed`): each of its literals is found in the reader as it is,
/// every atom (negated, placeless) a reader's atom of its relation and sign, each comparison one of the reader's, since
/// the rule's whole body must hold in the reader's rows, not only its reads. A reader's read of a located form
/// (`twin`: `q_at(d, …)`, decision 0680) holds its original's row (`q(…)`), its document column aside.
#[allow(clippy::too_many_arguments)]
fn maps_at<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: Option<usize>, head: &[Option<MapT>], anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], twin: &dyn Fn(u32) -> Option<u32>, exact: bool) -> bool {
    type T = MapT;
    let n = rule.var_count();
    let (rc, qc) = (value_classes(rule), value_classes(reader));
    let target = |t: TermRef<'_>| MapT::of(&qc, t);
    let mut bind: Vec<Vec<T>> = vec![Vec::new(); n];
    let mut anchor_vars: Vector<u32> = Vector::new();
    // A head column a call fills (`negate(a, @concat("-", a))`): a function of its variables, which, as a
    // comparison's, the head must fix.
    let mut head_calls: Vector<u32> = Vector::new();
    if rule.head().args().count() != head.len() {
        return false;
    }
    for (c, (h, u)) in rule.head().args().zip(head.iter().copied()).enumerate() {
        match h {
            TermRef::Var(s) => {
                if let Some(t) = u {
                    bind[rc[s.0 as usize] as usize].push(t);
                }
                if anchors.contains(&c) {
                    anchor_vars.push(s.0);
                }
            }
            // An exact rule's constant head must be the value asked.
            TermRef::Int(i) if exact => {
                if u.is_some_and(|u| u != T::Int(i)) {
                    return false;
                }
            }
            TermRef::Str(a) if exact => {
                if u.is_some_and(|u| u != T::Str(a.0)) {
                    return false;
                }
            }
            TermRef::Int(_) | TermRef::Str(_) => {}
            TermRef::Call(..) if !exact => {
                fn vars(t: TermRef<'_>, out: &mut Vector<u32>) {
                    match t {
                        TermRef::Var(s) => out.push(s.0),
                        TermRef::Call(_, xs) => xs.for_each(|x| vars(x, out)),
                        _ => {}
                    }
                }
                vars(h, &mut head_calls);
            }
            _ => return false,
        }
    }
    let given: Vec<bool> = bind.iter().map(|b| !b.is_empty()).collect();
    // The rule's documents: its positive located reads' location columns, joined by `a = b`, `@doc` and `nearest`.
    let mut doc: Vector<u32> = (0..n as u32).collect();
    for (a, b) in super::var_equalities_in(v, rule) {
        join(&mut doc, a.0, b.0);
    }
    let located_vars = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| -> Vector<u32> {
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
    fn vars(t: TermRef<'_>, out: &mut Vector<u32>) {
        match t {
            TermRef::Var(s) => out.push(s.0),
            TermRef::Call(_, xs) => xs.for_each(|x| vars(x, out)),
            _ => {}
        }
    }
    let mut mapped = Vec::new();
    // An exact rule's comparisons, each found in the reader once its reads are (`same_term`).
    let mut cmps: Vec<(TermRef<'_>, CmpOp, TermRef<'_>)> = Vec::new();
    let mut in_mapped = vec![false; n];
    let mut others: Vec<Vector<u32>> = Vec::new();
    for l in rule.body() {
        let mut vs = Vector::new();
        if exact {
            match l.view() {
                L::Atom(a) => {
                    for t in a.args() {
                        match t {
                            TermRef::Var(s) => in_mapped[rc[s.0 as usize] as usize] = true,
                            TermRef::Wild | TermRef::Int(_) | TermRef::Str(_) => {}
                            _ => return false,
                        }
                    }
                    mapped.push(a);
                }
                L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } if rc[a.0 as usize] == rc[b.0 as usize] => {}
                L::Cmp { l, op, r } => cmps.push((l, op, r)),
                _ => return false,
            }
            continue;
        }
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
    if !head_calls.iter().all(|&s| given[rc[s as usize] as usize]) {
        return false;
    }
    if others.iter().flatten().any(|&s| {
        let c = rc[s as usize] as usize;
        in_mapped[c] && !given[c]
    }) {
        return false;
    }
    // Each target as its relation, sign and terms: a located form's as its original's, its document column aside.
    let targets: Vec<Target<'_>> = reader
        .body()
        .enumerate()
        .filter_map(|(i, l)| match l.view() {
            L::Atom(a) if (exact || !a.neg()) && Some(i) != at => Some(match twin(a.rel().0) {
                Some(q) => Target { rel: q, neg: a.neg(), args: a.args().skip(1).collect() },
                None => Target { rel: a.rel().0, neg: a.neg(), args: a.args().collect() },
            }),
            _ => None,
        })
        .collect();
    // Backtracking over the mapped reads, each to a target of its relation; bounded, a failure past the bound.
    #[allow(clippy::too_many_arguments)]
    fn go(k: usize, mapped: &[crate::code_rule_dsl::logic::program_image::AtomRef<'_>], targets: &[Target<'_>], bind: &mut Vec<Vec<T>>, rc: &[u32], target: &dyn Fn(TermRef<'_>) -> Option<T>, steps: &mut usize, done: &dyn Fn(&mut Vec<Vec<T>>) -> bool) -> bool {
        let Some(a) = mapped.get(k) else { return done(bind) };
        // Of its sign: a negated read is found only as that negated read, its `_` as `_` (`!r(e, _)` holds less often
        // than `!r(e, y)`).
        for b in targets.iter().filter(|b| b.rel == a.rel().0 && b.neg == a.neg()) {
            *steps += 1;
            if *steps > 4096 {
                return false;
            }
            let saved = bind.clone();
            let fits = a.args().zip(b.args.iter().copied()).all(|(x, y)| match x {
                TermRef::Wild => !a.neg() || matches!(y, TermRef::Wild),
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
            if fits && go(k + 1, mapped, targets, bind, rc, target, steps, done) {
                return true;
            }
            *bind = saved;
        }
        false
    }
    // A term of the rule as the reader's under the map: a variable bound to it (or, unbound, now), a constant, or a
    // call of the same function on such terms.
    fn same_term(v: &Version, x: TermRef<'_>, y: TermRef<'_>, bind: &mut Vec<Vec<T>>, rc: &[u32], target: &dyn Fn(TermRef<'_>) -> Option<T>) -> bool {
        match (x, y) {
            (TermRef::Var(s), _) => {
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
            (TermRef::Int(a), TermRef::Int(b)) => a == b,
            (TermRef::Str(a), TermRef::Str(b)) => v.text(a) == v.text(b),
            (TermRef::Call(f, xs), TermRef::Call(g, ys)) => {
                let (xs, ys): (Vec<_>, Vec<_>) = (xs.collect(), ys.collect());
                v.text(f) == v.text(g) && xs.len() == ys.len() && xs.into_iter().zip(ys).all(|(x, y)| same_term(v, x, y, bind, rc, target))
            }
            _ => false,
        }
    }
    let theirs: Vec<(TermRef<'_>, CmpOp, TermRef<'_>)> = reader
        .body()
        .filter_map(|l| match l.view() {
            L::Cmp { l, op, r } => Some((l, op, r)),
            _ => None,
        })
        .collect();
    // Each comparison found as one of the reader's, `=` and `!=` either way round; the first found is kept.
    let done = |bind: &mut Vec<Vec<T>>| -> bool {
        cmps.iter().all(|&(l, op, r)| {
            theirs.iter().any(|&(l2, op2, r2)| {
                if op != op2 {
                    return false;
                }
                let saved = bind.clone();
                if same_term(v, l, l2, bind, &rc, &target) && same_term(v, r, r2, bind, &rc, &target) {
                    return true;
                }
                *bind = saved.clone();
                if matches!(op, CmpOp::Eq | CmpOp::Ne) && same_term(v, l, r2, bind, &rc, &target) && same_term(v, r, l2, bind, &rc, &target) {
                    return true;
                }
                *bind = saved;
                false
            })
        })
    };
    let mut steps = 0usize;
    go(0, &mapped, &targets, &mut bind, &rc, &target, &mut steps, &done)
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

/// Whether rule `r` reads a node's text or value by a function (`syntax::NODE_READS`, `t = @rr_node_text_any(v)`):
/// a read of the long-text nodes' binding (`syntax::LONG_TEXT_NODES`), which holds the batch the node is read from.
pub fn reads_a_node(v: &Version, r: RuleRef<'_>) -> bool {
    fn calls(v: &Version, t: TermRef<'_>) -> bool {
        match t {
            TermRef::Call(f, mut args) => crate::code_rule_dsl::facts::vtab::syntax::NODE_READS.contains(&v.text(f)) || args.any(|a| calls(v, a)),
            _ => false,
        }
    }
    r.body().any(|l| match l.view() {
        L::Cmp { l, r, .. } => calls(v, l) || calls(v, r),
        _ => false,
    })
}

pub(super) fn crossing(v: &Version, f: Fields<'_>, local: &BTreeSet<String>) -> Crossing {
    let is_local = |rel: &str| local.contains(rel);
    let head = |r: &RuleRef<'_>| v.rel_name(r.head().rel());
    // A rule's reads: its atoms', by relation id, each named once (a name made per atom read, twice over the rules,
    // was 5 ms of a warm run); and the long-text nodes' binding where it reads a node's text or value
    // (`syntax::NODE_READS`: the binding holds the batch every node read goes through, the corpus batch's in the
    // cross-file part).
    let read = |keep: &dyn Fn(&str) -> bool| -> BTreeSet<String> {
        let ids: std::collections::HashSet<_> = v.rules().filter(|r| keep(head(r))).flat_map(|r| r.reads().map(|a| a.rel())).collect();
        let mut names: BTreeSet<String> = ids.into_iter().map(|r| v.rel_name(r).to_owned()).collect();
        if v.rules().filter(|r| keep(head(r))).any(|r| reads_a_node(v, r)) {
            names.insert(crate::code_rule_dsl::facts::vtab::syntax::LONG_TEXT_NODES.to_owned());
        }
        names
    };
    let cross_reads = read(&|h| !is_local(h));
    let projection = |rel: &str| {
        let Some(id) = v.rel(rel) else { return false };
        let mut rules = v.heads(id).peekable();
        rules.peek().is_some()
            && !f.lattice(rel)
            && rules.all(|r| {
                let mut body = r.body();
                // Not over a matcher's relation (`rr_ast_<k>`): its rows are made by a wave over each document's tape,
                // which the chunks run already; read in the cross part, the corpus batch ran the wave again over every
                // file and made every document's rows for it (`raw_all_name` on t50's product run, 24 s of CPU).
                // Its projection is exported instead.
                match (body.next().map(|l| l.view()), body.next()) {
                    (Some(L::Atom(a)), None) => !a.neg() && f.is_input(v.rel_name(a.rel())) && !v.rel_name(a.rel()).starts_with(crate::code_rule_dsl::facts::core::ast_matches::PREFIX),
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
    /// Placed where its inputs are, and read as a test is where it is per-file: a native function of strings
    /// (`match_all`), or a relation that chooses its readers' evaluation, not their rows (`STRATEGY`).
    functional: Vec<bool>,
    text: Vec<Option<&'static [usize]>>,
    /// A base relation's columns that place its document: its locations and its path key (`Program::base_placing`).
    base_locs: Vec<Vector<usize>>,
    /// Its locations alone (`Program::base_locations`): what a row's document is read off.
    base_real: Vec<Vector<usize>>,
    candidate: Vec<bool>,
    output: Vec<bool>,
    /// A located form `locate` made (`super::LOCATED_FORM`, `q_at(d, …)`): the relation whose rows it holds, each with
    /// its document (decision 0680).
    twin_of: Vec<Option<u32>>,
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
        let mut by_head: Vec<Vector<usize>> = vec![Vector::new(); n];
        let mut reading: Vec<Vector<usize>> = vec![Vector::new(); n];
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
        let p = Program { v, f, rules: Vec::new(), rels: Rels::empty(), holds: Default::default() };
        let name = |r: usize| v.rel_name(RelId(r as u32));
        // A part's relation another part derives is derived (`fields::GIVEN`), by no rule here.
        for (r, d) in derived.iter_mut().enumerate() {
            if !*d && read[r] && f.given(name(r)) {
                *d = true;
            }
        }
        let base = |r: usize| !derived[r] && read[r];
        let test = |r: &str| r.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
        let functional = |r: &str| f.native(r).is_some_and(|nat| matches!(nat.spec, ArchivedNativeSpec::MatchAll { .. })) || f.qualified(r, super::STRATEGY);
        Rels {
            n,
            derived_count: derived.iter().filter(|d| **d).count(),
            fact: (0..n).map(|r| base(r) && p.fact(name(r)) && !f.is_input(name(r))).collect(),
            placeless_input: (0..n).map(|r| (derived[r] || read[r]) && p.placeless_input(name(r))).collect(),
            chunk_value: (0..n).map(|r| base(r) && CHUNK_VALUES.contains(&name(r))).collect(),
            test: (0..n).map(|r| (derived[r] || read[r]) && test(name(r))).collect(),
            functional: (0..n).map(|r| derived[r] && functional(name(r))).collect(),
            text: (0..n).map(|r| if derived[r] || read[r] { crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(t, _)| *t == name(r)).map(|(_, c)| *c) } else { None }).collect(),
            base_locs: (0..n).map(|r| if base(r) { p.base_placing(name(r)) } else { Vector::new() }).collect(),
            base_real: (0..n).map(|r| if base(r) { p.base_locations(name(r)) } else { Vector::new() }).collect(),
            candidate: (0..n).map(|r| derived[r] && f.native(name(r)).is_none() && !f.lattice(name(r)) && !f.is_input(name(r))).collect(),
            output: (0..n).map(|r| derived[r] && f.is_output(name(r))).collect(),
            twin_of: (0..n)
                .map(|r| {
                    let at = name(r);
                    (derived[r] && f.qualified(at, super::LOCATED_FORM)).then(|| at.trim_end_matches('_').strip_suffix("_at")).flatten().and_then(|q| v.rel(q)).map(|q| q.0)
                })
                .collect(),
            rules: by_head,
            reading,
            head,
            derived,
            _v: std::marker::PhantomData,
        }
    }

    fn empty() -> Rels<'v> {
        Rels { n: 0, derived_count: 0, derived: Vec::new(), fact: Vec::new(), placeless_input: Vec::new(), chunk_value: Vec::new(), test: Vec::new(), functional: Vec::new(), text: Vec::new(), base_locs: Vec::new(), base_real: Vec::new(), candidate: Vec::new(), output: Vec::new(), twin_of: Vec::new(), rules: Vec::new(), reading: Vec::new(), head: Vector::new(), _v: std::marker::PhantomData }
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
/// on a string, which is no one document's), `splice_of(_, n, _, t3)`'s t3 with node n, and `sub_tape(n, _, t)`'s
/// t with node n.
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

/// A body literal as the split reads it.
enum Lit {
    /// A read: its variables by argument, and its string constants by argument (decision 0665: a path written out,
    /// `bcgp_line("a.py", l, sp)`), each numbered among the rule's (`Form::paths`).
    Read { neg: bool, plain: bool, rel: u32, args: Vec<Option<u32>>, consts: Vec<Option<u32>>, binder: bool, tapes: Vector<u32> },
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
    /// The first id of the rule's string constants (`Lit::Read`'s `consts`), numbered after the literals: a string in
    /// a column that places a document is a file's path (a location is a node or a number), so each names one
    /// document, the same for every atom that writes it (decision 0665).
    paths: u32,
    /// A variable built from another (`x = f(y, …)`), by class: (x's, y's), where y leads the string x is built from
    /// (`name_built`). It names y's document where y does (decision 0665): a module from its file's path, a symbol
    /// from its module.
    built: Vec<(u32, u32)>,
}

/// The functions a document's name is built through (decision 0665): a named group of a whole-text pattern (`match`'s
/// capture, `rx_group`), a replacement in a string (`@replace_all`), and a string the name leads (`cat`'s `concat`).
const NAME_BUILDERS: [&str; 3] = ["rx_group", "replace_all", "concat"];

/// The variable a term's string is built from, where it leads it: the term itself, or the first argument of a
/// `NAME_BUILDERS` call, through each such call (`cat(cat(m, "."), q)`'s `m`).
fn name_built(v: &Version, t: TermRef<'_>) -> Option<crate::code_rule_dsl::logic::program_image::Slot> {
    match t {
        TermRef::Var(s) => Some(s),
        TermRef::Call(f, mut xs) if NAME_BUILDERS.contains(&v.text(f)) => xs.next().and_then(|x| name_built(v, x)),
        _ => None,
    }
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
        let built = rule
            .body()
            .filter_map(|l| match l.view() {
                L::Cmp { l: TermRef::Var(x), op: CmpOp::Eq, r: r @ TermRef::Call(..) } | L::Cmp { l: r @ TermRef::Call(..), op: CmpOp::Eq, r: TermRef::Var(x) } => {
                    name_built(v, r).map(|y| (class[x.0 as usize], class[y.0 as usize]))
                }
                _ => None,
            })
            .collect();
        Form { named, class, lits, splices, ids, binders, paths, built }
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
            Lit::Cmp | Lit::Same(..) => true,
            _ => false,
        })
    }

    /// The classes the placing columns place, and each built from one of them (`built`).
    fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vector<usize>]) -> Bits {
        let mut out = self.placed_classes_over(rels, locs, &rels.base_locs);
        loop {
            let grew: Vector<u32> = self.built.iter().filter(|&&(x, y)| out.has(y) && !out.has(x)).map(|&(x, _)| x).collect();
            if grew.is_empty() {
                return out;
            }
            grew.into_iter().for_each(|x| out.set(x));
        }
    }

    /// `placed_classes` with the base relations' columns `base`: their placing columns, or their locations alone.
    fn placed_classes_over(&self, rels: &Rels<'_>, locs: &[Vector<usize>], base: &[Vector<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            // `nearest(n, a, set)`: both are tree nodes, `a` an ancestor of `n` in `n`'s document. A head filled by
            // `a` (`rdefn(s, x, f) :- …, nearest(f, s, scope)`) is a location, as one filled by `n` is.
            if let Lit::Nearest(n, a) = l {
                out.set(*n);
                out.set(*a);
                continue;
            }
            if let Lit::Read { neg: false, rel, args, .. } = l {
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
    fn placed_or_null(&self, rels: &Rels<'_>, locs: &[Vector<usize>], nulls: &[Vector<usize>]) -> Bits {
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

    /// `head_placed`, or a string built from a placed variable (`name_built`): a placing column, not a location.
    fn head_placed_built(&self, v: &Version, rule: RuleRef<'_>, c: usize, placed: &Bits) -> bool {
        self.head_placed(rule, c, placed)
            || rule.head().args().nth(c).filter(|t| matches!(t, TermRef::Call(..))).and_then(|t| name_built(v, t)).is_some_and(|s| self.named[s.0 as usize] && placed.has(self.class[s.0 as usize]))
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

    fn one_document(&self, head: u32, rels: &Rels<'_>, placeless: &[bool], local: &[bool], filters: &[bool], covered: &[bool], locs: &[Vector<usize>], nulls: &[Vector<usize>], whole: &[bool], family: &[Option<u32>], root_held: &dyn Fn(u32, u32) -> bool) -> bool {
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
        // A keyed family's reads at one key (decision 0680): its rows there follow from its root's row at that key
        // alone, so in a chunk they are as over every file wherever the chunk holds the root's row. A rule holds it
        // through a located read the root's rule maps into (`root_held`): its family reads at that key are tests. Or
        // through a positive read of the family, or of a located form of it (`twin_of`): they place that read's
        // document, the first such; a negated or aggregated one only where a positive one holds its key.
        let family_read = |rel: u32, args: &[Option<u32>]| -> Option<(u32, u32, u32, bool)> {
            let (q, at, twin) = match rels.twin_of[rel as usize] {
                Some(q) => (q, 1, true),
                None => (rel, 0, false),
            };
            if !rels.is_derived(q) || !local[q as usize] {
                return None;
            }
            let x = (*args.get(at)?)?;
            Some((family[q as usize]?, self.class[x as usize], x, twin))
        };
        let mut held: Vec<((u32, u32), u32)> = Vec::new();
        for (index, l) in self.lits.iter().enumerate() {
            if let Lit::Read { neg: false, plain: true, rel, args, .. } = l
                && rels.is_derived(*rel)
                && let Some((root, class, x, _)) = family_read(*rel, args)
                && !root_held(root, x)
                && !held.iter().any(|(h, _)| *h == (root, class))
            {
                held.push(((root, class), self.binders + index as u32));
            }
        }
        // The family keys its reads as tests are at, and whether each is positive (decision 0680).
        let mut tested: Vec<((u32, u32), bool)> = Vec::new();
        let mut root: Vector<u32> = (0..self.ids as u32).collect();
        let mut firsts: Vector<u32> = Vector::new();
        for &(n, t) in &self.splices {
            join(&mut root, n, t);
            firsts.push(n);
        }
        let mut group: Vector<u32> = Vector::new();
        for (index, l) in self.lits.iter().enumerate() {
            match l {
                Lit::Read { neg, plain, rel, args, consts, binder, tapes } => {
                    let r = *rel;
                    let derived = rels.is_derived(r);
                    // A function of strings where it is per-file is read as a test is.
                    if placeless[r as usize] || head_filter || (rels.functional[r as usize] && local[r as usize]) {
                        continue;
                    }
                    if (derived && filters[r as usize]) || covered[r as usize] {
                        tested.extend(family_read(r, args).map(|(root, class, ..)| ((root, class), *plain && !*neg)));
                        continue;
                    }
                    if derived && !local[r as usize] {
                        return false;
                    }
                    let family = if derived { family_read(r, args) } else { None };
                    let at_key = family.and_then(|(root, class, ..)| held.iter().find(|(h, _)| *h == (root, class)).map(|(_, at)| *at));
                    if let Some((root, _, x, twin)) = family {
                        // A twin read naming its document is located, and holds the key besides.
                        let named = twin && args.first().copied().flatten().is_some();
                        if root_held(root, x) && !named {
                            continue;
                        }
                        if let (Some(at), false) = (at_key, twin) {
                            firsts.push(at);
                            continue;
                        }
                    }
                    let columns = if derived { &locs[r as usize] } else { &rels.base_locs[r as usize] };
                    group.clear();
                    group.extend(columns.iter().filter_map(|&c| args.get(c).copied().flatten().map(|v| self.class[v as usize])));
                    // A path written out places the one document of that path (decision 0665).
                    group.extend(columns.iter().filter_map(|&c| consts.get(c).copied().flatten().map(|k| self.paths + k)));
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
                    // A read naming no variable of its document, of rows each one document's (decision 0680):
                    // `format_call(_, v)`, or a relation of values derived per file. Each row is its document's, so
                    // the read places a document of its own: alone it is a union, and joined with another read it is
                    // two documents.
                    if group.is_empty() && *plain && !*neg && if derived { whole[r as usize] } else { !columns.is_empty() } {
                        group.push(self.binders + index as u32);
                    }
                    if !group.is_empty() {
                        group.extend(tapes.iter().copied());
                        group.extend(at_key);
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
        let mut roots: Vector<u32> = firsts.iter().map(|&f| find(&mut root, f)).collect();
        roots.sort_unstable();
        roots.dedup();
        // Reads all tests, of one family at one key, one of them positive: that key's rows (decision 0680).
        let one_key = tested.iter().any(|(_, positive)| *positive) && tested.iter().all(|(k, _)| *k == tested[0].0);
        roots.len() == 1 || (roots.is_empty() && (head_filter || one_key))
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
