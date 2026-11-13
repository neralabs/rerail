//! A program as a run prepares it, kept in the store across processes (decisions 0073, 0155): pruned, simplified,
//! demanded, lowered, its joins pushed, located and split.
//!
//! - **Programs.** Its two programs (`front`, the program as demanded; `lowered`, as lowered) are each kept as their
//!   `.rr` text (`program_text`), a source of the store by its sha256, and read back by the rr parser straight into a
//!   program: nothing is lowered or joined again.
//! - **Pointer.** One facts-memo entry (decision 0052) under the program text's source, for the run's plan: what
//!   preparing found (`prepared_*` relations, small: per relation or per intern call, not per node) and the two
//!   programs' digests (`prepared_program`). A run reads the pointer and the two texts; when any is gone it prepares.
//!
//! The plan names what preparing reads besides the texts: the engine build, the grammar version, the dialect and what
//! the run added to the program (`prepared_key`). The split (`file_local::programs`) is a function of the lowered
//! program, and what it found is kept too (`prepared_file_local`, `prepared_constant`, `prepared_location` …): finding
//! it again cost about 0.6 s of the Python pack's warm run. Its two parts are not kept: each is a part of the lowered
//! program's rules, made from it again on reading (`file_local::assemble`). The text tests' matchers are made again
//! on reading (from their specs).
//!
//! A prepared program holds atoms of the store, which lowering interned. They are never given out again (decision
//! 0049); `prepared_interned` lists every intern call, and the run asks them again before it uses the program, as it
//! does of one kept in memory (decision 0060).

pub use super::Prepared;
use super::lower;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front::Value;
use crate::code_rule_dsl::logic::program_facts::{self, Facts};
use crate::code_rule_dsl::logic::program_text;
use crate::code_rule_dsl::logic::strings::Vocabulary;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, SourceId, StoreError, StoreResult};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The pointer's schema: bumped when its meaning changes (105: the programs as their text, decision 0155).
const POINTER: u32 = crate::content_store::live_fact_schema(105);

/// The path a lookup names. The entries read no path (`path_dependent: false`); the store keeps them under none.
const PATH: &str = "rerail/prepared";

/// The pointer's plan: the engine build, the grammar version and `key` (the program's texts, the dialect and the
/// run's additions).
fn pointer_plan(key: &[u8; 32]) -> FactsPlanSet {
    let mut hash = Sha256::new();
    hash.update(b"rerail.prepared/v2\0");
    hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
    hash.update(key);
    FactsPlanSet::new(vec![hash.finalize().into()])
}

/// The plans the program `key` names is stored under: the pointer's. The run's manifest names it
/// (`record_corpus_run_naming`), so collection keeps its rows while it keeps the run, and takes those of an earlier
/// engine build or grammar once no kept run names them (the program's text, their source, may live on).
pub(super) fn plans(key: &[u8; 32]) -> Vec<[u8; 32]> {
    vec![pointer_plan(key).plans[0]]
}

/// The relations a prepared program adds to the program schema, and their columns.
pub fn schema() -> BTreeMap<String, Vec<(String, String)>> {
    const SCHEMA: &[(&str, &[&str])] = &[
        ("prepared_domain", &["rel:sym", "i:int", "domain:sym"]),
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
        // The split (`file_local::programs`): what it found, the file-local relations and those of constants. Its two
        // parts are the lowered program's rules, a part each, made from it again on reading (`file_local::assemble`).
        ("prepared_split", &["ok:int"]),
        ("prepared_file_local", &["k:int", "rel:sym"]),
        ("prepared_constant", &["k:int", "rel:sym"]),
        ("prepared_location", &["rel:sym", "i:int", "col:int"]),
        ("prepared_same_doc", &["rel:sym", "i:int", "col:int"]),
    ];
    SCHEMA
        .iter()
        .map(|(rel, cols)| ((*rel).to_owned(), cols.iter().map(|c| c.split_once(':').expect("name:type")).map(|(n, t)| (n.to_owned(), t.to_owned())).collect()))
        .collect()
}

