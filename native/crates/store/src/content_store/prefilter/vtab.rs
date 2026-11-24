//! The prefilter's tables through SQL (t33 steps 6 and 7; user, 2026-11-22: "Keep the SQL, and provide an
//! alternative and equivalent path that allows access to the kept facts via a virtual table"): eponymous tables of
//! the names and columns the store's tables had, read from the kept rows (`super::kept`) where they lie.
//!
//! - `prefilter_sets(tape_id, version, nodes)`
//! - `tape_kind_counts(tape_id, kind, n, size, postings)`
//! - `term_postings(lang, term, tape_id, kind, n, postings)`
//! - `term_coverage(tape_id, kind, bad, residue, missing)`
//! - `kind_size(kind, n, size)`, `term_kind_freq(kind, n)`, `term_node_freq(lang, term, kind, n)`,
//!   `term_freq(lang, term, n)`: the store-wide counts. `term_freq.n` is the (tape, kind) rows that carry the term, as
//!   the table's was.
//! - `vocabulary_term(lang, expression, term)`: a regex's matching terms (`super::vocabulary`), as kept, less those no
//!   tape holds.
//! - `search_index_sets(tape_id, index_version, node_count, complete, has_errors)`: the search index's sets
//!   (`content_store::search_index`).
//! - `name_bindings(name_atom, digest, revision)`, `derivations(output_atom, producer, status, cost)` and
//!   `derivation_inputs(output_atom, ordinal, name_atom, digest)`: the derivation graph (`content_store::derivation`),
//!   one unit a name (a derivation's output); `name_atom` is null for an unnamed input.
//!
//! - `corpus_roots(root_id, path)`, `corpus_manifests(manifest_id, root_id, last_run, last_time, grammar_version,
//!   tape_format, plan_digest, digest, sources, lane, policy)`, `corpus_manifest_plans(manifest_id, plan_digest)`,
//!   `corpus_live(source_id, grammar_version, tape_format)`, `corpus_plans(plan_digest, live)`,
//!   `corpus_plan_relations(plan_digest, relation, columns)`, `corpus_state(singleton, run, history_bytes, due,
//!   archive_due)` and `corpus_decided(singleton, run, time, swept)`: the corpus runs (`content_store::corpus_runs::
//!   kept`), one unit a root, a manifest or a source; `sources` delta-varint ids, as the table's were.
//!
//! `tape_id = ?` (and `kind = ?`, `term = ?`, `lang = ?`) choose the rows; a table without `tape_id` reads every
//! tape, one at a time. SQLite rechecks every constraint. Values are as the store numbers them: a term is the store's
//! atom, as the `atoms` table answers. The space is read as the statement's transaction finds it (`facts_chain`),
//! its files mapped once while the chain stays.
use super::super::kept_facts::{ChainRows, KeptSpace, chain_rows};
use super::kept;
use super::super::derivation;
use rusqlite::{
    Connection, Result, ffi,
    types::Value,
    vtab::{Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection, VTabCursor},
};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Which table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Sets,
    Kinds,
    Terms,
    Covers,
    KindSize,
    TermKindFreq,
    TermNodeFreq,
    TermFreq,
    Vocabulary,
    SearchSets,
    Bindings,
    Derivations,
    Inputs,
    Roots,
    Manifests,
    ManifestPlans,
    Live,
    Plans,
    PlanRelations,
    State,
    Decided,
}

