//! The passes before lowering run per part, each part a program of its own against its interface (decision 0345):
//! the second answer to work queue Q18, beside 0333's stored runs. `RERAIL_PREPARE=parts` selects it.
//!
//! - **Parts.** Each rule of the combined program belongs to the program (`p<i>_`) or request (`q<k>_`) whose
//!   relations it reads or makes, rules that join two of them joining their parts; the rest are the library's.
//! - **Interface.** A part's inputs are the program's and what it reads of other parts; its outputs are what other
//!   parts read of it and the program's outputs it makes. The passes keep both as they are (`simplify`'s `kept`).
//! - **Kept.** A prepared part is stored under a key of its header, its rules' hashes, the engine build and the
//!   switches the passes read, made without building the part; a part found is read as it is.
//! - **Link.** The parts' rules and what each part's passes changed of the header. A name a pass made keeps its
//!   form: numbered families are renumbered past the earlier parts' (`strtest_<k>`, `matchall_<k>`, `rr_ast_<k>_<m>`),
//!   and only a name two parts made gets the part's suffix (`_x<j>`).

use super::{Front, text::Matchers};
use crate::code_rule_dsl::logic::front::{self, Value};
use crate::code_rule_dsl::logic::program_facts::{self, Facts};
use crate::code_rule_dsl::logic::program_image::{Builder, Rewrite, Version, AtomRef};
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields, Shown};
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

/// The fields every part shares with the program (decision 0345 §2): all but the per-relation fields a part
/// restricts (`Own`), its rules, inputs and outputs. Hashed once a run, into every part's key; a part's version is
/// stated them from the program's (`program_of`) and keeps only those its passes changed (`stripped`).
fn common_of(v: &Version) -> [u8; 32] {
    use sha2::Digest as _;
    let shared = (
        v.kindsets(),
        v.demand_dropped(),
        v.magic_extras(),
        v.magic_report(),
        v.gates(),
        v.policies(),
        v.settings(),
        v.natives(),
        v.rx_spans(),
        v.ast_patterns(),
    );
    let more = (
        v.doc_scopes(),
        v.answers(),
        v.text_globs(),
        v.asked(),
        v.shapes(),
    );
    sha2::Sha256::digest(format!("{shared:?}\0{more:?}").as_bytes()).into()
}

/// The relations every part's header names: the base relations (no part makes them: a pass may add a read of one)
/// and what natives read.
fn shared_of(v: &Version, made_by_parts: &BTreeSet<&str>) -> BTreeSet<String> {
    let mut shared: BTreeSet<String> = v.natives().values().flat_map(|n| n.reads().into_iter().map(str::to_owned).collect::<Vec<_>>()).collect();
    let named: Vec<String> = v
        .declared_names()
        .into_iter()
        .chain(v.nullary())
        .chain(v.lattice().into_keys())
        .chain(v.fact_names())
        .chain(v.qualifiers().into_keys())
        .chain(v.tapes().into_keys())
        .chain(v.demand())
        .collect();
    shared.extend(named.into_iter().filter(|r| !made_by_parts.contains(r.as_str())));
    shared
}

/// A part's own header (decision 0345 §2), as a selection of the program's fields: the relations whose declarations,
/// facts and per-relation lists it holds (`names`: those it makes and reads, and the shared ones), its inputs and
/// outputs, and the words its interface adds to the qualifiers. The fields every part shares (`common_of`) are not
/// in it. Another part's relations are not its interface, and a change of one does not change its key. Read over the
/// program as fields (`Selected`).
#[derive(Clone)]
struct Own {
    names: BTreeSet<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    words: BTreeMap<String, BTreeSet<String>>,
}

/// A part's own header (`Own`) read over the program's version `v`: its relations' fields, none of the shared ones.
struct Selected<'a> {
    v: &'a Version,
    own: &'a Own,
}

impl Selected<'_> {
    fn has(&self, rel: &str) -> bool {
        self.own.names.contains(rel)
    }
}

