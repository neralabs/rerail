//! Shared immutable syntax facts. SQL retains all policy decisions.
use super::{Result, invalid, source, sql_error};
use crate::code_rule_dsl::{
    eval, language::GrammarDialect, lines::LineTable, model::FileInput, tape::TapeDoc,
    violations::Span,
};
use ast_grep_core::source::SgNode;
use rusqlite::{Connection, types::Value, vtab::Context};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize},
    },
};

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::code_rule_dsl) struct Owner {
    pub kind: String,
    #[serde(default)]
    pub outermost: bool,
}
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::code_rule_dsl) struct Spec {
    pub name: String,
    pub gate: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(skip)]
    pub field_codes: BTreeMap<String, u16>,
    #[serde(default, rename = "excludeKinds")]
    pub exclude_kinds: Vec<String>,
    #[serde(default, rename = "emptyTextKinds")]
    pub empty_text_kinds: Vec<String>,
    pub edges: Option<String>,
    pub ownership: Option<String>,
    #[serde(default)]
    pub owners: BTreeMap<String, Owner>,
}

pub(super) trait Source: Send + Sync {
    fn columns(&self) -> Vec<String>;
    fn column_types(&self) -> Vec<String>;
    fn len(&self) -> usize;
    fn estimated_len(&self) -> usize {
        self.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize;
    fn initialize(&self) -> rusqlite::Result<()> {
        Ok(())
    }
    fn indexed(&self, column: usize) -> bool;
    fn range_upper_compatible(&self, _lower: usize, _upper: usize) -> bool {
        false
    }
    fn range_indexed(&self, _column: usize) -> bool {
        false
    }
    fn range_lookup(
        &self,
        _column: usize,
        _lower: Option<(&Value, bool)>,
        _upper: Option<(&Value, bool)>,
    ) -> rusqlite::Result<Option<Vec<usize>>> {
        Ok(None)
    }
    fn lookup(&self, column: usize, values: &[Value]) -> rusqlite::Result<Option<Vec<usize>>>;
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> rusqlite::Result<()>;
    fn charge(&self, count: usize) -> rusqlite::Result<()>;
}
#[derive(Default)]
pub(super) struct Counters {
    pub scans: AtomicU64,
    pub batches: AtomicU64,
    pub candidates: AtomicU64,
    pub columns: AtomicU64,
}
struct OwnerIndex {
    values: Vec<Option<u32>>,
    postings: HashMap<u32, Vec<usize>>,
}
type Selections = parking_lot::Mutex<HashMap<(Vec<String>, bool), Arc<Selection>>>;
pub(in crate::code_rule_dsl) struct Facts {
    doc: TapeDoc,
    lines: Arc<LineTable>,
    rows: Vec<u32>,
    positions: Vec<Option<usize>>,
    ordinal: Vec<usize>,
    children: HashMap<u32, Vec<usize>>,
    kinds: HashMap<String, Vec<usize>>,
    fields: HashMap<(u32, u16), u32>,
    owners: parking_lot::Mutex<HashMap<(String, bool), Arc<OwnerIndex>>>,
    selections: Selections,
}
impl Facts {
    pub(in crate::code_rule_dsl) fn build(file: &FileInput) -> Result<Arc<Self>> {
        #[cfg(feature = "code-rule-profile")]
        let _span = crate::code_rule_dsl::profile::trace::Span::start("syntax-prepare", "document");
        let language =
            eval::file_language(file).ok_or_else(|| invalid("syntax source has no grammar"))?;
        let (doc, lines) = eval::file_tape(language, file)?;
        if !doc.syntax_error_ranges().is_empty() {
            return Err(invalid("syntax source is incomplete"));
        }
        let mut facts = Self {
            positions: vec![None; doc.transducer_len()],
            doc,
            lines,
            rows: Vec::new(),
            ordinal: Vec::new(),
            children: HashMap::new(),
            kinds: HashMap::new(),
            fields: HashMap::new(),
            owners: parking_lot::Mutex::new(HashMap::new()),
            selections: parking_lot::Mutex::new(HashMap::new()),
        };
        for id in 0..facts.doc.transducer_len() as u32 {
            if crate::code_rule_dsl::budget::step_budget_exhausted() {
                return Err(invalid("interrupted: syntax preparation"));
            }
            let field = facts.doc.transducer_field(id);
            if field != 0
                && let Some(parent) = facts.doc.transducer_parent(id)
            {
                facts.fields.entry((parent, field)).or_insert(id);
            }
            let node = facts.doc.node(id);
            let kind = node.kind();
            if facts.doc.transducer_flags(id) & 1 == 0 {
                continue;
            }
            let row = facts.rows.len();
            facts.positions[id as usize] = Some(row);
            facts.rows.push(id);
            facts.ordinal.push(0);
            if let Some(rows) = facts.kinds.get_mut(kind.as_ref()) {
                rows.push(row)
            } else {
                facts.kinds.insert(kind.into_owned(), vec![row]);
            }
            if let Some(parent) = facts.doc.transducer_parent(id) {
                facts.children.entry(parent).or_default().push(row);
            }
        }
        for children in facts.children.values_mut() {
            children.sort_by_key(|&r| {
                let id = facts.rows[r];
                let span = facts.doc.transducer_span(id);
                (span.start, span.end, facts.doc.node(id).kind().into_owned())
            });
            for (ordinal, &row) in children.iter().enumerate() {
                facts.ordinal[row] = ordinal;
            }
        }
        Ok(Arc::new(facts))
    }

