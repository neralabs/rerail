//! The corpus runs as kept facts of the store (t33 step 3, the manifest tables; user, 2026-11-23: "move them in
//! stages: write both forms first, check collection agrees on every test store, then switch the readers, then drop
//! the tables"). The eight `corpus_*` tables are gone: every writer writes this space, in its write's transaction
//! (`Write`, `write_with`), every reader reads it as its connection finds it (`Corpus`, `with_corpus`), and SQL reads
//! it through virtual tables of the tables' names and columns (`prefilter::vtab`). A store that kept its runs in the
//! tables has them written here by the schema's upgrade, before they are dropped (`from_tables`).
//!
//! One relation a table, keyed as the table is:
//!
//! - `corpus_root`, a partition a root (`root_id`): its path, as bytes (`BlockFlat::of_bytes`).
//! - `corpus_manifest`, a partition a manifest (`manifest_id`): one row, `root_id, last_run, last_time,
//!   grammar_version, tape_format`, the plan and the digest (four words each), then whether it has a lane and the lane,
//!   whether it has a policy and the policy.
//! - `corpus_manifest_source`, a partition a manifest: the sources it read, one a row, ascending.
//! - `corpus_manifest_plan`, a partition a manifest: the other plans it names, four words a row, ascending.
//! - `corpus_live`, a partition a source: its `(grammar_version, tape_format)` pairs, ascending.
//! - `corpus_plan`, a plan's own entry (its identity the plan, partition 0): one row, `live`.
//! - `corpus_plan_relation`, a plan's own entry: its relations and their columns, as bytes, each `relation\0columns\0`,
//!   by relation.
//! - `corpus_state`, partition 0: `run, history_bytes, due, archive_due`, then `corpus_decided`'s `run, time, swept`;
//!   none read as zeros (a new store's).
//!
//! A digest is four words, little-endian.

use super::super::kept_facts::{BlockFlat, ChainRows, KeptPut, KeptSpace, KeptWhat, sorted_put, write_space};
use super::super::{ContentStore, StoreError, StoreResult};
use rerail_blocks::kept::{FactKey, Kept, KeptRead};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

const ROOT: &str = "corpus_root";
const MANIFEST: &str = "corpus_manifest";
const SOURCE: &str = "corpus_manifest_source";
const NAMED: &str = "corpus_manifest_plan";
const LIVE: &str = "corpus_live";
const PLAN: &str = "corpus_plan";
const RELATIONS: &str = "corpus_plan_relation";
const STATE: &str = "corpus_state";

/// The space.
pub fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.corpus-runs/v1").into()
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.corpus-runs/v1\0{rel}").as_bytes()).into()
}

fn words(d: &[u8]) -> [i64; 4] {
    std::array::from_fn(|i| d.get(i * 8..i * 8 + 8).map_or(0, |w| i64::from_le_bytes(w.try_into().expect("eight bytes"))))
}

fn digest(d: &[u8]) -> StoreResult<[u8; 32]> {
    d.try_into().map_err(|_| StoreError::Corrupt("a corpus digest is not 32 bytes".into()))
}

/// One partition as the tables have it: its key, its rows (flat, `width` a row; none: gone), how they are kept.
struct Want {
    rel: &'static str,
    ident: [u8; 32],
    part: i64,
    width: usize,
    rows: Vec<i64>,
    /// Bytes, kept as given (`of_bytes`), else rows in canonical order.
    bytes: bool,
}

impl Want {
    fn sorted(rel: &'static str, ident: [u8; 32], part: i64, width: usize, mut rows: Vec<i64>) -> Want {
        let mut set: Vec<&[i64]> = rows.chunks_exact(width).collect();
        set.sort_unstable();
        set.dedup();
        rows = set.concat();
        Want { rel, ident, part, width, rows, bytes: false }
    }

    fn text(rel: &'static str, ident: [u8; 32], part: i64, text: Option<&[u8]>) -> Want {
        // As `BlockFlat::of_bytes` lays them out: the length, then the bytes, 8 a word, little-endian.
        let rows = text
            .map(|t| std::iter::once(t.len() as i64).chain(t.chunks(8).map(|c| {
                let mut w = [0u8; 8];
                w[..c.len()].copy_from_slice(c);
                i64::from_le_bytes(w)
            })).collect())
            .unwrap_or_default();
        Want { rel, ident, part, width: 1, rows, bytes: true }
    }