impl ProgramFields for Selected<'_> {
    fn input_names(&self) -> Vec<String> {
        self.own.inputs.clone()
    }
    fn output_names(&self) -> Vec<String> {
        self.own.outputs.clone()
    }
    fn declares(&self, rel: &str) -> bool {
        self.has(rel) && self.v.declares(rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        if self.has(rel) { self.v.columns_of(rel) } else { None }
    }
    fn declared_names(&self) -> Vec<String> {
        self.v.declared_names().into_iter().filter(|r| self.has(r)).collect()
    }
    fn fact_names(&self) -> Vec<String> {
        ProgramFields::fact_names(self.v).into_iter().filter(|r| self.has(r)).collect()
    }
    fn states_facts(&self, rel: &str) -> bool {
        self.has(rel) && self.v.states_facts(rel)
    }
    fn fact_rows(&self, rel: &str) -> Vec<front::List<Value>> {
        if self.has(rel) { self.v.fact_rows(rel) } else { Vec::new() }
    }
    fn about(&self, _relations: &[&str]) -> Facts {
        Facts::new()
    }
    fn settings(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        Vec::new()
    }
    fn text_globs(&self) -> Vec<String> {
        Vec::new()
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        None
    }
    fn natives(&self) -> BTreeMap<String, front::ast::Native> {
        BTreeMap::new()
    }
    fn nullary(&self) -> BTreeSet<String> {
        self.v.nullary().into_iter().filter(|r| self.has(r)).collect()
    }
    fn qualifiers(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = self.v.qualifiers().into_iter().filter(|(r, _)| self.has(r)).collect();
        for (r, words) in &self.own.words {
            out.entry(r.clone()).or_default().extend(words.iter().cloned());
        }
        out
    }
    fn tapes(&self) -> BTreeMap<String, BTreeSet<usize>> {
        self.v.tapes().into_iter().filter(|(r, _)| self.has(r)).collect()
    }
    fn lattice(&self) -> BTreeMap<String, front::ast::LatticeDecl> {
        self.v.lattice().into_iter().filter(|(r, _)| self.has(r)).collect()
    }
    fn kindsets(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn rx_spans(&self) -> BTreeSet<(front::ast::SpanIn, String)> {
        BTreeSet::new()
    }
    fn ast_patterns(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn demand(&self) -> Vec<String> {
        self.v.demand().into_iter().filter(|r| self.has(r)).collect()
    }
    fn demand_dropped(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn gates(&self) -> Vec<(String, front::ast::Gate)> {
        Vec::new()
    }
    fn shapes(&self) -> Vec<front::ast::Shape> {
        Vec::new()
    }
    fn answers(&self) -> BTreeMap<String, Vec<String>> {
        BTreeMap::new()
    }
    fn asked(&self) -> BTreeMap<String, Vec<BTreeSet<String>>> {
        BTreeMap::new()
    }
    fn magic_extras(&self) -> Option<usize> {
        None
    }
    fn magic_report(&self) -> Option<front::ast::Report> {
        None
    }
}

/// The per-relation fields `p` holds of the relations `pick` takes, and `inputs`: what `with_shared` and `key_of`
/// hash.
fn per_relation(p: &dyn ProgramFields, pick: &dyn Fn(&str) -> bool) -> String {
    let decls: Vec<_> = p.declared_names().into_iter().filter(|r| pick(r)).map(|r| {
        let cols = p.columns_of(&r);
        (r, cols)
    }).collect();
    let nullary: Vec<_> = p.nullary().into_iter().filter(|r| pick(r)).collect();
    let lattice: Vec<_> = p.lattice().into_iter().filter(|(r, _)| pick(r)).collect();
    let facts: Vec<_> = p.fact_names().into_iter().filter(|r| pick(r)).map(|r| {
        let rows = p.fact_rows(&r);
        (r, rows)
    }).collect();
    let qualifiers: Vec<_> = p.qualifiers().into_iter().filter(|(r, _)| pick(r)).collect();
    let tapes: Vec<_> = p.tapes().into_iter().filter(|(r, _)| pick(r)).collect();
    let demand: Vec<_> = p.demand().into_iter().filter(|r| pick(r)).collect();
    format!("{:?}", (decls, nullary, lattice, facts, qualifiers, tapes, demand))
}

/// `common` with what every part's header holds of the program's relations: the shared ones' fields (`shared_of`)
/// and the program's inputs, which a part's header starts from. Hashed once a run, into every part's key.
fn with_shared(common: &[u8; 32], v: &Version, shared: &BTreeSet<String>) -> [u8; 32] {
    use sha2::Digest as _;
    let all = (per_relation(v, &|r| shared.contains(r)), v.input_names());
    sha2::Sha256::new().chain_update(common).chain_update(format!("{all:?}").as_bytes()).finalize().into()
}

/// A part's own header (`Own`, decision 0345 §2): the relations it makes and reads and the `shared` ones, and its
/// inputs and outputs.
fn part_header(v: &Version, part: &Part, read_elsewhere: &BTreeSet<String>, made_by_parts: &BTreeSet<&str>, shared: &BTreeSet<String>) -> Own {
    let names: BTreeSet<String> = part.made.iter().chain(&part.read).chain(shared).cloned().collect();
    let program_outputs = v.output_names();
    let outputs = part.made.iter().filter(|r| program_outputs.contains(*r) || read_elsewhere.contains(*r)).cloned().collect();
    // Its inputs: the program's, and what it reads that another part makes. A relation no part makes (a base
    // relation, a request the runner answers: `dir_entry`, `loaded`) stays as the whole program has it, for the
    // part's own passes to treat as the whole's do.
    let mut inputs = v.input_names();
    for r in part.read.iter().filter(|r| !part.made.contains(*r)) {
        if made_by_parts.contains(r.as_str()) && v.declares(r) && !inputs.contains(r) {
            inputs.push(r.clone());
        }
    }
    Own { names, inputs, outputs, words: BTreeMap::new() }
}

/// A part's key (decision 0345 §5), without building it: the engine, the dialect, the switches its passes read, the
/// shared header's digest (`common_of`), its own header's per-relation fields, and its rules' hashes in order (a
/// rule's hash is its text's: its relations by name).
fn key_of(head: &Selected<'_>, common: &[u8; 32], shared: &BTreeSet<String>, inputs_shared: usize, rules: &[crate::code_rule_dsl::logic::program_image::RuleRef<'_>], part: &Part, dialect: crate::code_rule_dsl::language::GrammarDialect) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/part/v5\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update(format!("\0{dialect:?}\0").as_bytes());
    for s in SWITCHES {
        h.update(format!("{s}={}\0", std::env::var(s).unwrap_or_default()).as_bytes());
    }
    // The fields every part shares (`common_of`, and the shared relations' and the program's inputs: `with_shared`)
    // were hashed once; the part's own are its relations' and what it adds to the inputs.
    h.update(common);
    let inputs = head.input_names();
    let own = (per_relation(head, &|r| !shared.contains(r)), &inputs[inputs_shared.min(inputs.len())..], head.output_names());
    h.update(format!("{own:?}").as_bytes());
    h.update((part.rules.len() as u64).to_le_bytes());
    for &i in &part.rules {
        h.update(rules[i].hash().to_le_bytes());
    }
    h.finalize().into()
}

/// States on `b` the declarations, facts, inputs, outputs and per-relation fields of `own`, and the shared fields of
/// `shared`; a shared field `whole` holds alike is left empty (`stripped`), where `whole` is given.
fn state_fields(b: &mut Builder<'_>, own: &dyn ProgramFields, shared: &dyn ProgramFields, whole: Option<&dyn ProgramFields>) {
    use crate::code_rule_dsl::logic::program_facts::Field;
    for rel in own.declared_names() {
        let cols = own.columns_of(&rel).unwrap_or_default();
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        FieldsMut::declare(b, &rel, &pairs);
    }
    for rel in own.fact_names() {
        b.set_facts(&rel, &own.fact_rows(&rel));
    }
    b.set_inputs(&own.input_names());
    b.set_outputs(&own.output_names());
    b.set_field(Field::Nullary(&own.nullary()));
    b.set_field(Field::Lattice(&own.lattice()));
    b.set_field(Field::Qualifiers(&own.qualifiers()));
    b.set_field(Field::Tapes(&own.tapes()));
    b.set_field(Field::Demand(&own.demand()));
    // A shared field as `shared` holds it, or none where `whole` holds it alike.
    macro_rules! shared {
        ($get:ident) => {{
            let x = shared.$get();
            if whole.is_some_and(|w| format!("{:?}", w.$get()) == format!("{x:?}")) { Default::default() } else { x }
        }};
    }
    b.set_field(Field::Kindsets(&shared!(kindsets)));
    b.set_field(Field::DemandDropped(&shared!(demand_dropped)));
    b.set_field(Field::MagicExtras(shared!(magic_extras)));
    b.set_field(Field::MagicReport(shared!(magic_report).as_ref()));
    b.set_field(Field::Gates(&shared!(gates)));
    b.set_field(Field::Policies(&shared!(policies)));
    b.set_field(Field::Settings(&shared!(settings)));
    b.set_field(Field::Natives(&shared!(natives)));
    b.set_field(Field::RxSpans(&shared!(rx_spans)));
    b.set_field(Field::AstPatterns(&shared!(ast_patterns)));
    b.set_field(Field::DocScopes(shared!(doc_scopes).as_deref()));
    b.set_field(Field::Answers(&shared!(answers)));
    b.set_field(Field::TextGlobs(&shared!(text_globs)));
    b.set_field(Field::Asked(&shared!(asked)));
    b.set_field(Field::Shapes(&shared!(shapes)));
}

/// A part's program: its own header (`Own`) on the shared fields of the program `v`, and its rules, copied from it.
fn program_of(v: &Version, rules: &[crate::code_rule_dsl::logic::program_image::RuleRef<'_>], own: &Own, part: &Part) -> Result<Arc<Version>> {
    let mut b = Builder::base();
    state_fields(&mut b, &Selected { v, own }, v, None);
    for &i in &part.rules {
        b.copy(v, rules[i]);
    }
    Ok(b.finish()?.0)
}

/// A prepared part as it is kept: its rules, its own header, and of the shared fields only those its passes
/// changed (each then whole). A field the passes left as the program `whole` has it is the link's already, so neither
/// the store nor the link holds a copy of it per part.
fn stripped(v: &Arc<Version>, whole: &Version) -> Result<Arc<Version>> {
    let mut b = Builder::base();
    state_fields(&mut b, &**v, &**v, Some(whole));
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
    /// The names its passes made, new to the part's program: what the link may rename.
    made: Vec<String>,
}

impl Prepared {
    fn of(version: Arc<Version>, f: Front, made: Vec<String>) -> Prepared {
        Prepared {
            made,
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

impl Prepared {
    /// What a link found, as a part's entry holds it (`facts_of`): a kept link's.
    fn of_front(version: Arc<Version>, f: &Front) -> Prepared {
        Prepared {
            made: Vec::new(),
            version,
            reaches: f.reaches,
            demanded: f.demanded.clone(),
            demand: f.demand.clone(),
            undone: f.undone.clone(),
            matchers: f.matchers.clone(),
            tests: f.vocabulary.tests.iter().map(|(r, _)| r.clone()).collect(),
            specs: f.vocabulary.specs.clone(),
            matches: f.vocabulary.matches.iter().map(|m| m.as_str().to_owned()).collect(),
            markers: f.vocabulary.markers.clone(),
        }
    }

    /// The part (or link) an entry's facts (`facts_of`) describe, over `version`.
    fn of_facts(version: Arc<Version>, facts: &Facts) -> Prepared {
        let num = |s: &str| s.parse::<usize>().unwrap_or(0);
        let reach = rows_of(facts, "part_reach").next().unwrap_or_default();
        let tests = ordered(facts, "part_test");
        let demand = facts.get("part_demand").map(|_| {
            let listed = ordered(facts, "part_demand_listed").into_iter().map(|r| r[1].clone()).collect();
            let row = rows_of(facts, "part_demand").next().unwrap_or_default();
            let outcome = if row.first().map(String::as_str) == Some("ok") && row.len() >= 7 {
                Ok(front::ast::Report { adornments: split(&row[1]), magic_rules: num(&row[2]), full: split(&row[3]), extras_kept: num(&row[4]), gates: num(&row[5]), full_for_negation: split(&row[6]) })
            } else {
                Err(row.get(1).cloned().unwrap_or_default())
            };
            (listed, outcome)
        });
        Prepared {
            version,
            reaches: (reach.first().map_or(0, |s| num(s)), reach.get(1).map_or(0, |s| num(s))),
            demanded: ordered(facts, "part_demanded").into_iter().map(|r| r[1].clone()).collect(),
            demand,
            undone: ordered(facts, "part_undone").into_iter().map(|r| r[1].clone()).collect(),
            matchers: ordered(facts, "part_matcher").into_iter().map(|r| (r[1].clone(), r[2].clone(), split(&r[3]))).collect(),
            tests: tests.iter().map(|r| r[1].clone()).collect(),
            specs: tests.iter().map(|r| (r[2].clone(), split(&r[3]))).collect(),
            matches: ordered(facts, "part_match").into_iter().map(|r| r[1].clone()).collect(),
            markers: ordered(facts, "part_marker").into_iter().map(|r| r[1].clone()).collect(),
            made: ordered(facts, "part_made").into_iter().map(|r| r[1].clone()).collect(),
        }
    }

    /// What a link found, from its kept form (`of_front`): its vocabulary made again, as `link` makes it.
    fn into_front(self) -> Result<Front> {
        let mut vocabulary = Vocabulary::from_specs(&self.tests, self.specs)?;
        vocabulary.matches = self.matches.iter().map(|m| regex::Regex::new(m).map(Arc::new).map_err(|e| e.to_string())).collect::<Result<_>>()?;
        vocabulary.markers = self.markers;
        Ok(Front { reaches: self.reaches, demanded: self.demanded, demand: self.demand, undone: self.undone, matchers: self.matchers, vocabulary })
    }
}

pub(in crate::code_rule_dsl::logic::plan::corpus) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(in crate::code_rule_dsl::logic::plan::corpus) fn unhex(text: &str) -> Option<[u8; 32]> {
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

pub(in crate::code_rule_dsl::logic::plan::corpus) fn str(v: &str) -> Value {
    Value::Str(v.to_owned().into())
}

pub(in crate::code_rule_dsl::logic::plan::corpus) fn rows_of<'a>(facts: &'a Facts, rel: &str) -> impl Iterator<Item = Vec<String>> + 'a {
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
    put("part_made", p.made.iter().enumerate().map(|(i, m)| vec![n(i), m.clone()]).collect());
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
pub(in crate::code_rule_dsl::logic::plan::corpus) fn ordered(facts: &Facts, rel: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = rows_of(facts, rel).collect();
    rows.sort_by_key(|r| r.first().and_then(|i| i.parse::<usize>().ok()).unwrap_or(0));
    rows
}

fn split(s: &str) -> Vec<String> {
    if s.is_empty() { Vec::new() } else { s.split('\u{1f}').map(str::to_owned).collect() }
}

/// The part kept under `key`, with the sources its image is kept as.
/// The version and facts kept under `key` (`keep`), and the sources they were read from.
pub(in crate::code_rule_dsl::logic::plan::corpus) fn kept(store: &ContentStore, key: &[u8; 32], lists: &front::Lists) -> StoreResult<Option<(Arc<Version>, Facts, Vec<SourceId>)>> {
    let Some((facts, digest, source)) = kept_facts(store, key)? else { return Ok(None) };
    let Some((version, mut sources)) = kept_image(store, &digest, lists)? else { return Ok(None) };
    sources.push(source);
    Ok(Some((version, facts, sources)))
}

/// The facts kept under `key`, its image's digest and the entry's source: the image not read.
fn kept_facts(store: &ContentStore, key: &[u8; 32]) -> StoreResult<Option<(Facts, [u8; 32], SourceId)>> {
    let Some(source) = store.source_by_hash(HashKey::sha256(source_text(key).as_bytes()))? else { return Ok(None) };
    let set = plan();
    let Some(result) = store.facts_memo(FactsMemoKey { source, fact_schema: SCHEMA, path: PATH })?.results.remove(&set.plans[0]) else { return Ok(None) };
    let Ok(facts) = program_facts::decode(&result.entry) else { return Ok(None) };
    let Some(digest) = rows_of(&facts, "part_program").next().and_then(|r| r.first().and_then(|d| unhex(d))) else { return Ok(None) };
    Ok(Some((facts, digest, source)))
}

fn kept_image(store: &ContentStore, digest: &[u8; 32], lists: &front::Lists) -> StoreResult<Option<(Arc<Version>, Vec<SourceId>)>> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "a part, from the store: its image".into());
    program_text::load_image(store, digest, lists)
}

fn lookup(store: &ContentStore, key: &[u8; 32], lists: &front::Lists) -> StoreResult<Option<(Prepared, Vec<SourceId>)>> {
    let Some((version, facts, sources)) = kept(store, key, lists)? else { return Ok(None) };
    Ok(Some((Prepared::of_facts(version, &facts), sources)))
}

/// Keeps the part under `key`, as an item of the run's record (decision 0306): its image and its entry, written by the
/// store's writer, their sources named by the run.
fn record(store: &ContentStore, key: &[u8; 32], p: &Prepared) {
    keep(store, key, &p.version, |digest| facts_of(p, digest));
}

/// Keeps `version` and the facts `facts` makes of its image's digest under `key` (either stage's part).
pub(in crate::code_rule_dsl::logic::plan::corpus) fn keep(store: &ContentStore, key: &[u8; 32], version: &Arc<Version>, facts: impl FnOnce(&[u8; 32]) -> Facts) {
    let Some(encoded) = program_text::encode_version(version) else { return };
    let entry = program_facts::encode(&facts(&encoded.digest));
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
fn names_of(v: &Version) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = v.declared_names().into_iter().collect();
    out.extend(v.input_names());
    out.extend(v.output_names());
    out.extend(ProgramFields::fact_names(v));
    out.extend(v.natives().into_keys());
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

/// What part `after`'s passes changed of its header `before`, applied to the link's fields `all` with its relations
/// renamed by `rename`: entries added or changed are set; entries of relations the part owns that its passes removed
/// go. A field no part changed is not read of `all`.
fn merge_header(all: &mut Builder<'_>, before: &dyn ProgramFields, after: &dyn ProgramFields, owned: &dyn Fn(&str) -> bool, rename: &dyn Fn(&str) -> String) {
    use crate::code_rule_dsl::logic::program_facts::Field;
    for rel in after.declared_names() {
        let cols = after.columns_of(&rel);
        if before.columns_of(&rel) != cols {
            let cols = cols.unwrap_or_default();
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            FieldsMut::declare(all, &rename(&rel), &pairs);
        }
    }
    for rel in before.declared_names() {
        if !after.declares(&rel) && owned(&rel) && ProgramFields::declares(all, &rel) {
            all.undeclare(&rel);
        }
    }
    for rel in after.fact_names() {
        let rows = after.fact_rows(&rel);
        if !before.states_facts(&rel) || before.fact_rows(&rel) != rows {
            all.set_facts(&rename(&rel), &rows);
        }
    }
    for rel in before.fact_names() {
        if !after.states_facts(&rel) && owned(&rel) && ProgramFields::states_facts(all, &rel) {
            all.drop_facts(&rel);
        }
    }
    macro_rules! map {
        ($get:ident, $field:ident, $value:expr) => {{
            let (b, a) = (before.$get(), after.$get());
            let set: Vec<_> = a.iter().filter(|(k, v)| b.get(*k) != Some(*v)).collect();
            let gone: Vec<&String> = b.keys().filter(|k| !a.contains_key(*k) && owned(k)).collect();
            if !set.is_empty() || !gone.is_empty() {
                let mut x = all.$get();
                for (k, v) in set {
                    x.insert(rename(k), $value(v.clone()));
                }
                for k in gone {
                    x.remove(k);
                }
                all.set_field(Field::$field(&x));
            }
        }};
    }
    map!(lattice, Lattice, |v| v);
    map!(qualifiers, Qualifiers, |v| v);
    map!(tapes, Tapes, |v| v);
    map!(natives, Natives, |mut n: front::ast::Native| {
        n.rename_reads(rename);
        n
    });
    map!(answers, Answers, |v: Vec<String>| v.iter().map(|r| rename(r)).collect::<Vec<_>>());
    map!(asked, Asked, |v: Vec<BTreeSet<String>>| v.into_iter().map(|s| s.iter().map(|r| rename(r)).collect()).collect::<Vec<_>>());
    let kindsets = after.kindsets();
    if !kindsets.is_empty() {
        let mut x = all.kindsets();
        let n = x.len();
        for (k, v) in kindsets {
            x.entry(k).or_insert(v);
        }
        if x.len() != n {
            all.set_field(Field::Kindsets(&x));
        }
    }
    let (b, a) = (before.nullary(), after.nullary());
    let (added, gone): (Vec<&String>, Vec<&String>) = (a.iter().filter(|r| !b.contains(*r)).collect(), b.iter().filter(|r| !a.contains(*r) && owned(r)).collect());
    if !added.is_empty() || !gone.is_empty() {
        let mut x = all.nullary();
        x.extend(added.into_iter().map(|r| rename(r)));
        for r in gone {
            x.remove(r);
        }
        all.set_field(Field::Nullary(&x));
    }
    let (b, a) = (before.demand(), after.demand());
    if a.iter().any(|r| !b.contains(r)) || b.iter().any(|r| !a.contains(r) && owned(r)) {
        let mut x = all.demand();
        for r in a.iter().filter(|r| !b.contains(*r)) {
            let r2 = rename(r);
            if !x.contains(&r2) {
                x.push(r2);
            }
        }
        x.retain(|r| !(b.contains(r) && !a.contains(r) && owned(r)));
        all.set_field(Field::Demand(&x));
    }
    let more = after.rx_spans();
    if !more.is_empty() {
        let mut x = all.rx_spans();
        let n = x.len();
        x.extend(more);
        if x.len() != n {
            all.set_field(Field::RxSpans(&x));
        }
    }
    let more = after.ast_patterns();
    if !more.is_empty() {
        let mut x = all.ast_patterns();
        let n = x.len();
        x.extend(more);
        if x.len() != n {
            all.set_field(Field::AstPatterns(&x));
        }
    }
    let (b, a) = (before.gates(), after.gates());
    let added: Vec<_> = a.into_iter().filter(|(rel, _)| !b.iter().any(|(r, _)| r == rel)).collect();
    if !added.is_empty() {
        let mut x = all.gates();
        x.extend(added.into_iter().map(|(rel, g)| (rename(&rel), g)));
        all.set_field(Field::Gates(&x));
    }
    // A list's entries `after` added: each the link has not.
    macro_rules! list {
        ($get:ident, $field:ident) => {{
            let (b, a) = (before.$get(), after.$get());
            let added: Vec<_> = a.into_iter().filter(|d| !b.contains(d)).collect();
            if !added.is_empty() {
                let mut x = all.$get();
                let n = x.len();
                for d in added {
                    if !x.contains(&d) {
                        x.push(d);
                    }
                }
                if x.len() != n {
                    all.set_field(Field::$field(&x));
                }
            }
        }};
    }
    list!(demand_dropped, DemandDropped);
    list!(shapes, Shapes);
    list!(settings, Settings);
    let (b, a) = (before.magic_report(), after.magic_report());
    if a != b && a.is_some() {
        all.set_field(Field::MagicReport(a.as_ref()));
    }
    let (b, a) = (before.magic_extras(), after.magic_extras());
    if a != b {
        let x = Some(all.magic_extras().unwrap_or(0) + a.unwrap_or(0));
        all.set_field(Field::MagicExtras(x));
    }
}

/// `preparation::front` per part (decision 0345): the linked version, what the passes found, and the sources of the
/// parts the store kept, for the run's manifest to name. `store` keeps the parts when `keep`.
///
/// The link is kept too (the kept link, Q18): the last linked version under the program's parts by name
/// (`link_key`), with each part's key and interface. Where only some parts' keys changed, and their interfaces
/// (`interface`) did not, those parts' rules are spliced into the last linked version in their places, and the
/// other parts are neither read nor linked again.
pub fn front(version: Arc<Version>, dialect: crate::code_rule_dsl::language::GrammarDialect, store: &ContentStore, keep: bool, lists: &front::Lists) -> Result<(Arc<Version>, Front, Vec<SourceId>)> {
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
    let common_digest = common_of(&version);
    let shared = shared_of(&version, &made_by_parts);
    let common_digest = with_shared(&common_digest, &version, &shared);
    let inputs_shared = version.input_names().len();
    // Every part's header and key, read from the program alone: all the kept link asks of a part that did not change.
    let (heads, keys): (Vec<Own>, Vec<[u8; 32]>) = parts
        .iter()
        .enumerate()
        .map(|(j, part)| {
            let read_elsewhere: BTreeSet<String> = part.made.iter().filter(|r| readers.get(r.as_str()).is_some_and(|js| js.iter().any(|&o| o != j))).cloned().collect();
            let head = part_header(&version, part, &read_elsewhere, &made_by_parts, &shared);
            let key = {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "a part's key".into());
                key_of(&Selected { v: &version, own: &head }, &common_digest, &shared, inputs_shared, &rules, part, dialect)
            };
            (head, key)
        })
        .unzip();
    let (found, made) = (std::cell::Cell::new(0usize), std::cell::Cell::new(0usize));
    let prepare = |j: usize| -> Result<(Prepared, Vec<SourceId>)> {
        let kept = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a part, from the store".into());
            lookup(store, &keys[j], lists).map_err(|e| e.to_string())?
        };
        if let Some(found_part) = kept {
            found.set(found.get() + 1);
            return Ok(found_part);
        }
        made.set(made.get() + 1);
        let part = &parts[j];
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("prepare part {} ({} rules)", part.name, part.rules.len()));
        let input = program_of(&version, &rules, &heads[j], part)?;
        let had = names_of(&input);
        let (v, f, _) = crate::code_rule_dsl::logic::profile::quiet(|| super::front(input, dialect, None))?;
        let made_names: Vec<String> = names_of(&v).into_iter().filter(|r| !had.contains(r)).collect();
        let v = stripped(&v, &version)?;
        let p = Prepared::of(v, f, made_names);
        if keep {
            record(store, &keys[j], &p);
        }
        Ok((p, Vec::new()))
    };
    let mut prepared: Vec<Option<(Prepared, Vec<SourceId>)>> = (0..parts.len()).map(|_| None).collect();
    let family = link_key("before lowering", &common_digest, &version.output_names(), &parts, format!("{dialect:?}").as_bytes());
    let last = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the last link, from the store (Q18)".into());
        kept_facts(store, &family).map_err(|e| e.to_string())?
    };
    let mut why = "no link kept".to_owned();
    if let Some((facts, digest, source)) = last {
        let slots = link_slots(&facts);
        if slots.len() == parts.len() && slots.iter().zip(&parts).all(|(s, p)| s.name == p.name) {
            let changed: Vec<usize> = (0..parts.len()).filter(|&j| slots[j].key != keys[j]).collect();
            let mut same = true;
            for &j in &changed {
                let (p, s) = prepare(j)?;
                let face = interface(&version, &heads[j], &parts[j], &p);
                prepared[j] = Some((p, s));
                if face != slots[j].interface {
                    why = format!("part {}'s interface changed", parts[j].name);
                    same = false;
                    break;
                }
            }
            if same {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "splice the changed parts into the last link (Q18)".into());
                if let Some((linked, front, sources)) = splice(store, &facts, &digest, source, &slots, &changed, &prepared, &keys, &heads, &version, &parts, &family, keep, lists)? {
                    crate::code_rule_dsl::logic::profile::decision("the kept link (Q18)", || format!("before lowering: {} of {} parts changed, spliced into the last link ({} read from the store, {} prepared)", changed.len(), parts.len(), found.get(), made.get()));
                    return Ok((linked, front, sources));
                }
                why = "the last link's image is missing or its rules miscounted".to_owned();
            }
        } else {
            why = "the parts are not the last link's".to_owned();
        }
    }
    for j in 0..parts.len() {
        if prepared[j].is_none() {
            prepared[j] = Some(prepare(j)?);
        }
    }
    crate::code_rule_dsl::logic::profile::decision("parts prepared (decision 0345)", || format!("{} parts: {} read from the store, {} prepared", parts.len(), found.get(), made.get()));
    crate::code_rule_dsl::logic::profile::decision("the kept link (Q18)", || format!("before lowering: linked again ({why})"));
    let (prepared, part_sources): (Vec<Prepared>, Vec<Vec<SourceId>>) = prepared.into_iter().map(|p| p.expect("every part prepared")).unzip();
    let sources: Vec<SourceId> = part_sources.iter().flatten().copied().collect();
    let with_heads: Vec<(Own, Prepared)> = heads.iter().cloned().zip(prepared).collect();
    let (linked, front, renames) = {
        let _link = crate::code_rule_dsl::logic::profile::span(|| "link the prepared parts (decision 0345)".into());
        link(&version, &parts, &with_heads)?
    };
    if keep {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the link (Q18)".into());
        let slots: Vec<LinkSlot> = parts
            .iter()
            .zip(&with_heads)
            .zip(&keys)
            .zip(part_sources)
            .map(|(((part, (head, p)), key), sources)| Ok(LinkSlot { name: part.name.clone(), key: *key, interface: interface(&version, head, part, p), rules: p.version.rules().count(), sources }))
            .collect::<Result<_>>()?;
        keep_link(store, &family, &linked, &front, &slots, &renames);
    }
    Ok((linked, front, sources))
}

