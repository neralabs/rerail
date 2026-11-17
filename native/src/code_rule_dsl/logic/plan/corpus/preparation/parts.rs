//! The passes before lowering run per part, each part a program of its own against its interface (decision 0345):
//! the second answer to work queue Q18, beside 0333's stored runs. `RERAIL_PREPARE=parts` selects it.
//!
//! - **Parts.** Each rule of the combined program belongs to the program (`p<i>_`) or request (`q<k>_`) whose
//!   relations it reads or makes, rules that join two of them joining their parts; the rest are the library's.
//! - **Interface.** A part's inputs are the program's and what it reads of other parts; its outputs are what other
//!   parts read of it and the program's outputs it makes. The passes keep both as they are (`simplify::kept`).
//! - **Kept.** A prepared part is stored under a key of its header, its rules' hashes, the engine build and the
//!   switches the passes read, made without building the part; a part found is read as it is.
//! - **Link.** The parts' rules and what each part's passes changed of the header. A name a pass made keeps its
//!   form: numbered families are renumbered past the earlier parts' (`strtest_<k>`, `matchall_<k>`, `rr_ast_<k>_<m>`),
//!   and only a name two parts made gets the part's suffix (`_x<j>`).

use super::{Front, text::Matchers};
use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::program_facts::{self, Facts};
use crate::code_rule_dsl::logic::program_image::{Builder, Rewrite, Version, AtomRef};
use crate::code_rule_dsl::logic::program_text;
use crate::code_rule_dsl::logic::strings::Vocabulary;
use crate::content_store::{ContentStore, FactsMemoKey, FactsPlanSet, FactsResult, HashKey, SourceId, StoreResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

type Result<T> = std::result::Result<T, String>;

/// Whether the run prepares by parts (`RERAIL_PREPARE=parts`).
pub fn on() -> bool {
    std::env::var("RERAIL_PREPARE").is_ok_and(|v| v == "parts")
}

/// The switches the passes before lowering read: a part kept under one value is not read under another.
const SWITCHES: &[&str] = &[
    "RERAIL_DEMAND",
    "RERAIL_PROJECT_COLUMNS",
    "RERAIL_EXISTENCE_COUNTS",
    "RERAIL_INLINE_FILTERS",
    "RERAIL_JOIN_KEYS",
    "RERAIL_AST_PARTS",
    "RERAIL_AST_PART_CAPTURES",
    "RERAIL_SAME",
    "RERAIL_NODE_SOURCES",
];

/// The entry's schema: bumped when its encoding or meaning changes.
const SCHEMA: u32 = crate::content_store::live_fact_schema(345);
const PATH: &str = "rerail/part";

/// The part a relation names by its prefix: `p<i>_` or `q<k>_`.
/// A counter-numbered relation a pass makes, `<stem><k><tail>`: its stem, number and tail (`rr_ast_<k>_<m>`'s
/// `_<m>`, which a reader parses).
fn numbered(rel: &str) -> Option<(&'static str, usize, &str)> {
    ["rr_ast_d", "rr_ast_", "rr_long_text", "rr_node_text"].into_iter().find_map(|stem| {
        let rest = rel.strip_prefix(stem)?;
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let (k, tail) = rest.split_at(digits);
        let tail_ok = tail.is_empty() || (stem == "rr_ast_" && tail.strip_prefix('_').is_some_and(|m| !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit())));
        if !tail_ok {
            return None;
        }
        Some((stem, k.parse().ok()?, tail))
    })
}

fn tag(rel: &str) -> Option<&str> {
    let (head, rest) = rel.split_once('_')?;
    let (kind, digits) = head.split_at(1);
    (matches!(kind, "p" | "q") && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty()).then_some(head)
}

/// A part of the combined program: its rules, by place in the version's rule order.
struct Part {
    name: String,
    rules: Vec<usize>,
    made: BTreeSet<String>,
    read: BTreeSet<String>,
}