    fn select(&self, excludes: &[String], edges: bool) -> Result<Arc<Selection>> {
        let mut key = excludes.to_vec();
        key.sort();
        let key = (key, edges);
        let mut cache = self.selections.lock();
        if let Some(rows) = cache.get(&key) {
            return Ok(Arc::clone(rows));
        }
        let mut selected = Selection {
            rows: Vec::new(),
            reverse: vec![None; self.rows.len()],
            ordinal: Vec::new(),
        };
        let mut ordinals = HashMap::new();
        for (r, &id) in self.rows.iter().enumerate() {
            if crate::code_rule_dsl::budget::step_budget_exhausted() {
                return Err(invalid("interrupted: syntax selection"));
            }
            if excludes
                .iter()
                .any(|k| k == self.doc.node(id).kind().as_ref())
            {
                continue;
            }
            let parent = self.doc.transducer_parent(id);
            let ordinal = ordinals.entry(parent).or_insert(0);
            let current = *ordinal;
            *ordinal += 1;
            if edges && parent.is_none() {
                continue;
            }
            selected.reverse[r] = Some(selected.rows.len());
            selected.rows.push(r);
            selected.ordinal.push(current);
        }
        let selected = Arc::new(selected);
        if cache.len() >= 4 {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&selected));
        Ok(selected)
    }
    pub(in crate::code_rule_dsl) fn estimated_bytes(&self) -> usize {
        self.doc.transducer_len() * 64 + self.len() * 320
    }
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.rows.len()
    }
    pub(super) fn read_ignores(&self) -> crate::code_rule_dsl::suppress::RerailIgnores {
        use ast_grep_core::Doc;
        let target = ast_grep_core::AstGrep::doc(self.doc.clone());
        crate::code_rule_dsl::suppress::RerailIgnores::read(
            &target.root(),
            self.doc.get_source(),
            &self.lines,
        )
    }
    pub(super) fn span(&self, start: usize, end: usize, excludes: &[String]) -> Option<Span> {
        // The preorder table is ordered by source start. Check only equal-start nodes.
        let at = self
            .rows
            .partition_point(|&id| self.doc.transducer_span(id).start < start);
        self.rows[at..]
            .iter()
            .take_while(|&&id| self.doc.transducer_span(id).start == start)
            .any(|&id| {
                self.doc.transducer_span(id).end == end
                    && !excludes
                        .iter()
                        .any(|k| k == self.doc.node(id).kind().as_ref())
            })
            .then(|| self.lines.span(start, end))
    }
    fn owner(&self, owner: &Owner) -> Result<Arc<OwnerIndex>> {
        let key = (owner.kind.clone(), owner.outermost);
        let mut cache = self.owners.lock();
        if let Some(index) = cache.get(&key) {
            return Ok(Arc::clone(index));
        }
        #[cfg(feature = "code-rule-profile")]
        let _span = crate::code_rule_dsl::profile::trace::Span::start("syntax-owners", &owner.kind);
        let mut values = Vec::with_capacity(self.rows.len());
        let mut postings: HashMap<u32, Vec<usize>> = HashMap::new();
        for (row, &id) in self.rows.iter().enumerate() {
            if crate::code_rule_dsl::budget::step_budget_exhausted() {
                return Err(invalid("interrupted: syntax owners"));
            }
            let parent = self
                .doc
                .transducer_parent(id)
                .and_then(|p| self.positions[p as usize])
                .and_then(|p| values[p]);
            let own = (self.doc.node(id).kind().as_ref() == owner.kind).then_some(id);
            let value = if owner.outermost {
                parent.or(own)
            } else {
                own.or(parent)
            };
            values.push(value);
            if let Some(id) = value {
                postings.entry(id).or_default().push(row);
            }
        }
        let index = Arc::new(OwnerIndex { values, postings });
        if cache.len() >= 4 {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&index));
        Ok(index)
    }
    pub(super) fn descendants(&self, root: i64) -> Result<&[u32]> {
        let root = u32::try_from(root)
            .ok()
            .filter(|id| (*id as usize) < self.positions.len())
            .ok_or_else(|| invalid("invalid syntax root"))?;
        let start = self.positions[root as usize]
            .ok_or_else(|| invalid("syntax root is not an eligible node"))?;
        let end = self.doc.transducer_descendants(root).end;
        let count = self.rows[start..].partition_point(|&id| id < end);
        Ok(&self.rows[start..start + count])
    }
}

