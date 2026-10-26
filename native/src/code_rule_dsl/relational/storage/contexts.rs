//! Stable SQLite backing with exact data bindings. Scope changes are data writes, never DDL.
use super::{Connection, Result, quote};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
const BINDINGS: &str = "__execution_bindings";
const KEY: &str = "__execution_key";
pub(super) fn managed(name: &str) -> bool {
    name.starts_with("__context_") || name.starts_with("__stage___context_")
}
fn backing(name: &str) -> String {
    format!("__rows_{name}")
}
#[derive(Default)]
pub(super) struct Contexts {
    lifecycle: Lifecycle,
    initialized: bool,
    owner: usize,
    active: BTreeMap<String, usize>,
    layouts: BTreeMap<String, Layout>,
    data: BTreeMap<(String, usize), Data>,
    owners: BTreeMap<usize, BTreeSet<(String, usize)>>,
    completed: VecDeque<(String, usize)>,
}
#[derive(Default, serde::Serialize)]
struct Lifecycle {
    declarations: usize,
    declaration_reuses: usize,
    index_builds: usize,
    index_reuses: usize,
    analysis_requests: usize,
    analysis_batches: usize,
    analyzed_relations: usize,
    owner_keys_visited: usize,
    rowset_deletions: usize,
    layout_drops: usize,
}
struct Layout {
    key_column: String,
    definition: String,
    outer_gate: bool,
    indexes: BTreeMap<String, String>,
    statistics_dirty: bool,
}
#[derive(Default)]
struct Data {
    #[cfg(feature = "code-rule-profile")]
    rows: usize,
    dirty: bool,
    ready: bool,
    pins: usize,
}
impl Contexts {
    pub fn bind(
        &mut self,
        db: &Connection,
        owner: usize,
        bindings: BTreeMap<String, usize>,
    ) -> Result<()> {
        if !self.initialized {
            db.execute_batch(&format!("CREATE TEMP TABLE {BINDINGS}(slot TEXT PRIMARY KEY,binding INTEGER NOT NULL) WITHOUT ROWID"))?;
            self.initialized = true;
        }
        if self.active != bindings {
            db.execute(&format!("DELETE FROM {BINDINGS}"), [])?;
            let rows = bindings.iter().collect::<Vec<_>>();
            for chunk in rows.chunks(256) {
                let values = std::iter::repeat_n("(?,?)", chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let args = chunk.iter().flat_map(|(slot, id)| {
                    [
                        rusqlite::types::Value::Text((*slot).clone()),
                        rusqlite::types::Value::Integer(**id as i64),
                    ]
                });
                db.execute(
                    &format!("INSERT INTO {BINDINGS} VALUES {values}"),
                    rusqlite::params_from_iter(args),
                )?;
            }
            self.active = bindings.clone();
        }
        let owned = self.owners.entry(owner).or_default();
        for (slot, &id) in &bindings {
            let key = (slot.clone(), id);
            if owned.insert(key.clone()) {
                self.data.entry(key).or_default().pins += 1;
            }
        }
        self.owner = owner;
        Ok(())
    }
    pub fn contains(&self, name: &str) -> bool {
        self.layouts.contains_key(name)
    }
    pub fn declare(
        &mut self,
        db: &Connection,
        name: &str,
        definition: &str,
        outer_gate: bool,
    ) -> Result<()> {
        if self
            .layouts
            .get(name)
            .is_some_and(|l| l.definition == definition && l.outer_gate == outer_gate)
        {
            self.lifecycle.declaration_reuses += 1;
            if let Some(key) = self.key(name)
                && self.data.get(&key).is_some_and(|d| d.dirty && !d.ready)
            {
                db.execute(
                    &format!(
                        "DELETE FROM {} WHERE {}=?1",
                        quote(&backing(name)),
                        quote(&self.layouts[name].key_column)
                    ),
                    [key.1 as i64],
                )?;
                self.data.get_mut(&key).unwrap().dirty = false;
                let layout = self.layouts.get_mut(name).unwrap();
                layout.statistics_dirty |= layout.outer_gate || !layout.indexes.is_empty();
            }
            return Ok(());
        }
        if self.contains(name) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let body = definition
            .split_once('(')
            .ok_or(rusqlite::Error::InvalidQuery)?
            .1;
        let mut key_column = KEY.to_owned();
        if body.to_ascii_lowercase().contains(KEY) {
            let fields = super::super::schema::register(
                db,
                "__scope_key_schema",
                definition.replacen("CREATE TEMP TABLE", "CREATE TABLE", 1),
            )?;
            let mut suffix = 0;
            while fields
                .iter()
                .any(|field| field.name.eq_ignore_ascii_case(&key_column))
            {
                suffix += 1;
                key_column = format!("{KEY}_{suffix}");
            }
        }
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {}({key_column} INTEGER NOT NULL,{body}",
            quote(&backing(name))
        ))?;
        let columns = {
            let stmt = db.prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&backing(name))))?;
            stmt.column_names()
                .into_iter()
                .skip(1)
                .map(|column| format!("r.{}", quote(column)))
                .collect::<Vec<_>>()
                .join(",")
        };
        let literal = name.replace('\'', "''");
        let source = if outer_gate {
            // Imported inputs have no authored lookup indexes. Keep the scope
            // gate outside their scan, so a scope-only index cannot push a
            // recursive frontier behind a Cartesian input scan.
            format!(
                "{BINDINGS} b CROSS JOIN {} r ON r.{}=b.binding WHERE b.slot='{literal}'",
                quote(&backing(name)),
                quote(&key_column)
            )
        } else {
            // Indexed cuts retain the lookup chosen by their authored access
            // paths. An extra join fence here can defeat selective readers.
            format!(
                "{} r WHERE r.{}=(SELECT binding FROM {BINDINGS} WHERE slot='{literal}')",
                quote(&backing(name)),
                quote(&key_column)
            )
        };
        db.execute_batch(&format!(
            "CREATE TEMP VIEW {} AS SELECT {columns} FROM {source}",
            quote(name)
        ))?;
        // Every imported row carries its file and source span. This replaces
        // the scope-only index, so interval/file joins gain a range lookup
        // without a second index or a per-consumer index build. Its prefix
        // still serves bulk scope cleanup.
        let lookup = if outer_gate {
            format!("{},\"__path\",\"__start\",\"__end\"", quote(&key_column))
        } else {
            quote(&key_column)
        };
        db.execute_batch(&format!(
            "CREATE INDEX {} ON {}({lookup})",
            quote(&format!("__binding_{name}")),
            quote(&backing(name))
        ))?;
        self.lifecycle.declarations += 1;
        self.layouts.insert(
            name.into(),
            Layout {
                key_column,
                definition: definition.into(),
                outer_gate,
                indexes: BTreeMap::new(),
                statistics_dirty: false,
            },
        );
        Ok(())
    }
    fn key(&self, name: &str) -> Option<(String, usize)> {
        self.active.get(name).map(|&id| (name.to_owned(), id))
    }
    pub fn target(&self, name: &str) -> Option<(String, Option<i64>)> {
        if !managed(name) {
            return None;
        }
        self.active
            .get(name)
            .map(|&id| (backing(name), Some(id as i64)))
    }
    pub fn index(&mut self, db: &Connection, slot: &str, name: &str, sql: &str) -> Result<()> {
        let layout = self
            .layouts
            .get_mut(slot)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        if layout.indexes.get(name).is_some_and(|s| s == sql) {
            self.lifecycle.index_reuses += 1;
            return Ok(());
        }
        if layout.indexes.contains_key(name) {
            db.execute_batch(&format!("DROP INDEX {}", quote(name)))?;
        }
        let fields = sql.split_once('(').ok_or(rusqlite::Error::InvalidQuery)?.1;
        db.execute_batch(&format!(
            "CREATE INDEX {} ON {}({},{fields}",
            quote(name),
            quote(&backing(slot)),
            quote(&layout.key_column)
        ))?;
        self.lifecycle.index_builds += 1;
        layout.indexes.insert(name.into(), sql.into());
        Ok(())
    }
    pub fn analyze(&mut self, _db: &Connection, name: &str, rows: usize) -> Result<()> {
        let layout = self
            .layouts
            .get_mut(name)
            .ok_or(rusqlite::Error::InvalidQuery)?;
        self.lifecycle.analysis_requests += 1;
        layout.statistics_dirty |= rows > 0;
        Ok(())
    }
    pub fn statistics_boundary(&mut self, db: &Connection) -> Result<()> {
        let mut refreshed = false;
        for (slot, layout) in &mut self.layouts {
            if !layout.statistics_dirty {
                continue;
            }
            // Stable contexts make the schema small. Refresh only the cuts
            // changed by this publication; ANALYZE temp rescans every retained
            // input and completed cut again at every dependent frontier.
            db.execute_batch(&format!("ANALYZE temp.{}", quote(&backing(slot))))?;
            self.lifecycle.analyzed_relations += 1;
            layout.statistics_dirty = false;
            refreshed = true;
        }
        self.lifecycle.analysis_batches += usize::from(refreshed);
        Ok(())
    }
    pub fn populated(&mut self, name: &str, rows: usize) {
        self.written(name, rows != 0);
        if let Some(layout) = self.layouts.get_mut(name) {
            // Inputs now have a source-span index too. Their distribution must
            // be published at the same cohort boundary as indexed stage cuts.
            layout.statistics_dirty |= layout.outer_gate && rows != 0;
        }
        #[cfg(feature = "code-rule-profile")]
        if let Some(key) = self.key(name) {
            self.data.entry(key).or_default().rows = rows;
        }
    }
    pub fn written(&mut self, name: &str, rows: bool) {
        if let Some(key) = self.key(name) {
            self.data.entry(key.clone()).or_default().dirty |= rows;
            self.owners.entry(self.owner).or_default().insert(key);
        }
    }
    pub fn ready(&self, name: &str) -> bool {
        self.key(name)
            .and_then(|key| self.data.get(&key))
            .is_some_and(|d| d.ready)
    }
    pub fn publish(&mut self, name: &str) {
        if let Some(key) = self.key(name) {
            self.data.entry(key).or_default().ready = true;
        }
    }
    pub fn retire(&mut self, db: &Connection, name: &str) -> Result<bool> {
        let Some(key) = self.key(name) else {
            return Ok(false);
        };
        if self.data.get(&key).is_some_and(|d| !d.ready) {
            self.remove(db, &key)?;
        }
        Ok(self.contains(name))
    }
    fn remove(&mut self, db: &Connection, key: &(String, usize)) -> Result<()> {
        let changed = self.data.get(key).is_some_and(|d| d.dirty);
        if changed {
            self.lifecycle.rowset_deletions += 1;
            db.execute(
                &format!(
                    "DELETE FROM {} WHERE {}=?1",
                    quote(&backing(&key.0)),
                    quote(&self.layouts[&key.0].key_column)
                ),
                [key.1 as i64],
            )?;
        }
        if changed && let Some(layout) = self.layouts.get_mut(&key.0) {
            layout.statistics_dirty |= layout.outer_gate || !layout.indexes.is_empty();
        }
        self.data.remove(key);
        Ok(())
    }
    pub fn release(&mut self, db: &Connection, owner: usize, slot: &str, id: usize) -> Result<()> {
        let key = (slot.to_owned(), id);
        if self
            .owners
            .get_mut(&owner)
            .is_some_and(|keys| keys.remove(&key))
        {
            self.unpin(db, key)?;
        }
        Ok(())
    }
    fn unpin(&mut self, db: &Connection, key: (String, usize)) -> Result<()> {
        self.lifecycle.owner_keys_visited += 1;
        let Some(data) = self.data.get_mut(&key) else {
            return Ok(());
        };
        data.pins = data.pins.saturating_sub(1);
        if data.pins == 0 {
            if data.ready {
                self.completed.push_back(key);
            } else {
                self.remove(db, &key)?;
            }
        }
        Ok(())
    }
    pub fn clear_owner(&mut self, db: &Connection, owner: usize) -> Result<()> {
        for key in self.owners.remove(&owner).unwrap_or_default() {
            self.unpin(db, key)?;
        }
        Ok(())
    }
    pub fn evict(&mut self, db: &Connection, capacity: usize) -> Result<()> {
        let mut retired = BTreeSet::new();
        let mut retained_bytes = self.bytes();
        while self.completed.len() > 128 || retained_bytes > capacity.min(4 * 1024 * 1024) {
            let Some(key) = self.completed.pop_front() else {
                break;
            };
            if self.data.get(&key).is_some_and(|d| d.pins == 0) && retired.insert(key) {
                retained_bytes = retained_bytes.saturating_sub(128);
            }
        }
        if let Err(error) = self.remove_batch(db, &retired) {
            self.completed.extend(retired);
            return Err(error);
        }
        // Only idle layouts can be evicted. Never drop a suspended consumer's schema.
        let layout_bytes = self
            .layouts
            .iter()
            .map(|(n, l)| {
                n.len()
                    + l.definition.len()
                    + l.indexes
                        .iter()
                        .map(|(n, q)| n.len() + q.len() + 64)
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>();
        if self.layouts.len() > 128 || layout_bytes > capacity.min(4 * 1024 * 1024) {
            let live = self
                .data
                .keys()
                .map(|(slot, _)| slot.clone())
                .collect::<BTreeSet<_>>();
            let idle = self
                .layouts
                .keys()
                .filter(|slot| !live.contains(*slot))
                .cloned()
                .collect::<Vec<_>>();
            for slot in idle {
                if self.layouts.len() <= 128 && self.bytes() <= capacity.min(4 * 1024 * 1024) {
                    break;
                }
                self.discard(db, &slot)?;
            }
        }
        Ok(())
    }
    fn remove_batch(&mut self, db: &Connection, keys: &BTreeSet<(String, usize)>) -> Result<()> {
        let mut groups = BTreeMap::<&str, Vec<usize>>::new();
        for (slot, id) in keys {
            if self.data.get(&(slot.clone(), *id)).is_some_and(|d| d.dirty) {
                groups.entry(slot).or_default().push(*id);
            } else {
                self.data.remove(&(slot.clone(), *id));
            }
        }
        for (slot, ids) in groups {
            for batch in ids.chunks(256) {
                let sql = format!(
                    "DELETE FROM {} WHERE {} IN ({})",
                    quote(&backing(slot)),
                    quote(&self.layouts[slot].key_column),
                    vec!["?"; batch.len()].join(",")
                );
                db.execute(
                    &sql,
                    rusqlite::params_from_iter(batch.iter().map(|&id| id as i64)),
                )?;
                self.lifecycle.rowset_deletions += batch.len();
                self.layouts.get_mut(slot).unwrap().statistics_dirty = true;
                for &id in batch {
                    self.data.remove(&(slot.to_owned(), id));
                }
            }
        }
        Ok(())
    }
    pub fn discard(&mut self, db: &Connection, name: &str) -> Result<()> {
        db.execute_batch(&format!(
            "DROP VIEW IF EXISTS {}; DROP TABLE IF EXISTS {}",
            quote(name),
            quote(&backing(name))
        ))?;
        self.lifecycle.layout_drops += 1;
        self.layouts.remove(name);
        self.data.retain(|(slot, _), _| slot != name);
        Ok(())
    }
    #[cfg(feature = "code-rule-profile")]
    pub fn inventory(&self) -> serde_json::Value {
        serde_json::json!({"layouts":self.layouts.len(),"dataBindings":self.data.len(),"pendingOwners":self.owners.len(),
            "dataPins":self.data.values().map(|d|d.pins).sum::<usize>(),"publishedSqlRows":self.data.values().map(|d|d.rows).sum::<usize>(),"idleCompletions":self.completed.len(),"lifecycle":self.lifecycle})
    }
    pub fn bytes(&self) -> usize {
        self.layouts
            .iter()
            .map(|(n, l)| {
                n.len()
                    + l.definition.len()
                    + l.indexes
                        .iter()
                        .map(|(n, q)| n.len() + q.len() + 64)
                        .sum::<usize>()
                    + 128
            })
            .sum::<usize>()
            + self.data.len() * 128
            + self.active.keys().map(|n| n.len() + 48).sum::<usize>()
    }
}