    /// Whether `kept` holds exactly these rows.
    fn kept_as(&self, kept: Option<Kept<'_>>) -> bool {
        let Some(k) = kept else { return self.rows.is_empty() };
        if k.rows.len() * self.width != self.rows.len() || k.rows.arity as usize != self.width {
            return false;
        }
        let mut at = 0;
        let mut same = true;
        k.each_row(|row| {
            same &= *row == self.rows[at..at + self.width];
            at += self.width;
        });
        same
    }

    fn put(self) -> StoreResult<KeptPut> {
        let n = self.rows.len() / self.width;
        if self.bytes && n > 0 {
            let h = Sha256::digest(self.rows.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            return Ok(KeptPut {
                rel: self.rel.to_owned(),
                ident: self.ident,
                part: self.part,
                digest: u64::from_le_bytes(h[..8].try_into().expect("8 bytes")),
                reads: Vec::new(),
                types: b"i".to_vec(),
                what: KeptWhat::Picked(Box::new(BlockFlat::of_rows(&self.rows, 1, true))),
            });
        }
        let types = vec![b'i'; self.width];
        sorted_put(self.rel, self.ident, self.part, &types, n, |push| -> StoreResult<()> {
            self.rows.iter().for_each(|v| push(*v));
            Ok(())
        })
    }
}

fn root_want(t: &Connection, root: i64) -> StoreResult<Want> {
    let path: Option<String> = t.query_row("SELECT path FROM corpus_roots WHERE root_id=?1", [root], |r| r.get(0)).optional()?;
    Ok(Want::text(ROOT, ident(ROOT), root, path.as_deref().map(str::as_bytes)))
}

fn manifest_wants(t: &Connection, manifest: i64) -> StoreResult<[Want; 3]> {
    type Row = (i64, i64, i64, i64, i64, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);
    let row: Option<Row> = t
        .query_row(
            "SELECT root_id, last_run, last_time, grammar_version, tape_format, plan_digest, digest, sources, lane, policy \
             FROM corpus_manifests WHERE manifest_id=?1",
            [manifest],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
        )
        .optional()?;
    let (mut m, mut sources) = (Vec::new(), Vec::new());
    if let Some((root, last_run, last_time, grammar, format, plan, digest, blob, lane, policy)) = row {
        m.extend([root, last_run, last_time, grammar, format]);
        m.extend(words(&plan));
        m.extend(words(&digest));
        for d in [lane, policy] {
            m.push(i64::from(d.is_some()));
            m.extend(words(d.as_deref().unwrap_or(&[])));
        }
        sources = super::decode_ids(&blob)?;
    }
    let named: Vec<i64> = t
        .prepare_cached("SELECT plan_digest FROM corpus_manifest_plans WHERE manifest_id=?1")?
        .query_map([manifest], |r| r.get::<_, Vec<u8>>(0))?
        .map(|p| p.map(|p| words(&p)))
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    Ok([
        Want::sorted(MANIFEST, ident(MANIFEST), manifest, 23, m),
        Want::sorted(SOURCE, ident(SOURCE), manifest, 1, sources),
        Want::sorted(NAMED, ident(NAMED), manifest, 4, named),
    ])
}

fn source_want(t: &Connection, source: i64) -> StoreResult<Want> {
    let rows: Vec<i64> = t
        .prepare_cached("SELECT grammar_version, tape_format FROM corpus_live WHERE source_id=?1")?
        .query_map([source], |r| Ok([r.get::<_, i64>(0)?, r.get(1)?]))?
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    Ok(Want::sorted(LIVE, ident(LIVE), source, 2, rows))
}

fn plan_wants(t: &Connection, plan: &[u8; 32]) -> StoreResult<[Want; 2]> {
    let live: Option<i64> = t.query_row("SELECT live FROM corpus_plans WHERE plan_digest=?1", [&plan[..]], |r| r.get(0)).optional()?;
    let relations: Vec<(String, String)> = t
        .prepare_cached("SELECT relation, columns FROM corpus_plan_relations WHERE plan_digest=?1 ORDER BY relation")?
        .query_map([&plan[..]], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let text: Vec<u8> = relations.iter().flat_map(|(r, c)| [r.as_bytes(), b"\0", c.as_bytes(), b"\0"].concat()).collect();
    Ok([
        Want::sorted(PLAN, *plan, 0, 1, live.into_iter().collect()),
        Want::text(RELATIONS, *plan, 0, (!relations.is_empty()).then_some(&text[..])),
    ])
}

fn state_want(t: &Connection) -> StoreResult<Want> {
    let state: [i64; 4] = t.query_row("SELECT run, history_bytes, due, archive_due FROM corpus_state WHERE singleton=1", [], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]))?;
    let decided: [i64; 3] = t
        .query_row("SELECT run, time, swept FROM corpus_decided WHERE singleton=1", [], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))
        .optional()?
        .unwrap_or_default();
    Ok(Want::sorted(STATE, ident(STATE), 0, 7, [&state[..], &decided[..]].concat()))
}

