//! A program as a run prepares it, kept in the store across processes (decisions 0073, 0155): pruned, simplified,
//! demanded, lowered, its joins pushed, located and split.
//!
//! - **Programs.** Its two programs (`source`, the program as demanded; `lowered`, as lowered) are each kept as their
//!   `.rerail` text (`program_text`), a source of the store by its sha256, and read back by the rr parser straight into a
//!   program: nothing is lowered or joined again.
//! - **Pointer.** Kept facts of the store (t33 step 2, `pointer`), one partition a program: what preparing found
//!   (`prepared_*` relations, small: per relation or per intern call, not per node) and the two programs' digests
//!   (`prepared_program`). A run reads the pointer and the two texts; when any is gone it prepares.
//!
//! The plan names what preparing reads besides the texts: the engine build, the grammar version, the dialect and what
//! the run added to the program (`prepared_key`). The split (`file_local::programs_of`) is a function of the lowered
//! program, and what it found is kept too (`prepared_file_local`, `prepared_constant`, `prepared_location` …): finding
//! it again cost about 0.6 s of the Python pack's warm run. Its two parts are not kept: each is a part of the lowered
//! program's rules, made from it again on reading (`file_local::assemble`). The text tests' matchers are made again
//! on reading (from their specs).
//!
//! A prepared program holds atoms of the store, which lowering interned. They are never given out again (decision
//! 0049); `prepared_interned` lists every intern call, and the run asks them again before it uses the program, as it
//! does of one kept in memory (decision 0060). A program another store prepared (the engine ships them, decision
//! 0264) has that store's atoms: the strings are interned here and its branded name atoms moved to these
//! (`lower::relocate`, decision 0270).