type PoolKey = (usize, GrammarDialect);
struct PoolEntry {
    source: Arc<String>,
    facts: Arc<Facts>,
}
#[derive(Default)]
struct PoolState {
    active: bool,
    entries: HashMap<PoolKey, PoolEntry>,
    order: std::collections::VecDeque<PoolKey>,
    bytes: usize,
}
thread_local! { static POOL: RefCell<PoolState> = RefCell::new(PoolState::default()); }
pub(in crate::code_rule_dsl) struct Pool(bool);
impl Pool {
    pub(in crate::code_rule_dsl) fn install() -> Self {
        Self(POOL.with(|p| {
            let mut p = p.borrow_mut();
            let outer = !p.active;
            p.active = true;
            outer
        }))
    }
    fn key(file: &FileInput) -> Result<PoolKey> {
        Ok((
            Arc::as_ptr(&file.source) as usize,
            eval::file_language(file).ok_or_else(|| invalid("syntax source has no grammar"))?,
        ))
    }
    pub(in crate::code_rule_dsl) fn seed(file: &FileInput, facts: Arc<Facts>) {
        let Ok(key) = Self::key(file) else {
            return;
        };
        POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.entries.contains_key(&key) {
                return;
            }
            let bytes = file.source.len() + facts.estimated_bytes();
            const LIMIT: usize = 64 * 1024 * 1024;
            if bytes > LIMIT {
                return;
            }
            while pool.bytes + bytes > LIMIT {
                let Some(old) = pool.order.pop_front() else {
                    break;
                };
                if let Some(old) = pool.entries.remove(&old) {
                    pool.bytes -= old.source.len() + old.facts.estimated_bytes();
                }
            }
            pool.bytes += bytes;
            pool.order.push_back(key);
            pool.entries.insert(
                key,
                PoolEntry {
                    source: Arc::clone(&file.source),
                    facts,
                },
            );
        });
    }
    pub(in crate::code_rule_dsl) fn get(file: &FileInput) -> Result<Arc<Facts>> {
        let key = Self::key(file)?;
        if let Some(facts) = POOL.with(|p| {
            p.borrow()
                .entries
                .get(&key)
                .map(|entry| Arc::clone(&entry.facts))
        }) {
            return Ok(facts);
        }
        let facts = Facts::build(file)?;
        if POOL.with(|p| p.borrow().active) {
            Self::seed(file, Arc::clone(&facts));
        }
        Ok(facts)
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        if self.0 {
            POOL.with(|p| *p.borrow_mut() = PoolState::default());
        }
    }
}