impl Which {
    fn schema(self) -> &'static CStr {
        match self {
            Which::Sets => c"CREATE TABLE x(tape_id INTEGER, version INTEGER, nodes INTEGER)",
            Which::Kinds => c"CREATE TABLE x(tape_id INTEGER, kind INTEGER, n INTEGER, size INTEGER, postings BLOB)",
            Which::Terms => c"CREATE TABLE x(lang INTEGER, term INTEGER, tape_id INTEGER, kind INTEGER, n INTEGER, postings BLOB)",
            Which::Covers => c"CREATE TABLE x(tape_id INTEGER, kind INTEGER, bad INTEGER, residue INTEGER, missing BLOB)",
            Which::KindSize => c"CREATE TABLE x(kind INTEGER, n INTEGER, size INTEGER)",
            Which::TermKindFreq => c"CREATE TABLE x(kind INTEGER, n INTEGER)",
            Which::TermNodeFreq => c"CREATE TABLE x(lang INTEGER, term INTEGER, kind INTEGER, n INTEGER)",
            Which::TermFreq => c"CREATE TABLE x(lang INTEGER, term INTEGER, n INTEGER)",
            Which::Vocabulary => c"CREATE TABLE x(lang INTEGER, expression TEXT, term INTEGER)",
            Which::SearchSets => c"CREATE TABLE x(tape_id INTEGER, index_version INTEGER, node_count INTEGER, complete INTEGER, has_errors INTEGER)",
            Which::Bindings => c"CREATE TABLE x(name_atom INTEGER, digest BLOB, revision INTEGER)",
            Which::Derivations => c"CREATE TABLE x(output_atom INTEGER, producer BLOB, status INTEGER, cost INTEGER)",
            Which::Inputs => c"CREATE TABLE x(output_atom INTEGER, ordinal INTEGER, name_atom INTEGER, digest BLOB)",
            Which::Roots => c"CREATE TABLE x(root_id INTEGER, path TEXT)",
            Which::Manifests => c"CREATE TABLE x(manifest_id INTEGER, root_id INTEGER, last_run INTEGER, last_time INTEGER, grammar_version INTEGER, tape_format INTEGER, plan_digest BLOB, digest BLOB, sources BLOB, lane BLOB, policy BLOB)",
            Which::ManifestPlans => c"CREATE TABLE x(manifest_id INTEGER, plan_digest BLOB)",
            Which::Live => c"CREATE TABLE x(source_id INTEGER, grammar_version INTEGER, tape_format INTEGER)",
            Which::Plans => c"CREATE TABLE x(plan_digest BLOB, live INTEGER)",
            Which::PlanRelations => c"CREATE TABLE x(plan_digest BLOB, relation TEXT, columns TEXT)",
            Which::State => c"CREATE TABLE x(singleton INTEGER, run INTEGER, history_bytes INTEGER, due INTEGER, archive_due INTEGER)",
            Which::Decided => c"CREATE TABLE x(singleton INTEGER, run INTEGER, time INTEGER, swept INTEGER)",
        }
    }

    /// The columns an equality chooses rows by, each with how much it narrows.
    fn keys(self) -> &'static [(c_int, f64)] {
        match self {
            Which::Sets | Which::Covers | Which::SearchSets | Which::Bindings | Which::Derivations | Which::Inputs => &[(0, 10_000.0)],
            Which::Roots | Which::Manifests | Which::ManifestPlans | Which::Live => &[(0, 10_000.0)],
            Which::Plans | Which::PlanRelations | Which::State | Which::Decided => &[],
            Which::Kinds => &[(0, 10_000.0), (1, 100.0)],
            Which::Terms => &[(2, 10_000.0), (1, 1000.0), (0, 2.0)],
            Which::KindSize | Which::TermKindFreq => &[(0, 1000.0)],
            Which::TermNodeFreq => &[(1, 100_000.0), (2, 100.0), (0, 2.0)],
            Which::TermFreq => &[(1, 100_000.0), (0, 2.0)],
            Which::Vocabulary => &[(1, 1000.0), (0, 2.0)],
        }
    }

    fn space(self) -> [u8; 32] {
        match self {
            Which::Vocabulary => super::vocabulary::space(),
            Which::SearchSets => super::super::search_index::space(),
            Which::Bindings | Which::Derivations | Which::Inputs => super::super::derivation::space(),
            Which::Roots | Which::Manifests | Which::ManifestPlans | Which::Live | Which::Plans | Which::PlanRelations | Which::State | Which::Decided => {
                super::super::corpus_runs::kept::space()
            }
            _ => kept::space(),
        }
    }
}