/// The parts of `v` (decision 0345 §1).
fn parts_of(v: &Version) -> Vec<Part> {
    // Each rule's tags; rules joining two tags join their parts (union-find over the tags).
    let rules: Vec<_> = v.rules().collect();
    let mut parent: HashMap<String, String> = HashMap::new();
    fn find(parent: &mut HashMap<String, String>, t: &str) -> String {
        let mut x = t.to_owned();
        while let Some(p) = parent.get(&x).filter(|p| **p != x).cloned() {
            x = p;
        }
        parent.insert(t.to_owned(), x.clone());
        x
    }
    let mut tags_of: Vec<Vec<String>> = Vec::with_capacity(rules.len());
    for r in &rules {
        let mut tags: Vec<String> = std::iter::once(r.head()).chain(r.reads()).filter_map(|a| tag(v.rel_name(a.rel())).map(str::to_owned)).collect();
        tags.sort();
        tags.dedup();
        for t in &tags {
            parent.entry(t.clone()).or_insert_with(|| t.clone());
        }
        if let Some(first) = tags.first() {
            let a = find(&mut parent, first);
            for t in &tags[1..] {
                let b = find(&mut parent, t);
                if a != b {
                    parent.insert(b, a.clone());
                }
            }
        }
        tags_of.push(tags);
    }
    let mut by: BTreeMap<String, Part> = BTreeMap::new();
    for (i, (r, tags)) in rules.iter().zip(&tags_of).enumerate() {
        let name = match tags.first() {
            Some(t) => find(&mut parent, t),
            None => "l".to_owned(),
        };
        let part = by.entry(name.clone()).or_insert_with(|| Part { name, rules: Vec::new(), made: BTreeSet::new(), read: BTreeSet::new() });
        part.rules.push(i);
        part.made.insert(v.rel_name(r.head().rel()).to_owned());
        part.read.extend(r.reads().map(|a| v.rel_name(a.rel()).to_owned()));
    }
    by.into_values().collect()
}

/// A part's program: the whole header, its rules, its interface as inputs and outputs (decision 0345 §2).
/// What every part's header shares with the program's: everything but the per-relation fields a part restricts
/// (`part_header`), its rules, inputs and outputs. Made once a run, and hashed once into every part's key.
fn common_of(header: &front::Program) -> (front::Program, [u8; 32]) {
    use sha2::Digest as _;
    let common = front::Program {
        rules: Vec::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        decls: Default::default(),
        nullary: Default::default(),
        lattice: Default::default(),
        facts: Default::default(),
        qualifiers: Default::default(),
        tapes: Default::default(),
        demand: Vec::new(),
        ..header.clone()
    };
    let digest = sha2::Sha256::digest(format!("{common:?}").as_bytes()).into();
    (common, digest)
}

/// The relations every part's header names: the base relations (no part makes them: a pass may add a read of one)
/// and what natives read.
fn shared_of<'a>(header: &'a front::Program, made_by_parts: &BTreeSet<&str>) -> BTreeSet<&'a str> {
    let mut shared: BTreeSet<&str> = header.natives.values().flat_map(|n| n.reads()).collect();
    let named = header.decls.keys().map(|r| r.as_ref()).chain(header.nullary.iter().map(String::as_str)).chain(header.lattice.keys().map(String::as_str)).chain(header.facts.keys().map(String::as_str));
    shared.extend(named.chain(header.qualifiers.keys().map(String::as_str)).chain(header.tapes.keys().map(String::as_str)).chain(header.demand.iter().map(String::as_str)).filter(|r| !made_by_parts.contains(r)));
    shared
}

/// A part's own header (decision 0345 §2): the declarations, facts and lists of the relations it makes and reads
/// and the `shared` ones, and its inputs and outputs; the fields every part shares (`common_of`) are not in it.
/// Another part's relations are not its interface, and a change of one does not change its key.
fn part_header(header: &front::Program, part: &Part, read_elsewhere: &BTreeSet<String>, made_by_parts: &BTreeSet<&str>, shared: &BTreeSet<&str>) -> front::Program {
    let names: BTreeSet<&str> = part.made.iter().chain(&part.read).map(String::as_str).chain(shared.iter().copied()).collect();
    macro_rules! pick {
        ($field:ident) => {
            names.iter().filter_map(|r| header.$field.get_key_value(*r).map(|(k, v)| (k.clone(), v.clone()))).collect()
        };
    }
    let mut head = front::Program::default();
    head.decls = pick!(decls);
    head.lattice = pick!(lattice);
    head.facts = pick!(facts);
    head.qualifiers = pick!(qualifiers);
    head.tapes = pick!(tapes);
    head.nullary = names.iter().filter(|r| header.nullary.contains(**r)).map(|r| r.to_string()).collect();
    head.demand = header.demand.iter().filter(|r| names.contains(r.as_str())).cloned().collect();
    head.outputs = part.made.iter().filter(|r| header.outputs.contains(*r) || read_elsewhere.contains(*r)).cloned().collect();
    // Its inputs: the program's, and what it reads that another part makes. A relation no part makes (a base
    // relation, a request the runner answers: `dir_entry`, `loaded`) stays as the whole program has it, for the
    // part's own passes to treat as the whole's do.
    let mut inputs = header.inputs.clone();
    for r in part.read.iter().filter(|r| !part.made.contains(*r)) {
        if made_by_parts.contains(r.as_str()) && header.decls.contains_key(r.as_str()) && !inputs.contains(r) {
            inputs.push(r.clone());
        }
    }
    head.inputs = inputs;
    head
}