// -- The kept link (Q18) --

/// The key a program's last link is kept under: its stage, the engine, the shared header's digest, the program's
/// outputs, `extra` (the stage's settings) and its parts by name. An edit of some parts' rules keeps it: the link's
/// slots say which parts changed.
fn link_key(stage: &str, common: &[u8; 32], outputs: &[String], parts: &[Part], extra: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/part-link/v1\0");
    h.update(stage.as_bytes());
    h.update(b"\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    h.update(common);
    h.update(format!("{outputs:?}\0").as_bytes());
    h.update(extra);
    h.update(b"\0");
    for p in parts {
        h.update(p.name.as_bytes());
        h.update(b"\0");
    }
    h.finalize().into()
}

/// A part's place in a kept link: its key, its interface's digest, how many of the linked rules are its (in the
/// parts' order), and the sources it was read from.
struct LinkSlot {
    name: String,
    key: [u8; 32],
    interface: [u8; 32],
    rules: usize,
    sources: Vec<SourceId>,
}

/// What of a prepared part the link reads besides its rules' bodies: its header before and after its passes, what it
/// makes and reads, the relations its rules derive, and what its passes found. Two parts of one interface link alike:
/// the same renames, the same header changes, the same counts the parts after them are numbered past.
fn interface(v: &Version, head: &Own, part: &Part, p: &Prepared) -> [u8; 32] {
    use sha2::Digest as _;
    let (head, after) = (Shown(&Selected { v, own: head }), Shown(&*p.version));
    let derived: BTreeSet<&str> = p.version.rules().map(|r| p.version.rel_name(r.head().rel())).collect();
    let found = facts_of(p, &[0; 32]);
    sha2::Sha256::digest(format!("{head:?}\0{:?}\0{:?}\0{after:?}\0{derived:?}\0{found:?}", part.made, part.read).as_bytes()).into()
}

/// The slots a kept link's facts hold, in the parts' order.
fn link_slots(facts: &Facts) -> Vec<LinkSlot> {
    let mut sources: HashMap<usize, Vec<SourceId>> = HashMap::new();
    for r in ordered(facts, "link_source") {
        if let (Some(j), Some(id)) = (r.get(1).and_then(|j| j.parse().ok()), r.get(2).and_then(|i| i.parse().ok())) {
            sources.entry(j).or_default().push(SourceId::from_raw(id));
        }
    }
    ordered(facts, "link_slot")
        .into_iter()
        .filter_map(|r| {
            let j: usize = r.first()?.parse().ok()?;
            Some(LinkSlot { name: r.get(1)?.clone(), key: unhex(r.get(2)?)?, interface: unhex(r.get(3)?)?, rules: r.get(4)?.parse().ok()?, sources: sources.remove(&j).unwrap_or_default() })
        })
        .collect()
}

/// The facts a kept link holds beside its image: its slots and their sources, and `extra`'s rows.
fn link_facts(digest: &[u8; 32], slots: &[LinkSlot], mut extra: Facts) -> Facts {
    let mut put = |rel: &str, rows: Vec<Vec<String>>| {
        extra.insert(rel.to_owned(), rows.into_iter().map(|r| r.iter().map(|c| str(c)).collect::<Vec<_>>().into()).collect());
    };
    put("part_program", vec![vec![hex(digest)]]);
    put("link_slot", slots.iter().enumerate().map(|(j, s)| vec![j.to_string(), s.name.clone(), hex(&s.key), hex(&s.interface), s.rules.to_string()]).collect());
    let mut k = 0usize;
    let mut rows = Vec::new();
    for (j, s) in slots.iter().enumerate() {
        for id in &s.sources {
            rows.push(vec![k.to_string(), j.to_string(), id.raw().to_string()]);
            k += 1;
        }
    }
    put("link_source", rows);
    extra
}

/// Keeps the link before lowering: its image, what its passes found (as a part's are), its slots and renames.
fn keep_link(store: &ContentStore, family: &[u8; 32], linked: &Arc<Version>, front: &Front, slots: &[LinkSlot], renames: &[HashMap<String, String>]) {
    let found = Prepared::of_front(linked.clone(), front);
    keep(store, family, linked, |digest| {
        let mut facts = facts_of(&found, digest);
        let mut k = 0usize;
        let mut rows = Vec::new();
        for (j, to) in renames.iter().enumerate() {
            let mut to: Vec<(&String, &String)> = to.iter().collect();
            to.sort();
            for (from, into) in to {
                rows.push(vec![str(&k.to_string()), str(&j.to_string()), str(from), str(into)].into());
                k += 1;
            }
        }
        facts.insert("link_rename".to_owned(), rows);
        link_facts(digest, slots, facts)
    });
}

/// The last link before lowering with the `changed` parts' rules spliced in, each part's renamed as the last link
/// renamed it; `None` when the link's image is missing or its rules are not its slots' count.
#[allow(clippy::too_many_arguments)]
fn splice(
    store: &ContentStore,
    facts: &Facts,
    digest: &[u8; 32],
    source: SourceId,
    slots: &[LinkSlot],
    changed: &[usize],
    prepared: &[Option<(Prepared, Vec<SourceId>)>],
    keys: &[[u8; 32]],
    heads: &[Own],
    whole: &Version,
    parts: &[Part],
    family: &[u8; 32],
    keep: bool,
    lists: &front::Lists,
) -> Result<Option<(Arc<Version>, Front, Vec<SourceId>)>> {
    let Some((last, mut sources)) = kept_image(store, digest, lists).map_err(|e| e.to_string())? else { return Ok(None) };
    sources.push(source);
    let last_rules: Vec<_> = last.rules().collect();
    if slots.iter().map(|s| s.rules).sum::<usize>() != last_rules.len() {
        return Ok(None);
    }
    let mut renames: Vec<HashMap<String, String>> = (0..slots.len()).map(|_| HashMap::new()).collect();
    for r in rows_of(facts, "link_rename") {
        if let (Some(j), Some(from), Some(to)) = (r.get(1).and_then(|j| j.parse::<usize>().ok()), r.get(2), r.get(3))
            && j < renames.len()
        {
            renames[j].insert(from.clone(), to.clone());
        }
    }
    let linked = if changed.is_empty() {
        last.clone()
    } else {
        let mut b = Builder::derive(&last);
        let mut at = 0;
        for (j, slot) in slots.iter().enumerate() {
            if changed.contains(&j) {
                let (p, _) = prepared[j].as_ref().expect("a changed part prepared");
                for r in p.version.rules() {
                    b.rewrite(&p.version, r, &mut Renaming { v: &p.version, to: &renames[j] }).finish()?;
                }
            } else {
                for r in &last_rules[at..at + slot.rules] {
                    b.copy_as_is(&last, *r);
                }
            }
            at += slot.rules;
        }
        b.finish()?.0
    };
    for (j, slot) in slots.iter().enumerate() {
        match &prepared[j] {
            Some((_, s)) if changed.contains(&j) => sources.extend(s.iter().copied()),
            _ => sources.extend(slot.sources.iter().copied()),
        }
    }
    let front = Prepared::of_facts(last.clone(), facts).into_front()?;
    if keep && !changed.is_empty() {
        let slots: Vec<LinkSlot> = slots
            .iter()
            .enumerate()
            .map(|(j, s)| match &prepared[j] {
                Some((p, ps)) if changed.contains(&j) => Ok(LinkSlot { name: s.name.clone(), key: keys[j], interface: interface(whole, &heads[j], &parts[j], p), rules: p.version.rules().count(), sources: ps.clone() }),
                _ => Ok(LinkSlot { name: s.name.clone(), key: s.key, interface: s.interface, rules: s.rules, sources: s.sources.clone() }),
            })
            .collect::<Result<_>>()?;
        keep_link(store, family, &linked, &front, &slots, &renames);
    }
    Ok(Some((linked, front, sources)))
}

/// The prepared parts linked (decision 0345 §6), with what their passes found, renamed.
fn link(version: &Version, parts: &[Part], prepared: &[(Own, Prepared)]) -> Result<(Arc<Version>, Front, Vec<HashMap<String, String>>)> {
    let s_names = crate::code_rule_dsl::logic::profile::span(|| "link: the parts' names".into());
    // The program's names: its relations by id (every name a rule holds has one) and its header's.
    let mut known: BTreeSet<String> = (0..version.rel_end()).map(|r| version.rel_name(crate::code_rule_dsl::logic::program_image::RelId(r)).to_owned()).collect();
    let outputs = version.output_names();
    known.extend(version.declared_names().into_iter().chain(version.input_names()).chain(outputs.iter().cloned()).chain(ProgramFields::fact_names(version)).chain(version.natives().into_keys()));
    // A relation made in one part alone is that part's own: what its passes remove of it goes.
    let mut makers: HashMap<&str, usize> = HashMap::new();
    for p in parts {
        for r in &p.made {
            *makers.entry(r.as_str()).or_default() += 1;
        }
    }
    // The link's fields: the program's, each part's changes merged in (`merge_header`).
    let mut all = Builder::base();
    state_fields(&mut all, version, version, None);
    let mut renames: Vec<HashMap<String, String>> = Vec::new();
    let (mut reaches, mut demanded, mut undone, mut matchers) = ((0, 0), Vec::new(), Vec::new(), Vec::new());
    let mut demand: Option<(Vec<String>, std::result::Result<front::ast::Report, String>)> = None;
    let (mut tests, mut specs, mut matches, mut markers) = (Vec::new(), Vec::new(), Vec::new(), Vec::<String>::new());
    let mut added_inputs: BTreeSet<String> = BTreeSet::new();
    let mut new_inputs: HashMap<String, String> = HashMap::new();
    // The names each part's passes made (kept with the part): one the program has already, or two parts made, is
    // renamed.
    let made_new: Vec<BTreeSet<String>> = prepared.iter().map(|(_, p)| p.made.iter().cloned().collect()).collect();
    let mut times: HashMap<&str, usize> = HashMap::new();
    for names in &made_new {
        for r in names {
            *times.entry(r.as_str()).or_insert(usize::from(known.contains(r))) += 1;
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
    for ((part, (before, p)), j) in parts.iter().zip(prepared).zip(0..) {
        let after: &Version = &p.version;
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
            } else if let Some(d) = after.columns_of(r) {
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
        let owned = |r: &str| makers.get(r) == Some(&1) && part.made.contains(r) && !outputs.iter().any(|o| o == r);
        merge_header(&mut all, &Selected { v: version, own: before }, after, &owned, &rename);
        // Every input of the part: the program's, what its passes added, and what it reads of the others (dropped
        // below where another part makes it, kept where none does: a base relation, `doc`).
        added_inputs.extend(after.input_names().iter().map(|r| rename(r)));
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
    let mut inputs: Vec<String> = version.input_names();
    inputs.extend(added_inputs.into_iter().filter(|r| !inputs.contains(r)).collect::<Vec<_>>());
    inputs.retain(|r| !derived.contains(r));
    all.set_inputs(&inputs);
    all.set_outputs(&outputs);
    let mut b = all;
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
    Ok((linked, Front { reaches, demanded, demand, undone, matchers, vocabulary }, renames))
}

// -- Stage 2: the passes after lowering, per part (decision 0345 stage 2) --

/// What a part's passes after lowering made and found.
struct After {
    /// Whether its cross-file part reads a base relation after the first push: its share of the run's push mode.
    reads_base: bool,
    pushed: Vec<String>,
    factorized: Vec<String>,
    demand: Vec<String>,
    located: std::result::Result<Vec<String>, Vec<String>>,
}

fn after_facts(a: &After, digest: &[u8; 32]) -> Facts {
    let mut f = Facts::new();
    let mut put = |rel: &str, rows: Vec<Vec<String>>| {
        f.insert(rel.to_owned(), rows.into_iter().map(|r| r.iter().map(|c| str(c)).collect::<Vec<_>>().into()).collect());
    };
    let listed = |l: &[String]| l.iter().enumerate().map(|(i, x)| vec![i.to_string(), x.clone()]).collect::<Vec<_>>();
    put("part_program", vec![vec![hex(digest)]]);
    put("part_reads_base", vec![vec![u8::from(a.reads_base).to_string()]]);
    put("part_pushed", listed(&a.pushed));
    put("part_factorized", listed(&a.factorized));
    put("part_export_demand", listed(&a.demand));
    let (ok, located) = match &a.located {
        Ok(l) => ("ok", l),
        Err(l) => ("blocked", l),
    };
    put("part_located_outcome", vec![vec![ok.to_owned()]]);
    put("part_located", listed(located));
    f
}

fn after_of(facts: &Facts) -> After {
    let listed = |rel: &str| ordered(facts, rel).into_iter().map(|r| r[1].clone()).collect::<Vec<_>>();
    let located = listed("part_located");
    After {
        reads_base: rows_of(facts, "part_reads_base").next().is_some_and(|r| r.first().map(String::as_str) == Some("1")),
        pushed: listed("part_pushed"),
        factorized: listed("part_factorized"),
        demand: listed("part_export_demand"),
        located: if rows_of(facts, "part_located_outcome").next().is_some_and(|r| r.first().map(String::as_str) == Some("ok")) { Ok(located) } else { Err(located) },
    }
}

/// The switches the passes after lowering read.
const AFTER_SWITCHES: &[&str] = &["RERAIL_EXPORT_DEMAND"];

/// A part's key after lowering: its key before (its header, now with its interface's split, and its rules), the
/// stage, whether exports are factorized, and the switches those passes read.
fn after_key(before: [u8; 32], factorize: bool) -> [u8; 32] {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(b"rerail/part-after/v1\0");
    h.update(before);
    h.update([u8::from(factorize)]);
    for s in AFTER_SWITCHES {
        h.update(format!("{s}={}\0", std::env::var(s).unwrap_or_default()).as_bytes());
    }
    h.finalize().into()
}

/// The passes after lowering, by part (decision 0345 stage 2): `preparation::run`'s passes, with what reads the whole
/// program (the nearest sets, the bound check, the split, the push's mode, the merges) run over the whole, and the
/// rewriting (push, factorize, the export cut, locate) run per part against its interface, kept under its key.
pub fn run(version: Arc<Version>, factorize: bool, dialect: crate::code_rule_dsl::language::GrammarDialect, store: &ContentStore, keep_parts: bool, lists: &front::Lists, bound: impl FnMut(&Version) -> Result<()>) -> Result<(Arc<Version>, super::Found, Vec<SourceId>)> {
    use crate::code_rule_dsl::logic::plan::fields::{GIVEN, PART_CROSS_READ, PART_LOCAL_READ, PART_OUT, PART_WHOLE, PUSH_BASE, PUSH_DECIDED};
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use crate::code_rule_dsl::typing::file_local::FileLocal;
    // The whole: the nearest sets, the bound check, and the split every part's interface is read from.
    let s_whole = crate::code_rule_dsl::logic::profile::span(|| "the whole before the parts' passes: nearest, bound, split".into());
    let mut p = Pipeline::new("before the parts' passes", version);
    let mut nearest = super::Nearest::default();
    p.rules(&mut nearest);
    p.program(&mut super::Check(bound));
    let v1 = p.current()?.clone();
    let split = p.get::<FileLocal>()?;
    drop(p);
    drop(s_whole);
    let parts = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the parts and their interfaces (decision 0345 stage 2)".into());
        parts_of(&v1)
    };
    let rules: Vec<_> = v1.rules().collect();
    let mut part_of_rule = vec![0usize; rules.len()];
    for (j, part) in parts.iter().enumerate() {
        for &k in &part.rules {
            part_of_rule[k] = j;
        }
    }
    // Who reads each relation: the part, and whether from a file-local rule.
    let local_head = |k: usize| {
        let h = v1.rel_name(rules[k].head().rel());
        split.local.contains(h) || split.constants.contains(h)
    };
    let mut readers: HashMap<&str, Vec<(usize, bool)>> = HashMap::new();
    for (k, r) in rules.iter().enumerate() {
        for a in r.reads() {
            readers.entry(v1.rel_name(a.rel())).or_default().push((part_of_rule[k], local_head(k)));
        }
    }
    let made_by_parts: BTreeSet<&str> = parts.iter().flat_map(|p| p.made.iter().map(String::as_str)).collect();
    let common_digest = common_of(&v1);
    let shared = shared_of(&v1, &made_by_parts);
    let common_digest = with_shared(&common_digest, &v1, &shared);
    let (inputs_shared, outputs) = (v1.input_names().len(), v1.output_names());
    // Each part's own header, with its interface: what it is given of the split, and how its outputs are read.
    let s_heads = crate::code_rule_dsl::logic::profile::span(|| "the parts' headers and interfaces".into());
    let heads: Vec<Own> = parts
        .iter()
        .enumerate()
        .map(|(j, part)| {
            let elsewhere = |r: &str| readers.get(r).into_iter().flatten().filter(move |(o, _)| *o != j);
            let read_elsewhere: BTreeSet<String> = part.made.iter().filter(|r| elsewhere(r).next().is_some()).cloned().collect();
            let mut head = part_header(&v1, part, &read_elsewhere, &made_by_parts, &shared);
            let mut word = |r: &str, w: String| {
                head.words.entry(r.to_owned()).or_default().insert(w);
            };
            for r in part.read.iter().filter(|r| !part.made.contains(*r) && made_by_parts.contains(r.as_str())) {
                word(r, GIVEN.to_owned());
                for (set, w) in [(&split.local, "part:local"), (&split.constants, "part:constant"), (&split.filters, "part:filter")] {
                    if set.contains(r) {
                        word(r, w.to_owned());
                    }
                }
                if let Some(cols) = split.locations.get(r) {
                    word(r, format!("part:loc={}", cols.iter().map(usize::to_string).collect::<Vec<_>>().join(",")));
                }
            }
            for r in &read_elsewhere {
                if !outputs.contains(r) {
                    word(r, PART_OUT.to_owned());
                }
                if elsewhere(r).any(|(_, local)| !*local) {
                    word(r, PART_CROSS_READ.to_owned());
                }
                if elsewhere(r).any(|(_, local)| *local) {
                    word(r, PART_LOCAL_READ.to_owned());
                }
            }
            head
        })
        .collect();
    drop(s_heads);
    let mut sources = Vec::new();
    let (found, made) = (std::cell::Cell::new(0usize), std::cell::Cell::new(0usize));
    // A part's passes, kept under its key: `decided` the run's push mode, where the run has decided it.
    // A part found is its facts until the run's mode is decided: only the image of the mode it runs in is read, and
    // only where the kept link cannot do without it.
    enum Slot {
        Kept(Facts, [u8; 32], [u8; 32]),
        Made(After, Arc<Version>, [u8; 32]),
    }
    let reads_base = |s: &Slot| match s {
        Slot::Kept(facts, _, _) => rows_of(facts, "part_reads_base").next().is_some_and(|r| r.first().map(String::as_str) == Some("1")),
        Slot::Made(a, _, _) => a.reads_base,
    };
    let prepare = |j: usize, decided: Option<bool>, sources: &mut Vec<SourceId>| -> Result<Slot> {
        let mut head = heads[j].clone();
        if let Some(base) = decided {
            let words = head.words.entry(PART_WHOLE.to_owned()).or_default();
            words.insert(PUSH_DECIDED.to_owned());
            if base {
                words.insert(PUSH_BASE.to_owned());
            }
        }
        let key = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a part's key".into());
            after_key(key_of(&Selected { v: &v1, own: &head }, &common_digest, &shared, inputs_shared, &rules, &parts[j], dialect), factorize)
        };
        let kept_part = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a part after lowering, from the store".into());
            kept_facts(store, &key).map_err(|e| e.to_string())?
        };
        if let Some((facts, digest, source)) = kept_part {
            found.set(found.get() + 1);
            sources.push(source);
            return Ok(Slot::Kept(facts, digest, key));
        }
        made.set(made.get() + 1);
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("prepare part {} after lowering ({} rules)", parts[j].name, parts[j].rules.len()));
        let input = program_of(&v1, &rules, &head, &parts[j])?;
        let mut p = Pipeline::new("a part's passes after lowering", input);
        let mut pushed = crate::code_rule_dsl::typing::file_local::PushLocalJoins::default();
        let mut factorized = crate::code_rule_dsl::typing::file_local::FactorizeExports::new(factorize);
        let mut demand = crate::code_rule_dsl::typing::file_local::DemandExports::default();
        let mut locate = crate::code_rule_dsl::typing::file_local::Locate::default();
        crate::code_rule_dsl::logic::profile::quiet(|| {
            p.program(&mut pushed);
            p.program(&mut factorized);
            p.program(&mut demand);
            p.program(&mut locate);
        });
        let v = stripped(p.current()?, &v1)?;
        let a = After { reads_base: pushed.reads_base, pushed: pushed.made, factorized: factorized.lines, demand: demand.lines, located: locate.result() };
        if keep_parts {
            keep(store, &key, &v, |digest| after_facts(&a, digest));
        }
        Ok(Slot::Made(a, v, key))
    };
    // First each part in its own mode; then, where any part's cross-file part reads a base relation, the run's mode
    // is that one, and a part that pushed otherwise pushes again in it.
    let mut slots: Vec<Slot> = (0..parts.len()).map(|j| prepare(j, None, &mut sources)).collect::<Result<_>>()?;
    let base = slots.iter().any(reads_base);
    if base {
        for j in 0..parts.len() {
            if !reads_base(&slots[j]) {
                slots[j] = prepare(j, Some(true), &mut sources)?;
            }
        }
    }
    let keys: Vec<[u8; 32]> = slots.iter().map(|s| match s {
        Slot::Kept(_, _, k) | Slot::Made(_, _, k) => *k,
    }).collect();
    let digests: Vec<Option<[u8; 32]>> = slots.iter().map(|s| match s {
        Slot::Kept(_, d, _) => Some(*d),
        Slot::Made(..) => None,
    }).collect();
    // Each part's version, with the sources its image was read from: a part made here has its own; a kept one's image
    // is read when it is linked.
    let mut versions: Vec<Option<(Arc<Version>, Vec<SourceId>)>> = slots.iter().map(|s| match s {
        Slot::Made(_, v, _) => Some((v.clone(), Vec::new())),
        Slot::Kept(..) => None,
    }).collect();
    let afters: Vec<After> = slots.into_iter().map(|s| match s {
        Slot::Made(a, _, _) => a,
        Slot::Kept(facts, _, _) => after_of(&facts),
    }).collect();
    let image = |j: usize, versions: &mut Vec<Option<(Arc<Version>, Vec<SourceId>)>>| -> Result<()> {
        if versions[j].is_none() {
            let digest = digests[j].ok_or_else(|| "a kept part's image is not named".to_owned())?;
            versions[j] = Some(kept_image(store, &digest, lists).map_err(|e| e.to_string())?.ok_or_else(|| "a kept part's image is missing".to_owned())?);
        }
        Ok(())
    };
    crate::code_rule_dsl::logic::profile::decision("parts after lowering (decision 0345 stage 2)", || format!("{} parts: {} read from the store, {} prepared; push mode {}", parts.len(), found.get(), made.get(), if base { "base" } else { "local" }));
    // The kept link (Q18): the last link after lowering, spliced where only some parts changed and their interfaces did
    // not.
    let family = link_key("after lowering", &common_digest, &outputs, &parts, &[u8::from(factorize), u8::from(base)]);
    let last = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "the last link after lowering, from the store (Q18)".into());
        kept_facts(store, &family).map_err(|e| e.to_string())?
    };
    let mut why = "no link kept".to_owned();
    let mut spliced = None;
    if let Some((facts, digest, source)) = last {
        let kept_slots = link_slots(&facts);
        if kept_slots.len() == parts.len() && kept_slots.iter().zip(&parts).all(|(s, p)| s.name == p.name) {
            let changed: Vec<usize> = (0..parts.len()).filter(|&j| kept_slots[j].key != keys[j]).collect();
            let mut same = true;
            for &j in &changed {
                image(j, &mut versions)?;
                let v = &versions[j].as_ref().expect("its image read").0;
                if interface_after(&v1, &heads[j], &parts[j], v) != kept_slots[j].interface {
                    why = format!("part {}'s interface changed", parts[j].name);
                    same = false;
                    break;
                }
            }
            if same {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "splice the changed parts into the last link after lowering (Q18)".into());
                spliced = splice_after(store, &digest, source, &kept_slots, &changed, &versions, &keys, &heads, &v1, &parts, &family, keep_parts, lists, &mut sources)?.map(|v| (v, changed.len()));
                if spliced.is_none() {
                    why = "the last link's image is missing or its rules miscounted".to_owned();
                }
            }
        } else {
            why = "the parts are not the last link's".to_owned();
        }
    }
    let linked = match spliced {
        Some((v, changed)) => {
            crate::code_rule_dsl::logic::profile::decision("the kept link (Q18)", || format!("after lowering: {changed} of {} parts changed, spliced into the last link", parts.len()));
            v
        }
        None => {
            for j in 0..parts.len() {
                image(j, &mut versions)?;
            }
            crate::code_rule_dsl::logic::profile::decision("the kept link (Q18)", || format!("after lowering: linked again ({why})"));
            let (vs, image_sources): (Vec<Arc<Version>>, Vec<Vec<SourceId>>) = versions.into_iter().map(|v| v.expect("every part's image read")).unzip();
            sources.extend(image_sources.iter().flatten().copied());
            let linked = {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "link the parts after lowering (decision 0345 stage 2)".into());
                link_after(&v1, &parts, &heads, &vs)?
            };
            if keep_parts {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "keep the link after lowering (Q18)".into());
                let slots: Vec<LinkSlot> = (0..parts.len())
                    .map(|j| Ok(LinkSlot { name: parts[j].name.clone(), key: keys[j], interface: interface_after(&v1, &heads[j], &parts[j], &vs[j]), rules: vs[j].rules().count(), sources: image_sources[j].clone() }))
                    .collect::<Result<_>>()?;
                keep(store, &family, &linked, |digest| link_facts(digest, &slots, Facts::new()));
            }
            linked
        }
    };
    // The whole again: the merges (helpers two parts pushed alike are one), the inputs read, the rule order.
    let mut p = Pipeline::new("after the parts' passes", linked);
    let mut merge = super::MergeIdentical::default();
    let mut repeats = super::DropRepeated::default();
    if std::env::var("RERAIL_SAME").is_ok_and(|v| v == "off") {
        p.program(&mut merge);
    } else {
        p.program(&mut repeats);
        p.program(&mut merge);
        p.program(&mut repeats);
    }
    p.program(&mut super::InputsRead);
    if let Ok(order) = std::env::var("RERAIL_RULE_ORDER") {
        p.program(&mut super::RuleOrder(order));
    }
    let out = p.current()?.clone();
    let out = match out.base_bytes() {
        Some(_) => out,
        None => out.fold()?,
    };
    let mut located: std::result::Result<Vec<String>, Vec<String>> = Ok(Vec::new());
    for a in &afters {
        located = match (located, &a.located) {
            (Ok(mut l), Ok(m)) => {
                l.extend(m.iter().cloned());
                Ok(l)
            }
            (Ok(_), Err(b)) => Err(b.clone()),
            (Err(mut l), Err(b)) => {
                l.extend(b.iter().cloned());
                Err(l)
            }
            (Err(l), Ok(_)) => Err(l),
        };
    }
    let cat = |f: fn(&After) -> &Vec<String>| afters.iter().flat_map(|a| f(a).iter().cloned()).collect::<Vec<_>>();
    let found = super::Found {
        sets: nearest.sets,
        pushed: cat(|a| &a.pushed),
        factorized: cat(|a| &a.factorized),
        demand: cat(|a| &a.demand),
        located,
        merged: merge.merged,
        shared: merge.shared,
        repeated: repeats.0,
    };
    Ok((out, found, sources))
}