/// The space's files, mapped while its chain stays (a correlated read filters once a row of its outer table).
type Opened = Arc<Mutex<Option<(ChainRows, Arc<KeptSpace>)>>>;

/// Registers the tables on `connection`, a connection to the store at `store`.
pub(in crate::content_store) fn install(connection: &Connection, store: &Path) -> Result<()> {
    const MODULE: Module<Table> = Module::eponymous_only_module();
    for (name, which) in [
        (c"prefilter_sets", Which::Sets),
        (c"tape_kind_counts", Which::Kinds),
        (c"term_postings", Which::Terms),
        (c"term_coverage", Which::Covers),
        (c"kind_size", Which::KindSize),
        (c"term_kind_freq", Which::TermKindFreq),
        (c"term_node_freq", Which::TermNodeFreq),
        (c"term_freq", Which::TermFreq),
        (c"vocabulary_term", Which::Vocabulary),
        (c"search_index_sets", Which::SearchSets),
        (c"name_bindings", Which::Bindings),
        (c"derivations", Which::Derivations),
        (c"derivation_inputs", Which::Inputs),
        (c"corpus_roots", Which::Roots),
        (c"corpus_manifests", Which::Manifests),
        (c"corpus_manifest_plans", Which::ManifestPlans),
        (c"corpus_live", Which::Live),
        (c"corpus_plans", Which::Plans),
        (c"corpus_plan_relations", Which::PlanRelations),
        (c"corpus_state", Which::State),
        (c"corpus_decided", Which::Decided),
    ] {
        connection.create_module(name, &MODULE, Some((store.to_path_buf(), which)))?;
    }
    Ok(())
}

#[repr(C)]
struct Table {
    base: ffi::sqlite3_vtab,
    store: PathBuf,
    which: Which,
    db: *mut ffi::sqlite3,
    opened: Opened,
    /// The counts' space, for `vocabulary_term` (which leaves out a term no tape holds).
    counts: Opened,
}

// SAFETY: SQLite's base is first; the table owns its path, and its handle is the connection SQLite calls it on.
unsafe impl<'a> VTab<'a> for Table {
    type Aux = (PathBuf, Which);
    type Cursor = Cursor;

    fn connect(db: &mut VTabConnection, aux: Option<&Self::Aux>, _: &[u8], _: &[u8], _: &[u8], _: &[&[u8]]) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::Innocuous)?;
        let (store, which) = aux.expect("the store's path").clone();
        // SAFETY: the handle of the connection this table is created on; it outlives the table.
        let handle = unsafe { db.handle() };
        Ok((Cow::Borrowed(which.schema()), Table { base: ffi::sqlite3_vtab::default(), store, which, db: handle, opened: Arc::default(), counts: Arc::default() }))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let keys = self.which.keys();
        let mut found: Vec<Option<usize>> = vec![None; keys.len()];
        for (i, c) in info.constraints().enumerate() {
            if !c.is_usable() || c.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
                continue;
            }
            if let Some(slot) = keys.iter().position(|(col, _)| *col == c.column()) {
                found[slot].get_or_insert(i);
            }
        }
        let (mut flags, mut arg, mut rows) = (0, 1, 1e8f64);
        for (slot, (_, narrows)) in keys.iter().enumerate() {
            if let Some(i) = found[slot] {
                info.constraint_usage(i).set_argv_index(arg);
                arg += 1;
                flags |= 1 << slot;
                rows /= narrows;
            }
        }
        info.set_idx_num(flags);
        info.set_estimated_rows(rows.max(1.0) as i64);
        info.set_estimated_cost(rows.max(1.0));
        Ok(true)
    }

    fn open(&'a mut self) -> Result<Cursor> {
        Ok(Cursor { base: ffi::sqlite3_vtab_cursor::default(), store: self.store.clone(), which: self.which, db: self.db, opened: Arc::clone(&self.opened), counts_opened: Arc::clone(&self.counts), counts: None, space: None, units: Vec::new(), unit: 0, keys: Vec::new(), rows: Vec::new(), row: 0, rowid: 0 })
    }
}