/// A part's key (decision 0345 §5), without building it: the engine, the dialect, the switches its passes read, the
/// shared header's digest (`common_of`), its own header's per-relation fields, and its rules' hashes in order (a
/// rule's hash is its text's: its relations by name).
fn key_of(head: &front::Program, common: &[u8; 32], rules: &[crate::code_rule_dsl::logic::program_image::RuleRef<'_>], part: &Part, dialect: crate::code_rule_dsl::language::GrammarDialect) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/part/v3\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update(format!("\0{dialect:?}\0").as_bytes());
    for s in SWITCHES {
        h.update(format!("{s}={}\0", std::env::var(s).unwrap_or_default()).as_bytes());
    }
    h.update(common);
    let own = (&head.decls, &head.nullary, &head.lattice, &head.facts, &head.qualifiers, &head.tapes, &head.demand, &head.inputs, &head.outputs);
    h.update(format!("{own:?}").as_bytes());
    h.update((part.rules.len() as u64).to_le_bytes());
    for &i in &part.rules {
        h.update(rules[i].hash().to_le_bytes());
    }
    h.finalize().into()
}

/// A part's program: its own header on the shared fields (`common`), and its rules, copied from the combined program.
fn program_of(v: &Version, rules: &[crate::code_rule_dsl::logic::program_image::RuleRef<'_>], own: &front::Program, common: &front::Program, part: &Part) -> Result<Arc<Version>> {
    let mut head = common.clone();
    head.decls = own.decls.clone();
    head.nullary = own.nullary.clone();
    head.lattice = own.lattice.clone();
    head.facts = own.facts.clone();
    head.qualifiers = own.qualifiers.clone();
    head.tapes = own.tapes.clone();
    head.demand = own.demand.clone();
    head.inputs = own.inputs.clone();
    head.outputs = own.outputs.clone();
    let mut b = Builder::of_program(&head)?;
    for &i in &part.rules {
        b.copy(v, rules[i]);
    }
    Ok(b.finish()?.0)
}

/// A prepared part as it is kept: its rules, its own header, and of the shared fields only those its passes
/// changed (each then whole). A field the passes left as `common` has it is the link's already, so neither the store
/// nor the link holds a copy of it per part.
fn stripped(v: &Arc<Version>, common: &front::Program, lists: &front::Lists) -> Result<Arc<Version>> {
    let front::Program {
        decls, nullary, lattice, facts, rules, inputs, outputs, kindsets, demand, demand_dropped, magic_extras, magic_report, gates, policies, settings, natives, rx_spans, ast_patterns, doc_scopes, answers, text_globs, qualifiers, tapes, again, together, asked_by, shapes,
    } = v.header(lists)?;
    macro_rules! changed {
        ($field:ident) => {
            if format!("{:?}", $field) == format!("{:?}", common.$field) { Default::default() } else { $field }
        };
    }
    let head = front::Program {
        decls,
        nullary,
        lattice,
        facts,
        rules,
        inputs,
        outputs,
        demand,
        qualifiers,
        tapes,
        kindsets: changed!(kindsets),
        demand_dropped: changed!(demand_dropped),
        magic_extras: changed!(magic_extras),
        magic_report: changed!(magic_report),
        gates: changed!(gates),
        policies: changed!(policies),
        settings: changed!(settings),
        natives: changed!(natives),
        rx_spans: changed!(rx_spans),
        ast_patterns: changed!(ast_patterns),
        doc_scopes: changed!(doc_scopes),
        answers: changed!(answers),
        text_globs: changed!(text_globs),
        again: changed!(again),
        together: changed!(together),
        asked_by: changed!(asked_by),
        shapes: changed!(shapes),
    };
    let mut b = Builder::of_program(&head)?;
    for r in v.rules() {
        b.copy(v, r);
    }
    Ok(b.finish()?.0)
}

/// What a part's front found, by its own names.
struct Prepared {
    version: Arc<Version>,
    reaches: (usize, usize),
    demanded: Vec<String>,
    demand: Option<(Vec<String>, std::result::Result<front::ast::Report, String>)>,
    undone: Vec<String>,
    matchers: Matchers,
    tests: Vec<String>,
    specs: Vec<(String, Vec<String>)>,
    matches: Vec<String>,
    markers: Vec<String>,
}

impl Prepared {
    fn of(version: Arc<Version>, f: Front) -> Prepared {
        Prepared {
            version,
            reaches: f.reaches,
            demanded: f.demanded,
            demand: f.demand,
            undone: f.undone,
            matchers: f.matchers,
            tests: f.vocabulary.tests.iter().map(|(r, _)| r.clone()).collect(),
            specs: f.vocabulary.specs.clone(),
            matches: f.vocabulary.matches.iter().map(|m| m.as_str().to_owned()).collect(),
            markers: f.vocabulary.markers.clone(),
        }
    }
}

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

/// The key a part's prepared form is kept under (decision 0345 §5).
fn source_text(key: &[u8; 32]) -> String {
    format!("rerail/part\0{}", hex(key))
}

fn plan() -> FactsPlanSet {
    FactsPlanSet::new(vec![HashKey::sha256(b"rerail/part plan v1").sha256_bytes().expect("a sha256 key")])
}

fn str(v: &str) -> Value {
    Value::Str(v.to_owned().into())
}

fn rows_of<'a>(facts: &'a Facts, rel: &str) -> impl Iterator<Item = Vec<String>> + 'a {
    facts.get(rel).into_iter().flatten().map(|r| r.iter().map(|c| match c {
        Value::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }).collect())
}