fn ids(t: &Connection, sql: &str) -> StoreResult<BTreeSet<i64>> {
    Ok(t.prepare(sql)?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

/// Every partition of either form, as the tables have them (an old store's, `from_tables`).
fn wants(t: &Connection, view: Option<&dyn KeptRead>) -> StoreResult<Vec<Want>> {
    let mut roots = ids(t, "SELECT root_id FROM corpus_roots")?;
    let mut manifests = ids(t, "SELECT manifest_id FROM corpus_manifests")?;
    let mut sources = ids(t, "SELECT DISTINCT source_id FROM corpus_live")?;
    let mut plans: BTreeSet<[u8; 32]> = BTreeSet::new();
    for sql in ["SELECT plan_digest FROM corpus_plans", "SELECT DISTINCT plan_digest FROM corpus_plan_relations"] {
        for p in t.prepare(sql)?.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
            plans.insert(digest(&p?)?);
        }
    }
    if let Some(view) = view {
        roots.extend(view.of(ROOT).map(|e| e.key.part));
        manifests.extend([MANIFEST, SOURCE, NAMED].iter().flat_map(|rel| view.of(rel).map(|e| e.key.part).collect::<Vec<_>>()));
        sources.extend(view.of(LIVE).map(|e| e.key.part));
        plans.extend([PLAN, RELATIONS].iter().flat_map(|rel| view.of(rel).map(|e| *e.key.ident).collect::<Vec<_>>()));
    }
    let mut out = vec![state_want(t)?];
    for root in roots {
        out.push(root_want(t, root)?);
    }
    for manifest in manifests {
        out.extend(manifest_wants(t, manifest)?);
    }
    for source in sources {
        out.push(source_want(t, source)?);
    }
    for plan in &plans {
        out.extend(plan_wants(t, plan)?);
    }
    Ok(out)
}

fn open(store: &Path, t: &Connection) -> StoreResult<Option<KeptSpace>> {
    KeptSpace::read(store, t, &space())
}

fn kept<'v>(view: Option<&'v dyn KeptRead>, w: &Want) -> Option<Kept<'v>> {
    view?.get(&FactKey { rel: w.rel, ident: &w.ident, part: w.part }).filter(|e| *e.key.ident == w.ident)
}

/// The tables the kept form replaced (`schema::RETIRED_TABLES`), each a relation of the space.
pub(in crate::content_store) const TABLES: [&str; 8] =
    ["corpus_roots", "corpus_manifests", "corpus_manifest_plans", "corpus_live", "corpus_plans", "corpus_plan_relations", "corpus_state", "corpus_decided"];

/// An old store's runs, from the tables they were kept in before their kept form, into it (in `t`, the schema's
/// upgrade, before the tables are dropped): every partition that differs. Nothing where the store has no such tables.
pub(in crate::content_store) fn from_tables(store: &Path, t: &Connection) -> StoreResult<()> {
    let held: i64 = t.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name IN (SELECT value FROM json_each(?1))",
        [serde_json::to_string(&TABLES).expect("names")],
        |r| r.get(0),
    )?;
    if held != TABLES.len() as i64 {
        return Ok(());
    }
    let old = open(store, t)?;
    let view = old.as_ref().map(KeptSpace::view);
    let view = view.as_ref().map(|v| v as &dyn KeptRead);
    let puts = wants(t, view)?.into_iter().filter(|w| !w.kept_as(kept(view, w))).map(Want::put).collect::<StoreResult<Vec<_>>>()?;
    drop(old);
    if !puts.is_empty() {
        write_space(store, t, &space(), puts, None)?;
    }
    Ok(())
}

