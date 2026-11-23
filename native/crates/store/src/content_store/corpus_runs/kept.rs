//! The corpus runs' tables as kept facts of the store (t33 step 3, the manifest tables; user, 2026-11-23: "move them
//! in stages: write both forms first, check collection agrees on every test store, then switch the readers, then drop
//! the tables"). This is the first stage: each write transaction on the eight tables also writes the partitions it
//! touched, read back from the tables, into one space of the store, in the same transaction (`sync`). `agree` says
//! where the two forms differ; collection asks it in a test build (`check`), so every test store that collects checks
//! the kept form against the tables.
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
//! - `corpus_state`, partition 0: `run, history_bytes, due, archive_due`, then `corpus_decided`'s `run, time, swept`.
//!   Always put once mirrored: a space without it was never mirrored, and its first sync mirrors every table.
//!
//! A digest is four words, little-endian.

use super::super::kept_facts::{BlockFlat, ChainRows, KeptPut, KeptSpace, KeptWhat, chain_rows, sorted_put, write_space};
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

/// What a write touched: the partitions `sync` reads back from the tables. `all`: every one.
#[derive(Default)]
pub(in crate::content_store) struct Touch {
    pub all: bool,
    pub roots: Vec<i64>,
    pub manifests: Vec<i64>,
    pub plans: Vec<[u8; 32]>,
    pub sources: Vec<i64>,
}

impl Touch {
    pub fn all() -> Touch {
        Touch { all: true, ..Touch::default() }
    }

    pub fn plans(plans: impl IntoIterator<Item = [u8; 32]>) -> Touch {
        Touch { plans: plans.into_iter().collect(), ..Touch::default() }
    }
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

/// The partitions of `touch`, as the tables have them; with `all`, every partition of either form.
fn wants(t: &Connection, view: Option<&dyn KeptRead>, touch: &Touch) -> StoreResult<Vec<Want>> {
    let (mut roots, mut manifests, mut plans, mut sources): (BTreeSet<i64>, BTreeSet<i64>, BTreeSet<[u8; 32]>, BTreeSet<i64>) =
        (touch.roots.iter().copied().collect(), touch.manifests.iter().copied().collect(), touch.plans.iter().copied().collect(), touch.sources.iter().copied().collect());
    if touch.all {
        roots.extend(ids(t, "SELECT root_id FROM corpus_roots")?);
        manifests.extend(ids(t, "SELECT manifest_id FROM corpus_manifests")?);
        sources.extend(ids(t, "SELECT DISTINCT source_id FROM corpus_live")?);
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
    let rows = chain_rows(t, &space())?;
    if rows.is_empty() { Ok(None) } else { KeptSpace::open(store, &space(), &rows).map(Some) }
}

fn kept<'v>(view: Option<&'v dyn KeptRead>, w: &Want) -> Option<Kept<'v>> {
    view?.get(&FactKey { rel: w.rel, ident: &w.ident, part: w.part }).filter(|e| *e.key.ident == w.ident)
}

/// Writes the partitions `touch` names as the tables now have them, in `t` (the write's transaction): those that
/// differ from the space. A space never mirrored (no `corpus_state`) is mirrored whole.
pub(in crate::content_store) fn sync(store: &Path, t: &Connection, touch: &Touch) -> StoreResult<Option<ChainRows>> {
    let old = open(store, t)?;
    let view = old.as_ref().map(KeptSpace::view);
    let view = view.as_ref().map(|v| v as &dyn KeptRead);
    let whole;
    let touch = if view.is_none_or(|v| v.get(&FactKey { rel: STATE, ident: &ident(STATE), part: 0 }).is_none()) {
        whole = Touch::all();
        &whole
    } else {
        touch
    };
    let puts = wants(t, view, touch)?.into_iter().filter(|w| !w.kept_as(kept(view, w))).map(Want::put).collect::<StoreResult<Vec<_>>>()?;
    drop(old);
    if puts.is_empty() { Ok(None) } else { write_space(store, t, &space(), puts, None) }
}

/// `sync` on the store `t` is a connection to (its main database): for a writer with no store handle. Its write is
/// noted once a store handle next writes or reads the space.
pub(in crate::content_store) fn sync_at(t: &Connection, touch: &Touch) -> StoreResult<()> {
    match t.path().filter(|p| !p.is_empty()) {
        Some(path) => sync(Path::new(path), t, touch).map(drop),
        None => Ok(()),
    }
}

/// Where the kept form differs from the tables, a line each (empty: they agree). A store whose tables are as a new
/// store's, never mirrored, agrees.
pub(in crate::content_store) fn agree(store: &Path, t: &Connection) -> StoreResult<Vec<String>> {
    let old = open(store, t)?;
    let view = old.as_ref().map(KeptSpace::view);
    let view = view.as_ref().map(|v| v as &dyn KeptRead);
    let wants = wants(t, view, &Touch::all())?;
    if view.is_none_or(|v| v.get(&FactKey { rel: STATE, ident: &ident(STATE), part: 0 }).is_none())
        && wants.len() == 1
        && wants[0].rows.iter().all(|v| *v == 0)
    {
        return Ok(Vec::new());
    }
    Ok(wants
        .iter()
        .filter(|w| !w.kept_as(kept(view, w)))
        .map(|w| {
            let had = kept(view, w).map(|k| (0..k.rows.len()).map(|r| k.row(r)).collect::<Vec<_>>());
            format!("{} {:02x?} {}: tables {:?}, kept {:?}", w.rel, &w.ident[..4], w.part, w.rows, had)
        })
        .collect())
}

impl ContentStore {
    /// Mirrors what `touch` names into the kept form, in `t` (`sync`), and returns what to note once committed.
    pub(in crate::content_store) fn mirror_corpus(&self, t: &Connection, touch: &Touch) -> StoreResult<Option<ChainRows>> {
        sync(&self.database_path, t, touch)
    }

    /// Where the corpus runs' kept form differs from their tables (`agree`).
    pub fn corpus_kept_differences(&self) -> StoreResult<Vec<String>> {
        let connection = self.connection.lock().expect("content store mutex poisoned");
        // One read transaction: the chain and the tables from one snapshot. Read as separate statements, a write
        // another handle committed between them showed tables a write ahead of their kept form.
        let read = connection.unchecked_transaction()?;
        let differences = agree(&self.database_path, &read)?;
        read.finish()?;
        Ok(differences)
    }

    /// In a test build, refuses a collection whose kept form of the corpus runs differs from their tables (stage 2 of
    /// the move: every test store that collects checks it).
    pub(in crate::content_store) fn check_corpus_kept(&self) -> StoreResult<()> {
        if cfg!(any(test, feature = "test-hooks")) {
            let differences = self.corpus_kept_differences()?;
            if !differences.is_empty() {
                return Err(StoreError::Corrupt(format!("the corpus runs' kept facts differ from their tables: {}", differences.join("; "))));
            }
        }
        Ok(())
    }
}