/// What the part's entry holds besides its image.
fn facts_of(p: &Prepared, digest: &[u8; 32]) -> Facts {
    let mut f = Facts::new();
    let mut put = |rel: &str, rows: Vec<Vec<String>>| {
        f.insert(rel.to_owned(), rows.into_iter().map(|r| r.iter().map(|c| str(c)).collect::<Vec<_>>().into()).collect());
    };
    let n = |x: usize| x.to_string();
    put("part_program", vec![vec![hex(digest)]]);
    put("part_reach", vec![vec![n(p.reaches.0), n(p.reaches.1)]]);
    put("part_demanded", p.demanded.iter().enumerate().map(|(i, r)| vec![n(i), r.clone()]).collect());
    put("part_undone", p.undone.iter().enumerate().map(|(i, r)| vec![n(i), r.clone()]).collect());
    put("part_matcher", p.matchers.iter().enumerate().map(|(i, (r, pat, caps))| vec![n(i), r.clone(), pat.clone(), caps.join("\u{1f}")]).collect());
    put("part_test", p.tests.iter().zip(&p.specs).enumerate().map(|(i, (r, (op, pats)))| vec![n(i), r.clone(), op.clone(), pats.join("\u{1f}")]).collect());
    put("part_match", p.matches.iter().enumerate().map(|(i, m)| vec![n(i), m.clone()]).collect());
    put("part_marker", p.markers.iter().enumerate().map(|(i, m)| vec![n(i), m.clone()]).collect());
    if let Some((listed, outcome)) = &p.demand {
        put("part_demand_listed", listed.iter().enumerate().map(|(i, r)| vec![n(i), r.clone()]).collect());
        let row = match outcome {
            Ok(r) => vec!["ok".to_owned(), r.adornments.join("\u{1f}"), n(r.magic_rules), r.full.join("\u{1f}"), n(r.extras_kept), n(r.gates), r.full_for_negation.join("\u{1f}")],
            Err(e) => vec!["err".to_owned(), e.clone()],
        };
        put("part_demand", vec![row]);
    }
    f
}

/// Rows in their first column's order.
fn ordered(facts: &Facts, rel: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = rows_of(facts, rel).collect();
    rows.sort_by_key(|r| r.first().and_then(|i| i.parse::<usize>().ok()).unwrap_or(0));
    rows
}

fn split(s: &str) -> Vec<String> {
    if s.is_empty() { Vec::new() } else { s.split('\u{1f}').map(str::to_owned).collect() }
}