fn of_words(row: &[i64]) -> [u8; 32] {
    let mut d = [0u8; 32];
    for (i, w) in row.iter().take(4).enumerate() {
        d[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    d
}

/// An entry's bytes (`of_bytes`'s layout: the length, then 8 a word).
fn bytes_of(e: &Kept<'_>) -> Vec<u8> {
    let words: Vec<i64> = (0..e.rows.len()).map(|r| e.rows.get(r, 0)).collect();
    let Some((&len, rest)) = words.split_first() else { return Vec::new() };
    let mut out: Vec<u8> = rest.iter().flat_map(|w| w.to_le_bytes()).collect();
    out.truncate(len.max(0) as usize);
    out
}

/// A manifest as the kept form holds it (`corpus_manifest`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::content_store) struct KeptManifest {
    pub id: i64,
    pub root: i64,
    pub last_run: i64,
    pub last_time: i64,
    pub grammar_version: i64,
    pub tape_format: i64,
    pub plan: [u8; 32],
    pub digest: [u8; 32],
    pub lane: Option<[u8; 32]>,
    pub policy: Option<[u8; 32]>,
}

/// `corpus_state`'s and `corpus_decided`'s values (zeros where never mirrored, as a new store's).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::content_store) struct KeptState {
    pub run: i64,
    pub history_bytes: i64,
    pub due: i64,
    pub archive_due: i64,
    pub decided_run: i64,
    pub decided_time: i64,
    pub swept: i64,
}

/// The corpus runs' kept form, as a transaction finds it (none: a store that keeps none).
pub(in crate::content_store) struct Corpus<'v>(Option<&'v dyn KeptRead>);

impl<'v> Corpus<'v> {
    /// The kept form `view` holds (the space's).
    pub fn of(view: &'v dyn KeptRead) -> Corpus<'v> {
        Corpus(Some(view))
    }

    /// No runs kept (a store with none).
    pub fn none() -> Corpus<'v> {
        Corpus(None)
    }

    fn entry(&self, rel: &str, ident: &[u8; 32], part: i64) -> Option<Kept<'_>> {
        self.0?.get(&FactKey { rel, ident, part }).filter(|e| e.key.ident == ident)
    }

    fn all_of(&self, rel: &'static str) -> Vec<Kept<'_>> {
        let id = ident(rel);
        self.0.map(|v| v.of(rel).filter(|e| *e.key.ident == id).collect()).unwrap_or_default()
    }

    /// Every root, by id, with its path.
    pub fn roots(&self) -> Vec<(i64, String)> {
        self.all_of(ROOT).iter().map(|e| (e.key.part, String::from_utf8_lossy(&bytes_of(e)).into_owned())).collect()
    }

    /// The root of `path`.
    pub fn root(&self, path: &str) -> Option<i64> {
        self.all_of(ROOT).iter().find(|e| bytes_of(e) == path.as_bytes()).map(|e| e.key.part)
    }

    fn manifest_of(e: &Kept<'_>) -> Option<KeptManifest> {
        if e.rows.len() != 1 || e.rows.arity != 23 {
            return None;
        }
        let r = e.row(0);
        let opt = |at: usize| (r[at] != 0).then(|| of_words(&r[at + 1..at + 5]));
        Some(KeptManifest {
            id: e.key.part,
            root: r[0],
            last_run: r[1],
            last_time: r[2],
            grammar_version: r[3],
            tape_format: r[4],
            plan: of_words(&r[5..9]),
            digest: of_words(&r[9..13]),
            lane: opt(13),
            policy: opt(18),
        })
    }

    /// Every manifest, newest run first.
    pub fn manifests(&self) -> Vec<KeptManifest> {
        let mut out: Vec<KeptManifest> = self.all_of(MANIFEST).iter().filter_map(Self::manifest_of).collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.last_run));
        out
    }

    /// Manifest `id`.
    pub fn manifest(&self, id: i64) -> Option<KeptManifest> {
        self.entry(MANIFEST, &ident(MANIFEST), id).as_ref().and_then(Self::manifest_of)
    }

    /// Every manifest's id, ascending.
    pub fn manifest_ids(&self) -> Vec<i64> {
        self.all_of(MANIFEST).iter().map(|e| e.key.part).collect()
    }

    /// Every source `corpus_live` holds, ascending.
    pub fn live_sources(&self) -> Vec<i64> {
        self.all_of(LIVE).iter().map(|e| e.key.part).collect()
    }

    /// Source `source`'s live tapes' `(grammar_version, tape_format)`.
    pub fn live_of(&self, source: i64) -> Vec<(i64, i64)> {
        self.entry(LIVE, &ident(LIVE), source).map(|e| (0..e.rows.len()).map(|r| (e.rows.get(r, 0), e.rows.get(r, 1))).collect()).unwrap_or_default()
    }

    /// `root`'s manifests, newest run first.
    pub fn manifests_of(&self, root: i64) -> Vec<KeptManifest> {
        self.manifests().into_iter().filter(|m| m.root == root).collect()
    }

    /// The sources manifest `manifest` read, ascending.
    pub fn sources(&self, manifest: i64) -> Vec<i64> {
        self.entry(SOURCE, &ident(SOURCE), manifest).map(|e| (0..e.rows.len()).map(|r| e.rows.get(r, 0)).collect()).unwrap_or_default()
    }

    /// The other plans manifest `manifest` names, ascending.
    pub fn named(&self, manifest: i64) -> Vec<[u8; 32]> {
        self.entry(NAMED, &ident(NAMED), manifest).map(|e| (0..e.rows.len()).map(|r| of_words(&e.row(r))).collect()).unwrap_or_default()
    }

    /// Every recorded plan, with whether it is live.
    pub fn plans(&self) -> Vec<([u8; 32], bool)> {
        self.0
            .map(|v| v.of(PLAN).filter(|e| e.rows.len() == 1 && e.key.part == 0).map(|e| (*e.key.ident, e.rows.get(0, 0) != 0)).collect())
            .unwrap_or_default()
    }

    /// `plan`'s relations and their columns (JSON), by relation.
    pub fn relations(&self, plan: &[u8; 32]) -> Vec<(String, String)> {
        let Some(e) = self.entry(RELATIONS, plan, 0) else { return Vec::new() };
        let text = bytes_of(&e);
        let parts: Vec<&[u8]> = text.split(|b| *b == 0).collect();
        parts.chunks_exact(2).map(|p| (String::from_utf8_lossy(p[0]).into_owned(), String::from_utf8_lossy(p[1]).into_owned())).collect()
    }

    pub fn state(&self) -> KeptState {
        let Some(e) = self.entry(STATE, &ident(STATE), 0).filter(|e| e.rows.len() == 1) else { return KeptState::default() };
        let r = e.row(0);
        KeptState { run: r[0], history_bytes: r[1], due: r[2], archive_due: r[3], decided_run: r[4], decided_time: r[5], swept: r[6] }
    }
}