fn domain_name(d: lower::Domain) -> &'static str {
    match d {
        lower::Domain::Int => "int",
        lower::Domain::Node => "node",
        lower::Domain::Kind => "kind",
        lower::Domain::Field => "field",
        lower::Domain::Name => "name",
    }
}

fn domain_of(name: &str) -> Option<lower::Domain> {
    Some(match name {
        "int" => lower::Domain::Int,
        "node" => lower::Domain::Node,
        "kind" => lower::Domain::Kind,
        "field" => lower::Domain::Field,
        "name" => lower::Domain::Name,
        _ => return None,
    })
}

/// What preparing found, as facts: `prepared`'s other than its two programs.
#[cfg(any(test, feature = "test-hooks"))]
pub fn facts_of(prepared: &Prepared) -> Facts {
    rest_of(prepared)
}

/// `prepared`'s facts other than its programs'.
fn rest_of(prepared: &Prepared) -> Facts {
    let mut facts = Facts::new();
    let s = |x: &str| Value::Str(x.to_owned());
    let n = |x: usize| Value::Int(x as i64);
    let mut row = |rel: &str, values: Vec<Value>| facts.entry(rel.to_owned()).or_default().push(values);
    for (rel, domains) in &prepared.lowered.domains {
        for (i, d) in domains.iter().enumerate() {
            row("prepared_domain", vec![s(rel), n(i), s(domain_name(*d))]);
        }
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
    row("prepared_split", vec![n(1)]);
    for (k, rel) in prepared.split.file_local.iter().enumerate() {
        row("prepared_file_local", vec![n(k), s(rel)]);
    }
    for (k, rel) in prepared.split.constants.iter().enumerate() {
        row("prepared_constant", vec![n(k), s(rel)]);
    }
    for (which, map) in [("prepared_location", &prepared.split.locations), ("prepared_same_doc", &prepared.split.same_doc)] {
        for (rel, cols) in map {
            // A presence row (index -1), so a relation with no columns reads back as itself.
            row(which, vec![s(rel), Value::Int(-1), Value::Int(-1)]);
            for (i, c) in cols.iter().enumerate() {
                row(which, vec![s(rel), n(i), n(*c)]);
            }
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
    facts
}

fn bad(what: impl std::fmt::Display) -> String {
    format!("prepared facts: {what}")
}

fn int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Str(x) => Err(bad(format!("{x:?} where a number goes"))),
    }
}

fn text(v: &Value) -> Result<String, String> {
    match v {
        Value::Str(x) => Ok(x.clone()),
        Value::Int(i) => Err(bad(format!("{i} where a string goes"))),
    }
}

/// The rows of `rel` by their first column (an index): each row less that column, in order.
fn indexed<'a>(facts: &'a Facts, rel: &str) -> Result<Vec<(i64, &'a [Value])>, String> {
    let mut rows: Vec<(i64, &[Value])> = facts.get(rel).into_iter().flatten().map(|r| Ok((int(&r[0])?, &r[1..]))).collect::<Result<_, String>>()?;
    rows.sort_by_key(|(k, _)| *k);
    Ok(rows)
}

/// The rows of `rel` grouped by their first column (an index), each group in the order of its second (an index), each
/// row less both.
fn nested<'a>(facts: &'a Facts, rel: &str) -> Result<BTreeMap<i64, Vec<&'a [Value]>>, String> {
    let mut out: BTreeMap<i64, Vec<(i64, &[Value])>> = BTreeMap::new();
    for r in facts.get(rel).into_iter().flatten() {
        out.entry(int(&r[0])?).or_default().push((int(&r[1])?, &r[2..]));
    }
    Ok(out
        .into_iter()
        .map(|(k, mut rows)| {
            rows.sort_by_key(|(i, _)| *i);
            (k, rows.into_iter().map(|(_, r)| r).collect())
        })
        .collect())
}