#[repr(C)]
struct Cursor {
    base: ffi::sqlite3_vtab_cursor,
    store: PathBuf,
    which: Which,
    db: *mut ffi::sqlite3,
    opened: Opened,
    counts_opened: Opened,
    counts: Option<Arc<KeptSpace>>,
    space: Option<Arc<KeptSpace>>,
    /// The tapes still to read (a per-tape table), else one unit.
    units: Vec<i64>,
    unit: usize,
    /// The constraints' values, by key slot.
    keys: Vec<Option<Value>>,
    /// The current unit's rows.
    rows: Vec<Vec<Value>>,
    row: usize,
    rowid: i64,
}

/// Space `space` as the statement's transaction finds it, mapped once while its chain stays (`opened`).
fn open_cached(opened: &Opened, connection: &Connection, store: &Path, space: &[u8; 32]) -> Result<Option<Arc<KeptSpace>>> {
    let rows = chain_rows(connection, space).map_err(error)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut opened = opened.lock().expect("the opened space");
    if let Some((at, kept)) = opened.as_ref().filter(|(at, _)| *at == rows) {
        let _ = at;
        return Ok(Some(Arc::clone(kept)));
    }
    let kept = Arc::new(KeptSpace::open(store, space, &rows).map_err(error)?);
    *opened = Some((rows, Arc::clone(&kept)));
    Ok(Some(kept))
}

fn error(e: super::super::StoreError) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(e.to_string().into())
}

fn int(v: &Option<Value>) -> Option<Option<i64>> {
    match v {
        None => Some(None),
        Some(Value::Integer(i)) => Some(Some(*i)),
        Some(_) => None,
    }
}

impl Cursor {
    fn per_tape(&self) -> bool {
        matches!(
            self.which,
            Which::Sets | Which::Kinds | Which::Terms | Which::Covers | Which::SearchSets | Which::Bindings | Which::Derivations | Which::Inputs | Which::Roots | Which::Manifests | Which::ManifestPlans | Which::Live
        )
    }

    /// The key slot's integer constraint: `Some(None)` none given, `None` one that matches nothing.
    fn key(&self, slot: usize) -> Option<Option<i64>> {
        int(self.keys.get(slot).unwrap_or(&None))
    }

    /// The rows of the next units, until some or none are left.
    fn fill(&mut self) {
        self.rows.clear();
        self.row = 0;
        let Some(space) = self.space.clone() else {
            // No runs kept: the state's one row, as a new store's (zeros).
            if self.unit < self.units.len() && matches!(self.which, Which::State | Which::Decided) {
                self.unit = self.units.len();
                self.rows = corpus_rows(self.which, &super::super::corpus_runs::kept::Corpus::none(), 0);
            }
            return;
        };
        let view = space.view();
        while self.rows.is_empty() && self.unit < self.units.len() {
            let unit = self.units[self.unit];
            self.unit += 1;
            self.rows = self.unit_rows(&view, unit);
        }
    }

