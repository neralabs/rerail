//! A program as a run prepares it, kept in the store across processes (decisions 0073, 0155): pruned, simplified,
//! demanded, lowered, its joins pushed, located and split.
//!
//! - **Programs.** Its two programs (`source`, the program as demanded; `lowered`, as lowered) are each kept as their
//!   `.rr` text (`program_text`), a source of the store by its sha256, and read back by the rr parser straight into a
//!   program: nothing is lowered or joined again.
//! - **Pointer.** One facts-memo entry (decision 0052) under the program text's source, for the run's plan: what
//!   preparing found (`prepared_*` relations, small: per relation or per intern call, not per node) and the two
//!   programs' digests (`prepared_program`). A run reads the pointer and the two texts; when any is gone it prepares.
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

/// The pointer's schema: bumped when its meaning changes (105: the programs as their text, decision 0155; 106: the
/// decisions preparing made, decision 0269).
const POINTER: u32 = crate::content_store::live_fact_schema(106);

/// The path a lookup names. The entries read no path (`path_dependent: false`); the store keeps them under none.
const PATH: &str = "rerail/prepared";

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
        // The split (`file_local::programs_of`): what it found, the file-local relations and those of constants. Its two
        // parts are the lowered program's rules, a part each, made from it again on reading (`file_local::assemble`).
        // The per-file memo's strata of the file-local part (`memo::Strata`, decision 0246 §4): the part's digest,
        // each relation's strata identity and the leaves it reaches.
        ("prepared_memo_part", &["digest:sym"]),
        ("prepared_memo_stratum", &["k:int", "rel:sym", "id:sym"]),
        ("prepared_memo_reach", &["k:int", "i:int", "rel:sym"]),
        ("prepared_split", &["ok:int"]),
        ("prepared_file_local", &["k:int", "rel:sym"]),
        ("prepared_constant", &["k:int", "rel:sym"]),
        ("prepared_location", &["rel:sym", "i:int", "col:int"]),
        ("prepared_same_doc", &["rel:sym", "i:int", "col:int"]),
        // The decisions preparing made (decision 0269): a run that reuses the program states them.
        ("prepared_decided", &["k:int", "topic:sym", "text:sym", "count:int"]),
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
    let s = |x: &str| Value::Str(x.to_owned().into());
    let n = |x: usize| Value::Int(x as i64);
    let mut row = |rel: &str, values: Vec<Value>| facts.entry(rel.to_owned()).or_default().push(values.into());
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
    // The per-file memo's strata (decision 0246 §4), with the digest of the file-local part they are of.
    if let Some((digest, strata)) = prepared.memo_strata.get() {
        row("prepared_memo_part", vec![s(&hex(digest))]);
        for (k, (rel, (id, reached))) in strata.by_rel.iter().enumerate() {
            row("prepared_memo_stratum", vec![n(k), s(rel), s(&hex(id))]);
            for (i, r) in reached.iter().enumerate() {
                row("prepared_memo_reach", vec![n(k), n(i), s(r)]);
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

/// The prepared program of `facts` (what preparing found, `rest_of`), its source's version (the program before
/// lowering) and its lowered program's version, for `dialect`.
pub fn prepared_of(
    facts: &Facts,
    source: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    dialect: GrammarDialect,
) -> Result<Prepared, String> {
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
            crate::code_rule_dsl::typing::file_local::assemble(version.clone(), set("prepared_file_local")?, set("prepared_constant")?, columns("prepared_location")?, columns("prepared_same_doc")?)?
        }
        false => {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "split the lowered program".into());
            crate::code_rule_dsl::typing::file_local::programs_of(version.clone())?
        }
    };
    // The memo's strata, when the program was kept with them; else the first run makes them.
    let memo_strata = std::sync::OnceLock::new();
    if let Some(part) = facts.get("prepared_memo_part").and_then(|rows| rows.first()) {
        let digest = unhex(&text(&part[0])?).ok_or_else(|| bad("a memo part's digest".to_owned()))?;
        let mut reach: BTreeMap<i64, Vec<(i64, String)>> = BTreeMap::new();
        for r in facts.get("prepared_memo_reach").into_iter().flatten() {
            reach.entry(int(&r[0])?).or_default().push((int(&r[1])?, text(&r[2])?));
        }
        let mut by_rel = BTreeMap::new();
        for r in facts.get("prepared_memo_stratum").into_iter().flatten() {
            let k = int(&r[0])?;
            let id = unhex(&text(&r[2])?).ok_or_else(|| bad("a memo stratum's identity".to_owned()))?;
            let mut reached = reach.remove(&k).unwrap_or_default();
            reached.sort();
            by_rel.insert(text(&r[1])?, (id, reached.into_iter().map(|(_, r)| r).collect()));
        }
        let _ = memo_strata.set((digest, super::memo::Strata { by_rel }));
    }
    Ok(Prepared {
        source,
        vocabulary: Arc::new(vocabulary),
        // Its stored image alone: the tree read is the split's, made above, and dropped.
        lowered: lower::Lowered { version, maint: None, domains, ops, dialect },
        sets,
        matchers,
        demanded: list("prepared_demanded")?,
        pushed: list("prepared_pushed")?,
        factorized: list("prepared_factorized")?,
        located,
        merged,
        split,
        interned,
        documents: Default::default(),
        cone: Default::default(),
        maintained: Default::default(),
        parts: Default::default(),
        decided: indexed(facts, "prepared_decided")?.iter().map(|(_, r)| Ok(((text(&r[0])?, text(&r[1])?), int(&r[2])? as u64))).collect::<Result<_, String>>()?,
        memo_strata,
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
    super::memo::off();
    let start = std::time::Instant::now();
    let set = pointer_plan(key);
    let kept = match source {
        Some(source) => store.facts_memo(FactsMemoKey { source, fact_schema: POINTER, path: PATH })?.results.remove(&set.plans[0]).map(|r| r.entry),
        None => None,
    };
    let entry: std::borrow::Cow<'static, [u8]> = match kept {
        Some(entry) => entry.into(),
        None => match crate::code_rule_dsl::logic::rr::images::shipped().and_then(|b| b.get(shipped_key(key))) {
            Some(bytes) => {
                crate::code_rule_dsl::prof::count("corpus.prepared_shipped", 1);
                bytes.into()
            }
            None => return Ok(None),
        },
    };
    let Ok(mut facts) = program_facts::decode(&entry) else { return Ok(None) };
    let mut texts: BTreeMap<String, [u8; 32]> = BTreeMap::new();
    for row in facts.remove(PROGRAM).unwrap_or_default() {
        let [Value::Str(name), Value::Str(digest)] = row.as_slice() else { return Ok(None) };
        let Some(digest) = unhex(digest) else { return Ok(None) };
        texts.insert(name.to_string(), digest);
    }
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
    let found = match relocate(store, &mut facts, &version) {
        Some(None) => Found::Kept,
        Some(Some(moved)) => {
            version = moved;
            crate::code_rule_dsl::prof::count("corpus.prepared_moved", 1);
            Found::Moved
        }
        None => return Ok(None),
    };
    let got = prepared_of(&facts, source, version, dialect);
    if std::env::var_os("RERAIL_PREPARED_TRACE").is_some() {
        eprintln!("RERAIL_PREPARED_TRACE lookup ({}): {:.1} kB of facts; read {:.3} s, to a prepared program {:.3} s{}",
            super::phases::current().unwrap_or_else(|| "main".into()), entry.len() as f64 / 1e3, read.as_secs_f64(), at.elapsed().as_secs_f64(),
            got.as_ref().err().map(|e| format!("; not read: {e}")).unwrap_or_default());
    }
    Ok(got.ok().map(|p| (p, documents, found)))
}

/// Moves the lowered program's name atoms, and the pointer's (`prepared_op`, `prepared_interned_atom`), to `store`'s
/// (decision 0270): its strings interned here in one call. `Some(None)` when they are already this store's,
/// `Some(version)` of the moved program when they moved; `None` when the program holds a name atom no intern call
/// made, or the store refuses.
pub fn relocate(
    store: &ContentStore,
    facts: &mut Facts,
    version: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
) -> Option<Option<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>>> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "the prepared program's atoms, in this store".into());
    let rows = facts.get_mut("prepared_interned_atom")?;
    let strings: Vec<&str> = rows.iter().map(|r| match r.get(2) { Some(Value::Str(x)) => Some(x.as_str()), _ => None }).collect::<Option<_>>()?;
    let old: Vec<u32> = rows.iter().map(|r| r.get(3).and_then(Value::integer).and_then(|a| u32::try_from(a).ok())).collect::<Option<_>>()?;
    let new: Vec<u32> = store.intern_atoms(&strings).ok()?.into_iter().map(u32::from).collect();
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
    for (row, &a) in rows.iter_mut().zip(&new) {
        let mut cells: Vec<Value> = row.to_vec();
        cells[3] = Value::Int(i64::from(a));
        *row = cells.into();
    }
    for row in facts.get_mut("prepared_op")? {
        let mut cells: Vec<Value> = row.to_vec();
        let a = u32::try_from(cells.get(1)?.integer()?).ok()?;
        cells[1] = Value::Int(i64::from(*to.get(&a)?));
        *row = cells.into();
    }
    Some(Some(moved))
}