/// The prepared program of `facts` (what preparing found, `rest_of`) and its two programs, for `dialect`.
pub fn prepared_of(facts: &Facts, prog: crate::code_rule_dsl::logic::front::Program, program: crate::code_rule_dsl::logic::front::Program, dialect: GrammarDialect) -> Result<Prepared, String> {
    let mut by_rel: BTreeMap<String, Vec<(i64, lower::Domain)>> = BTreeMap::new();
    for r in facts.get("prepared_domain").into_iter().flatten() {
        let d = text(&r[2])?;
        by_rel.entry(text(&r[0])?).or_default().push((int(&r[1])?, domain_of(&d).ok_or_else(|| bad(format!("no domain {d}")))?));
    }
    let domains: lower::Domains = by_rel
        .into_iter()
        .map(|(rel, mut ds)| {
            ds.sort_by_key(|(i, _)| *i);
            (rel, ds.into_iter().map(|(_, d)| d).collect())
        })
        .collect();
    let ops: Vec<i64> = indexed(facts, "prepared_op")?.iter().map(|(_, r)| int(&r[0])).collect::<Result<_, _>>()?;
    let ops: [i64; 3] = ops.try_into().map_err(|_| bad("not three operators"))?;
    let kinds = nested(facts, "prepared_set_kind")?;
    let mut sets = Vec::new();
    for (k, r) in indexed(facts, "prepared_set")? {
        let members = kinds.get(&k).into_iter().flatten().map(|v| int(&v[0])).collect::<Result<_, _>>()?;
        sets.push((text(&r[0])?, members));
    }
    let list = |rel: &str| -> Result<Vec<String>, String> { indexed(facts, rel)?.iter().map(|(_, r)| text(&r[0])).collect() };
    // A program kept before decision 0106 has no matchers.
    let matchers = if facts.contains_key("prepared_matcher") {
        indexed(facts, "prepared_matcher")?
            .iter()
            .map(|(_, r)| {
                // Captured names (decision 0107): none in a program kept before.
                let captures = r.get(2).map(text).transpose()?.filter(|c| !c.is_empty()).map(|c| c.split(',').map(str::to_owned).collect()).unwrap_or_default();
                Ok((text(&r[0])?, text(&r[1])?, captures))
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        Vec::new()
    };
    let ok = facts.get("prepared_located").and_then(|r| r.first()).map(|r| int(&r[0])).transpose()?.ok_or_else(|| bad("no located"))?;
    let located_rels = list("prepared_located_rel")?;
    let located = if ok == 1 { Ok(located_rels) } else { Err(located_rels) };
    let merged = facts.get("prepared_merged").and_then(|r| r.first()).ok_or_else(|| bad("no merged"))?;
    let merged = (int(&merged[0])? as usize, int(&merged[1])? as usize);
    let atoms = nested(facts, "prepared_interned_atom")?;
    let mut interned = Vec::new();
    for (k, r) in indexed(facts, "prepared_interned")? {
        let rows = atoms.get(&k).cloned().unwrap_or_default();
        if rows.len() as i64 != int(&r[0])? {
            return Err(bad(format!("intern call {k} has {} of its {} strings", rows.len(), int(&r[0])?)));
        }
        let strings = rows.iter().map(|v| text(&v[0])).collect::<Result<Vec<_>, _>>()?;
        let ids = rows.iter().map(|v| int(&v[1]).map(|a| a as u32)).collect::<Result<Vec<_>, _>>()?;
        interned.push((strings, ids));
    }
    let patterns = nested(facts, "prepared_test_pattern")?;
    let (mut relations, mut specs) = (Vec::new(), Vec::new());
    for (k, r) in indexed(facts, "prepared_test")? {
        relations.push(text(&r[0])?);
        specs.push((text(&r[1])?, patterns.get(&k).into_iter().flatten().map(|v| text(&v[0])).collect::<Result<Vec<_>, _>>()?));
    }
    let vocabulary = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the text tests' vocabulary".into());
        let mut vocabulary = Vocabulary::from_specs(&relations, specs)?;
        vocabulary.matches = indexed(facts, "prepared_match")?
            .iter()
            .map(|(_, r)| regex::Regex::new(&text(&r[0])?).map(Arc::new).map_err(bad))
            .collect::<Result<_, String>>()?;
        vocabulary.markers = list("prepared_marker")?;
        vocabulary
    };
    let split = match facts.get("prepared_split").is_some_and(|r| !r.is_empty()) {
        true => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "the split's parts, of the lowered program and what the split found".into());
            let columns = |rel: &str| -> Result<BTreeMap<String, Vec<usize>>, String> {
                let mut by: BTreeMap<String, Vec<(i64, usize)>> = BTreeMap::new();
                for r in facts.get(rel).into_iter().flatten() {
                    let at = by.entry(text(&r[0])?).or_default();
                    if int(&r[1])? >= 0 {
                        at.push((int(&r[1])?, int(&r[2])? as usize));
                    }
                }
                Ok(by.into_iter().map(|(rel, mut cs)| {
                    cs.sort_by_key(|(i, _)| *i);
                    (rel, cs.into_iter().map(|(_, c)| c).collect())
                }).collect())
            };
            let set = |rel: &str| -> Result<std::collections::BTreeSet<String>, String> { Ok(list(rel)?.into_iter().collect()) };
            crate::code_rule_dsl::typing::file_local::assemble(&program, set("prepared_file_local")?, set("prepared_constant")?, columns("prepared_location")?, columns("prepared_same_doc")?)
        }
        false => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "split the lowered program".into());
            crate::code_rule_dsl::typing::file_local::programs(&program)
        }
    };
    Ok(Prepared {
        prog,
        vocabulary: Arc::new(vocabulary),
        lowered: lower::Lowered { program, domains, ops, dialect },
        sets,
        matchers,
        demanded: list("prepared_demanded")?,
        pushed: list("prepared_pushed")?,
        factorized: list("prepared_factorized")?,
        located,
        merged,
        split,
        interned,
    })
}