use rerail_blocks::vector::Vector;
pub use super::Prepared;
use super::lower;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_facts::{self, Facts};
use crate::code_rule_dsl::logic::program_text;
use crate::code_rule_dsl::logic::strings::Vocabulary;
use crate::content_store::{ContentStore, FactsPlanSet, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The pointer's schema: bumped when its meaning changes (105: the programs as their text, decision 0155; 106: the
/// decisions preparing made, decision 0269; 107: the memo's strata's leaves, each set of them once). It names the
/// shipped pointer (`shipped_key`).
const POINTER: u32 = 108;

/// The pointer's plan: the engine build, the grammar version and `key` (the program's texts, the dialect and the
/// run's additions).
fn pointer_plan(key: &[u8; 32]) -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.prepared/v3\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
    hash.update(key);
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The plans the program `key` names is stored under: the pointer's. The run's manifest names it
/// (`record_corpus_run_naming`), so collection keeps its rows while it keeps the run, and takes those of an earlier
/// engine build or grammar once no kept run names them (the program's text, their source, may live on).
pub(super) fn plans(key: &[u8; 32]) -> Vector<[u8; 32]> {
    rerail_blocks::vector![pointer_plan(key).plans[0]]
}

/// The relations a prepared program adds to the program schema, and their columns.
pub fn schema() -> BTreeMap<String, Vec<(String, String)>> {
    const SCHEMA: &[(&str, &[&str])] = &[
        // A relation's columns' domains, one letter a column (`domain_code`): a row a relation, not a column (t33 K4:
        // the pointer's rows are what each new process reads).
        ("prepared_domain", &["rel:sym", "domains:sym"]),
        ("prepared_op", &["k:int", "atom:int"]),
        ("prepared_set", &["k:int", "rel:sym"]),
        ("prepared_set_kind", &["k:int", "i:int", "kind:int"]),
        ("prepared_demanded", &["k:int", "rel:sym"]),
        ("prepared_pushed", &["k:int", "rel:sym"]),
        ("prepared_factorized", &["k:int", "line:sym"]),
        ("prepared_located", &["ok:int"]),
        ("prepared_located_rel", &["k:int", "rel:sym"]),
        ("prepared_merged", &["relations:int", "classes:int"]),
        ("prepared_interned", &["k:int", "width:int"]),
        ("prepared_interned_atom", &["k:int", "i:int", "s:sym", "atom:int"]),
        ("prepared_test", &["k:int", "rel:sym", "op:sym"]),
        ("prepared_test_pattern", &["k:int", "i:int", "pattern:sym"]),
        // The vocabulary's `match_all` regexes (their sources) and `fstring_template` markers, in order.
        ("prepared_match", &["k:int", "regex:sym"]),
        ("prepared_marker", &["k:int", "marker:sym"]),
        // The split (`file_local::programs_of`): what it found, the file-local relations and those of constants. Its two
        // parts are the lowered program's rules, a part each, made from it again on reading (`file_local::assemble`).
        // The per-file memo's strata of the file-local part (`memo::Strata`, decision 0246 §4): the part's digest,
        // each relation's strata identity and the set of leaves it reaches (`set`), each set once (relations of one
        // stratum and their readers reach the same leaves: kept a relation each, they were most of the pointer's rows).
        ("prepared_memo_part", &["digest:sym"]),
        ("prepared_memo_stratum", &["k:int", "rel:sym", "id:sym", "set:int"]),
        ("prepared_memo_reach", &["set:int", "rels:sym"]),
        ("prepared_split", &["ok:int"]),
        ("prepared_file_local", &["k:int", "rel:sym"]),
        ("prepared_constant", &["k:int", "rel:sym"]),
        // A relation's columns, in order, comma-separated (empty for none): a row a relation.
        ("prepared_location", &["rel:sym", "cols:sym"]),
        ("prepared_same_doc", &["rel:sym", "cols:sym"]),
        // The decisions preparing made (decision 0269): a run that reuses the program states them.
        ("prepared_decided", &["k:int", "topic:sym", "text:sym", "count:int"]),
    ];
    SCHEMA
        .iter()
        .map(|(rel, cols)| ((*rel).to_owned(), cols.iter().map(|c| c.split_once(':').expect("name:type")).map(|(n, t)| (n.to_owned(), t.to_owned())).collect()))
        .collect()
}

/// A domain's letter in `prepared_domain`'s `domains`.
fn domain_code(d: lower::Domain) -> char {
    match d {
        lower::Domain::Int => 'i',
        lower::Domain::Node => 'n',
        lower::Domain::Kind => 'k',
        lower::Domain::Field => 'f',
        lower::Domain::Name => 'm',
    }
}

fn domain_of_code(c: char) -> Option<lower::Domain> {
    Some(match c {
        'i' => lower::Domain::Int,
        'n' => lower::Domain::Node,
        'k' => lower::Domain::Kind,
        'f' => lower::Domain::Field,
        'm' => lower::Domain::Name,
        _ => return None,
    })
}

/// A list of columns as `prepared_location`'s `cols` keeps it, and back.
fn cols_text(cols: &[usize]) -> String {
    cols.iter().map(usize::to_string).collect::<Vec<_>>().join(",")
}

fn cols_of_text(text: &str) -> Option<Vector<usize>> {
    if text.is_empty() {
        return Some(Vector::new());
    }
    text.split(',').map(|c| c.parse::<usize>().ok()).collect()
}

/// What preparing found, as facts: `prepared`'s other than its two programs.
#[cfg(any(test, feature = "test-hooks"))]
pub fn facts_of(prepared: &Prepared) -> Facts {
    rest_of(prepared)
}

/// `prepared`'s facts other than its programs'.
fn rest_of(prepared: &Prepared) -> Facts {
    let mut facts = Facts::new();
    let s = |x: &str| Value::Str(x.to_owned().into());
    let n = |x: usize| Value::Int(x as i64);
    let mut row = |rel: &str, values: Vec<Value>| facts.entry(rel.to_owned()).or_default().push(values.into());
    for (rel, domains) in prepared.lowered.domains.iter() {
        row("prepared_domain", vec![s(rel), s(&domains.iter().map(|d| domain_code(*d)).collect::<String>())]);
    }
    for (k, atom) in prepared.lowered.ops.iter().enumerate() {
        row("prepared_op", vec![n(k), Value::Int(*atom)]);
    }
    for (k, (rel, kinds)) in prepared.sets.iter().enumerate() {
        row("prepared_set", vec![n(k), s(rel)]);
        for (i, kind) in kinds.iter().enumerate() {
            row("prepared_set_kind", vec![n(k), n(i), Value::Int(*kind)]);
        }
    }
    for (k, (rel, matcher, captures)) in prepared.matchers.iter().enumerate() {
        row("prepared_matcher", vec![n(k), s(rel), s(matcher), s(&captures.join(","))]);
    }
    for (k, rel) in prepared.demanded.iter().enumerate() {
        row("prepared_demanded", vec![n(k), s(rel)]);
    }
    for (k, rel) in prepared.pushed.iter().enumerate() {
        row("prepared_pushed", vec![n(k), s(rel)]);
    }
    for (k, line) in prepared.factorized.iter().enumerate() {
        row("prepared_factorized", vec![n(k), s(line)]);
    }
    let (ok, located) = match &prepared.located {
        Ok(rels) => (1, rels),
        Err(rels) => (0, rels),
    };
    row("prepared_located", vec![n(ok)]);
    for (k, rel) in located.iter().enumerate() {
        row("prepared_located_rel", vec![n(k), s(rel)]);
    }
    row("prepared_merged", vec![n(prepared.merged.0), n(prepared.merged.1)]);
    for (k, (strings, atoms)) in prepared.interned.iter().enumerate() {
        row("prepared_interned", vec![n(k), n(strings.len())]);
        for (i, (x, atom)) in strings.iter().zip(atoms).enumerate() {
            row("prepared_interned_atom", vec![n(k), n(i), s(x), Value::Int(i64::from(*atom))]);
        }
    }
    // The per-file memo's strata (decision 0246 §4), with the digest of the file-local part they are of.
    if let Some((digest, strata)) = prepared.memo_strata.get() {
        row("prepared_memo_part", vec![s(&hex(digest))]);
        let mut sets: std::collections::HashMap<&[String], usize> = std::collections::HashMap::new();
        for (k, (rel, (id, reached))) in strata.by_rel.iter().enumerate() {
            let fresh = sets.len();
            let set = *sets.entry(&reached[..]).or_insert(fresh);
            row("prepared_memo_stratum", vec![n(k), s(rel), s(&hex(id)), n(set)]);
            if set == fresh {
                row("prepared_memo_reach", vec![n(set), s(&reached.join(","))]);
            }
        }
    }
    row("prepared_split", vec![n(1)]);
    for (k, rel) in prepared.split.file_local.iter().enumerate() {
        row("prepared_file_local", vec![n(k), s(rel)]);
    }
    for (k, rel) in prepared.split.constants.iter().enumerate() {
        row("prepared_constant", vec![n(k), s(rel)]);
    }
    for (which, map) in [("prepared_location", &prepared.split.locations), ("prepared_same_doc", &prepared.split.same_doc)] {
        for (rel, cols) in map {
            row(which, vec![s(rel), s(&cols_text(cols))]);
        }
    }
    for (k, ((rel, _), (op, patterns))) in prepared.vocabulary.tests.iter().zip(&prepared.vocabulary.specs).enumerate() {
        row("prepared_test", vec![n(k), s(rel), s(op)]);
        for (i, pattern) in patterns.iter().enumerate() {
            row("prepared_test_pattern", vec![n(k), n(i), s(pattern)]);
        }
    }
    for (k, re) in prepared.vocabulary.matches.iter().enumerate() {
        row("prepared_match", vec![n(k), s(re.as_str())]);
    }
    for (k, marker) in prepared.vocabulary.markers.iter().enumerate() {
        row("prepared_marker", vec![n(k), s(marker)]);
    }
    for (k, ((topic, text), count)) in prepared.decided.iter().enumerate() {
        row("prepared_decided", vec![n(k), s(topic), s(text), Value::Int(*count as i64)]);
    }
    facts
}

fn bad(what: impl std::fmt::Display) -> String {
    format!("prepared facts: {what}")
}

fn int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Str(x) => Err(bad(format!("{x:?} where a number goes"))),
        Value::Branded(b) => Err(bad(format!("the atom {b} where a number goes"))),
    }
}