/// The part kept under `key`, with the sources its image is kept as.
fn lookup(store: &ContentStore, key: &[u8; 32], lists: &front::Lists) -> StoreResult<Option<(Prepared, Vec<SourceId>)>> {
    let Some(source) = store.source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? else { return Ok(None) };
    let set = plan();
    let Some(result) = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?.results.remove(&set.plans[0]) else { return Ok(None) };
    let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(None) };
    let Some(digest) = rows_of(&facts, "part_program").next().and_then(|r| r.first().and_then(|d| unhex(d))) else { return Ok(None) };
    let s_image = crate::code_rule_dsl::logic::profile::span(|| "a part, from the store: its image".into());
    let Some((version, mut sources)) = program_text::load_image(store, &digest, lists)? else { return Ok(None) };
    drop(s_image);
    sources.push(source);
    let num = |s: &str| s.parse::<usize>().unwrap_or(0);
    let reach = rows_of(&facts, "part_reach").next().unwrap_or_default();
    let tests = ordered(&facts, "part_test");
    let demand = facts.get("part_demand").map(|_| {
        let listed = ordered(&facts, "part_demand_listed").into_iter().map(|r| r[1].clone()).collect();
        let row = rows_of(&facts, "part_demand").next().unwrap_or_default();
        let outcome = if row.first().map(String::as_str) == Some("ok") && row.len() >= 7 {
            Ok(front::ast::Report { adornments: split(&row[1]), magic_rules: num(&row[2]), full: split(&row[3]), extras_kept: num(&row[4]), gates: num(&row[5]), full_for_negation: split(&row[6]) })
        } else {
            Err(row.get(1).cloned().unwrap_or_default())
        };
        (listed, outcome)
    });
    Ok(Some((
        Prepared {
            version,
            reaches: (reach.first().map_or(0, |s| num(s)), reach.get(1).map_or(0, |s| num(s))),
            demanded: ordered(&facts, "part_demanded").into_iter().map(|r| r[1].clone()).collect(),
            demand,
            undone: ordered(&facts, "part_undone").into_iter().map(|r| r[1].clone()).collect(),
            matchers: ordered(&facts, "part_matcher").into_iter().map(|r| (r[1].clone(), r[2].clone(), split(&r[3]))).collect(),
            tests: tests.iter().map(|r| r[1].clone()).collect(),
            specs: tests.iter().map(|r| (r[2].clone(), split(&r[3]))).collect(),
            matches: ordered(&facts, "part_match").into_iter().map(|r| r[1].clone()).collect(),
            markers: ordered(&facts, "part_marker").into_iter().map(|r| r[1].clone()).collect(),
        },
        sources,
    )))
}

/// Keeps the part under `key`, as an item of the run's record (decision 0306): its image and its entry, written by the
/// store's writer, their sources named by the run.
fn record(store: &ContentStore, key: &[u8; 32], p: &Prepared) {
    let Some(encoded) = program_text::encode_version(&p.version) else { return };
    let entry = program_facts::encode(&facts_of(p, &encoded.digest));
    let (packed, bytes) = program_text::pack(vec![encoded], entry);
    fn apply(cx: &crate::content_store::record::Apply<'_>, body: crate::content_store::record::codec::Body) -> StoreResult<()> {
        let (key, packed): ([u8; 32], program_text::Packed) = crate::content_store::record::codec::from_body("a prepared part", body)?;
        let (sources, entry) = program_text::store_packed(cx.store, &packed, cx.bytes)?;
        cx.name(sources);
        let source = super::super::memo::source(cx.store, &source_text(&key))?;
        cx.name([source]);
        let set = plan();
        cx.store.put_facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH }, &set, &[FactsResult { plan: set.plans[0], path_dependent: false, entry: entry.to_vec() }], &[]).map(drop)
    }
    store.record("a prepared part", Vec::new(), &(*key, packed), Some(bytes), apply);
}

/// Every relation name `p` mentions: its header's and its rules'.
fn names_of(v: &Version, h: &front::Program) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = h.decls.keys().map(|k| k.to_string()).collect();
    out.extend(h.inputs.iter().cloned());
    out.extend(h.outputs.iter().cloned());
    out.extend(h.facts.keys().cloned());
    out.extend(h.natives.keys().cloned());
    for r in v.rules() {
        out.insert(v.rel_name(r.head().rel()).to_owned());
        out.extend(r.reads().map(|a| v.rel_name(a.rel()).to_owned()));
    }
    out
}

/// Renames a part's relations in its rules (decision 0345 §4).
struct Renaming<'a> {
    v: &'a Version,
    to: &'a HashMap<String, String>,
}

impl Rewrite for Renaming<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        self.to.get(self.v.rel_name(a.rel())).cloned()
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        self.to.get(self.v.rel_name(a.rel())).map(|r| (r.clone(), a.neg(), a.gated()))
    }
}