fn invalid(e: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(e.to_string())
}

/// The relation naming each kept program's text: (name, sha256 hex).
const PROGRAM: &str = "prepared_program";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

/// The program `key` names, prepared for `dialect`, as an earlier process stored it under `source`; `None` when none
/// did, or when what it stored no longer reads. With it, the sources of its programs' texts.
pub(super) fn lookup(store: &ContentStore, source: SourceId, dialect: GrammarDialect, key: &[u8; 32]) -> StoreResult<Option<(Prepared, Vec<SourceId>)>> {
    super::memo::off();
    let start = std::time::Instant::now();
    let set = pointer_plan(key);
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: POINTER, path: PATH })?;
    let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(None) };
    let Ok(mut facts) = program_facts::decode(&result.entry) else { return Ok(None) };
    let mut texts: BTreeMap<String, [u8; 32]> = BTreeMap::new();
    for row in facts.remove(PROGRAM).unwrap_or_default() {
        let [Value::Str(name), Value::Str(digest)] = row.as_slice() else { return Ok(None) };
        let Some(digest) = unhex(digest) else { return Ok(None) };
        texts.insert(name.clone(), digest);
    }
    let (Some(front), Some(lowered)) = (texts.get("front"), texts.get("lowered")) else { return Ok(None) };
    let mut documents = Vec::new();
    let mut programs = Vec::new();
    for digest in [front, lowered] {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "read a program's text".into());
        let Some((program, document)) = program_text::load(store, digest)? else { return Ok(None) };
        programs.push(program);
        documents.push(document);
    }
    let read = start.elapsed();
    let at = std::time::Instant::now();
    let lowered = programs.pop().expect("two");
    let front = programs.pop().expect("two");
    let got = prepared_of(&facts, front, lowered, dialect);
    if std::env::var_os("RERAIL_PREPARED_TRACE").is_some() {
        eprintln!("RERAIL_PREPARED_TRACE lookup ({}): {:.1} kB of facts; read {:.3} s, to a prepared program {:.3} s{}",
            super::phases::current().unwrap_or_else(|| "main".into()), result.entry.len() as f64 / 1e3, read.as_secs_f64(), at.elapsed().as_secs_f64(),
            got.as_ref().err().map(|e| format!("; not read: {e}")).unwrap_or_default());
    }
    Ok(got.ok().map(|p| (p, documents)))
}