fn text(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(x) => Ok(x.to_string()),
        Value::Int(i) => Err(bad(format!("{i} where a string goes"))),
        Value::Branded(b) => Err(bad(format!("the atom {b} where a string goes"))),
    }
}

/// The pointer's rows, where they are: a kept entry of the store, read in place (`kept_entry::Entries::visit`,
/// decision 0353), or facts (the engine's shipped bundle, a program this process prepared for another store).
enum Rows<'a> {
    Facts(&'a Facts),
    Kept(&'a ContentStore, i64),
}

/// A row's cells, as `Rows` hands them.
trait Cells {
    fn len(&self) -> usize;
    fn int(&self, c: usize) -> Result<i64, String>;
    fn text(&self, c: usize) -> Result<String, String>;
}

/// A row of facts' values.
struct Values<'a>(&'a [Value]);

impl Cells for Values<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn int(&self, c: usize) -> Result<i64, String> {
        int(self.0.get(c).ok_or_else(|| bad(format!("no column {c}")))?)
    }
    fn text(&self, c: usize) -> Result<String, String> {
        text(self.0.get(c).ok_or_else(|| bad(format!("no column {c}")))?)
    }
}

impl Cells for crate::code_rule_dsl::logic::plan::kept_entry::Row<'_> {
    fn len(&self) -> usize {
        crate::code_rule_dsl::logic::plan::kept_entry::Row::len(self)
    }
    fn int(&self, c: usize) -> Result<i64, String> {
        crate::code_rule_dsl::logic::plan::kept_entry::Row::int(self, c).ok_or_else(|| bad(format!("column {c} is no number")))
    }
    fn text(&self, c: usize) -> Result<String, String> {
        crate::code_rule_dsl::logic::plan::kept_entry::Row::text(self, c).map(|t| t.to_string()).ok_or_else(|| bad(format!("column {c} is no string")))
    }
}