/// What part `after`'s passes changed of its header `before`, applied to the link's `all` with its relations renamed
/// by `rename`: entries added or changed are set; entries of relations the part owns that its passes removed go.
fn merge_header(all: &mut front::Program, before: &front::Program, after: &front::Program, owned: &dyn Fn(&str) -> bool, rename: &dyn Fn(&str) -> String) {
    macro_rules! map {
        ($field:ident, $value:expr) => {
            for (k, v) in &after.$field {
                if before.$field.get(k) != Some(v) {
                    all.$field.insert(rename(k).into(), $value(v.clone()));
                }
            }
            for k in before.$field.keys() {
                if !after.$field.contains_key(k) && owned(k) {
                    all.$field.remove(k);
                }
            }
        };
    }
    map!(decls, |v| v);
    map!(lattice, |v| v);
    map!(facts, |v| v);
    map!(qualifiers, |v| v);
    map!(tapes, |v| v);
    map!(natives, |mut n: front::Native| {
        n.rename_reads(rename);
        n
    });
    map!(answers, |v: Vec<String>| v.iter().map(|r| rename(r)).collect::<Vec<_>>());
    map!(asked_by, |v: Vec<BTreeSet<String>>| v.into_iter().map(|s| s.iter().map(|r| rename(r)).collect()).collect::<Vec<_>>());
    for (k, v) in &after.kindsets {
        all.kindsets.entry(k.clone()).or_insert_with(|| v.clone());
    }
    macro_rules! names {
        ($field:ident) => {
            for r in &after.$field {
                let r2 = rename(r);
                if !before.$field.contains(r) && !all.$field.contains(&r2) {
                    all.$field.insert(r2);
                }
            }
            for r in &before.$field {
                if !after.$field.contains(r) && owned(r) {
                    all.$field.remove(r);
                }
            }
        };
    }
    names!(nullary);
    for r in &after.demand {
        let r2 = rename(r);
        if !before.demand.contains(r) && !all.demand.contains(&r2) {
            all.demand.push(r2);
        }
    }
    all.demand.retain(|r| !(before.demand.contains(r) && !after.demand.contains(r) && owned(r)));
    for x in &after.rx_spans {
        all.rx_spans.insert(x.clone());
    }
    for x in &after.ast_patterns {
        all.ast_patterns.insert(x.clone());
    }
    for (rel, g) in &after.gates {
        if !before.gates.iter().any(|(r, _)| r == rel) {
            all.gates.push((rename(rel), g.clone()));
        }
    }
    for d in &after.demand_dropped {
        if !before.demand_dropped.contains(d) && !all.demand_dropped.contains(d) {
            all.demand_dropped.push(d.clone());
        }
    }
    for s in &after.shapes {
        if !before.shapes.contains(s) && !all.shapes.contains(s) {
            all.shapes.push(s.clone());
        }
    }
    for s in &after.settings {
        if !before.settings.contains(s) && !all.settings.contains(s) {
            all.settings.push(s.clone());
        }
    }
    if after.magic_report != before.magic_report && after.magic_report.is_some() {
        all.magic_report = after.magic_report.clone();
    }
    if after.magic_extras != before.magic_extras {
        all.magic_extras = Some(all.magic_extras.unwrap_or(0) + after.magic_extras.unwrap_or(0));
    }
}

/// `preparation::front` per part (decision 0345): the linked version, what the passes found, and the sources of the
/// parts the store kept, for the run's manifest to name. `store` keeps the parts when `keep`.
pub fn front(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, store: &ContentStore, keep: bool, lists: &front::Lists) -> Result<(Arc<Version>, Front, Vec<SourceId>)> {
    let header = version.header(lists)?;
    let parts = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the parts and their interfaces (decision 0345)".into());
        parts_of(&version)
    };
    let rules: Vec<_> = version.rules().collect();
    // What each part reads, for the others' outputs.
    let mut readers: HashMap<&str, BTreeSet<usize>> = HashMap::new();
    for (j, p) in parts.iter().enumerate() {
        for r in &p.read {
            readers.entry(r.as_str()).or_default().insert(j);
        }
    }
    let made_by_parts: BTreeSet<&str> = parts.iter().flat_map(|p| p.made.iter().map(String::as_str)).collect();
    let (common, common_digest) = common_of(&header);
    let shared = shared_of(&header, &made_by_parts);
    let mut prepared: Vec<(front::Program, Prepared)> = Vec::with_capacity(parts.len());
    let mut sources = Vec::new();
    let (mut found, mut made) = (0usize, 0usize);
    for (j, part) in parts.iter().enumerate() {
        let read_elsewhere: BTreeSet<String> = part.made.iter().filter(|r| readers.get(r.as_str()).is_some_and(|js| js.iter().any(|&o| o != j))).cloned().collect();
        let head = part_header(&header, part, &read_elsewhere, &made_by_parts, &shared);
        let key = key_of(&head, &common_digest, &rules, part, dialect);
        let kept = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a part, from the store".into());
            lookup(store, &key, lists).map_err(|e| e.to_string())?
        };
        let p = match kept {
            Some((p, s)) => {
                found += 1;
                sources.extend(s);
                p
            }
            None => {
                made += 1;
                let _s = crate::code_rule_dsl::logic::profile::span(|| format!("prepare part {} ({} rules)", part.name, part.rules.len()));
                let input = program_of(&version, &rules, &head, &common, part)?;
                let (v, f, _) = crate::code_rule_dsl::logic::profile::quiet(|| super::front(input, dialect, None, &mut |_, _| {}))?;
                let v = stripped(&v, &common, lists)?;
                let p = Prepared::of(v, f);
                if keep {
                    record(store, &key, &p);
                }
                p
            }
        };
        prepared.push((head, p));
    }
    crate::code_rule_dsl::logic::profile::decision("parts prepared (decision 0345)", || format!("{} parts: {found} read from the store, {made} prepared", parts.len()));
    let _link = crate::code_rule_dsl::logic::profile::span(|| "link the prepared parts (decision 0345)".into());
    link(&version, &header, &parts, prepared, lists).map(|(v, f)| (v, f, sources))
}