/// A write of the corpus runs' kept form, in a write's transaction: each partition it names replaced whole by what
/// it gives (none: gone), as the tables' rows were replaced by their statements.
#[derive(Default)]
pub(in crate::content_store) struct Write {
    puts: std::collections::BTreeMap<(&'static str, [u8; 32], i64), Want>,
}

impl Write {
    fn put(&mut self, w: Want) {
        self.puts.insert((w.rel, w.ident, w.part), w);
    }

    /// Root `id`'s path (none: the root gone).
    pub fn root(&mut self, id: i64, path: Option<&str>) {
        self.put(Want::text(ROOT, ident(ROOT), id, path.map(str::as_bytes)));
    }

    /// Manifest `m` (by its id), or manifest `id` gone with its sources and named plans.
    pub fn manifest(&mut self, id: i64, m: Option<&KeptManifest>) {
        let mut row = Vec::new();
        if let Some(m) = m {
            row.extend([m.root, m.last_run, m.last_time, m.grammar_version, m.tape_format]);
            row.extend(words(&m.plan));
            row.extend(words(&m.digest));
            for d in [m.lane, m.policy] {
                row.push(i64::from(d.is_some()));
                row.extend(words(&d.unwrap_or([0; 32])));
            }
        } else {
            self.sources(id, &[]);
            self.named(id, &[]);
        }
        self.put(Want::sorted(MANIFEST, ident(MANIFEST), id, 23, row));
    }

    /// The sources manifest `id` read.
    pub fn sources(&mut self, id: i64, sources: &[i64]) {
        self.put(Want::sorted(SOURCE, ident(SOURCE), id, 1, sources.to_vec()));
    }

    /// The other plans manifest `id` names.
    pub fn named(&mut self, id: i64, plans: &[[u8; 32]]) {
        self.put(Want::sorted(NAMED, ident(NAMED), id, 4, plans.iter().flat_map(|p| words(p)).collect()));
    }

    /// Source `source`'s live tapes.
    pub fn live(&mut self, source: i64, tapes: &[(i64, i64)]) {
        self.put(Want::sorted(LIVE, ident(LIVE), source, 2, tapes.iter().flat_map(|&(g, f)| [g, f]).collect()));
    }