impl Rows<'_> {
    /// Each row of the pointer's relations (`prepared_*`), with its relation, in no order: one read of the entry.
    fn all(&self, f: &mut dyn FnMut(&str, &dyn Cells) -> Result<(), String>) -> Result<(), String> {
        match self {
            Rows::Facts(facts) => facts.iter().try_for_each(|(rel, rows)| rows.iter().try_for_each(|r| f(rel, &Values(&r[..])))),
            Rows::Kept(store, part) => {
                let mut failed = None;
                let (_, bad_parts) = pointer::ENTRIES.visit(store, Some(&[*part]), &|r| r.starts_with("prepared_"), &mut |_, rel, row| match f(rel, row) {
                    Ok(()) => true,
                    Err(e) => {
                        failed = Some(e);
                        false
                    }
                });
                match failed {
                    Some(e) => Err(e),
                    None if !bad_parts.is_empty() => Err(bad("a row that does not read")),
                    None => Ok(()),
                }
            }
        }
    }
}

/// The pointer's rows as read (`gather`), each relation's parsed, with the places its indexes give: what
/// `prepared_from` makes the prepared program of, and `lookup` reads the program's texts and atoms from.
#[derive(Default)]
struct Gathered {
    program: BTreeMap<String, [u8; 32]>,
    domains: lower::Domains,
    ops: Vec<(i64, i64)>,
    set_kinds: BTreeMap<i64, Vec<(i64, i64)>>,
    sets: Vec<(i64, String)>,
    lists: BTreeMap<String, Vec<(i64, String)>>,
    matchers: Vec<(i64, (String, String, Vec<String>))>,
    located: Option<i64>,
    merged: Option<(usize, usize)>,
    interned: Vec<(i64, i64)>,
    interned_atoms: BTreeMap<i64, Vec<(i64, (String, u32))>>,
    tests: Vec<(i64, (String, String))>,
    patterns: BTreeMap<i64, Vec<(i64, String)>>,
    matches: Vec<(i64, String)>,
    split: bool,
    locations: BTreeMap<String, Vector<usize>>,
    same_doc: BTreeMap<String, Vector<usize>>,
    memo_part: Option<String>,
    memo_reach: BTreeMap<i64, Arc<[String]>>,
    memo_stratum: Vec<(String, [u8; 32], i64)>,
    decided: Vec<(i64, ((String, String), u64))>,
}

/// The pointer's rows, read once.
fn gather(rows: &Rows<'_>) -> Result<Gathered, String> {
    let mut g = Gathered::default();
    rows.all(&mut |rel, r| {
        match rel {
            PROGRAM => {
                g.program.insert(r.text(0)?, unhex(&r.text(1)?).ok_or_else(|| bad("a program's digest"))?);
            }
            "prepared_domain" => {
                let codes = r.text(1)?;
                let ds = codes.chars().map(|c| domain_of_code(c).ok_or_else(|| bad(format!("no domain {c}")))).collect::<Result<Vec<_>, String>>()?;
                g.domains.insert(r.text(0)?, ds);
            }
            "prepared_op" => g.ops.push((r.int(0)?, r.int(1)?)),
            "prepared_set_kind" => g.set_kinds.entry(r.int(0)?).or_default().push((r.int(1)?, r.int(2)?)),
            "prepared_set" => g.sets.push((r.int(0)?, r.text(1)?)),
            "prepared_located_rel" | "prepared_demanded" | "prepared_pushed" | "prepared_factorized" | "prepared_marker" | "prepared_file_local" | "prepared_constant" => {
                g.lists.entry(rel.to_owned()).or_default().push((r.int(0)?, r.text(1)?));
            }
            // Captured names (decision 0107): none in a program kept before.
            "prepared_matcher" => {
                let captures = if r.len() > 3 { Some(r.text(3)?) } else { None };
                let captures = captures.filter(|c| !c.is_empty()).map(|c| c.split(',').map(str::to_owned).collect()).unwrap_or_default();
                g.matchers.push((r.int(0)?, (r.text(1)?, r.text(2)?, captures)));
            }
            "prepared_located" => g.located = Some(r.int(0)?),
            "prepared_merged" => g.merged = Some((r.int(0)? as usize, r.int(1)? as usize)),
            "prepared_interned" => g.interned.push((r.int(0)?, r.int(1)?)),
            "prepared_interned_atom" => {
                let atom = u32::try_from(r.int(3)?).map_err(bad)?;
                g.interned_atoms.entry(r.int(0)?).or_default().push((r.int(1)?, (r.text(2)?, atom)));
            }
            "prepared_test" => g.tests.push((r.int(0)?, (r.text(1)?, r.text(2)?))),
            "prepared_test_pattern" => g.patterns.entry(r.int(0)?).or_default().push((r.int(1)?, r.text(2)?)),
            "prepared_match" => g.matches.push((r.int(0)?, r.text(1)?)),
            "prepared_split" => g.split = true,
            "prepared_location" | "prepared_same_doc" => {
                let cols = r.text(1)?;
                let cols = cols_of_text(&cols).ok_or_else(|| bad(format!("columns {cols}")))?;
                let map = if rel == "prepared_location" { &mut g.locations } else { &mut g.same_doc };
                map.insert(r.text(0)?, cols);
            }
            "prepared_memo_part" => g.memo_part = Some(r.text(0)?),
            // Each set once, shared by the relations that reach it.
            "prepared_memo_reach" => {
                let rels = r.text(1)?;
                let rels: Arc<[String]> = if rels.is_empty() { Arc::from(Vec::new()) } else { rels.split(',').map(str::to_owned).collect() };
                g.memo_reach.insert(r.int(0)?, rels);
            }
            "prepared_memo_stratum" => {
                let id = unhex(&r.text(2)?).ok_or_else(|| bad("a memo stratum's identity".to_owned()))?;
                g.memo_stratum.push((r.text(1)?, id, r.int(3)?));
            }
            "prepared_decided" => g.decided.push((r.int(0)?, ((r.text(1)?, r.text(2)?), r.int(3)? as u64))),
            _ => {}
        }
        Ok(())
    })?;
    Ok(g)
}