    fn unit_rows(&self, view: &rerail_blocks::kept::KeptChain<'_>, unit: i64) -> Vec<Vec<Value>> {
        let blob = |t: &kept::Tape<'_>, at: i64, len: i64| Value::Blob(t.bytes(at, len));
        match self.which {
            Which::Roots | Which::Manifests | Which::ManifestPlans | Which::Live | Which::Plans | Which::PlanRelations | Which::State | Which::Decided => {
                corpus_rows(self.which, &super::super::corpus_runs::kept::Corpus::of(view), unit)
            }
            Which::SearchSets => super::super::search_index::set(view, unit).map(|s| vec![std::iter::once(unit).chain(s).map(Value::Integer).collect()]).unwrap_or_default(),
            Which::Bindings => derivation::bound(view, unit).map(|(d, revision)| vec![vec![Value::Integer(unit), Value::Blob(d.to_vec()), Value::Integer(revision)]]).unwrap_or_default(),
            Which::Derivations => derivation::stored(view, unit)
                .map(|d| vec![vec![Value::Integer(unit), Value::Blob(d.producer.to_vec()), Value::Integer(d.status), Value::Integer(d.cost)]])
                .unwrap_or_default(),
            Which::Inputs => derivation::stored(view, unit)
                .map(|d| {
                    d.inputs
                        .iter()
                        .enumerate()
                        .map(|(o, (name, digest))| vec![Value::Integer(unit), Value::Integer(o as i64), name.map_or(Value::Null, |n| Value::Integer(i64::from(n))), Value::Blob(digest.to_vec())])
                        .collect()
                })
                .unwrap_or_default(),
            Which::Sets => kept::tape(view, unit).map(|t| vec![vec![Value::Integer(unit), Value::Integer(t.version), Value::Integer(t.nodes)]]).unwrap_or_default(),
            Which::Kinds => {
                let Some(t) = kept::tape(view, unit) else { return Vec::new() };
                let Some(kind) = self.key(1) else { return Vec::new() };
                let rows: Vec<kept::KindRow> = match kind {
                    Some(k) => t.kind(k).into_iter().collect(),
                    None => t.kinds().collect(),
                };
                rows.into_iter().map(|k| vec![Value::Integer(unit), Value::Integer(k.kind), Value::Integer(k.n), Value::Integer(k.size), blob(&t, k.at, k.len)]).collect()
            }
            Which::Terms => {
                let Some(t) = kept::tape(view, unit) else { return Vec::new() };
                let (Some(term), Some(lang)) = (self.key(1), self.key(2)) else { return Vec::new() };
                if lang.is_some_and(|l| l != t.lang) {
                    return Vec::new();
                }
                let rows: Vec<kept::TermRow> = match term {
                    Some(term) => t.term(term).collect(),
                    None => t.terms().collect(),
                };
                rows.into_iter().map(|r| vec![Value::Integer(t.lang), Value::Integer(r.term), Value::Integer(unit), Value::Integer(r.kind), Value::Integer(r.n), blob(&t, r.at, r.len)]).collect()
            }
            Which::Covers => {
                let Some(t) = kept::tape(view, unit) else { return Vec::new() };
                t.covers().map(|c| vec![Value::Integer(unit), Value::Integer(c.kind), Value::Integer(c.bad), Value::Integer(c.residue), blob(&t, c.at, c.len)]).collect()
            }
            Which::KindSize | Which::TermKindFreq => {
                let sums = kept::sums(view);
                let Some(kind) = self.key(0) else { return Vec::new() };
                let kinds: Vec<(i64, kept::KindSum)> = match kind {
                    Some(k) => vec![(k, sums.kind(k))],
                    None => sums.kinds(),
                };
                kinds
                    .into_iter()
                    .filter_map(|(k, s)| match self.which {
                        Which::KindSize => (s.n != 0).then(|| vec![Value::Integer(k), Value::Integer(s.n), Value::Integer(s.size)]),
                        _ => (s.covered != 0).then(|| vec![Value::Integer(k), Value::Integer(s.covered)]),
                    })
                    .collect()
            }
            Which::TermNodeFreq => {
                let sums = kept::sums(view);
                let (Some(term), Some(kind), Some(lang)) = (self.key(0), self.key(1), self.key(2)) else { return Vec::new() };
                match (lang, term, kind) {
                    (Some(l), Some(t), Some(k)) => {
                        let n = sums.term_nodes(l, t, k);
                        if n == 0 { Vec::new() } else { vec![vec![Value::Integer(l), Value::Integer(t), Value::Integer(k), Value::Integer(n)]] }
                    }
                    _ => sums
                        .term_rows(lang)
                        .into_iter()
                        .filter(|r| term.is_none_or(|t| r[1] == t) && kind.is_none_or(|k| r[2] == k))
                        .map(|r| r[..4].iter().copied().map(Value::Integer).collect())
                        .collect(),
                }
            }
            Which::TermFreq => {
                let sums = kept::sums(view);
                let (Some(term), Some(lang)) = (self.key(0), self.key(1)) else { return Vec::new() };
                match (lang, term) {
                    (Some(l), Some(t)) => {
                        let n = sums.term_tape_rows(l, t);
                        if n == 0 { Vec::new() } else { vec![vec![Value::Integer(l), Value::Integer(t), Value::Integer(n)]] }
                    }
                    _ => {
                        let mut by_term: std::collections::BTreeMap<(i64, i64), i64> = std::collections::BTreeMap::new();
                        for r in sums.term_rows(lang).into_iter().filter(|r| term.is_none_or(|t| r[1] == t)) {
                            *by_term.entry((r[0], r[1])).or_default() += r[4];
                        }
                        by_term.into_iter().filter(|(_, n)| *n != 0).map(|((l, t), n)| vec![Value::Integer(l), Value::Integer(t), Value::Integer(n)]).collect()
                    }
                }
            }
            Which::Vocabulary => {
                let Some(lang) = self.key(1) else { return Vec::new() };
                let expression = match self.keys.first().cloned().flatten() {
                    None => None,
                    Some(Value::Text(e)) => Some(e),
                    Some(_) => return Vec::new(),
                };
                let counts = self.counts.as_ref().map(|c| c.view());
                let sums = counts.as_ref().map(|v| kept::sums(v));
                let held = |l: i64, t: i64| sums.as_ref().is_some_and(|s| s.has_term(l, t));
                let keys: Vec<(i64, String, Vec<i64>)> = match (lang, expression) {
                    (Some(l), Some(e)) => super::vocabulary::kept_terms(view, super::vocabulary::REGEX, l, &e).map(|t| vec![(l, e, t)]).unwrap_or_default(),
                    (lang, e) => super::vocabulary::kept_all(view, super::vocabulary::REGEX)
                        .into_iter()
                        .filter(|(l, k, _)| lang.is_none_or(|lang| lang == *l) && e.as_ref().is_none_or(|e| e == k))
                        .collect(),
                };
                keys.into_iter()
                    .flat_map(|(l, e, terms)| terms.into_iter().filter(move |&t| held(l, t)).map(move |t| vec![Value::Integer(l), Value::Text(e.clone()), Value::Integer(t)]).collect::<Vec<_>>())
                    .collect()
            }
        }
    }
}