    /// Plan `plan`, live or dead (none: forgotten, with its relations).
    pub fn plan(&mut self, plan: [u8; 32], live: Option<bool>) {
        if live.is_none() {
            self.relations(plan, &[]);
        }
        self.put(Want::sorted(PLAN, plan, 0, 1, live.map(i64::from).into_iter().collect()));
    }

    /// Plan `plan`'s relations and their columns.
    pub fn relations(&mut self, plan: [u8; 32], relations: &[(String, String)]) {
        let mut sorted: Vec<&(String, String)> = relations.iter().collect();
        sorted.sort();
        let text: Vec<u8> = sorted.iter().flat_map(|(r, c)| [r.as_bytes(), b"\0", c.as_bytes(), b"\0"].concat()).collect();
        self.put(Want::text(RELATIONS, plan, 0, (!relations.is_empty()).then_some(&text[..])));
    }

    pub fn state(&mut self, s: KeptState) {
        self.put(Want::sorted(STATE, ident(STATE), 0, 7, vec![s.run, s.history_bytes, s.due, s.archive_due, s.decided_run, s.decided_time, s.swept]));
    }
}

/// `f` over the kept form as `t` (a write's transaction on the store at `store`) finds it, then its write, in `t`:
/// what to note once `t` commits (`ContentStore::note_kept`).
pub(in crate::content_store) fn write_with<T>(store: &Path, t: &Connection, f: impl FnOnce(&Corpus<'_>, &mut Write) -> StoreResult<T>) -> StoreResult<(T, Option<ChainRows>)> {
    let kept = open(store, t)?;
    let view = kept.as_ref().map(KeptSpace::view);
    let mut w = Write::default();
    let out = f(&Corpus(view.as_ref().map(|v| v as &dyn KeptRead)), &mut w)?;
    let puts = w.puts.into_values().filter(|p| !p.kept_as(view.as_ref().and_then(|v| v.get(&FactKey { rel: p.rel, ident: &p.ident, part: p.part }).filter(|e| *e.key.ident == p.ident)))).map(Want::put).collect::<StoreResult<Vec<_>>>()?;
    drop(view);
    drop(kept);
    let written = if puts.is_empty() { None } else { write_space(store, t, &space(), puts, None)? };
    Ok((out, written))
}

/// `write_with` on the store `t` is a connection to (for a writer with no store handle; its write noted once a store
/// handle next reads the space).
pub(in crate::content_store) fn write_at<T>(t: &Connection, f: impl FnOnce(&Corpus<'_>, &mut Write) -> StoreResult<T>) -> StoreResult<T> {
    let store = std::path::PathBuf::from(t.path().unwrap_or_default());
    Ok(write_with(&store, t, f)?.0)
}

/// `f` over the kept form of the corpus runs as `t` (a connection to the store at `store`) finds it.
pub(in crate::content_store) fn with_corpus<T>(store: &Path, t: &Connection, f: impl FnOnce(&Corpus<'_>) -> T) -> StoreResult<T> {
    let kept = open(store, t)?;
    let view = kept.as_ref().map(KeptSpace::view);
    Ok(f(&Corpus(view.as_ref().map(|v| v as &dyn KeptRead))))
}

impl ContentStore {
    /// `write_with` in `t`, a write's transaction of this store.
    pub(in crate::content_store) fn corpus_write<T>(&self, t: &Connection, f: impl FnOnce(&Corpus<'_>, &mut Write) -> StoreResult<T>) -> StoreResult<(T, Option<ChainRows>)> {
        write_with(&self.database_path, t, f)
    }

    /// Changes the corpus runs' state alone, in a transaction of its own.
    pub(in crate::content_store) fn corpus_state_with(&self, f: impl Fn(&mut KeptState)) -> StoreResult<()> {
        let ((), written) = self.immediate_transaction(|t| {
            self.corpus_write(t, |c, w| {
                let mut s = c.state();
                f(&mut s);
                w.state(s);
                Ok(())
            })
        })?;
        self.note_kept(&space(), written);
        Ok(())
    }

    /// `with_corpus` on the store's connection.
    pub(in crate::content_store) fn corpus_kept<T>(&self, f: impl FnOnce(&Corpus<'_>) -> T) -> StoreResult<T> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        with_corpus(&self.database_path, &connection, f)
    }
}