/// Rows by their place (an index), in its order.
fn in_order<T>(mut rows: Vec<(i64, T)>) -> Vec<T> {
    rows.sort_by_key(|(k, _)| *k);
    rows.into_iter().map(|(_, t)| t).collect()
}

/// The prepared program of `facts` (what preparing found, `rest_of`), its source's version (the program before
/// lowering) and its lowered program's version, for `dialect`.
pub fn prepared_of(
    facts: &Facts,
    source: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    dialect: GrammarDialect,
) -> Result<Prepared, String> {
    prepared_from(gather(&Rows::Facts(facts))?, None, source, version, dialect)
}

/// The prepared program of the pointer's rows `g`, its atoms moved by `moved` where they moved (`moves`): the interned
/// names' atoms and the operators'.
fn prepared_from(
    mut g: Gathered,
    moved: Option<&rustc_hash::FxHashMap<u32, u32>>,
    source: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    dialect: GrammarDialect,
) -> Result<Prepared, String> {
    let atom = |a: i64| -> Result<i64, String> {
        match moved {
            None => Ok(a),
            Some(to) => u32::try_from(a).ok().and_then(|a| to.get(&a)).map(|a| i64::from(*a)).ok_or_else(|| bad(format!("the atom {a} did not move"))),
        }
    };
    let ops: Vec<i64> = in_order(g.ops).into_iter().map(atom).collect::<Result<_, _>>()?;
    let ops: [i64; 3] = ops.try_into().map_err(|_| bad("not three operators"))?;
    let sets = in_order(g.sets.into_iter().map(|(k, rel)| (k, (rel, in_order(g.set_kinds.remove(&k).unwrap_or_default()).into_iter().collect()))).collect());
    let mut lists = std::mem::take(&mut g.lists);
    let mut list = |rel: &str| in_order(lists.remove(rel).unwrap_or_default());
    let ok = g.located.ok_or_else(|| bad("no located"))?;
    let located_rels = list("prepared_located_rel");
    let located = if ok == 1 { Ok(located_rels) } else { Err(located_rels) };
    let merged = g.merged.ok_or_else(|| bad("no merged"))?;
    let mut interned = Vec::new();
    for (k, width) in in_order(g.interned.iter().map(|(k, w)| (*k, (*k, *w))).collect()) {
        let call = in_order(g.interned_atoms.remove(&k).unwrap_or_default());
        if call.len() as i64 != width {
            return Err(bad(format!("intern call {k} has {} of its {width} strings", call.len())));
        }
        let (strings, ids): (Vec<String>, Vec<i64>) = call.into_iter().map(|(x, a)| (x, i64::from(a))).unzip();
        interned.push((strings, ids.into_iter().map(|a| atom(a).map(|a| a as u32)).collect::<Result<Vector<u32>, String>>()?));
    }
    let (mut relations, mut specs) = (Vec::new(), Vec::new());
    for (k, (rel, op)) in in_order(g.tests.into_iter().map(|(k, t)| (k, (k, t))).collect()) {
        relations.push(rel);
        specs.push((op, in_order(g.patterns.remove(&k).unwrap_or_default())));
    }
    let vocabulary = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the text tests' vocabulary".into());
        let mut vocabulary = Vocabulary::from_kept_specs(&relations, specs);
        vocabulary.matches = in_order(g.matches).iter().map(|m| regex::Regex::new(m).map(Arc::new).map_err(bad)).collect::<Result<_, String>>()?;
        vocabulary.markers = list("prepared_marker");
        vocabulary
    };
    let split = match g.split {
        true => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the split's parts, of the lowered program and what the split found".into());
            let set = |rels: Vec<String>| rels.into_iter().collect::<std::collections::BTreeSet<String>>();
            let (local, constants) = (set(list("prepared_file_local")), set(list("prepared_constant")));
            crate::code_rule_dsl::typing::file_local::assemble(version.clone(), local, constants, std::mem::take(&mut g.locations), std::mem::take(&mut g.same_doc))?
        }
        false => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "split the lowered program".into());
            let split = crate::code_rule_dsl::typing::file_local::programs_of(version.clone())?;
            // No relation is cross-file without a declaration (decision 0652).
            crate::code_rule_dsl::typing::file_local::crossings_checked(&split)?;
            split
        }
    };
    // The memo's strata, when the program was kept with them; else the first run makes them.
    let memo_strata = std::sync::OnceLock::new();
    if let Some(digest) = &g.memo_part {
        let digest = unhex(digest).ok_or_else(|| bad("a memo part's digest".to_owned()))?;
        let by_rel = g.memo_stratum.into_iter().map(|(rel, id, set)| (rel, (id, g.memo_reach.get(&set).cloned().unwrap_or_else(|| Arc::from(Vec::new()))))).collect();
        let _ = memo_strata.set((digest, super::memo::Strata { by_rel }));
    }
    Ok(Prepared {
        source,
        vocabulary: Arc::new(vocabulary),
        // Its stored image alone: the tree read is the split's, made above, and dropped.
        lowered: lower::Lowered { version, domains: Arc::new(g.domains), ops, dialect, normal: Default::default() },
        sets,
        matchers: in_order(g.matchers),
        demanded: list("prepared_demanded"),
        pushed: list("prepared_pushed"),
        factorized: list("prepared_factorized"),
        located,
        merged,
        split,
        interned,
        documents: Default::default(),
        runs: None,
        cone: Default::default(),
        maintained: Default::default(),
        parts: Default::default(),
        decided: in_order(g.decided).into_iter().collect(),
        memo_strata,
    })
}