/// The parts after lowering linked: their rules, and what each part's passes changed of its header, over the
/// whole's header; the interface's words (`part:`) dropped. A relation two parts' passes made alike (a library
/// relation's located form) is one relation, its rules the parts'.
fn link_after(v1: &Version, parts: &[Part], heads: &[Own], versions: &[Arc<Version>]) -> Result<Arc<Version>> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    let mut makers: HashMap<&str, usize> = HashMap::new();
    for p in parts {
        for r in &p.made {
            *makers.entry(r.as_str()).or_default() += 1;
        }
    }
    let outputs = v1.output_names();
    let mut all = Builder::base();
    state_fields(&mut all, v1, v1, None);
    let mut added: BTreeSet<String> = BTreeSet::new();
    let s_headers = crate::code_rule_dsl::logic::profile::span(|| "link after lowering: the parts' headers".into());
    for ((part, before), v) in parts.iter().zip(heads).zip(versions) {
        let owned = |r: &str| makers.get(r) == Some(&1) && part.made.contains(r) && !outputs.iter().any(|o| o == r);
        merge_header(&mut all, &Selected { v: v1, own: before }, &**v, &owned, &|r: &str| r.to_owned());
        added.extend(v.input_names().into_iter().filter(|r| !before.inputs.contains(r)));
    }
    // The interface's words (`part:`) dropped.
    let mut qualifiers = all.qualifiers();
    for words in qualifiers.values_mut() {
        words.retain(|w| !w.starts_with("part:"));
    }
    qualifiers.retain(|r, words| !words.is_empty() && !r.starts_with("part:"));
    all.set_field(Field::Qualifiers(&qualifiers));
    let derived: BTreeSet<&str> = versions.iter().flat_map(|v| v.rules().map(|r| v.rel_name(r.head().rel()))).collect();
    let mut inputs: Vec<String> = v1.input_names();
    inputs.extend(added.into_iter().filter(|r| !inputs.contains(r)).collect::<Vec<_>>());
    inputs.retain(|r| !derived.contains(r.as_str()));
    all.set_inputs(&inputs);
    all.set_outputs(&outputs);
    drop(s_headers);
    let _s_rules = crate::code_rule_dsl::logic::profile::span(|| "link after lowering: the rules".into());
    let mut b = all;
    for v in versions {
        for r in v.rules() {
            b.copy(v, r);
        }
    }
    Ok(b.finish()?.0)
}