/// A corpus table's rows of `unit` (a root, a manifest or a source; 0 for the others), as the table had them.
fn corpus_rows(which: Which, c: &super::super::corpus_runs::kept::Corpus<'_>, unit: i64) -> Vec<Vec<Value>> {
    let blob = |d: &[u8; 32]| Value::Blob(d.to_vec());
    match which {
        Which::Roots => c.roots().into_iter().filter(|r| r.0 == unit).map(|(id, path)| vec![Value::Integer(id), Value::Text(path)]).collect(),
        Which::Manifests => c
            .manifest(unit)
            .map(|m| {
                vec![vec![
                    Value::Integer(m.id),
                    Value::Integer(m.root),
                    Value::Integer(m.last_run),
                    Value::Integer(m.last_time),
                    Value::Integer(m.grammar_version),
                    Value::Integer(m.tape_format),
                    blob(&m.plan),
                    blob(&m.digest),
                    Value::Blob(super::super::corpus_runs::encode_ids(&c.sources(m.id))),
                    m.lane.as_ref().map_or(Value::Null, blob),
                    m.policy.as_ref().map_or(Value::Null, blob),
                ]]
            })
            .unwrap_or_default(),
        Which::ManifestPlans => c.named(unit).iter().map(|p| vec![Value::Integer(unit), blob(p)]).collect(),
        Which::Live => c.live_of(unit).into_iter().map(|(g, f)| vec![Value::Integer(unit), Value::Integer(g), Value::Integer(f)]).collect(),
        Which::Plans => c.plans().into_iter().map(|(p, live)| vec![blob(&p), Value::Integer(i64::from(live))]).collect(),
        Which::PlanRelations => c
            .plans()
            .into_iter()
            .flat_map(|(p, _)| c.relations(&p).into_iter().map(move |(r, cols)| vec![blob(&p), Value::Text(r), Value::Text(cols)]))
            .collect(),
        Which::State => {
            let s = c.state();
            vec![[1, s.run, s.history_bytes, s.due, s.archive_due].into_iter().map(Value::Integer).collect()]
        }
        Which::Decided => {
            let s = c.state();
            vec![[1, s.decided_run, s.decided_time, s.swept].into_iter().map(Value::Integer).collect()]
        }
        _ => Vec::new(),
    }
}