/// What the write of the program `key` is queued under (`write_back`, decision 0102).
pub(super) fn queued(key: &[u8; 32]) -> crate::content_store::write_back::Key {
    crate::content_store::write_back::Key::of(&[b"facts_memo/prepared", key])
}

/// Stores `prepared` as the program `key` names under `source`: its two programs' texts, and the pointer to them with
/// what preparing found; the texts' sources, which the run's manifest names so collection keeps them. `None` when a
/// program does not read back from its text: it is not kept, and the next run prepares again.
pub(super) fn remember(store: &ContentStore, source: SourceId, key: &[u8; 32], prepared: &Prepared) -> StoreResult<Option<Vec<SourceId>>> {
    super::memo::off();
    let mut facts = rest_of(prepared);
    let mut documents = Vec::new();
    for (name, prog) in [("front", &prepared.prog), ("lowered", &prepared.lowered.program)] {
        let Some((digest, document)) = program_text::store(store, prog)? else { return Ok(None) };
        facts.entry(PROGRAM.to_owned()).or_default().push(vec![Value::Str(name.to_owned()), Value::Str(hex(&digest))]);
        documents.push(document);
    }
    let entry = program_facts::encode(&facts);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, entry.len() as u64);
    let set = pointer_plan(key);
    store.put_facts_memo(FactsMemoKey { source, fact_schema: POINTER, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry }], &[])?;
    Ok(Some(documents))
}

/// The compiled groups' schema (decision 0075).
const DFAS: u32 = crate::content_store::live_fact_schema(103);

/// The path a compiled group is kept under (read by no path, as the other entries).
const DFA_PATH: &str = "rerail/dfa";

/// The plan a compiled group is kept under: what its key does not already name (the key names the engine, its version
/// and the patterns).
fn dfa_plan() -> FactsPlanSet {
    FactsPlanSet::new(vec![Sha256::digest(b"rerail.dfa-entry/v1").into()])
}

/// Compiled groups of automata in the facts memo (decision 0075): each under its key, stored as a source, so a later
/// process reads a group's DFA instead of determinizing it. An entry is a tag (1: a DFA follows; 0: the group does
/// not determinize within the limit) and the DFA's bytes.
/// Its second field lists the sources of every group read or stored, for the run's manifest: no other root holds them.
pub(super) struct StoredDfas<'a>(pub(super) &'a ContentStore, pub(super) std::sync::Arc<std::sync::Mutex<Vec<SourceId>>>);

impl crate::code_rule_dsl::text::vocabulary::DfaCache for StoredDfas<'_> {
    fn get(&self, key: &str) -> Option<Option<Vec<u8>>> {
        let source = super::memo::stored_source(self.0, key).ok()??;
        self.1.lock().expect("the groups' sources").push(source);
        let memo = self.0.facts_memo(FactsMemoKey { source, fact_schema: DFAS, path: DFA_PATH }).ok()?;
        match memo.results.get(&dfa_plan().plans[0])?.entry.split_first() {
            Some((1, bytes)) => Some(Some(bytes.to_vec())),
            Some((0, _)) => Some(None),
            _ => None,
        }
    }

    fn put(&self, key: &str, dfa: Option<&[u8]>) {
        let mut entry = vec![u8::from(dfa.is_some())];
        entry.extend_from_slice(dfa.unwrap_or_default());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, entry.len() as u64);
        // Stored by the store's writer, behind the run (decision 0104); its source reaches the manifest, queued after.
        // A group not kept is compiled again by the next process.
        let (key, sources) = (key.to_owned(), std::sync::Arc::clone(&self.1));
        self.0.write_back(
            "a group of automata",
            entry.len() + key.len(),
            Vec::new(),
            Box::new(move |store| {
                let source = super::memo::source(store, &key)?;
                let set = dfa_plan();
                store.put_facts_memo(FactsMemoKey { source, fact_schema: DFAS, path: DFA_PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry: entry.clone() }], &[])?;
                sources.lock().expect("the groups' sources").push(source);
                Ok(())
            }),
        );
    }
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).

// Its tests run in the engine's test binary (its facade of this module mounts `identity_tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
