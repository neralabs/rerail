//! The file-local split, read from the program's version (decision 0281's path, step 5), as an analysis of it
//! (`FileLocal`), maintained across an edit (`Analysis::maintain`).
//!
//! A split of a program a few rules apart computes only what the difference reaches. The numbers are the image's
//! own: a relation is its `RelId`, a variable its `Slot`, and the last split is the analysis of the edit's input,
//! carried to its output by the manager, which hands `maintain` the edit.

use super::{CHUNK_VALUES, document_column, option_input, path_key_column, setting_input};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::manager::{Analysis, Cx, Delta, Reads};
use crate::code_rule_dsl::logic::program_image::{CmpOp, Lit as L, NativeSpec, RelId, RuleRef, Slot, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

mod flags_last;
pub use flags_last::{FLAG_COPY, LastFlags, flag_copy};

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
    pub locations: BTreeMap<String, Vec<usize>>,
    pub filters: BTreeSet<String>,
    pub causes: BTreeMap<String, String>,
    pub same_doc: BTreeMap<String, Vec<usize>>,
    /// Each derived relation's nullable location columns (decision 0649): not its `locations`, but filled in every
    /// rule by a location or by `unset` (-1, no node). A rule joins through one only where the same rule places its
    /// variable by a location column too, which -1 never is; a row's document is never read off one.
    pub nullable: BTreeMap<String, Vec<usize>>,
    /// The undeclared crossings (decision 0652): each relation in the cross-file part that is not declared
    /// `cross_file`, whose own rule places rows of two documents, or which is made natively. Its value is the cause and
    /// the rule. Not one: a relation whose rules read a cross-file relation (it follows it), or are on one document
    /// but for an input naming no document (the run's settings, a phase's export, the files beyond the corpus).
    pub crossings: BTreeMap<String, String>,
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
    pub fn placing(&self, rel: &str) -> Vec<usize> {
        let mut c: Vec<usize> = self.locations.get(rel).into_iter().chain(self.path_keys.get(rel)).flatten().copied().collect();
        c.sort_unstable();
        c.dedup();
        c
    }

    /// The covered views of values (no location column): the cross part derives its own of each it reads (`crossing`,
    /// decision 0672).
    pub fn views(&self) -> BTreeSet<String> {
        match self.no_document {
            true => Default::default(),
            false => self.covered.iter().filter(|r| self.locations.get(*r).is_none_or(Vec::is_empty)).cloned().collect(),
        }
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

/// What staging makes of a program (decision 0675): the relations staged, the split of the program they leave (their
/// readers read them as placeless inputs), and the relations refused for a cycle, with the cause.
pub struct Staging {
    pub staged: BTreeSet<String>,
    pub refused: BTreeMap<String, String>,
    pub split: FileLocal,
}

impl FileLocal {
    /// `v`'s staging (decision 0675 §1): the cross-file relations of names a file-local rule would read, were they
    /// placeless, and which read none of their own readers. Found to a fixed point: a relation no file-local rule reads
    /// once the others are staged is not staged.
    ///
    /// `requests`: the program's requests and the relations their answers make (`corpus::world::answers`): answered
    /// inside the program's fixpoint, by loads, so none is read by a stage, nor what they read.
    pub fn staging(v: &std::sync::Arc<Version>, requests: &BTreeSet<String>) -> Result<Staging, String> {
        let d = crate::code_rule_dsl::logic::program_image::manager::analysis::<FieldsData>(v)?;
        let f = Fields::over(v, &d);
        let p = Program::of(v, f);
        let whole = p.split(None, None);
        let (able, not_able) = p.stageable(&whole, requests);
        let comp = p.components();
        // A relation of names the split found cross-file, and why no stage takes it: its cause, for the report.
        let mut refused: BTreeMap<String, String> = not_able.into_iter().filter(|(r, _)| !whole.local.contains(r)).collect();
        let mut taken: BTreeSet<u32> = able.clone();
        // The staged relations asked whether the split makes them per-file itself (below): each once.
        let mut asked: BTreeSet<u32> = BTreeSet::new();
        loop {
            // A relation whose component holds one not taken reads one of its readers: a cycle.
            let cyclic: Vec<u32> = taken
                .iter()
                .copied()
                .filter(|&r| (0..p.rels.n as u32).any(|x| x != r && comp[x as usize] == comp[r as usize] && p.rels.is_derived(x) && !taken.contains(&x)))
                .collect();
            for r in cyclic {
                taken.remove(&r);
                let x = (0..p.rels.n as u32).find(|&x| x != r && comp[x as usize] == comp[r as usize] && p.rels.is_derived(x) && !taken.contains(&x)).map_or("", |x| p.name(x));
                refused.insert(p.name(r).to_owned(), format!("not staged: a cycle through its reader {x}"));
            }
            let names: BTreeSet<String> = taken.iter().map(|&r| p.name(r).to_owned()).collect();
            if names.is_empty() {
                let mut split = whole;
                for (r, c) in &refused {
                    // After the split's own cause, where it has one.
                    let e = split.causes.entry(r.clone()).or_default();
                    *e = if e.is_empty() { c.clone() } else { format!("{e}; {c}") };
                }
                return Ok(Staging { staged: names, refused, split });
            }
            let q = Program::without(v, f, &names);
            let mut split = q.split(None, None);
            // A relation staging leaves crossing files undeclared (decision 0652), which followed a cross-file one before
            // (`used_def(q) :- defq(_, q), imp(_, q)`, `defq` per-file once what it read is staged): staging never makes
            // a program the split refuses. What it reads is not staged.
            let crossing: Vec<u32> = split.crossings.keys().filter(|c| !whole.crossings.contains_key(*c)).filter_map(|c| v.rel(c).map(|r| r.0)).collect();
            if !crossing.is_empty() {
                // What the crossing relation reads, staged, made it cross. Failing that, what its readers read: a view
                // proved per-file by its readers' rows (decision 0652's `covers`) is proved no longer once a relation
                // they read is staged (`pc_modattr`, read by `pc_base_s`). Failing both, nothing is staged: the loop
                // always ends.
                let before = taken.len();
                let crossings = split.crossings.clone();
                let mut unstage = |from: Vec<(u32, u32)>, taken: &mut BTreeSet<u32>| {
                    let mut seen: BTreeSet<u32> = BTreeSet::new();
                    let mut todo = from;
                    while let Some((x, by)) = todo.pop() {
                        if !seen.insert(x) {
                            continue;
                        }
                        if taken.remove(&x) {
                            let cause = crossings.get(p.name(by)).map_or("", |s| s.as_str());
                            refused.insert(p.name(x).to_owned(), format!("not staged: {} would join documents undeclared ({cause})", p.name(by)));
                        }
                        for &k in &p.rels.rules[x as usize] {
                            todo.extend(p.rules[k].reads().map(|a| (a.rel().0, by)).filter(|(r, _)| p.rels.is_derived(*r)));
                        }
                    }
                };
                unstage(crossing.iter().map(|&c| (c, c)).collect(), &mut taken);
                if taken.len() == before {
                    let readers: Vec<(u32, u32)> =
                        crossing.iter().flat_map(|&c| p.rels.reading[c as usize].iter().map(move |&k| (k, c))).map(|(k, c)| (p.rels.head[k], c)).collect();
                    unstage(readers, &mut taken);
                }
                if taken.len() == before {
                    // Every relation that would cross, named: each is a declaration to read against its rule (0652).
                    let all: Vec<String> = crossing.iter().map(|&c| format!("{} ({})", p.name(c), split.crossings.get(p.name(c)).map_or("", |s| s.as_str()))).collect();
                    for &x in &taken {
                        refused.insert(p.name(x).to_owned(), format!("not staged: {} would join documents undeclared", all.join(", ")));
                    }
                    taken.clear();
                }
                continue;
            }
            // Those a file-local rule reads.
            // And those a staged relation kept reads: its stage holds them (`exports`, read by `resolve_export` alone).
            let mut read: BTreeSet<u32> = taken.iter().copied().filter(|&r| q.rels.reading[r as usize].iter().any(|&k| split.local.contains(q.name(q.rels.head[k])))).collect();
            let mut todo: Vec<u32> = read.iter().copied().collect();
            while let Some(x) = todo.pop() {
                for &k in &p.rels.rules[x as usize] {
                    for a in p.rules[k].reads() {
                        let y = a.rel().0;
                        if taken.contains(&y) && read.insert(y) {
                            todo.push(y);
                        }
                    }
                }
            }
            if read == taken {
                // A staged relation the split makes per-file itself once the others are staged is the split's, not a
                // stage's: a view of values its per-file readers hold (decision 0672) is covered in each chunk, and no
                // stage derives it over every file (UP032's `mixed`, per-file once Python's resolution is staged). Asked
                // of each one no other staged relation reads, a split each, once; the set is found again after.
                let tops: Vec<u32> = taken
                    .iter()
                    .copied()
                    .filter(|&r| !asked.contains(&r) && !taken.iter().any(|&t| t != r && p.rels.rules[t as usize].iter().any(|&k| p.rules[k].reads().any(|a| a.rel().0 == r))))
                    .collect();
                let mut released = false;
                for r in tops {
                    asked.insert(r);
                    let mut rest = names.clone();
                    rest.remove(p.name(r));
                    let alone = Program::without(v, f, &rest).split(None, None);
                    // Per-file, every per-file relation still so, and no crossing made.
                    if alone.local.contains(p.name(r))
                        && split.local.iter().all(|x| alone.local.contains(x))
                        && alone.crossings.keys().all(|c| whole.crossings.contains_key(c))
                    {
                        taken.remove(&r);
                        released = true;
                        break;
                    }
                }
                if released {
                    continue;
                }
                for (r, c) in &refused {
                    // After the split's own cause, where it has one.
                    let e = split.causes.entry(r.clone()).or_default();
                    *e = if e.is_empty() { c.clone() } else { format!("{e}; {c}") };
                }
                return Ok(Staging { staged: names, refused, split });
            }
            for &r in taken.difference(&read) {
                // Its readers, and why each is not file-local.
                let readers: BTreeSet<&str> = q.rels.reading[r as usize].iter().map(|&k| q.name(q.rels.head[k])).filter(|h| !taken.iter().any(|&t| p.name(t) == *h)).collect();
                let why: Vec<String> = readers.iter().take(3).map(|h| format!("{h}: {}", split.causes.get(*h).map_or("?", |c| c.as_str()))).collect();
                refused.insert(p.name(r).to_owned(), format!("not staged: no file-local rule reads it ({})", why.join("; ")));
            }
            taken = read;
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
        Some(out)
    }
    fn of_rows(_v: &Version, rows: &[crate::code_rule_dsl::logic::program_image::manager::Row]) -> Option<FileLocal> {
        use crate::code_rule_dsl::logic::front::Value;
        let cols = |s: &str| -> Option<Vec<usize>> { if s.is_empty() { Some(Vec::new()) } else { s.split(',').map(|x| x.parse().ok()).collect() } };
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
                [Value::Int(9), Value::Str(n), Value::Str(c)] => drop(f.nullable.insert(n.to_string(), cols(c)?)),
                [Value::Int(10), Value::Str(n), Value::Str(c)] => drop(f.crossings.insert(n.to_string(), c.to_string())),
                [Value::Int(11), Value::Str(n)] => drop(f.covered.insert(n.to_string())),
                [Value::Int(12), Value::Str(n), Value::Str(x)] => drop(f.cover_deps.entry(n.to_string()).or_default().insert(x.to_string())),
                [Value::Int(13), Value::Str(n), Value::Str(c)] => drop(f.row_documents.insert(n.to_string(), cols(c)?)),
                [Value::Int(14), Value::Str(n), Value::Str(c)] => drop(f.path_keys.insert(n.to_string(), cols(c)?)),
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
        let rules: Vec<RuleRef<'v>> = v.rules().filter(|r| !dead(v, *r)).collect();
        let rels = Rels::of(v, f, &rules, &BTreeSet::new());
        Program { v, f, rules, rels }
    }

    /// `v` as the program its stage leaves (decision 0675): the relations of `staged` derived by no rule here, read as
    /// placeless inputs, as the stage's exports are read once the program is renamed to read them.
    fn without(v: &'v Version, f: Fields<'v>, staged: &BTreeSet<String>) -> Program<'v> {
        let rules: Vec<RuleRef<'v>> = v.rules().filter(|r| !staged.contains(v.rel_name(r.head().rel())) && !dead(v, *r)).collect();
        let rels = Rels::of(v, f, &rules, staged);
        Program { v, f, rules, rels }
    }

    /// The derived relations a stage may take (decision 0675 §1): each whose rows name no document (no placing column,
    /// node, document key or path, nor a nullable one, and no covered view), of no node column, neither an output, native, a lattice, a test, nor moved or declared
    /// cross-file, whose rules and the rules below them read no position. A relation whose strongly connected
    /// component holds one not in `taken` reads one of its own readers: refused, a cycle.
    fn stageable(&self, whole: &FileLocal, requests: &BTreeSet<String>) -> (BTreeSet<u32>, BTreeMap<String, String>) {
        let rels = &self.rels;
        let n = rels.n;
        // Whether a relation's rules, or those below them, read a position: to their fixed point.
        let mut positional: Vec<bool> = (0..n).map(|r| super::POSITIONS.contains(&self.name(r as u32))).collect();
        loop {
            let mut changed = false;
            for r in 0..n {
                if !positional[r] && rels.is_derived(r as u32) && rels.rules[r].iter().any(|&k| self.rules[k].reads().any(|a| positional[a.rel().0 as usize])) {
                    positional[r] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let moved = |rel: &str| self.f.qualified(rel, super::CROSS_FILE) || self.f.qualified(rel, super::READ_LOCATED) || self.f.qualified(rel, super::MOVED_CROSS_FILE);
        // A request's cone and what a load answers (decision 0065 §2a) are the program's fixpoint's own: the documents a
        // load brings are read there, after any stage ran. A relation a request is derived from, or one derived from an
        // answer, is not staged.
        let below = |from: Vec<u32>| -> Vec<bool> {
            let mut seen = vec![false; n];
            let mut todo = from;
            while let Some(r) = todo.pop() {
                if std::mem::replace(&mut seen[r as usize], true) {
                    continue;
                }
                for &k in &rels.rules[r as usize] {
                    todo.extend(self.rules[k].reads().map(|a| a.rel().0));
                }
            }
            seen
        };
        let requested = below((0..n as u32).filter(|&r| requests.contains(self.name(r))).collect());
        let natives_read = self.f.natives_read();
        // A symbol column of another domain than a name (a kind, a field: decision 0270) is branded by the rules that
        // make it; an export reads back as names, so a relation holding one stays the program's.
        let domains = crate::code_rule_dsl::logic::plan::lower::domains_of(self.v).unwrap_or_default();
        let branded = |rel: &str| {
            use crate::code_rule_dsl::logic::plan::lower::Domain;
            domains.get(rel).is_none_or(|ds| ds.iter().any(|d| matches!(d, Domain::Kind | Domain::Field)))
        };
        // A relation a rule names as a value (a kind set, `nearest(e, a, fk)`, `has(a, _, fk)`) is compiled into its
        // reader by name: it stays the program's.
        let mut named: BTreeSet<&str> = BTreeSet::new();
        fn strs<'a>(v: &'a Version, t: TermRef<'_>, out: &mut BTreeSet<&'a str>) {
            match t {
                TermRef::Str(a) => drop(out.insert(v.text(a))),
                TermRef::Call(_, xs) => xs.for_each(|x| strs(v, x, out)),
                _ => {}
            }
        }
        for r in &self.rules {
            for l in r.body() {
                match l.view() {
                    L::Nearest { kindset, .. } => drop(named.insert(self.v.text(kindset))),
                    L::Atom(a) | L::Agg { atom: a, .. } => a.args().for_each(|t| strs(self.v, t, &mut named)),
                    L::Cmp { l, r, .. } => {
                        strs(self.v, l, &mut named);
                        strs(self.v, r, &mut named);
                    }
                    _ => {}
                }
            }
        }
        let mut answered: Vec<bool> = (0..n).map(|r| requests.contains(self.name(r as u32)) || self.f.produced(self.name(r as u32))).collect();
        loop {
            let mut changed = false;
            for r in 0..n {
                if !answered[r] && rels.is_derived(r as u32) && rels.rules[r].iter().any(|&k| self.rules[k].reads().any(|a| answered[a.rel().0 as usize])) {
                    answered[r] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut able = BTreeSet::new();
        let mut why = BTreeMap::new();
        for r in 0..n as u32 {
            let rel = self.name(r);
            let i = r as usize;
            // A relation of names: what a stage could take, but for the reasons below.
            let names = rels.is_derived(r)
                && !rels.rules[i].is_empty()
                && whole.placing(rel).is_empty()
                && !whole.nullable.contains_key(rel)
                && !whole.covered.contains(rel)
                && !whole.constants.contains(rel)
                && !self.f.nullary(rel)
                && self.f.columns(rel).is_some_and(|c| !c.is_empty() && c.iter().all(|(_, t)| *t != "node"));
            if !names {
                continue;
            }
            let reason = if rels.output[i] || rels.test[i] || rels.functional[i] || !rels.candidate[i] || moved(rel) {
                Some("an output, a test, a function, native or lattice, or declared cross-file")
            } else if positional[i] && self.f.columns(rel).is_some_and(|c| c.iter().any(|(_, t)| !matches!(*t, "sym" | "symbol"))) {
                // A name found through positions (`qn`: a name's binding by where it is, in its own file) changes only
                // with its file, as any per-file fact. A value that may be one (a number) would move with every edit
                // above it.
                Some("it reads positions, and a column may hold one")
            } else if crate::code_rule_dsl::logic::rr::lower::made_on_request(rel) {
                Some("the on-request rewrite's (decision 0174)")
            } else if requested[i] {
                Some("a request is derived from it (decision 0065 §2a)")
            } else if answered[i] {
                Some("it is derived from what a load answers (decision 0065 §2a)")
            } else if named.contains(rel) {
                Some("a rule names it as a value")
            } else if branded(rel) {
                Some("a column holds kinds or fields (decision 0270)")
            } else if natives_read.contains(rel) {
                // A native's input (a demand, `rr_ctx…`) is read by the native, which the program keeps.
                Some("a native reads it")
            } else {
                None
            };
            match reason {
                None => drop(able.insert(r)),
                Some(c) => drop(why.insert(rel.to_owned(), format!("not staged: {c}"))),
            }
        }
        (able, why)
    }

    /// Each derived relation's strongly connected component of the read graph, by a representative (Tarjan's).
    fn components(&self) -> Vec<u32> {
        let rels = &self.rels;
        let n = rels.n;
        let succ = |r: usize| -> Vec<usize> { rels.rules[r].iter().flat_map(|&k| self.rules[k].reads().map(|a| a.rel().0 as usize)).filter(|&x| rels.is_derived(x as u32)).collect() };
        let (mut index, mut low, mut on, mut comp) = (vec![usize::MAX; n], vec![0usize; n], vec![false; n], (0..n as u32).collect::<Vec<u32>>());
        let (mut stack, mut next) = (Vec::new(), 0usize);
        for root in 0..n {
            if index[root] != usize::MAX || !rels.is_derived(root as u32) {
                continue;
            }
            // Iterative: (node, its successors, the next one to visit).
            let mut work: Vec<(usize, Vec<usize>, usize)> = vec![(root, succ(root), 0)];
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on[root] = true;
            while let Some((v, ws, at)) = work.last_mut() {
                if *at < ws.len() {
                    let w = ws[*at];
                    *at += 1;
                    if index[w] == usize::MAX {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on[w] = true;
                        let s = succ(w);
                        work.push((w, s, 0));
                    } else if on[w] {
                        let v = *v;
                        low[v] = low[v].min(index[w]);
                    }
                } else {
                    let v = *v;
                    work.pop();
                    if let Some((u, _, _)) = work.last() {
                        low[*u] = low[*u].min(low[v]);
                    }
                    if low[v] == index[v] {
                        while let Some(w) = stack.pop() {
                            on[w] = false;
                            comp[w] = v as u32;
                            if w == v {
                                break;
                            }
                        }
                    }
                }
            }
        }
        comp
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
                let cols = |c: &str| c.split(',').filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect::<Vec<usize>>();
                match w.split_once('=') {
                    Some(("part:loc", c)) => drop(given.locations.insert(rel.to_owned(), cols(c))),
                    Some(("part:key", c)) => drop(given.path_keys.insert(rel.to_owned(), cols(c))),
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
        option_input(rel) || setting_input(rel) || (crate::code_rule_dsl::logic::plan::phased::is_phased(rel) && self.f.is_input(rel))
    }

    /// An input a file-local rule may join and stay file-local: the run's options and its settings (decision 0665:
    /// the configuration files' rows, which a source file's edit does not change). An earlier phase's export is
    /// a run constant, but not placeless: it is every file's rows together, and an edit's delta of it (a function
    /// added) is the reader's input delta (decision 0083 §2.3). A file-local rule that joined it made every file's
    /// memo entry a function of all its rows, so each such edit missed every file (the 50-file data-layer pack: 5.3 s
    /// an edit that adds a definition, 1.6 s one that does not). A rule reading one reads an input with no location:
    /// cross-file, maintained there by the export's delta.
    ///
    /// Decision 0675 makes them placeless (`Rels::placeless_input`): a file's entry is keyed by the rows of one its
    /// probes read (`memo::StageDigests`), not by every row.
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
        let mut about: Vec<u64> = self.v.about_facts().into_iter().map(|f| f.hash()).collect();
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
    fn base_locations(&self, rel: &str) -> Vec<usize> {
        // An undeclared one (a tree axis before lowering declares it) has none of its own, as the tree's.
        let mut columns: Vec<usize> = self.f.columns(rel).unwrap_or_default().iter().enumerate().filter(|(_, (_, t))| *t == "node").map(|(i, _)| i).collect();
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
        // The placing columns, the location columns and the nullable ones, found again with the nullable ones the last
        // round found: a rule testing a nullable column's variable set places it (`Form::set`, decision 0672). Each
        // round is sound, its nullable columns proven over placing columns at least the last round's, and the placing
        // and nullable columns together only grow, so the rounds end.
        let mut hint: Vec<Vec<usize>> = vec![Vec::new(); n];
        let tests_set = (0..self.rules.len()).any(|k| !forms.get(k).set.is_empty());
        let (locs, real, nulls) = loop {
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
                        let placed: Vec<Bits> = rules.iter().map(|&k| forms.get(k).placed_classes_set(rels, &locs, &hint)).collect();
                        let kept: Vec<usize> = locs[r as usize].iter().copied().filter(|&c| rules.iter().zip(&placed).all(|(&k, p)| forms.get(k).head_placed_built(self.v, self.rules[k], c, p))).collect();
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
                        let placed: Vec<Bits> = rules
                            .iter()
                            .map(|&k| {
                                let mut p = forms.get(k).placed_classes_over(rels, &real, &rels.base_real);
                                forms.get(k).tested_set(rels, &hint, &mut p);
                                p
                            })
                            .collect();
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
            if nulls == hint || !tests_set {
                break (locs, real, nulls);
            }
            hint = nulls;
        };
        let derived = || (0..n as u32).filter(|&r| rels.is_derived(r));
        // The split places by `placing`; it publishes `locations` and `path_keys` (decision 0665).
        let placing: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), locs[r as usize].clone())).collect();
        let locations: BTreeMap<String, Vec<usize>> = derived().map(|r| (self.name(r).to_owned(), real[r as usize].clone())).collect();
        let path_keys: BTreeMap<String, Vec<usize>> = derived()
            .map(|r| (self.name(r).to_owned(), locs[r as usize].iter().copied().filter(|c| !real[r as usize].contains(c)).collect::<Vec<usize>>()))
            .filter(|(_, c)| !c.is_empty())
            .collect();
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
                    && !self.f.qualified(rel, crate::code_rule_dsl::logic::plan::fields::PART_CROSS_READ)
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
        // A native read at its inputs' values (`native_inputs`), each input a covered view of values, is one itself: its
        // rows at a tuple are made in the chunk from theirs there (a rank's of the format string's fields, decision 0672).
        // It is local while they are covered, and read as they are.
        let native_view = |r: u32, local_now: &[bool], covered_now: &[bool]| {
            let view = |x: u32| covered_now[x as usize] && local_now[x as usize] && rels.is_derived(x) && locs[x as usize].is_empty();
            local_now[r as usize] && { let inputs = self.native_inputs(r); !inputs.is_empty() && inputs.iter().all(|(i, ..)| view(*i)) }
        };
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
                    // A view of values no longer covered whose rules do not place one document is cross-file now, not
                    // at the round's end: a view it reads is asked in it next as in any cross-file reader (decision 0672).
                    if valueless[r as usize] && local_now[r as usize] {
                        let skip: Vec<bool> = (0..n).map(|x| (covered_now[x] && local_now[x] && locs[x].is_empty()) || native_view(x as u32, &local_now, &covered_now)).collect();
                        if rels.rules[r as usize].iter().any(|&k| !forms.get(k).one_document(r, rels, &placeless, &local_now, &filtering, &skip, &locs, &nulls)) {
                            local_now[r as usize] = false;
                            let filters_now: BTreeSet<String> = filters.iter().cloned().chain((0..n as u32).filter(|&x| skip[x as usize]).map(|x| self.name(x).to_owned())).collect();
                            let why = rels.rules[r as usize].iter().find_map(|&k| self.why_not_local(self.rules[k], &local_now, &constants, &filters_now, &placing, &nullable));
                            causes.entry(self.name(r).to_owned()).or_insert_with(|| why.unwrap_or_else(|| "places more than one document".to_owned()));
                            also.extend(rels.rules[r as usize].iter().flat_map(|&k| self.rules[k].reads().map(|a| a.rel().0).collect::<Vec<_>>()).filter(|&x| covered_now[x as usize]).flat_map(|x| rels.rules[x as usize].iter().copied()));
                        }
                    }
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
            // The covered views of values, in the scope and out of it (where `valueless`, a candidate's, is not asked),
            // and the natives read at their values (`native_view`).
            let skip: Vec<bool> = (0..n).map(|r| (covered_now[r] && local_now[r] && locs[r].is_empty()) || native_view(r as u32, &local_now, &covered_now)).collect();
            let read_as_tests: BTreeSet<String> = filters.iter().cloned().chain((0..n as u32).filter(|&r| skip[r as usize]).map(|r| self.name(r).to_owned())).collect();
            let explain = |k: usize, local_now: &[bool]| self.why_not_local(self.rules[k], local_now, &constants, &read_as_tests, &placing, &nullable);
            let check = |k: usize| -> Option<(u32, String)> {
                let head = rels.head[k];
                if (covered_now[head as usize] && local_now[head as usize]) || native_view(head, &local_now, &covered_now) {
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
            // A covered view of values passed through others is asked again when any relation leaves: one it passes
            // through may be the one (decision 0672).
            if left {
                next.extend((0..n).filter(|&r| in_scope[r] && covered_now[r] && valueless[r]).flat_map(|r| rels.rules[r].iter().copied()));
            }
            next.sort_unstable();
            next.dedup();
            dirty = Some(next);
            if !left {
                // A cause naming a flag's choice between its copies (decision 0686) is the flag's, not the relation's:
                // where another of its rules fails for another reason, that is named, the reason it stays cross-file
                // whatever the flag (`plp_colored`: its walk over `pc_edge`, not `plp_in_rec`).
                let reads_copy = |r: RelId| rels.rules[r.0 as usize].iter().any(|&k| self.rules[k].reads().any(|a| self.name(a.rel().0).starts_with(FLAG_COPY)));
                let a_choice = |why: &str| why.strip_prefix("reads cross-file ").and_then(|x| self.v.rel(x)).is_some_and(reads_copy);
                for r in (0..n as u32).filter(|&r| in_scope[r as usize] && !local_now[r as usize]) {
                    if causes.get(self.name(r)).is_some_and(|c| a_choice(c))
                        && let Some(other) = rels.rules[r as usize].iter().filter_map(|&k| explain(k, &local_now)).find(|w| !a_choice(w))
                    {
                        causes.insert(self.name(r).to_owned(), other);
                    }
                }
                break;
            }
        }
        let local: BTreeSet<String> = derived().filter(|&r| local_now[r as usize]).map(|r| self.name(r).to_owned()).collect();
        let covered: BTreeSet<String> = derived().filter(|&r| covered_now[r as usize] && local_now[r as usize]).map(|r| self.name(r).to_owned()).collect();
        // What placed each: its readers, and theirs through every located reader (one may anchor it, `Anchoring`) and
        // every native read at its inputs' values (`native_inputs`: its readers read them there, `covers`).
        let cover_deps: BTreeMap<String, BTreeSet<String>> = derived()
            .filter(|&r| covered_now[r as usize] && local_now[r as usize])
            .map(|r| {
                let (mut deps, mut seen, mut work) = (BTreeSet::new(), BTreeSet::from([r]), vec![r]);
                while let Some(x) = work.pop() {
                    for &k in rels.reading(x) {
                        let h = rels.head[k];
                        deps.insert(self.name(h).to_owned());
                        if (!locs[h as usize].is_empty() || !self.native_inputs(h).is_empty()) && seen.insert(h) {
                            work.push(h);
                        }
                    }
                }
                // The covered views of values its rules pass through, each read at the values its readers give.
                let member = |x: u32| covered_now[x as usize] && local_now[x as usize] && rels.is_derived(x) && locs[x as usize].is_empty();
                let (mut seen, mut work) = (BTreeSet::from([r]), vec![r]);
                while let Some(x) = work.pop() {
                    for &k in &rels.rules[x as usize] {
                        for a in self.rules[k].reads() {
                            let y = a.rel().0;
                            if member(y) && seen.insert(y) {
                                deps.insert(self.name(y).to_owned());
                                work.push(y);
                            }
                        }
                    }
                }
                (self.name(r).to_owned(), deps)
            })
            .collect();
        let skip: Vec<bool> = (0..n).map(|r| (covered_now[r] && local_now[r] && locs[r].is_empty()) || native_view(r as u32, &local_now, &covered_now)).collect();
        // A covered view of values is read as a filter is: a part given it reads it so (`part:filter`).
        let filters: BTreeSet<String> = filters.into_iter().chain(derived().filter(|&r| skip[r as usize]).map(|r| self.name(r).to_owned())).collect();
        // The undeclared crossings (decision 0652): a cross-file relation's rule that reads no cross-file relation and,
        // with every input naming no document taken as placeless, still does not place one document.
        // A covered view of values a cross-file rule reads is the cross part's own (`crossing`'s `projected`): rows of no
        // document, as a placeless relation's.
        let placeless_or_input: Vec<bool> = (0..n).map(|r| placeless[r] || skip[r] || (!rels.is_derived(r as u32) && rels.base_locs[r].is_empty())).collect();
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
                // Its reads of documents' rows. None: the run's inputs alone, the run's rows. One plain positive read:
                // each row is one file's, a union (as the cross part reads the chunks' findings back). Neither is a
                // crossing; documents meet only where a rule joins two reads of them, or aggregates one.
                let documents: Vec<&Lit> = form.lits.iter().filter(|l| match l {
                    Lit::Read { rel: x, unset, .. } => !placeless_or_input[*x as usize] && !rels.at_unset(*x, unset, &locs),
                    Lit::Ast | Lit::Nearest(..) => true,
                    Lit::Cmp | Lit::Same(..) => false,
                }).collect();
                let union = matches!(documents.as_slice(), [] | [Lit::Read { neg: false, plain: true, .. }]);
                if reads_cross || union || form.one_document(r, rels, &placeless_or_input, &local_now, &filtering, &skip, &locs, &nulls) {
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
        self.finish(FileLocal { local, constants, locations, filters, causes, nullable, crossings, covered, cover_deps, row_documents, path_keys, ..Default::default() }, self.keys())
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
        // A covered view of values its rules and its readers pass through, at the values a reader gives (decision 0672).
        // Whether asked here or kept from before the edit (outside the scope, where no relation is a candidate).
        let member = |x: u32| covered[x as usize] && local_now[x as usize] && rels.is_derived(x) && locs[x as usize].is_empty();
        // A native read at its inputs' values (`native_inputs`), each input such a view, is one too: UP030's
        // `index_at(v, r, …) :- …, r = sum : { rr_rank1(v, k, n) }`, a field's rank among the string's fields.
        let member_native = |x: u32| local_now[x as usize] && { let inputs = self.native_inputs(x); !inputs.is_empty() && inputs.iter().all(|(i, ..)| member(*i)) };
        for rule in &own {
            for l in rule.body() {
                match l.view() {
                    L::Atom(a) | L::Agg { atom: a, .. } if member(a.rel().0) || member_native(a.rel().0) => {}
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
        // A native whose rows at a tuple are its inputs' rows there (`native_inputs`: `match_all`'s over its demand, a
        // rank's over its query and counted relations) is read at its readers' values: its readers are `r`'s, at the
        // columns of theirs it maps `r`'s to (decision 0672).
        let input_of = |h: u32| -> Option<(Vec<Option<usize>>, bool)> {
            if !local_now[h as usize] {
                return None;
            }
            self.native_inputs(h).into_iter().find(|(x, ..)| *x == r).map(|(_, cols, every)| (cols, every))
        };
        // The file-local readers a view of values is found in: with none it is the cross part's alone, as any relation
        // only cross-file rules read is (its cross-file readers, skipped below, prove nothing).
        let mut local_readers = 0usize;
        for &j in readers {
            let h = rels.head[j];
            // A view of values read by another (or by itself) is read at the values that one's readers give: asked of
            // those readers, through it.
            if valueless[r as usize] && member(h) {
                local_readers += 1;
                continue;
            }
            if valueless[r as usize] && let Some((cols, every_row)) = input_of(h) {
                for &j2 in rels.reading(h) {
                    let h2 = rels.head[j2];
                    if member(h2) {
                        local_readers += 1;
                        continue;
                    }
                    // A cross-file reader reads the cross part's own (`crossing`'s `projected`).
                    if !local_now[h2 as usize] {
                        continue;
                    }
                    local_readers += 1;
                    let reader = self.rules[j2];
                    let qc = value_classes(reader);
                    let targets = Held::of(self.targets_of(reader, r, local_now, locs));
                    let cx = ThroughCx { local_now, covered, placeless, locs, targets: &targets, seen: Default::default(), known: Default::default(), steps: Default::default() };
                    let through = |x: u32, head: &[Vec<Bound>]| self.through(x, head, reader, &cx);
                    for l in reader.body() {
                        let (a, every) = match l.view() {
                            L::Atom(a) => (a, every_row),
                            L::Agg { atom, .. } => (atom, true),
                            _ => continue,
                        };
                        if a.rel().0 != h {
                            continue;
                        }
                        let args: Vec<TermRef<'_>> = a.args().collect();
                        let head: Vec<Vec<Bound>> = cols.iter().map(|c| c.and_then(|c| args.get(c).and_then(|&t| Bound::of(t, &qc))).into_iter().collect()).collect();
                        if let Some(rule) = own.iter().find(|&&rule| !maps_at(self.v, rule, &head, every, &targets, &[], &|x| placeless[x as usize], &columns, &through)) {
                            return Err(format!("rule {} not found in reader {}", self.v.show_rule(*rule), self.v.show_rule(reader)));
                        }
                    }
                }
                continue;
            }
            // A cross-file reader covers a located view too, where its own reads place the view's location columns in
            // one document: the rows it joins are that document's, which the view per chunk derives. A view of values
            // maps into its reader's rows, which are one document's only in a file-local rule: a cross-file reader of
            // one reads the cross part's own, derived there over every file's rows as the whole program does
            // (`crossing`'s `projected`, decision 0672).
            let cross_reader = !local_now[h as usize];
            if h == r {
                return Err(format!("reader {} not local", self.name(h)));
            }
            if cross_reader && valueless[r as usize] {
                continue;
            }
            local_readers += 1;
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
            let targets = Held::of(self.targets_of(reader, r, local_now, locs));
            let cx = ThroughCx { local_now, covered, placeless, locs, targets: &targets, seen: Default::default(), known: Default::default(), steps: Default::default() };
            let through = |x: u32, head: &[Vec<Bound>]| self.through(x, head, reader, &cx);
            for (at, l) in reader.body().enumerate() {
                match l.view() {
                    L::Agg { atom: a, .. } if a.rel().0 == r && valueless[r as usize] => {
                        let found = |rule: RuleRef<'v>| maps(self.v, rule, reader, at, &targets, &[], &|x| placeless[x as usize], &columns, &through);
                        if let Some(rule) = own.iter().find(|&&rule| !found(rule)) {
                            return Err(format!("rule {} not found in reader {}", self.v.show_rule(*rule), self.v.show_rule(reader)));
                        }
                    }
                    L::Atom(a) if a.rel().0 == r => {
                        let args: Vec<TermRef<'_>> = a.args().collect();
                        if !columns(r).iter().all(|&c| args.get(c).and_then(|t| t.var()).is_some_and(|s| placed.contains(&qc[s.0 as usize]))) {
                            return Err(format!("reader leaves a location open: {}", self.v.show_rule(reader)));
                        }
                        if cross_reader && !one_place(self.v, reader, r, &|x| placeless[x as usize], &columns, columns(r).iter().filter_map(|&c| args.get(c).and_then(|t| t.var()))) {
                            return Err(format!("cross-file reader places the view in two documents: {}", self.v.show_rule(reader)));
                        }
                        // A rule the reader's rows do not hold is answered by one they hold whose tests of the head's
                        // values are among its own (`same_tests`): an integer's zero answers a reader of an integer's text
                        // where a float's zero would, over every file, at the same text (decision 0672).
                        let found = |rule: RuleRef<'v>| maps(self.v, rule, reader, at, &targets, columns(r), &|x| placeless[x as usize], &columns, &through);
                        let held: Vec<bool> = own.iter().map(|&rule| found(rule)).collect();
                        let answered = |i: usize| held[i] || own.iter().zip(&held).any(|(&h, &f)| f && same_tests(self.v, h, own[i], &|x| placeless[x as usize]));
                        if let Some(rule) = (0..own.len()).find(|&i| !answered(i)).map(|i| &own[i]) {
                            return Err(format!("rule {} not found in reader {}", self.v.show_rule(*rule), self.v.show_rule(reader)));
                        }
                    }
                    L::Agg { atom, .. } if atom.rel().0 == r => return Err("aggregated by a reader".into()),
                    _ => {}
                }
            }
        }
        if valueless[r as usize] && local_readers == 0 {
            return Err("read by no file-local rule".into());
        }
        Ok(())
    }

    /// The rows `reader` holds in its own document (`maps`' targets): each positive read's, and those each row of a
    /// derived relation it reads implies (decision 0672), a read every rule of that relation makes, its columns the
    /// relation's head columns (`braced(c, v) :- call_str(c, v), …` implies `call_str(c, v)`), two steps down. Only a
    /// file-local relation with a location implies rows: the reader's row places its row in the reader's document,
    /// where its rules derived it. A view of values (`r`, asked, above all) is what is asked, never a ground: its row
    /// may be another file's.
    fn targets_of(&self, reader: RuleRef<'v>, r: u32, local_now: &[bool], locs: &[Vec<usize>]) -> (Vec<Target>, Vec<Test>) {
        let grounds = |x: u32| x != r && local_now[x as usize] && self.rels.is_derived(x) && !locs[x as usize].is_empty();
        let qc = value_classes(reader);
        let mut out: Vec<Target> = Vec::new();
        // The reader's tests, and those of the one rule of a relation grounding a row it reads (a factored helper,
        // `fl…_refers`, holding the batch's scope tests): a variable of that rule's own, none of the head's, a value
        // made for it, one per rule read.
        let mut tests: Vec<Test> = reader.body().filter_map(|l| test_shape(l.view())).map(|(k, ts)| (k, ts.into_iter().map(|t| Bound::of(t, &qc)).collect())).collect();
        let mut made = 1u32 << 31;
        // A plain read only: an aggregate's rows are counted over, not held.
        for a in reader.body().filter_map(|l| match l.view() {
            L::Atom(a) if !a.neg() => Some(a),
            _ => None,
        }) {
            let args: Vec<Option<Bound>> = a.args().map(|t| Bound::of(t, &qc)).collect();
            out.push((a.rel().0, args.clone()));
            self.implied(a.rel().0, &args, 2, &grounds, &mut out);
            if let [k] = self.rels.rules[a.rel().0 as usize].as_slice()
                && grounds(a.rel().0)
            {
                let rule = self.rules[*k];
                let head: Vec<TermRef<'_>> = rule.head().args().collect();
                let mut own: BTreeMap<u32, Bound> = BTreeMap::new();
                for (key, ts) in rule.body().filter_map(|l| test_shape(l.view())) {
                    let leaves = ts
                        .into_iter()
                        .map(|t| match t {
                            TermRef::Var(s) => match head.iter().position(|h| h.var() == Some(s)) {
                                Some(c) => args.get(c).copied().flatten(),
                                None => Some(*own.entry(s.0).or_insert_with(|| {
                                    made += 1;
                                    Bound::Made(made)
                                })),
                            },
                            t => Bound::of(t, &[]),
                        })
                        .collect();
                    tests.push((key, leaves));
                }
            }
        }
        (out, tests)
    }

    /// The rows a row `args` of `x` implies (`targets_of`), `depth` steps down.
    fn implied(&self, x: u32, args: &[Option<Bound>], depth: usize, grounds: &dyn Fn(u32) -> bool, out: &mut Vec<Target>) {
        let rules = &self.rels.rules[x as usize];
        if depth == 0 || !grounds(x) || rules.is_empty() {
            return;
        }
        // A read of a rule as its head's columns: for each argument, the head column holding the same variable.
        let shape = |rule: RuleRef<'_>, a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| -> Vec<Option<usize>> {
            let head: Vec<TermRef<'_>> = rule.head().args().collect();
            a.args().map(|t| t.var().and_then(|s| head.iter().position(|h| h.var() == Some(s)))).collect()
        };
        let reads = |rule: RuleRef<'v>| rule.body().filter_map(|l| match l.view() {
            L::Atom(a) if !a.neg() => Some(a),
            _ => None,
        });
        let first = self.rules[rules[0]];
        for a in reads(first) {
            let y = a.rel().0;
            let s = shape(first, a);
            if s.iter().all(Option::is_none) {
                continue;
            }
            if !rules[1..].iter().all(|&k| reads(self.rules[k]).any(|b| b.rel().0 == y && shape(self.rules[k], b) == s)) {
                continue;
            }
            let row: Vec<Option<Bound>> = s.iter().map(|c| c.and_then(|c| args.get(c).copied().flatten())).collect();
            if out.iter().any(|(r, a)| *r == y && *a == row) {
                continue;
            }
            out.push((y, row.clone()));
            self.implied(y, &row, depth - 1, grounds, out);
        }
    }

    /// The inputs of native `h` whose rows at a tuple of `h` are those it reads there (decision 0672): each input, the
    /// column of `h` each of its columns takes (`None` for one left open), and whether every row there is read.
    /// `match_all`'s rows are a function of its input's text (column 0). A rank's (`rank_form`) at its group and query
    /// columns read the query's row there, its bound open, and every row of the counted relation in the group.
    fn native_inputs(&self, h: u32) -> Vec<(u32, Vec<Option<usize>>, bool)> {
        let width = |x: RelId| self.f.columns(self.v.rel_name(x)).map_or(0, |c| c.len());
        match self.f.native(self.name(h)).map(|nat| &nat.spec) {
            Some(NativeSpec::MatchAll { input, .. }) if self.rels.functional[h as usize] => vec![(input.0, vec![Some(0)], false)],
            Some(NativeSpec::Rank { query, counted, group, .. }) => {
                let (wq, wc) = (width(*query), width(*counted));
                let q = (0..wq).map(|c| (c + 1 < wq).then_some(c)).collect();
                let k = (0..wc).map(|c| (c < *group).then_some(c)).collect();
                vec![(query.0, q, false), (counted.0, k, true)]
            }
            _ => Vec::new(),
        }
    }

    /// How deep `through` asks views through views before it gives up: a proof of a view read through others is a few
    /// levels deep (the format family's, 0672: five).
    const THROUGH_DEPTH: usize = 64;

    /// Whether the covered view of values `s`, its columns at `head`, is found in `reader`'s rows (decision 0672):
    /// each of its rules is (`maps_at`), passing through the covered views of values they read in turn. `None` where
    /// `s` is no such view. A view asked again at the same values on the way is taken as found: a row of it there is
    /// derived from rows of lower height, which the asking proves found. What is known is kept for the reader's one
    /// question (`ThroughCx`). A native is asked of its inputs.
    fn through(&self, s: u32, head: &[Vec<Bound>], reader: RuleRef<'v>, cx: &ThroughCx<'_>) -> Option<bool> {
        // A native whose rows at a tuple are its inputs' rows there (`native_inputs`) is asked of each input there.
        let inputs = if cx.local_now[s as usize] { self.native_inputs(s) } else { Vec::new() };
        if !inputs.is_empty() {
            for (x, cols, _) in inputs {
                let at: Vec<Vec<Bound>> = cols.iter().map(|c| c.and_then(|c| head.get(c).cloned()).unwrap_or_default()).collect();
                if !self.through(x, &at, reader, cx)? {
                    return Some(false);
                }
            }
            return Some(true);
        }
        if !(cx.covered[s as usize] && cx.local_now[s as usize] && self.rels.is_derived(s) && cx.locs[s as usize].is_empty()) {
            return None;
        }
        let key = (s, head.to_vec());
        if let Some(&known) = cx.known.borrow().get(&key) {
            return Some(known);
        }
        if cx.seen.borrow().contains(&key) {
            return Some(true);
        }
        let steps = cx.steps.get() + 1;
        cx.steps.set(steps);
        // Unproven past either bound, so the view is not covered (sound: its readers stay cross-file). Each view asked
        // at new values goes a level deeper (`maps_at`, `through`): about 4 KB of stack a level, and a run nested in
        // another (a stage, decision 0675) went 1,000 deep on a test thread's 2 MB.
        if steps > 4096 || cx.seen.borrow().len() >= Self::THROUGH_DEPTH {
            return Some(false);
        }
        cx.seen.borrow_mut().push(key.clone());
        let rels = &self.rels;
        let columns = |x: u32| -> &[usize] { if rels.is_derived(x) { &cx.locs[x as usize] } else { &rels.base_locs[x as usize] } };
        let through = |x: u32, h: &[Vec<Bound>]| self.through(x, h, reader, cx);
        let found = rels.rules[s as usize].iter().all(|&k| maps_at(self.v, self.rules[k], head, true, cx.targets, &[], &|x| cx.placeless[x as usize], &columns, &through));
        cx.seen.borrow_mut().pop();
        // Found while a view on the way was taken as found holds only if it is: kept only where none was.
        if !found || cx.seen.borrow().is_empty() {
            cx.known.borrow_mut().insert(key, found);
        }
        Some(found)
    }

    /// Why `rule` places more than one document, from its text: `None` where it does
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
        // A cross-file read that is a flag's choice between its copies (decision 0686) is cross-file because the flag
        // is, and a read of the head itself because the head is: where the rule reads another cross-file relation,
        // that one is named, the reason the rule stays cross-file whatever the flag (`plp_colored`: `pc_edge` across
        // files, not `plp_in_rec`, the flag's choice, nor `plp_colored`).
        let choice = |r: RelId| self.rels.rules[r.0 as usize].iter().any(|&k| self.rules[k].reads().any(|a| v.rel_name(a.rel()).starts_with(FLAG_COPY)));
        if let Some(x) = rule.body().find_map(|l| match l.view() {
            L::Atom(a) | L::Agg { atom: a, .. } if self.f.derives(v.rel_name(a.rel())) && !local(v.rel_name(a.rel())) && !constants.contains(v.rel_name(a.rel())) && !filters.contains(v.rel_name(a.rel())) && !self.rels.placeless_input[a.rel().0 as usize] => Some(a.rel()),
            _ => None,
        }) && (choice(x) || x == rule.head().rel())
            && let Some(y) = rule.body().find_map(|l| match l.view() {
                L::Atom(a) | L::Agg { atom: a, .. } if self.f.derives(v.rel_name(a.rel())) && !local(v.rel_name(a.rel())) && !constants.contains(v.rel_name(a.rel())) && !filters.contains(v.rel_name(a.rel())) && !self.rels.placeless_input[a.rel().0 as usize] && !choice(a.rel()) && a.rel() != rule.head().rel() => Some(a.rel()),
                _ => None,
            })
        {
            return Some(format!("reads cross-file {}", v.rel_name(y)));
        }
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
            let constant = (!derived && self.fact(rel) && !self.f.is_input(rel)) || constants.contains(rel) || self.rels.placeless_input[atom.rel().0 as usize] || self.placeless_input(rel);
            let chunk_value = !derived && CHUNK_VALUES.contains(&rel);
            let functional = local(rel) && (self.f.native(rel).is_some_and(|nat| matches!(nat.spec, NativeSpec::MatchAll { .. })) || self.f.qualified(rel, super::STRATEGY));
            if constant || chunk_value || functional || rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION) || filters.contains(rel) || filters.contains(head) {
                continue;
            }
            if derived && !local(rel) {
                return Some(format!("reads cross-file {rel}"));
            }
            let columns = if derived { locations[rel].clone() } else { self.base_placing(rel) };
            let args: Vec<TermRef<'_>> = atom.args().collect();
            // A read at `unset` of a location column that never holds it reads no document (`Rels::at_unset`).
            if columns.iter().any(|&c| matches!(args.get(c), Some(TermRef::Int(-1))) && Rels::never_unset(rel, c)) {
                continue;
            }
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

/// One reader's question of the covered views of values it passes through (`Program::through`): the split's state,
/// the views being asked on the way, what is known, and the steps taken, bounded.
struct ThroughCx<'a> {
    local_now: &'a [bool],
    covered: &'a [bool],
    placeless: &'a [bool],
    locs: &'a [Vec<usize>],
    targets: &'a Held,
    seen: std::cell::RefCell<Vec<(u32, Vec<Vec<Bound>>)>>,
    known: std::cell::RefCell<BTreeMap<(u32, Vec<Vec<Bound>>), bool>>,
    steps: std::cell::Cell<usize>,
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

/// Whether `held`'s rows, at a row of `rule`'s values, hold wherever `held`'s reads are the reader's own (decision 0672):
/// the two rules have the same head, and each of `held`'s literals other than its positive located reads (its tests:
/// comparisons, placeless and negated reads) reads only head values and is one of `rule`'s, the head's values named by
/// their columns. A row of `rule` over every file then passes `held`'s tests at its values too.
fn same_tests(v: &Version, held: RuleRef<'_>, rule: RuleRef<'_>, placeless: &dyn Fn(u32) -> bool) -> bool {
    fn shown(v: &Version, t: TermRef<'_>, col: &dyn Fn(Slot) -> Option<usize>, out: &mut String) -> bool {
        match t {
            TermRef::Var(s) => match col(s) {
                Some(c) => out.push_str(&format!("h{c}")),
                None => return false,
            },
            TermRef::Wild => out.push('_'),
            TermRef::Top => out.push('T'),
            TermRef::Int(i) => out.push_str(&format!("i{i}")),
            TermRef::Str(a) => out.push_str(&format!("s{:?}", v.text(a))),
            TermRef::Branded(b) => out.push_str(&format!("b{b}")),
            TermRef::Call(f, xs) => {
                out.push_str(&format!("{}(", v.text(f)));
                for x in xs {
                    if !shown(v, x, col, out) {
                        return false;
                    }
                    out.push(',');
                }
                out.push(')');
            }
        }
        true
    }
    // A rule's head, its tests that read only head values (the head's variables named by column), and whether it has
    // a test that reads another.
    let of = |r: RuleRef<'_>| -> (String, BTreeSet<String>, bool) {
        let heads: Vec<TermRef<'_>> = r.head().args().collect();
        let col = |s: Slot| heads.iter().position(|t| t.var() == Some(s));
        let mut head = String::new();
        for t in &heads {
            if !shown(v, *t, &col, &mut head) {
                head.push('?');
            }
            head.push(',');
        }
        let mut tests = BTreeSet::new();
        let mut other = false;
        for l in r.body() {
            let mut t = String::new();
            let ok = match l.view() {
                L::Atom(a) if !a.neg() && !placeless(a.rel().0) => continue,
                l if id_range_bound(l) => continue,
                L::Atom(a) => {
                    t.push_str(&format!("{}{}(", if a.neg() { "!" } else { "" }, v.rel_name(a.rel())));
                    a.args().all(|x| {
                        let ok = shown(v, x, &col, &mut t);
                        t.push(',');
                        ok
                    })
                }
                L::Cmp { l, op, r } => {
                    let ok = shown(v, l, &col, &mut t);
                    t.push_str(&format!(" {op:?} "));
                    ok && shown(v, r, &col, &mut t)
                }
                _ => false,
            };
            if ok {
                tests.insert(t);
            } else {
                other = true;
            }
        }
        (head, tests, other)
    };
    let (h, held_tests, held_other) = of(held);
    let (g, rule_tests, _) = of(rule);
    !held_other && !h.contains('?') && h == g && held_tests.is_subset(&rule_tests)
}

/// A bound of an id's range (`$n < 6917529027641081856`, `$d < 1610612736`), as the lowering writes it beside a read
/// of a node or a document (the planner reads it so, `plan::body`): a fact of the id's type, which every id passes.
fn id_range_bound(l: L<'_>) -> bool {
    matches!(l, L::Cmp { l: TermRef::Var(_), op: CmpOp::Lt, r: TermRef::Int(c) } if c.unsigned_abs() >= 1 << 30)
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

/// A value a rule's head column takes in its reader (`maps`): a class of the reader's variables, or a constant.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Bound {
    Var(u32),
    Int(i64),
    Str(u32),
    Branded(crate::code_rule_dsl::logic::front::Branded),
    /// A value of a row held in the reader's chunk (`Held`), none of the reader's terms: one per value made.
    Made(u32),
}

impl Bound {
    /// A reader's term as a bound value, its variables by class (`qc`); `None` for `_` and a call.
    fn of(t: TermRef<'_>, qc: &[u32]) -> Option<Bound> {
        match t {
            TermRef::Var(s) => Some(Bound::Var(qc[s.0 as usize])),
            TermRef::Int(i) => Some(Bound::Int(i)),
            TermRef::Str(a) => Some(Bound::Str(a.0)),
            TermRef::Branded(b) => Some(Bound::Branded(b)),
            _ => None,
        }
    }
}

/// A row a reader holds in its own document, by relation, its columns as bound values (`None` where unknown): one it
/// reads, or one a row it reads implies (`Program::targets_of`).
type Target = (u32, Vec<Option<Bound>>);

/// The rows a reader's chunk holds, as `maps` asks one reader (decision 0672): the reader's targets
/// (`Program::targets_of`), then, while a rule is asked, each row of a view of values it read through at values the
/// chunk holds (the view's rows there are the chunk's), its other values made (`Bound::Made`).
struct Held {
    rows: std::cell::RefCell<Vec<Target>>,
    tests: Vec<Test>,
    made: std::cell::Cell<u32>,
}

impl Held {
    fn of((rows, tests): (Vec<Target>, Vec<Test>)) -> Held {
        Held { rows: std::cell::RefCell::new(rows), tests, made: std::cell::Cell::new(0) }
    }
}

/// A test a reader holds (a comparison, a negated read), as its shape (`test_shape`) and its leaves as bound values:
/// a rule's test on a value only a mapped read binds is found among them (`maps_in`, decision 0672).
type Test = (String, Vec<Option<Bound>>);

/// A test literal's shape and its leaves, in order: a comparison (its sides ordered, for `=` and `!=`) or a negated
/// read; `None` for any other literal or a term it does not compare (`⊤`).
fn test_shape<'a>(l: L<'a>) -> Option<(String, Vec<TermRef<'a>>)> {
    fn term<'a>(t: TermRef<'a>, out: &mut Vec<TermRef<'a>>) -> Option<String> {
        match t {
            TermRef::Var(_) | TermRef::Wild | TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => {
                out.push(t);
                Some("_".to_owned())
            }
            TermRef::Call(f, xs) => {
                let inner: Option<Vec<String>> = xs.map(|x| term(x, out)).collect();
                Some(format!("{}({})", f.0, inner?.join(",")))
            }
            TermRef::Top => None,
        }
    }
    match l {
        L::Cmp { l, op, r } => {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let (sa, sb) = (term(l, &mut a)?, term(r, &mut b)?);
            let swap = matches!(op, CmpOp::Eq | CmpOp::Ne) && sb < sa;
            let (sa, sb, a, b) = if swap { (sb, sa, b, a) } else { (sa, sb, a, b) };
            Some((format!("{op:?} {sa} {sb}"), a.into_iter().chain(b).collect()))
        }
        L::Atom(a) if a.neg() => {
            let mut out = Vec::new();
            let cols: Option<Vec<String>> = a.args().map(|t| term(t, &mut out)).collect();
            Some((format!("!{} {}", a.rel().0, cols?.join(",")), out))
        }
        _ => None,
    }
}

/// How `maps` reads a view of values it passes through (decision 0672): `Some(found)` for a covered view of values,
/// asked at the values its read's columns take (`[]` where none), `None` for any other relation.
type Through<'a> = dyn Fn(u32, &[Vec<Bound>]) -> Option<bool> + 'a;

/// Whether `rule`'s rows are found in `reader`'s at its literal `at`, a read of `rule`'s head, plain or aggregated
/// (decision 0652): `maps_at` with the head's columns at the read's arguments.
#[allow(clippy::too_many_arguments)]
fn maps<'c>(v: &Version, rule: RuleRef<'_>, reader: RuleRef<'_>, at: usize, targets: &Held, anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], through: &Through<'_>) -> bool {
    let qc = value_classes(reader);
    // An aggregate counts every row its read leaves open; a plain read asks for one.
    let (read, every) = match reader.body().nth(at).map(|l| l.view()) {
        Some(L::Atom(a)) => (a, false),
        Some(L::Agg { atom, .. }) => (atom, true),
        _ => return false,
    };
    let head: Vec<Vec<Bound>> = read.args().map(|u| Bound::of(u, &qc).into_iter().collect()).collect();
    maps_at(v, rule, &head, every, targets, anchors, placeless, columns, through)
}

/// Whether `rule`'s rows, its head's columns at `head` (each the reader's values it takes, none where the read leaves
/// it open), are found in `reader`'s (decision 0652). Where `every`, each of its rows at those values is (a view
/// passed through, below): a column left open is made from the fixed ones, never bound by a mapped read, which
/// would find only the row of the reader's own term.
///
/// The head's location columns (`anchors`) are in the reader's document (`covers`), and so is each positive read of a
/// located relation that a location of the rule ties to one of them (through the reads' location columns, `a = b` and
/// `@doc`): anchored. Every other positive located read maps to one of the reader's positive reads of the same
/// relation, argument by argument, under a map of the rule's variables to the reader's terms that takes the head to
/// the read's arguments. A row over every file is then derived from the reader's document alone: its anchored reads
/// are there already, and the others are the reader's own rows, at the variables they share with the rest of the body
/// the head fixes. The rule's other literals (comparisons, placeless reads, negated reads, which must be anchored)
/// read no mapped variable but those the head fixes, and nor does a head value made by a call. A head argument the
/// read leaves `_` fixes nothing. A class of the rule's variables may stand for several of the reader's terms: its
/// head columns' arguments, equal in any row.
///
/// A read of a covered view of values (`through`, decision 0672), positive, negated or aggregated, is asked of that
/// view at the values the head fixes in its columns: where each of the view's rules is found in the reader so, the
/// view's rows there are the reader's document's, and the read reads them alike per chunk and over every file.
#[allow(clippy::too_many_arguments)]
fn maps_at<'c>(v: &Version, rule: RuleRef<'_>, head: &[Vec<Bound>], every: bool, targets: &Held, anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], through: &Through<'_>) -> bool {
    // The rows this rule's reads hold are the rule's: gone once it is asked.
    let n = targets.rows.borrow().len();
    let found = maps_in(v, rule, head, every, targets, anchors, placeless, columns, through);
    targets.rows.borrow_mut().truncate(n);
    found
}

#[allow(clippy::too_many_arguments)]
fn maps_in<'c>(v: &Version, rule: RuleRef<'_>, head: &[Vec<Bound>], every: bool, targets: &Held, anchors: &[usize], placeless: &dyn Fn(u32) -> bool, columns: &dyn Fn(u32) -> &'c [usize], through: &Through<'_>) -> bool {
    fn vars(t: TermRef<'_>, out: &mut Vec<u32>) {
        match t {
            TermRef::Var(s) => out.push(s.0),
            TermRef::Call(_, xs) => xs.for_each(|x| vars(x, out)),
            _ => {}
        }
    }
    let n = rule.var_count();
    let rc = value_classes(rule);
    let mut bind: Vec<Vec<Bound>> = vec![Vec::new(); n];
    let mut anchor_vars: Vec<u32> = Vec::new();
    // The variables of the head's values made by a call (`spelled(v, cat("'", v))`, `brace_name(v, i * 1000, …)`):
    // checked as the other literals' are, below.
    let mut made: Vec<u32> = Vec::new();
    for (c, h) in rule.head().args().enumerate() {
        match h {
            TermRef::Var(s) => {
                for &t in head.get(c).map(Vec::as_slice).unwrap_or(&[]) {
                    if !bind[rc[s.0 as usize] as usize].contains(&t) {
                        bind[rc[s.0 as usize] as usize].push(t);
                    }
                }
                if anchors.contains(&c) {
                    anchor_vars.push(s.0);
                }
            }
            TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => {}
            TermRef::Call(..) => vars(h, &mut made),
            _ => return false,
        }
    }
    let given: Vec<bool> = bind.iter().map(|b| !b.is_empty()).collect();
    // A read's columns at the values the head fixes.
    let fixed = |bind: &[Vec<Bound>], a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>| -> Vec<Vec<Bound>> {
        a.args()
            .map(|t| match t {
                TermRef::Var(s) => bind[rc[s.0 as usize] as usize].clone(),
                t => Bound::of(t, &[]).into_iter().collect(),
            })
            .collect()
    };
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
    // How often each variable occurs: one occurring once in a read is any value there, as `_` is.
    let mut occurring: Vec<u32> = Vec::new();
    rule.head().args().for_each(|t| vars(t, &mut occurring));
    for l in rule.body() {
        match l.view() {
            L::Atom(a) | L::Agg { atom: a, .. } => a.args().for_each(|t| vars(t, &mut occurring)),
            L::Cmp { l, r, .. } => {
                vars(l, &mut occurring);
                vars(r, &mut occurring);
            }
            L::Nearest { n: x, a: y, .. } => occurring.extend([x.0, y.0]),
            // Any other literal's variables: none counts as occurring once.
            _ => occurring.extend((0..n as u32).flat_map(|s| [s, s])),
        }
    }
    let mut uses = vec![0usize; n];
    occurring.into_iter().for_each(|s| uses[s as usize] += 1);
    let mut mapped = Vec::new();
    let mut in_mapped = vec![false; n];
    let mut others: Vec<Vec<u32>> = Vec::new();
    // The tests (`test_shape`) a mapped value reaches: each found among the reader's (`Held::tests`), after the reads.
    let mut tests: Vec<(L<'_>, Vec<u32>)> = Vec::new();
    for l in rule.body() {
        let mut vs = Vec::new();
        // A row the chunk holds (`Held`) at the values the read fixes, its other columns any value: one is enough.
        if let L::Atom(a) = l.view()
            && !a.neg()
            && targets.rows.borrow().iter().any(|(x, row)| {
                *x == a.rel().0
                    && a.args().zip(row).all(|(t, y)| match t {
                        TermRef::Wild => true,
                        TermRef::Var(s) if uses[s.0 as usize] == 1 => true,
                        TermRef::Var(s) => y.is_some_and(|y| bind[rc[s.0 as usize] as usize].contains(&y)),
                        t => Bound::of(t, &[]).is_some_and(|b| *y == Some(b)),
                    })
            })
        {
            a.args().for_each(|t| vars(t, &mut vs));
            others.push(vs);
            continue;
        }
        // A covered view of values, read at the values fixed so far.
        if let L::Atom(a) | L::Agg { atom: a, .. } = l.view()
            && let Some(found) = through(a.rel().0, &fixed(&bind, a))
        {
            if !found {
                return false;
            }
            // Its rows at those values are the chunk's: a positive read's are held, each value it leaves open one
            // of theirs, made.
            if let L::Atom(a) = l.view()
                && !a.neg()
            {
                for t in a.args() {
                    if let TermRef::Var(s) = t
                        && bind[rc[s.0 as usize] as usize].is_empty()
                    {
                        let m = targets.made.get();
                        targets.made.set(m + 1);
                        bind[rc[s.0 as usize] as usize].push(Bound::Made(m));
                    }
                }
                let row = a.args().map(|t| match t {
                    TermRef::Var(s) => bind[rc[s.0 as usize] as usize].first().copied(),
                    t => Bound::of(t, &[]),
                });
                targets.rows.borrow_mut().push((a.rel().0, row.collect()));
            }
            a.args().for_each(|t| vars(t, &mut vs));
            others.push(vs);
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
                        TermRef::Wild | TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_) => {}
                        _ => return false,
                    }
                }
                mapped.push(a);
            }
            L::Atom(a) => {
                a.args().for_each(|t| vars(t, &mut vs));
                // A negated read of rows of a document the rule does not anchor holds where the reader holds it.
                if a.neg() && !placeless(a.rel().0) && !anchored(a) {
                    tests.push((l.view(), vs));
                    continue;
                }
                match test_shape(l.view()) {
                    Some(_) => tests.push((l.view(), vs)),
                    None => others.push(vs),
                }
            }
            L::Cmp { l: TermRef::Var(a), op: CmpOp::Eq, r: TermRef::Var(b) } if rc[a.0 as usize] == rc[b.0 as usize] => {}
            // Every id of its column passes, the reader's as the rule's.
            l if id_range_bound(l) => {}
            L::Cmp { l: x, r: y, .. } => {
                vars(x, &mut vs);
                vars(y, &mut vs);
                match test_shape(l.view()) {
                    Some(_) => tests.push((l.view(), vs)),
                    None => others.push(vs),
                }
            }
            L::Nearest { .. } => {}
            _ => return false,
        }
    }
    // A test on a value a mapped read binds, and not the head, holds for the reader's row only where the reader holds
    // it there: such a test is found among the reader's (`Held::tests`), its own values (`s = @doc(c)`'s `s`) bound
    // by the finding, and a test on those is one too. A negated read of an unanchored document's rows is one always.
    let offending = |in_mapped: &[bool], vs: &[u32]| vs.iter().any(|&s| in_mapped[rc[s as usize] as usize] && !given[rc[s as usize] as usize]);
    let mut matched: Vec<L<'_>> = Vec::new();
    let mut open: Vec<(L<'_>, Vec<u32>)> = Vec::new();
    for (l, vs) in tests {
        let unanchored = matches!(l, L::Atom(a) if !placeless(a.rel().0) && !anchored(a));
        if unanchored { matched.push(l); vs.iter().for_each(|&s| in_mapped[rc[s as usize] as usize] = true); } else { open.push((l, vs)); }
    }
    loop {
        let (now, rest): (Vec<_>, Vec<_>) = open.into_iter().partition(|(_, vs)| offending(&in_mapped, vs));
        open = rest;
        if now.is_empty() {
            break;
        }
        for (l, vs) in now {
            matched.push(l);
            vs.iter().for_each(|&s| in_mapped[rc[s as usize] as usize] = true);
        }
    }
    others.extend(open.into_iter().map(|(_, vs)| vs));
    if others.iter().flatten().chain(&made).any(|&s| {
        let c = rc[s as usize] as usize;
        in_mapped[c] && !given[c]
    }) {
        return false;
    }
    let matched: Vec<(String, Vec<TermRef<'_>>)> = match matched.into_iter().map(test_shape).collect::<Option<Vec<_>>>() {
        Some(m) => m,
        None => return false,
    };
    if every && rule.head().args().filter_map(|t| t.var()).any(|s| in_mapped[rc[s.0 as usize] as usize] && !given[rc[s.0 as usize] as usize]) {
        return false;
    }
    // Backtracking over the mapped reads, each to a target of its relation; bounded, a failure past the bound.
    // A term against a held value: `_` any, a constant itself, a variable its class's value, bound here if none yet.
    fn fit(x: TermRef<'_>, y: Option<Bound>, bind: &mut [Vec<Bound>], rc: &[u32]) -> bool {
        match x {
            TermRef::Wild => true,
            TermRef::Int(i) => y == Some(Bound::Int(i)),
            TermRef::Str(s) => y == Some(Bound::Str(s.0)),
            TermRef::Branded(b) => y == Some(Bound::Branded(b)),
            TermRef::Var(s) => {
                let c = rc[s.0 as usize] as usize;
                match y {
                    None => false,
                    Some(u) if bind[c].is_empty() => {
                        bind[c].push(u);
                        true
                    }
                    Some(u) => bind[c].contains(&u),
                }
            }
            _ => false,
        }
    }
    // The tests after the reads, each to one of the reader's of its shape.
    fn go_tests(k: usize, tests: &[(String, Vec<TermRef<'_>>)], held: &[Test], bind: &mut Vec<Vec<Bound>>, rc: &[u32], steps: &mut usize) -> bool {
        let Some((key, ts)) = tests.get(k) else { return true };
        for (_, b) in held.iter().filter(|(hk, b)| hk == key && b.len() == ts.len()) {
            *steps += 1;
            if *steps > 4096 {
                return false;
            }
            let saved = bind.clone();
            if ts.iter().zip(b).all(|(&x, &y)| fit(x, y, bind, rc)) && go_tests(k + 1, tests, held, bind, rc, steps) {
                return true;
            }
            *bind = saved;
        }
        false
    }
    #[allow(clippy::too_many_arguments)]
    fn go(k: usize, mapped: &[crate::code_rule_dsl::logic::program_image::AtomRef<'_>], targets: &[Target], tests: &[(String, Vec<TermRef<'_>>)], held: &[Test], bind: &mut Vec<Vec<Bound>>, rc: &[u32], steps: &mut usize) -> bool {
        let Some(a) = mapped.get(k) else { return go_tests(0, tests, held, bind, rc, steps) };
        for (_, b) in targets.iter().filter(|(rel, _)| *rel == a.rel().0) {
            *steps += 1;
            if *steps > 4096 {
                return false;
            }
            let saved = bind.clone();
            let fits = a.args().zip(b).all(|(x, &y)| fit(x, y, bind, rc));
            if fits && go(k + 1, mapped, targets, tests, held, bind, rc, steps) {
                return true;
            }
            *bind = saved;
        }
        false
    }
    let mut steps = 0usize;
    go(0, &mapped, &targets.rows.borrow(), &matched, &targets.tests, &mut bind, &rc, &mut steps)
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

pub(super) fn crossing(v: &Version, f: Fields<'_>, local: &BTreeSet<String>, views: &BTreeSet<String>) -> Crossing {
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
    let mut projected: BTreeSet<String> = cross_reads.iter().map(String::as_str).chain(local.iter().map(String::as_str).filter(|r| elsewhere(r))).filter(|r| is_local(r) && projection(r)).map(str::to_owned).collect();
    // A guard (`STRATEGY`: a pair cache's `small`) chooses how its readers are evaluated, not their rows, and a chunk's
    // is the chunk's choice. The cross-file part reading one derives its own, and the relations it guards that it
    // reads (the pairs), from the factors it reads: an export of a guard names no document, and `locate` would move
    // it with every reader (decision 0672).
    let strategy = |rel: &str| is_local(rel) && f.qualified(rel, super::STRATEGY);
    let guards = |rel: &str| v.rel(rel).is_some_and(|id| v.heads(id).any(|r| r.body().any(|l| l.atom().is_some_and(|a| !a.neg() && strategy(v.rel_name(a.rel()))))));
    let guarded: Vec<&str> = cross_reads.iter().map(String::as_str).filter(|r| is_local(r) && !strategy(r) && guards(r)).collect();
    let own_guards: Vec<&str> = cross_reads
        .iter()
        .map(String::as_str)
        .chain(guarded.iter().flat_map(|g| v.rel(g).into_iter().flat_map(|id| v.heads(id).flat_map(|r| r.reads().map(|a| v.rel_name(a.rel())).collect::<Vec<_>>()))))
        .filter(|r| strategy(r))
        .collect();
    projected.extend(guarded.iter().chain(&own_guards).map(|r| (*r).to_owned()));
    // A covered view of values (`FileLocal::views`) a cross-file rule reads is the cross part's own too, derived there
    // over every file's rows as the whole program derives it, with the views its rules read and the natives read at
    // values (`match_all`'s, a rank's) over them: their rows name no document, so they cannot be handed on (decision
    // 0672). Each chunk keeps its own for its readers.
    let native_reads = |rel: &str| -> Vec<String> { f.natives().iter().filter(|n| v.rel_name(n.rel) == rel).flat_map(|n| n.reads.iter()).map(|&r| v.rel_name(r).to_owned()).collect() };
    let at_values = |rel: &str| {
        let reads = native_reads(rel);
        f.native(rel).is_some_and(|n| matches!(n.spec, NativeSpec::MatchAll { .. } | NativeSpec::Rank { .. })) && !reads.is_empty() && reads.iter().all(|r| views.contains(r))
    };
    let view = |rel: &str| is_local(rel) && (views.contains(rel) || at_values(rel));
    let mut work: Vec<String> = cross_reads.iter().filter(|r| view(r)).cloned().collect();
    while let Some(x) = work.pop() {
        if !projected.insert(x.clone()) {
            continue;
        }
        let reads: Vec<String> = v.rel(&x).into_iter().flat_map(|id| v.heads(id).flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())).chain(native_reads(&x)).collect();
        for y in reads {
            if !projected.contains(&y) && view(&y) {
                work.push(y);
            }
        }
    }
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
    base_locs: Vec<Vec<usize>>,
    /// Its locations alone (`Program::base_locations`): what a row's document is read off.
    base_real: Vec<Vec<usize>>,
    candidate: Vec<bool>,
    output: Vec<bool>,
    /// A derived relation's rules, by index in the program's order.
    rules: Vec<Vec<usize>>,
    /// The rules reading each relation, by index, each once, in order.
    reading: Vec<Vec<usize>>,
    /// Each rule's head.
    head: Vec<u32>,
    /// The tape's relation, whose parent column holds `unset` at a root.
    tape: Option<u32>,
    _v: std::marker::PhantomData<&'v ()>,
}

impl<'v> Rels<'v> {
    /// Whether a location column of `rel` never holds `unset` (-1): every one but the tape's parent column, whose root's
    /// parent is -1. A derived relation's nullable columns (decision 0649) are not its locations.
    fn never_unset(rel: &str, c: usize) -> bool {
        rel != "tape" || c != 1
    }

    /// Whether a read is at `unset` in one of its relation's location columns that never holds it: it matches no row
    /// of any document, so it reads none (a test, empty in every file). The patterns' lowering writes such reads for a
    /// metavariable a branch leaves unbound (`unset(T)`, then `kind(T, "none")`; `!file_descriptor(S)`).
    fn at_unset(&self, r: u32, unset: &[bool], locs: &[Vec<usize>]) -> bool {
        let columns = if self.is_derived(r) { &locs[r as usize] } else { &self.base_real[r as usize] };
        columns.iter().any(|&c| unset.get(c).copied().unwrap_or(false) && (Some(r) != self.tape || c != 1))
    }

    fn of(v: &'v Version, f: Fields<'v>, rules: &[RuleRef<'v>], staged: &BTreeSet<String>) -> Rels<'v> {
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
        // A dead rule's head is derived, by no rule the split reads (`dead`).
        for r in v.rules().filter(|r| dead(v, *r) && !staged.contains(v.rel_name(r.head().rel()))) {
            derived[r.head().rel().0 as usize] = true;
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
        let functional = |r: &str| f.native(r).is_some_and(|nat| matches!(nat.spec, NativeSpec::MatchAll { .. })) || f.qualified(r, super::STRATEGY);
        Rels {
            n,
            derived_count: derived.iter().filter(|d| **d).count(),
            fact: (0..n).map(|r| base(r) && p.fact(name(r)) && !f.is_input(name(r))).collect(),
            // A stage's relations, and an earlier phase's export (decision 0675), are placeless: a file's memo entry is
            // keyed by the rows of them its probes read.
            placeless_input: (0..n).map(|r| (derived[r] || read[r]) && (p.placeless_input(name(r)) || staged.contains(name(r)) || (!derived[r] && crate::code_rule_dsl::logic::plan::phased::is_probed(name(r))))).collect(),
            chunk_value: (0..n).map(|r| base(r) && CHUNK_VALUES.contains(&name(r))).collect(),
            test: (0..n).map(|r| (derived[r] || read[r]) && test(name(r))).collect(),
            functional: (0..n).map(|r| derived[r] && functional(name(r))).collect(),
            text: (0..n).map(|r| if derived[r] || read[r] { crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(t, _)| *t == name(r)).map(|(_, c)| *c) } else { None }).collect(),
            base_locs: (0..n).map(|r| if base(r) && !staged.contains(name(r)) { p.base_placing(name(r)) } else { Vec::new() }).collect(),
            base_real: (0..n).map(|r| if base(r) && !staged.contains(name(r)) { p.base_locations(name(r)) } else { Vec::new() }).collect(),
            candidate: (0..n).map(|r| derived[r] && f.native(name(r)).is_none() && !f.lattice(name(r)) && !f.is_input(name(r))).collect(),
            output: (0..n).map(|r| derived[r] && f.is_output(name(r))).collect(),
            rules: by_head,
            reading,
            head,
            derived,
            tape: v.rel("tape").map(|r| r.0),
            _v: std::marker::PhantomData,
        }
    }

    fn empty() -> Rels<'v> {
        Rels { n: 0, derived_count: 0, derived: Vec::new(), fact: Vec::new(), placeless_input: Vec::new(), chunk_value: Vec::new(), test: Vec::new(), functional: Vec::new(), text: Vec::new(), base_locs: Vec::new(), base_real: Vec::new(), candidate: Vec::new(), output: Vec::new(), rules: Vec::new(), reading: Vec::new(), head: Vec::new(), tape: None, _v: std::marker::PhantomData }
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

/// A body literal as the split reads it.
enum Lit {
    /// A read: its variables by argument, and its string constants by argument (decision 0665: a path written out,
    /// `bcgp_line("a.py", l, sp)`), each numbered among the rule's (`Form::paths`).
    Read { neg: bool, plain: bool, rel: u32, args: Vec<Option<u32>>, consts: Vec<Option<u32>>, binder: bool, tapes: Vec<u32>, unset: Vec<bool> },
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
    class: Vec<u32>,
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
    /// The classes the body tests set (`!unset(x)`, `x != -1`): a nullable location column's variable among them is a
    /// node of its row's document, so it places as a location column does (decision 0672).
    set: Vec<u32>,
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
                // Both nodes are bound by the read of the tree (decision 0675).
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
                        binder: placed.by_binder.values().any(|b| *b == index),
                        tapes: stand,
                        unset: args.iter().map(|t| matches!(t, TermRef::Int(-1))).collect(),
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
        let unset = crate::code_rule_dsl::logic::rr::lower::UNSET;
        let set = rule
            .body()
            .filter_map(|l| match l.view() {
                L::Cmp { l: TermRef::Var(x), op: CmpOp::Ne, r: TermRef::Int(u) } | L::Cmp { l: TermRef::Int(u), op: CmpOp::Ne, r: TermRef::Var(x) } if u == unset => Some(class[x.0 as usize]),
                _ => None,
            })
            .collect();
        Form { named, class, lits, splices, ids, binders, paths, built, set }
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
    fn placed_classes(&self, rels: &Rels<'_>, locs: &[Vec<usize>]) -> Bits {
        self.placed_classes_set(rels, locs, &[])
    }

    /// `placed_classes`, and the classes of the nullable location columns `nulls` (by relation; none where empty) whose
    /// variables the body tests set (`set`, decision 0672).
    fn placed_classes_set(&self, rels: &Rels<'_>, locs: &[Vec<usize>], nulls: &[Vec<usize>]) -> Bits {
        let mut out = self.placed_classes_over(rels, locs, &rels.base_locs);
        self.tested_set(rels, nulls, &mut out);
        loop {
            let grew: Vec<u32> = self.built.iter().filter(|&&(x, y)| out.has(y) && !out.has(x)).map(|&(x, _)| x).collect();
            if grew.is_empty() {
                return out;
            }
            grew.into_iter().for_each(|x| out.set(x));
        }
    }

    /// `placed_classes` with the base relations' columns `base`: their placing columns, or their locations alone.
    fn placed_classes_over(&self, rels: &Rels<'_>, locs: &[Vec<usize>], base: &[Vec<usize>]) -> Bits {
        let mut out = Bits::new(self.ids);
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l {
                let columns = if rels.is_derived(*rel) { &locs[*rel as usize] } else { &base[*rel as usize] };
                for &c in columns {
                    if let Some(Some(v)) = args.get(c) {
                        out.set(self.class[*v as usize]);
                    }
                }
            }
        }
        // `nearest(f, s, k)`: `s` is an ancestor of `f`, in its tree. One placed places the other (`rdefn(s, x, f)`,
        // a definition and its scope), as the prepared program's read of `nearest` from the transducer does.
        loop {
            let mut changed = false;
            for l in &self.lits {
                if let Lit::Nearest(n, a) = l
                    && out.has(*n) != out.has(*a)
                {
                    out.set(*n);
                    out.set(*a);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        out
    }

    /// The classes of the nullable location columns `nulls` of the body's positive reads whose variables it tests set.
    fn tested_set(&self, rels: &Rels<'_>, nulls: &[Vec<usize>], out: &mut Bits) {
        if self.set.is_empty() {
            return;
        }
        for l in &self.lits {
            if let Lit::Read { neg: false, rel, args, .. } = l
                && rels.is_derived(*rel)
            {
                for &c in nulls.get(*rel as usize).map(Vec::as_slice).unwrap_or(&[]) {
                    if let Some(Some(v)) = args.get(c)
                        && self.set.contains(&self.class[*v as usize])
                    {
                        out.set(self.class[*v as usize]);
                    }
                }
            }
        }
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
                Lit::Read { neg, plain, rel, args, consts, binder, tapes, unset } => {
                    let r = *rel;
                    let derived = rels.is_derived(r);
                    // A function of strings where it is per-file is read as a test is.
                    if placeless[r as usize] || (derived && filters[r as usize]) || covered[r as usize] || head_filter || (rels.functional[r as usize] && local[r as usize]) {
                        continue;
                    }
                    if derived && !local[r as usize] {
                        return false;
                    }
                    // After a cross-file read's refusal, as `why_not_local`: a chunk holds no row of one.
                    if rels.at_unset(r, unset, locs) {
                        continue;
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

/// Whether a rule derives nothing: a comparison of two constants in its body fails (`-1 != -1`, `1 = 2`). The
/// patterns' lowering writes one for a branch whose `unset(x)` and `!unset(x)` meet. The split leaves it out: it places
/// no column and joins no document (its head stays derived, `Rels::of`).
fn dead(v: &Version, rule: RuleRef<'_>) -> bool {
    rule.body().any(|l| match l.view() {
        L::Cmp { l: TermRef::Int(a), op, r: TermRef::Int(b) } => !match op {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        },
        L::Cmp { l: TermRef::Str(a), op: CmpOp::Eq, r: TermRef::Str(b) } => v.text(a) != v.text(b),
        L::Cmp { l: TermRef::Str(a), op: CmpOp::Ne, r: TermRef::Str(b) } => v.text(a) == v.text(b),
        _ => false,
    })
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