/// The sources of the programs' texts the program `key` names under `source` (`lookup`'s), with neither program read:
/// for a run that holds the prepared program in memory. Empty when the store keeps none.
pub(super) fn documents(store: &ContentStore, source: SourceId, key: &[u8; 32]) -> StoreResult<Vec<SourceId>> {
    let set = pointer_plan(key);
    let memo = store.facts_memo(FactsMemoKey { source, fact_schema: POINTER, path: PATH })?;
    let Some(result) = memo.results.get(&set.plans[0]) else { return Ok(Vec::new()) };
    let Ok(mut facts) = program_facts::decode(&result.entry) else { return Ok(Vec::new()) };
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
    super::memo::off();
    let mut facts = rest_of(prepared);
    let mut programs = Vec::new();
    for (name, version) in [("source", &prepared.source), ("lowered", &prepared.lowered.version)] {
        // Each program's image is its version's, taken as it is (decision 0281). One whose image is not made is not
        // kept: the program is prepared again.
        let Some(encoded) = program_text::encode_version(version) else { return };
        facts.entry(PROGRAM.to_owned()).or_default().push([Value::Str(name.to_owned().into()), Value::Str(hex(&encoded.digest).into())].into());
        programs.push(encoded);
    }
    let entry = program_facts::encode(&facts);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, entry.len() as u64);
    // A run building the engine's bundle (`RERAIL_EXPORT_IMAGES`, decision 0264) ships the pointer; its programs'
    // images ship with `program_text::encode_version`.
    crate::code_rule_dsl::logic::rr::images::export(shipped_key(key), &entry);
    let (packed, bytes) = program_text::pack(programs, entry);
    store.record("the prepared program", vec![queued(key)], &(source, text, *key, packed), Some(bytes), apply);
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
    let (documents, entry) = program_text::store_packed(cx.store, &packed, cx.bytes)?;
    cx.name(documents);
    let set = pointer_plan(&key);
    cx.store.put_facts_memo(FactsMemoKey { source, fact_schema: POINTER, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry: entry.to_vec() }], &[]).map(drop)
}

// Its tests run in the engine's test binary (its facade of this module mounts `tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).

// Its tests run in the engine's test binary (its facade of this module mounts `identity_tests.rs`): they read what the
// engine registers as it loads (the languages, their `.rr` libraries).