fn invalid(e: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(e.to_string())
}

/// The relation naming each kept program's text: (name, sha256 hex).
const PROGRAM: &str = "prepared_program";

fn hex(bytes: &[u8]) -> String {
    rerail_prof::code_rule_dsl::prof::lower_hex(bytes)
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// What the engine ships the pointer of the program `key` names under (decision 0264): the pointer's plan names the
/// engine build, so a bundle of another build is never read.
fn shipped_key(key: &[u8; 32]) -> HashKey {
    HashKey::sha256(&[format!("rerail prepared pointer {POINTER}\0").as_bytes(), pointer_plan(key).plans[0].as_slice()].concat())
}

/// Where a prepared program `lookup` found came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Found {
    /// The store's, with its atoms.
    Kept,
    /// Another store's (the engine's bundle, or the store's own of other atoms), its atoms moved to this store's:
    /// the run keeps it in the store, so the next one reads it as `Kept`.
    Moved,
}

/// The program `key` names, prepared for `dialect`, as an earlier process stored it under `source`, or else as the
/// engine ships it; `None` when neither has it, or when what is kept no longer reads. With it, the sources of its
/// programs' texts, and whether its atoms moved.
pub(super) fn lookup(store: &ContentStore, source: Option<SourceId>, dialect: GrammarDialect, key: &[u8; 32]) -> StoreResult<Option<(Prepared, Vec<SourceId>, Found)>> {
    let start = std::time::Instant::now();
    // The store's, where its program's text is stored (a pointer is kept only beside its text, `record`): its rows read
    // where the store keeps them, once. Else the engine's shipped one.
    let kept = source.and_then(|_| {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the pointer, read".into());
        gather(&Rows::Kept(store, pointer::part(key))).ok().filter(|g| !g.program.is_empty())
    });
    let (g, size) = match kept {
        Some(g) => (g, 0),
        None => match crate::code_rule_dsl::logic::rr::images::shipped().and_then(|b| b.get(shipped_key(key))) {
            Some(bytes) => {
                crate::code_rule_dsl::prof::count("corpus.prepared_shipped", 1);
                let Ok(facts) = program_facts::decode(bytes) else { return Ok(None) };
                let Ok(g) = gather(&Rows::Facts(&facts)) else { return Ok(None) };
                (g, bytes.len())
            }
            None => return Ok(None),
        },
    };
    let texts = &g.program;
    let (Some(source), Some(lowered)) = (texts.get("source"), texts.get("lowered")) else { return Ok(None) };
    let mut documents = Vec::new();
    // Both programs' versions opened from their images, no tree decoded (decision 0281's path, steps 5 and R): the
    // split's parts are views of the lowered one.
    let source = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "open the program's image before lowering".into());
        let Some((version, sources)) = program_text::load_image(store, source)? else { return Ok(None) };
        documents.extend(sources);
        version
    };
    let version = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "open the lowered program's image".into());
        let Some((version, sources)) = program_text::load_image(store, lowered)? else { return Ok(None) };
        documents.extend(sources);
        version
    };
    let read = start.elapsed();
    let at = std::time::Instant::now();
    // Its atoms this store's, the version is read as opened; another store's (decision 0270), its tree is decoded
    // once to move them and imaged again (the one tree a lookup reads, and only on a move: work queue Q43).
    let mut version = version;
    let (found, moved) = match moves(store, &g, &version) {
        Some(None) => (Found::Kept, None),
        Some(Some((to, moved))) => {
            version = moved;
            crate::code_rule_dsl::prof::count("corpus.prepared_moved", 1);
            (Found::Moved, Some(to))
        }
        None => return Ok(None),
    };
    let got = { let _s = crate::code_rule_dsl::logic::profile::span(|| "what preparing found, read".into()); prepared_from(g, moved.as_ref(), source, version, dialect) };
    if std::env::var_os("RERAIL_PREPARED_TRACE").is_some() {
        eprintln!("RERAIL_PREPARED_TRACE lookup ({}): {:.1} kB of shipped facts; read {:.3} s, to a prepared program {:.3} s{}",
            super::phases::current().unwrap_or_else(|| "main".into()), size as f64 / 1e3, read.as_secs_f64(), at.elapsed().as_secs_f64(),
            got.as_ref().err().map(|e| format!("; not read: {e}")).unwrap_or_default());
    }
    Ok(got.ok().map(|p| (p, documents, found)))
}