#[derive(Clone)]
enum Column {
    Path,
    Start,
    End,
    Id,
    Parent,
    Kind,
    Line,
    Text,
    Ordinal,
    ParentStart,
    ParentEnd,
    ParentKind,
    Field(u16, u8),
    Owner(Arc<OwnerIndex>),
}
struct Selection {
    rows: Vec<usize>,
    reverse: Vec<Option<usize>>,
    ordinal: Vec<usize>,
}
struct Meter {
    work: Arc<AtomicUsize>,
    steps: AtomicUsize,
}
impl Meter {
    fn charge(&self, count: usize) -> rusqlite::Result<()> {
        use std::sync::atomic::Ordering;
        let before = self.steps.fetch_add(count, Ordering::Relaxed);
        let units = before.saturating_add(count).div_ceil(64) - before.div_ceil(64);
        source::charge(&self.work, units)
            .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))
    }
}
struct Nodes {
    path: String,
    facts: Arc<Facts>,
    selected: Arc<Selection>,
    empty_text_kinds: Vec<String>,
    columns: Vec<(String, Column)>,
    meter: Arc<Meter>,
}
impl Source for Nodes {
    fn columns(&self) -> Vec<String> {
        self.columns.iter().map(|(n, _)| n.clone()).collect()
    }
    fn column_types(&self) -> Vec<String> {
        // Match evidence tables: spans and paths have affinity; projected values
        // and canonical identities do not. Typed ID declarations prevent SQLite
        // from indexing joins to the generic graph adapter's untyped key view.
        self.columns
            .iter()
            .map(|(_, column)| {
                match column {
                    Column::Path => "TEXT",
                    Column::Start | Column::End => "INTEGER",
                    _ => "",
                }
                .to_owned()
            })
            .collect()
    }
    fn len(&self) -> usize {
        self.selected.rows.len()
    }
    fn estimated_matches(&self, c: usize, batch: bool) -> usize {
        let n = match &self.columns[c].1 {
            Column::Id | Column::Start => 1,
            Column::Parent => 8,
            Column::Kind => self.len().div_ceil(50),
            Column::Owner(o) => self.len().div_ceil(o.postings.len().max(1)),
            _ => self.len(),
        };
        if batch {
            n.saturating_mul(256).min(self.len())
        } else {
            n.max(1)
        }
    }
    fn indexed(&self, c: usize) -> bool {
        matches!(
            self.columns[c].1,
            Column::Path
                | Column::Id
                | Column::Start
                | Column::Parent
                | Column::Kind
                | Column::Owner(_)
        )
    }
    fn charge(&self, count: usize) -> rusqlite::Result<()> {
        self.meter.charge(count)
    }
    fn range_upper_compatible(&self, lower: usize, upper: usize) -> bool {
        matches!(self.columns[lower].1, Column::Start)
            && matches!(self.columns[upper].1, Column::End)
    }
    fn range_indexed(&self, c: usize) -> bool {
        matches!(self.columns[c].1, Column::Start)
    }
    fn range_lookup(
        &self,
        c: usize,
        lower: Option<(&Value, bool)>,
        upper: Option<(&Value, bool)>,
    ) -> rusqlite::Result<Option<Vec<usize>>> {
        if !self.range_indexed(c) {
            return Ok(None);
        }
        let bound = |value: Option<(&Value, bool)>| -> Option<Option<(i64, bool)>> {
            match value {
                None => Some(None),
                Some((Value::Integer(n), inclusive)) => Some(Some((*n, inclusive))),
                _ => None,
            }
        };
        let (Some(lower), Some(upper)) = (bound(lower), bound(upper)) else {
            return Ok(None);
        };
        let rows = &self.facts.rows;
        let doc = &self.facts.doc;
        let begin = lower.map_or(0, |(n, inclusive)| {
            rows.partition_point(|&id| {
                let start = doc.transducer_span(id).start as i64;
                if inclusive { start < n } else { start <= n }
            })
        });
        let end = upper.map_or(rows.len(), |(n, inclusive)| {
            rows.partition_point(|&id| {
                let start = doc.transducer_span(id).start as i64;
                if inclusive { start <= n } else { start < n }
            })
        });
        self.charge(end.saturating_sub(begin) + 1)?;
        Ok(Some(
            (begin..end)
                .filter_map(|r| self.selected.reverse[r])
                .collect(),
        ))
    }
    fn lookup(&self, c: usize, values: &[Value]) -> rusqlite::Result<Option<Vec<usize>>> {
        let mut rows = Vec::new();
        for value in values {
            self.charge(1)?;
            match (&self.columns[c].1, value) {
                (_, Value::Null) => {}
                (Column::Path, Value::Text(p)) => {
                    if p == &self.path {
                        return Ok(None);
                    }
                }
                (Column::Kind, Value::Text(k)) => {
                    if let Some(v) = self.facts.kinds.get(k) {
                        self.charge(v.len())?;
                        rows.extend_from_slice(v)
                    }
                }
                (Column::Start, Value::Integer(start)) => {
                    if let Ok(start) = usize::try_from(*start) {
                        let begin = self.facts.rows.partition_point(|&id| {
                            self.facts.doc.transducer_span(id).start < start
                        });
                        let end = self.facts.rows.partition_point(|&id| {
                            self.facts.doc.transducer_span(id).start <= start
                        });
                        self.charge(end - begin)?;
                        rows.extend(begin..end);
                    }
                }
                (Column::Id, Value::Integer(id)) => {
                    if let Some(Some(r)) = usize::try_from(*id)
                        .ok()
                        .and_then(|i| self.facts.positions.get(i))
                    {
                        rows.push(*r)
                    }
                }
                (Column::Parent, Value::Integer(id)) => {
                    if let Ok(id) = u32::try_from(*id)
                        && let Some(v) = self.facts.children.get(&id)
                    {
                        self.charge(v.len())?;
                        rows.extend_from_slice(v)
                    }
                }
                (Column::Owner(o), Value::Integer(id)) => {
                    if let Ok(id) = u32::try_from(*id)
                        && let Some(v) = o.postings.get(&id)
                    {
                        self.charge(v.len())?;
                        rows.extend_from_slice(v)
                    }
                }
                _ => return Ok(None),
            }
        }
        let mut rows = rows
            .into_iter()
            .filter_map(|r| self.selected.reverse[r])
            .collect::<Vec<_>>();
        rows.sort_unstable();
        rows.dedup();
        Ok(Some(rows))
    }
    fn column(&self, row: usize, c: usize, ctx: &mut Context) -> rusqlite::Result<()> {
        let ordinal = self.selected.ordinal[row];
        let row = self.selected.rows[row];
        let id = self.facts.rows[row];
        let doc = &self.facts.doc;
        let node = doc.node(id);
        let span = doc.transducer_span(id);
        match &self.columns[c].1 {
            Column::Path => ctx.set_result(&self.path),
            Column::Id => ctx.set_result(&i64::from(id)),
            Column::Start => ctx.set_result(&(span.start as i64)),
            Column::End => ctx.set_result(&(span.end as i64)),
            Column::Parent => ctx.set_result(&doc.transducer_parent(id).map(i64::from)),
            Column::Kind => ctx.set_result(&node.kind().as_ref()),
            Column::Line => ctx.set_result(&(self.facts.lines.line(span.start) as i64)),
            Column::Text => ctx.set_result(&if self
                .empty_text_kinds
                .iter()
                .any(|k| k == node.kind().as_ref())
            {
                ""
            } else {
                let text = doc.transducer_text_str(id);
                source::charge(&self.meter.work, text.len().div_ceil(256))
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))?;
                text
            }),
            Column::Ordinal => ctx.set_result(&(ordinal as i64)),
            Column::Owner(o) => ctx.set_result(&o.values[row].map(i64::from)),
            Column::ParentStart | Column::ParentEnd | Column::ParentKind => {
                let parent = doc.transducer_parent(id);
                match self.columns[c].1 {
                    Column::ParentStart => {
                        ctx.set_result(&parent.map(|p| doc.transducer_span(p).start as i64))
                    }
                    Column::ParentEnd => {
                        ctx.set_result(&parent.map(|p| doc.transducer_span(p).end as i64))
                    }
                    _ => ctx.set_result(&parent.map(|p| doc.node(p).kind().into_owned())),
                }
            }
            Column::Field(field, part) => {
                let value = self.facts.fields.get(&(id, *field)).copied();
                match part {
                    0 => ctx.set_result(&value.map(i64::from)),
                    1 => ctx.set_result(&value.map(|n| doc.transducer_span(n).start as i64)),
                    2 => ctx.set_result(&value.map(|n| doc.transducer_span(n).end as i64)),
                    3 => ctx.set_result(&value.map(|n| doc.node(n).kind().into_owned())),
                    _ => {
                        let text = value.map(|n| doc.transducer_text_str(n));
                        if let Some(text) = text {
                            source::charge(&self.meter.work, text.len().div_ceil(256))
                                .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))?;
                        }
                        ctx.set_result(&text)
                    }
                }
            }
        }
    }
}
fn columns(spec: &Spec) -> Vec<(String, Column)> {
    let mut columns = vec![
        ("__path", Column::Path),
        ("__start", Column::Start),
        ("__end", Column::End),
        ("id", Column::Id),
        ("parent", Column::Parent),
        ("edge_parent", Column::Parent),
        ("kind", Column::Kind),
        ("line", Column::Line),
        ("text", Column::Text),
        ("ordinal", Column::Ordinal),
        ("parent_start", Column::ParentStart),
        ("parent_end", Column::ParentEnd),
        ("parent_kind", Column::ParentKind),
        ("ancestor_start", Column::ParentStart),
        ("ancestor_end", Column::ParentEnd),
        ("ancestor_kind", Column::ParentKind),
    ]
    .into_iter()
    .map(|(n, c)| (n.to_owned(), c))
    .collect::<Vec<_>>();
    for field in &spec.fields {
        for (i, suffix) in ["id", "start", "end", "kind", "source"].iter().enumerate() {
            columns.push((
                format!("{field}_{suffix}"),
                Column::Field(spec.field_codes.get(field).copied().unwrap_or(0), i as u8),
            ))
        }
    }
    if !spec.columns.is_empty() {
        columns.retain(|(name, _)| spec.columns.contains(name));
    }
    columns
}
pub(in crate::code_rule_dsl) fn schema(db: &Connection, spec: &Spec) -> Result<Vec<String>> {
    let mut names = vec![spec.name.clone()];
    let selected = columns(spec);
    if spec.columns.len()
        != spec
            .columns
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
        || spec
            .columns
            .iter()
            .any(|name| !selected.iter().any(|(n, _)| n == name))
        || ["__path", "__start", "__end", "id"]
            .iter()
            .any(|name| !selected.iter().any(|(n, _)| n == name))
    {
        return Err(invalid(
            "syntax columns must name distinct available fields and retain node identity and span",
        ));
    }
    let fields = selected.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    db.execute_batch(&format!(
        "CREATE TABLE {} ({})",
        super::quote(&spec.name),
        fields
            .iter()
            .map(|n| super::quote(n))
            .collect::<Vec<_>>()
            .join(",")
    ))
    .map_err(sql_error)?;
    for (name, fields) in [
        (spec.edges.as_ref(), vec!["__path", "src", "dst", "ordinal"]),
        (
            spec.ownership.as_ref(),
            std::iter::once("__path")
                .chain(std::iter::once("id"))
                .chain(spec.owners.keys().map(String::as_str))
                .collect(),
        ),
    ] {
        if let Some(name) = name {
            db.execute_batch(&format!(
                "CREATE TABLE {} ({})",
                super::quote(name),
                fields
                    .iter()
                    .map(|n| super::quote(n))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .map_err(sql_error)?;
            names.push(name.clone());
        }
    }
    Ok(names)
}
fn load_one(spec: &Spec, path: &str, facts: Arc<Facts>, meter: Arc<Meter>) -> Result<Vec<Nodes>> {
    let selected = facts.select(&spec.exclude_kinds, false)?;
    let mut result = Vec::new();
    let nodes = Nodes {
        path: path.to_owned(),
        facts: Arc::clone(&facts),
        selected: Arc::clone(&selected),
        empty_text_kinds: spec.empty_text_kinds.clone(),
        columns: columns(spec),
        meter: Arc::clone(&meter),
    };
    result.push(nodes);
    if spec.edges.is_some() {
        let nodes = Nodes {
            path: path.to_owned(),
            facts: Arc::clone(&facts),
            selected: facts.select(&spec.exclude_kinds, true)?,
            empty_text_kinds: Vec::new(),
            columns: vec![
                ("__path".into(), Column::Path),
                ("src".into(), Column::Parent),
                ("dst".into(), Column::Id),
                ("ordinal".into(), Column::Ordinal),
            ],
            meter: Arc::clone(&meter),
        };
        result.push(nodes);
    }
    if spec.ownership.is_some() {
        let mut columns = vec![("__path".into(), Column::Path), ("id".into(), Column::Id)];
        for (name, owner) in &spec.owners {
            columns.push((name.clone(), Column::Owner(facts.owner(owner)?)))
        }
        let nodes = Nodes {
            path: path.to_owned(),
            facts,
            selected,
            empty_text_kinds: Vec::new(),
            columns,
            meter,
        };
        result.push(nodes);
    }
    Ok(result)
}

struct Union {
    sources: Vec<Nodes>,
    offsets: Vec<usize>,
}
impl Source for Union {
    fn columns(&self) -> Vec<String> {
        self.sources[0].columns()
    }
    fn column_types(&self) -> Vec<String> {
        self.sources[0].column_types()
    }
    fn len(&self) -> usize {
        *self.offsets.last().unwrap()
    }
    fn estimated_matches(&self, c: usize, b: bool) -> usize {
        self.sources.iter().map(|s| s.estimated_matches(c, b)).sum()
    }
    fn range_upper_compatible(&self, lower: usize, upper: usize) -> bool {
        self.sources[0].range_upper_compatible(lower, upper)
    }
    fn range_indexed(&self, c: usize) -> bool {
        self.sources[0].range_indexed(c)
    }
    fn range_lookup(
        &self,
        c: usize,
        lo: Option<(&Value, bool)>,
        hi: Option<(&Value, bool)>,
    ) -> rusqlite::Result<Option<Vec<usize>>> {
        let mut result = Vec::new();
        for (i, s) in self.sources.iter().enumerate() {
            match s.range_lookup(c, lo, hi)? {
                Some(rows) => result.extend(rows.into_iter().map(|r| r + self.offsets[i])),
                None => return Ok(None),
            }
        }
        Ok(Some(result))
    }
    fn indexed(&self, c: usize) -> bool {
        self.sources[0].indexed(c)
    }
    fn charge(&self, n: usize) -> rusqlite::Result<()> {
        self.sources[0].charge(n)
    }
    fn lookup(&self, c: usize, v: &[Value]) -> rusqlite::Result<Option<Vec<usize>>> {
        let mut result = Vec::new();
        for (i, s) in self.sources.iter().enumerate() {
            match s.lookup(c, v)? {
                Some(rows) => result.extend(rows.into_iter().map(|r| r + self.offsets[i])),
                None => result.extend(self.offsets[i]..self.offsets[i + 1]),
            }
        }
        Ok(Some(result))
    }
    fn column(&self, r: usize, c: usize, ctx: &mut Context) -> rusqlite::Result<()> {
        let i = self.offsets.partition_point(|&offset| offset <= r) - 1;
        self.sources[i].column(r - self.offsets[i], c, ctx)
    }
}
pub(super) type Files = Vec<(String, Arc<Facts>)>;
pub(super) fn load(
    db: &Connection,
    spec: &Spec,
    files: &Files,
    work: Arc<AtomicUsize>,
) -> Result<Vec<String>> {
    let meter = Arc::new(Meter {
        work,
        steps: AtomicUsize::new(0),
    });
    let mut names = vec![spec.name.clone()];
    names.extend(spec.edges.iter().cloned());
    names.extend(spec.ownership.iter().cloned());
    if files.is_empty() {
        schema(db, spec)?;
        return Ok(names);
    }
    let mut families: Vec<Vec<Nodes>> = (0..names.len()).map(|_| Vec::new()).collect();
    for (path, facts) in files {
        for (family, nodes) in
            families
                .iter_mut()
                .zip(load_one(spec, path, Arc::clone(facts), Arc::clone(&meter))?)
        {
            family.push(nodes)
        }
    }
    for (name, sources) in names.iter().zip(families) {
        let mut offsets = vec![0];
        for s in &sources {
            offsets.push(offsets.last().unwrap() + s.len())
        }
        super::table::register(db, name, Arc::new(Union { sources, offsets }))
            .map_err(sql_error)?;
    }
    Ok(names)
}

pub(super) fn closure(
    db: &Connection,
    stage: &super::Stage,
    spec: &super::Reachability,
    syntax: &Spec,
    files: &Files,
    work: &Arc<AtomicUsize>,
) -> Result<()> {
    #[cfg(feature = "code-rule-profile")]
    let mut span = crate::code_rule_dsl::profile::trace::Span::start("sql-reach-walk", &stage.name);
    db.execute_batch(&format!(
        "CREATE TEMP TABLE {} (__path TEXT,root,node)",
        super::quote(&stage.name)
    ))
    .map_err(sql_error)?;
    let mut roots = db
        .prepare(&format!(
            "SELECT DISTINCT __path,id FROM ({}) WHERE __path IS NOT NULL AND id IS NOT NULL",
            spec.roots
        ))
        .map_err(sql_error)?;
    let mut roots = roots.query([]).map_err(sql_error)?;
    let mut output = Vec::new();
    let mut requested = 0;
    let by_path = files
        .iter()
        .map(|(path, facts)| {
            Ok((
                path.as_str(),
                (facts, facts.select(&syntax.exclude_kinds, false)?),
            ))
        })
        .collect::<Result<HashMap<_, _>>>()?;
    while let Some(row) = roots.next().map_err(sql_error)? {
        let path: String = row.get(0).map_err(sql_error)?;
        let root: i64 = row.get(1).map_err(sql_error)?;
        let (facts, selection) = by_path
            .get(path.as_str())
            .ok_or_else(|| invalid("syntax root is outside the eligible files"))?;
        let nodes = facts.descendants(root)?;
        requested += 1;
        source::charge(work, nodes.len().div_ceil(64))?;
        for &node in nodes {
            if facts.positions[node as usize].is_none_or(|r| selection.reverse[r].is_none()) {
                continue;
            }
            output.push([
                Value::Text(path.clone()),
                Value::Integer(root),
                Value::Integer(i64::from(node)),
            ]);
            if output.len() == 256 {
                super::insert::stage_rows(db, &stage.name, 3, output.drain(..), work)?;
            }
        }
    }
    super::insert::stage_rows(db, &stage.name, 3, output, work)?;
    #[cfg(feature = "code-rule-profile")]
    {
        span.detail(|| serde_json::json!({"roots":requested,"directSyntax":true,"batchRows":256}));
        drop(span);
    }
    #[cfg(not(feature = "code-rule-profile"))]
    let _ = requested;
    #[cfg(feature = "code-rule-profile")]
    let _span = crate::code_rule_dsl::profile::trace::Span::start("sql-reach-index", &stage.name);
    db.execute_batch(&format!("CREATE INDEX {} ON {} (__path,root,node);CREATE INDEX {} ON {} (__path,node,root);ANALYZE temp.{}",super::quote(&format!("__{}_forward",stage.name)),super::quote(&stage.name),super::quote(&format!("__{}_reverse",stage.name)),super::quote(&stage.name),super::quote(&stage.name))).map_err(sql_error)?;
    Ok(())
}