/// What of a part after lowering its link reads besides its rules' bodies (`link_after`): its header before and after
/// its passes, what it makes and reads, and the relations its rules derive.
fn interface_after(whole: &Version, head: &Own, part: &Part, v: &Version) -> [u8; 32] {
    use sha2::Digest as _;
    let (head, after) = (Shown(&Selected { v: whole, own: head }), Shown(v));
    let derived: BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).collect();
    sha2::Sha256::digest(format!("{head:?}\0{:?}\0{:?}\0{after:?}\0{derived:?}", part.made, part.read).as_bytes()).into()
}

/// The last link after lowering with the `changed` parts' rules in their places; `None` when its image is missing or
/// its rules are not its slots' count. The sources read are added to `sources`.
#[allow(clippy::too_many_arguments)]
fn splice_after(
    store: &ContentStore,
    digest: &[u8; 32],
    source: SourceId,
    slots: &[LinkSlot],
    changed: &[usize],
    versions: &[Option<(Arc<Version>, Vec<SourceId>)>],
    keys: &[[u8; 32]],
    heads: &[Own],
    whole: &Version,
    parts: &[Part],
    family: &[u8; 32],
    keep_parts: bool,
    lists: &front::Lists,
    sources: &mut Vec<SourceId>,
) -> Result<Option<Arc<Version>>> {
    let Some((last, read)) = kept_image(store, digest, lists).map_err(|e| e.to_string())? else { return Ok(None) };
    let last_rules: Vec<_> = last.rules().collect();
    if slots.iter().map(|s| s.rules).sum::<usize>() != last_rules.len() {
        return Ok(None);
    }
    let changed_version = |j: usize| versions[j].as_ref().filter(|_| changed.contains(&j));
    let linked = if changed.is_empty() {
        last.clone()
    } else {
        let mut b = Builder::derive(&last);
        let mut at = 0;
        for (j, slot) in slots.iter().enumerate() {
            match changed_version(j) {
                Some((v, _)) => {
                    for r in v.rules() {
                        b.copy(v, r);
                    }
                }
                None => {
                    for r in &last_rules[at..at + slot.rules] {
                        b.copy(&last, *r);
                    }
                }
            }
            at += slot.rules;
        }
        b.finish()?.0
    };
    sources.extend(read);
    sources.push(source);
    for (j, slot) in slots.iter().enumerate() {
        match changed_version(j) {
            Some((_, s)) => sources.extend(s.iter().copied()),
            None => sources.extend(slot.sources.iter().copied()),
        }
    }
    if keep_parts && !changed.is_empty() {
        let slots: Vec<LinkSlot> = slots
            .iter()
            .enumerate()
            .map(|(j, s)| match changed_version(j) {
                Some((v, vs)) => Ok(LinkSlot { name: s.name.clone(), key: keys[j], interface: interface_after(whole, &heads[j], &parts[j], v), rules: v.rules().count(), sources: vs.clone() }),
                None => Ok(LinkSlot { name: s.name.clone(), key: s.key, interface: s.interface, rules: s.rules, sources: s.sources.clone() }),
            })
            .collect::<Result<_>>()?;
        keep(store, family, &linked, |digest| link_facts(digest, &slots, Facts::new()));
    }
    Ok(Some(linked))
}