/// `prepared`, kept in this process for another store, with its name atoms moved to `store`'s (decision 0270), as
/// `lookup` moves one another store kept: a process that runs one program over several stores prepares it once.
/// `None` when a name has no atom here or the program does not read back.
pub(super) fn moved(store: &ContentStore, prepared: &Prepared, dialect: GrammarDialect) -> Option<Prepared> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "the prepared program of another store, moved to this one's atoms".into());
    let mut facts = rest_of(prepared);
    let version = match relocate(store, &mut facts, &prepared.lowered.version)? {
        Some(moved) => moved,
        None => std::sync::Arc::clone(&prepared.lowered.version),
    };
    crate::code_rule_dsl::prof::count("corpus.prepared_moved", 1);
    prepared_of(&facts, std::sync::Arc::clone(&prepared.source), version, dialect).ok()
}

/// Moves the lowered program's name atoms, and the pointer's (`prepared_op`, `prepared_interned_atom`), to `store`'s
/// (decision 0270): its strings interned here in one call. `Some(None)` when they are already this store's,
/// `Some(version)` of the moved program when they moved; `None` when the program holds a name atom no intern call
/// Moves the lowered program's name atoms, and the pointer's (`prepared_op`, `prepared_interned_atom`), to `store`'s
/// (decision 0270): its strings interned here in one call. `Some(None)` when they are already this store's,
/// `Some(version)` of the moved program when they moved; `None` when the program holds a name atom no intern call
/// made, or the store refuses.
pub fn relocate(
    store: &ContentStore,
    facts: &mut Facts,
    version: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
) -> Option<Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>> {
    let Some((to, moved)) = moves(store, &gather(&Rows::Facts(facts)).ok()?, version)? else { return Some(None) };
    let to_of = |v: &Value| -> Option<Value> { Some(Value::Int(i64::from(*to.get(&u32::try_from(v.integer()?).ok()?)?))) };
    for row in facts.get_mut("prepared_interned_atom")? {
        let mut cells: Vec<Value> = row.to_vec();
        cells[3] = to_of(cells.get(3)?)?;
        *row = cells.into();
    }
    for row in facts.get_mut("prepared_op")? {
        let mut cells: Vec<Value> = row.to_vec();
        cells[1] = to_of(cells.get(1)?)?;
        *row = cells.into();
    }
    Some(Some(moved))
}

/// Where the pointer's name atoms (`g`'s) move to in `store` (decision 0270), and the lowered program `version` with
/// its atoms moved: `Some(None)` when they are this store's already; `None` when a name has no atom here, or the
/// store refuses.
#[allow(clippy::type_complexity)]
fn moves(
    store: &ContentStore,
    g: &Gathered,
    version: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
) -> Option<Option<(rustc_hash::FxHashMap<u32, u32>, std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>)>> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "the prepared program's atoms, in this store".into());
    let (strings, old): (Vec<&str>, Vec<u32>) = g.interned_atoms.values().flatten().map(|(_, (x, atom))| (x.as_str(), *atom)).unzip();
    if strings.is_empty() {
        return None;
    }
    let new: Vec<u32> = store.intern_atoms(&strings).ok()?.into_iter().collect();
    if new == old {
        return Some(None);
    }
    let to: rustc_hash::FxHashMap<u32, u32> = old.iter().copied().zip(new.iter().copied()).collect();
    // The moved program, a version of its own: the run reads it in place.
    let moved = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the moved program's image".into());
        lower::relocate(version, &to).ok()??
    };
    crate::code_rule_dsl::logic::profile::decision("the prepared program's atoms (decision 0270)", || format!("prepared by another store: its {} names moved to this store's atoms", strings.len()));
    Some(Some((to, moved)))
}