/// The prepared parts linked (decision 0345 §6), with what their passes found, renamed.
fn link(version: &Version, header: &front::Program, parts: &[Part], prepared: Vec<(front::Program, Prepared)>, lists: &front::Lists) -> Result<(Arc<Version>, Front)> {
    let s_names = crate::code_rule_dsl::logic::profile::span(|| "link: the parts' names".into());
    let known = names_of(version, header);
    // A relation made in one part alone is that part's own: what its passes remove of it goes.
    let mut makers: HashMap<&str, usize> = HashMap::new();
    for p in parts {
        for r in &p.made {
            *makers.entry(r.as_str()).or_default() += 1;
        }
    }
    let mut all = header.clone();
    all.rules.clear();
    let mut renames: Vec<HashMap<String, String>> = Vec::new();
    let (mut reaches, mut demanded, mut undone, mut matchers) = ((0, 0), Vec::new(), Vec::new(), Vec::new());
    let mut demand: Option<(Vec<String>, std::result::Result<front::ast::Report, String>)> = None;
    let (mut tests, mut specs, mut matches, mut markers) = (Vec::new(), Vec::new(), Vec::new(), Vec::<String>::new());
    let mut added_inputs: BTreeSet<String> = BTreeSet::new();
    let mut new_inputs: HashMap<String, String> = HashMap::new();
    // The names each part's passes made, and how many parts made each.
    let afters: Vec<front::Program> = prepared.iter().map(|(_, p)| p.version.header(lists)).collect::<Result<_>>()?;
    let made_new: Vec<BTreeSet<String>> = prepared.iter().zip(&afters).map(|((_, p), after)| names_of(&p.version, after).into_iter().filter(|r| !known.contains(r)).collect()).collect();
    let mut times: HashMap<&str, usize> = HashMap::new();
    for names in &made_new {
        for r in names {
            *times.entry(r.as_str()).or_default() += 1;
        }
    }
    // The numbers each counter-numbered family has taken: the program's own, then each part's as it is linked.
    let mut taken: HashMap<&'static str, BTreeSet<usize>> = HashMap::new();
    for r in &known {
        if let Some((stem, k, _)) = numbered(r) {
            taken.entry(stem).or_default().insert(k);
        }
    }
    drop(s_names);
    let s_merge = crate::code_rule_dsl::logic::profile::span(|| "link: renames and headers".into());
    for ((part, (before, p)), j) in parts.iter().zip(&prepared).zip(0..) {
        let after = &afters[j];
        let derived: BTreeSet<String> = p.version.rules().map(|r| p.version.rel_name(r.head().rel()).to_owned()).collect();
        let renamed: BTreeSet<&str> = p.tests.iter().chain(p.matchers.iter().map(|m| &m.0)).map(String::as_str).collect();
        let mut to: HashMap<String, String> = HashMap::new();
        // A number another part (or the program) has taken is moved to the family's next free one, for every
        // relation of this part with that number (`rr_ast_<k>` and `rr_ast_<k>_<m>` are one matcher's).
        let mut moved: HashMap<(&'static str, usize), usize> = HashMap::new();
        let mut kept_here: Vec<(&'static str, usize)> = Vec::new();
        for r in &made_new[j] {
            // Numbered families are read by their name's form (`strtest_<k>` is test k, `matchall_<k>` regex k,
            // `rr_ast_<k>_<m>` has m captures): renumbered past the earlier parts'. A name with a part's tag is that
            // part's already. Any other name two parts made gets the part's suffix.
            if let Some(k) = r.strip_prefix("matchall_").and_then(|k| k.parse::<usize>().ok()) {
                to.insert(r.clone(), format!("matchall_{}", k + matches.len()));
            } else if let Some(k) = r.strip_prefix("strtest_").and_then(|k| k.parse::<usize>().ok()) {
                to.insert(r.clone(), format!("strtest_{}", k + tests.len()));
            } else if let Some((stem, k, tail)) = numbered(r) {
                let set = taken.entry(stem).or_default();
                let to_k = if let Some(&m) = moved.get(&(stem, k)) {
                    m
                } else if set.contains(&k) && !kept_here.contains(&(stem, k)) {
                    let m = set.iter().next_back().map_or(0, |x| x + 1);
                    set.insert(m);
                    moved.insert((stem, k), m);
                    m
                } else {
                    set.insert(k);
                    kept_here.push((stem, k));
                    k
                };
                if to_k != k {
                    to.insert(r.clone(), format!("{stem}{to_k}{tail}"));
                }
            } else if tag(r).is_some() {
                continue;
            } else if times.get(r.as_str()).copied().unwrap_or(0) > 1 && (derived.contains(r) || renamed.contains(r.as_str())) {
                to.insert(r.clone(), format!("{r}_x{j}"));
            } else if let Some(d) = after.decls.get(r.as_str()) {
                // A new input a pass declared (`span_within`): the run's, one name in every part.
                let shown = format!("{d:?}");
                if let Some(other) = new_inputs.insert(r.clone(), shown.clone())
                    && other != shown
                {
                    return Err(format!("parts declare {r} differently: {other} and {shown}"));
                }
            }
        }
        let rename = |r: &str| to.get(r).cloned().unwrap_or_else(|| r.to_owned());
        let owned = |r: &str| makers.get(r) == Some(&1) && part.made.contains(r) && !header.outputs.iter().any(|o| o == r);
        merge_header(&mut all, before, after, &owned, &rename);
        // Every input of the part: the program's, what its passes added, and what it reads of the others (dropped
        // below where another part makes it, kept where none does: a base relation, `doc`).
        added_inputs.extend(after.inputs.iter().map(|r| rename(r)));
        reaches = (reaches.0 + p.reaches.0, reaches.1 + p.reaches.1);
        demanded.extend(p.demanded.iter().map(|r| rename(r)));
        undone.extend(p.undone.iter().map(|r| rename(r)));
        matchers.extend(p.matchers.iter().map(|(r, pat, caps)| (rename(r), pat.clone(), caps.clone())));
        if let Some((listed, outcome)) = &p.demand {
            let listed: Vec<String> = listed.iter().map(|r| rename(r)).collect();
            demand = Some(match (demand.take(), outcome) {
                (None, o) => (listed, o.clone()),
                (Some((mut l, Ok(mut a))), Ok(b)) => {
                    l.extend(listed);
                    a.adornments.extend(b.adornments.iter().cloned());
                    a.full.extend(b.full.iter().map(|r| rename(r)));
                    a.full_for_negation.extend(b.full_for_negation.iter().map(|r| rename(r)));
                    a.magic_rules += b.magic_rules;
                    a.extras_kept += b.extras_kept;
                    a.gates += b.gates;
                    (l, Ok(a))
                }
                (Some((mut l, Err(e))), _) => {
                    l.extend(listed);
                    (l, Err(e))
                }
                (Some((mut l, Ok(_))), Err(e)) => {
                    l.extend(listed);
                    (l, Err(e.clone()))
                }
            });
        }
        tests.extend(p.tests.iter().map(|r| rename(r)));
        specs.extend(p.specs.iter().cloned());
        matches.extend(p.matches.iter().cloned());
        for m in &p.markers {
            if !markers.contains(m) {
                markers.push(m.clone());
            }
        }
        renames.push(to);
    }
    // The program's inputs and every part's, less what a part derives; its outputs as they were.
    drop(s_merge);
    let s_build = crate::code_rule_dsl::logic::profile::span(|| "link: the linked version".into());
    let derived: BTreeSet<String> = prepared.iter().zip(&renames).flat_map(|((_, p), to)| p.version.rules().map(|r| {
        let rel = p.version.rel_name(r.head().rel());
        to.get(rel).cloned().unwrap_or_else(|| rel.to_owned())
    }).collect::<Vec<_>>()).collect();
    let mut inputs: Vec<String> = header.inputs.clone();
    inputs.extend(added_inputs.into_iter().filter(|r| !inputs.contains(r)).collect::<Vec<_>>());
    inputs.retain(|r| !derived.contains(r));
    all.inputs = inputs;
    all.outputs = header.outputs.clone();
    let mut b = Builder::of_program(&all)?;
    for ((_, p), to) in prepared.iter().zip(&renames) {
        for r in p.version.rules() {
            b.rewrite(&p.version, r, &mut Renaming { v: &p.version, to }).finish()?;
        }
    }
    let (linked, _) = b.finish()?;
    drop(s_build);
    let _s_vocabulary = crate::code_rule_dsl::logic::profile::span(|| "link: the vocabulary and regexes".into());
    let mut vocabulary = Vocabulary::from_specs(&tests, specs)?;
    vocabulary.matches = matches.iter().map(|m| regex::Regex::new(m).map(Arc::new).map_err(|e| e.to_string())).collect::<Result<_>>()?;
    vocabulary.markers = markers;
    Ok((linked, Front { reaches, demanded, demand, undone, matchers, vocabulary }))
}