// SAFETY: SQLite's base is first; the cursor owns its rows and holds its space mapped.
unsafe impl VTabCursor for Cursor {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        let slots = self.which.keys().len();
        let mut arg = 0;
        self.keys = (0..slots)
            .map(|slot| {
                if idx_num & (1 << slot) == 0 {
                    return Ok(None);
                }
                arg += 1;
                args.get::<Value>(arg - 1).map(Some)
            })
            .collect::<Result<Vec<_>>>()?;
        // SAFETY: the connection SQLite calls this filter on; not closed by the wrapper.
        let connection = unsafe { Connection::from_handle(self.db) }?;
        self.space = open_cached(&self.opened, &connection, &self.store, &self.which.space())?;
        self.counts = if self.which == Which::Vocabulary { open_cached(&self.counts_opened, &connection, &self.store, &kept::space())? } else { None };
        self.units = match (self.per_tape(), self.space.as_ref()) {
            (false, None) if matches!(self.which, Which::State | Which::Decided) => vec![0],
            (_, None) => Vec::new(),
            (false, Some(_)) => vec![0],
            (true, Some(space)) => {
                // The tape's slot: `tape_id` leads every per-tape table's keys.
                match self.key(0) {
                    None => Vec::new(),
                    Some(Some(tape)) => vec![tape],
                    Some(None) if self.which == Which::SearchSets => super::super::search_index::tapes(&space.view()).collect(),
                    Some(None) if self.which == Which::Bindings => derivation::names(&space.view()).collect(),
                    Some(None) if matches!(self.which, Which::Derivations | Which::Inputs) => derivation::outputs(&space.view()).collect(),
                    Some(None) if self.which == Which::Roots => super::super::corpus_runs::kept::Corpus::of(&space.view()).roots().into_iter().map(|r| r.0).collect(),
                    Some(None) if matches!(self.which, Which::Manifests | Which::ManifestPlans) => super::super::corpus_runs::kept::Corpus::of(&space.view()).manifest_ids(),
                    Some(None) if self.which == Which::Live => super::super::corpus_runs::kept::Corpus::of(&space.view()).live_sources(),
                    Some(None) => kept::tapes(&space.view()).collect(),
                }
            }
        };
        self.unit = 0;
        self.rowid = 0;
        self.fill();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.row += 1;
        self.rowid += 1;
        if self.row >= self.rows.len() {
            self.fill();
        }
        Ok(())
    }

    fn eof(&self) -> bool {
        self.row >= self.rows.len()
    }

    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        match self.rows[self.row].get(column as usize) {
            Some(v) => ctx.set_result(v),
            None => ctx.set_result(&rusqlite::types::Null),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.rowid)
    }
}