/// The sources of the programs' texts the program `key` names under `source` (`lookup`'s), with neither program read:
/// for a run that holds the prepared program in memory. Empty when the store keeps none.
pub(super) fn documents(store: &ContentStore, key: &[u8; 32]) -> StoreResult<Vec<SourceId>> {
    let Some(mut facts) = pointer::get(store, key) else { return Ok(Vec::new()) };
    let mut digests = Vec::new();
    for row in facts.remove(PROGRAM).unwrap_or_default() {
        if let [Value::Str(name), Value::Str(digest)] = row.as_slice()
            && (name.as_ref() == "source" || name.as_ref() == "lowered")
            && let Some(digest) = unhex(digest)
        {
            digests.push(digest);
        }
    }
    program_text::sources(store, &digests)
}

/// What the write of the program `key` is queued under (`write_back`, decision 0102).
pub(super) fn queued(key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/prepared", key])
}

/// Keeps `prepared` as the program `key` names, in the run's record (decisions 0073, 0306): its two programs and the
/// pointer to them with what preparing found, made here; written under `source` (else the source of `text`, stored)
/// by the store's writer, which names the programs' sources so collection keeps them. Nothing when a program does not
/// read back from its text: the next run prepares again.
pub(super) fn record(store: &ContentStore, source: Option<SourceId>, text: String, key: &[u8; 32], prepared: &Prepared) {
    let mut facts = rest_of(prepared);
    let mut programs = Vec::new();
    for (name, version) in [("source", &prepared.source), ("lowered", &prepared.lowered.version)] {
        // Each program's image is its version's, taken as it is (decision 0281). One whose image is not made is not
        // kept: the program is prepared again.
        let Some(encoded) = program_text::encode_version(version) else { return };
        facts.entry(PROGRAM.to_owned()).or_default().push([Value::Str(name.to_owned().into()), Value::Str(hex(&encoded.digest).into())].into());
        programs.push(encoded);
    }
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships the pointer, encoded; its
    // programs' images ship with `program_text::encode_version`.
    if std::env::var_os("RERAIL_EXPORT_IMAGES").is_some() {
        crate::code_rule_dsl::logic::rr::images::export(shipped_key(key), &program_facts::encode(&facts));
    }
    // The programs' texts (and the run's text, which the pointer is kept beside), by the store's writer in the run's
    // record; the pointer as kept facts, behind the run.
    let (packed, pieces) = program_text::pack(programs, Vec::new());
    pointer::put(store, &text, key, &facts);
    store.record_held("the prepared program", vec![queued(key)], &(source, text, *key, packed), pieces, apply);
}


fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
    let (source, text, key, packed): (Option<SourceId>, String, [u8; 32], program_text::Packed) = crate::content_store::record::codec::from_body("the prepared program", body)?;
    let source = match source {
        Some(id) => id,
        None => {
            let id = super::memo::source(cx.store, &text)?;
            cx.name([id]);
            id
        }
    };
    let _ = (source, key);
    let (documents, _) = program_text::store_packed(cx.store, &packed, cx.pieces)?;
    cx.name(documents);
    Ok(())
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).

// Its tests run in the engine's test binary (its facade of this module mounts `identity_tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rerail` libraries).

/// The pointer as kept facts of the store (t33 step 2, decisions 0104 and 0456): each of its relations
/// (`prepared_*`, `prepared_program`) a kept relation of the store's prepared programs' space, the program's rows one
/// partition (its pointer plan's: the engine build, the grammar version and `key`), held by the run's text's source
/// and the pointer plan (`kept_entry`), as the facts-memo entry it stood in (decision 0052) was.
mod pointer {
    use super::Facts;
    use crate::code_rule_dsl::logic::plan::kept_entry::Entries;
    use crate::content_store::{ContentStore, HashKey};

    pub(super) const ENTRIES: Entries = Entries::new("rerail.prepared");

    fn plan(key: &[u8; 32]) -> [u8; 32] {
        super::pointer_plan(key).plans[0]
    }

    /// The program `key`'s pointer, as kept; `None` when the store keeps none (or one that no longer reads).
    pub(super) fn part(key: &[u8; 32]) -> i64 {
        Entries::part(&plan(key))
    }

    pub(super) fn get(store: &ContentStore, key: &[u8; 32]) -> Option<Facts> {
        ENTRIES.get(store, Entries::part(&plan(key)))
    }

    /// Keeps `facts` as the program `key`'s pointer, held by the source of the run's `text`, behind the run.
    pub(super) fn put(store: &ContentStore, text: &str, key: &[u8; 32], facts: &Facts) {
        let plan = plan(key);
        let holder = HashKey::sha256(text.as_bytes()).sha256_bytes().expect("a sha256 key");
        ENTRIES.put(store, Entries::part(&plan), holder, Some(plan), facts);
    }
}
