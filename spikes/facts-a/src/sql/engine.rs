//! Planner and executor for the SQLite SELECT subset. Plans are fixed by the SQL text: joins run
//! in FROM order as index nested loops; equality conjuncts between a FROM item's column and
//! earlier items (or outer rows, or constants) probe a hash index built once per table; every
//! other conjunct is applied at the earliest level where its columns are bound. Correlated
//! subqueries probe the same indexes; uncorrelated ones run once per statement execution.
use super::parse::{self, Body, Expr, JoinKind, Query, SelItem, SetOp, Source};
use super::value::{self as sv, Aff, DKey, Key, V};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub cols: Vec<String>,
    pub affs: Vec<Aff>,
}
impl Schema {
    fn find(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.eq_ignore_ascii_case(name))
    }
}

/// A gather index entry naming no row: the null-extended row of a LEFT JOIN.
pub const NULL_ROW: u32 = u32::MAX;

/// One column of a relation. Results of projections, sorts, filters and unions are views
/// (`Gather`, `Chain`) over the columns they read, so a stage that selects `n.*` stores one row
/// index per binding, not a copy of every value.
#[derive(Debug)]
pub enum Column {
    /// Stored values.
    Vals(Vec<V>),
    /// The rows of `src` named by `idx` (`None`: every row, in order; `NULL_ROW`: NULL), each
    /// converted by `aff` (CTAS storage; `Aff::None` converts nothing).
    Gather { src: Rc<Column>, idx: Option<Rc<[u32]>>, aff: Aff },
    /// Concatenation (UNION ALL): each part with its first row, in order, none empty.
    Chain(Vec<(u32, Rc<Column>)>),
    /// Text read from the file: a cell is a byte range of `source` (a node's text, read on
    /// first use and kept) or, with start `TEXT_VAL`, an index into `vals`. Node texts nest, so
    /// storing them would cost O(nodes × depth) bytes. 8 bytes a row; the read cache (16 bytes a
    /// row) exists once a row is read.
    Text { source: Rc<str>, cells: Vec<(u32, u32)>, vals: Vec<V>, cache: std::cell::OnceCell<Box<[std::cell::OnceCell<Rc<str>>]>> },
    /// The same value on every row (a file's path).
    Const(V),
    /// Integers from fact columns (ids, offsets, lines, ordinals): 4 bytes a row, `U32_NULL` is
    /// NULL.
    U32(Vec<u32>),
    /// Interned names from fact columns (node kinds), `facts::NONE` is NULL.
    Sym(Vec<u32>),
    /// Tree reachability as interval membership (I2: the descendants of `id` are the nodes in
    /// `(id, subtree_end)`): per root `k`, rows `offs[k]..offs[k + 1]` are `eligible[lo[k]..]`,
    /// the eligible nodes of its subtree in preorder. `node` selects the column: the reached
    /// node, or the root. O(roots + nodes) bytes for O(roots × depth) rows.
    Interval { roots: Rc<[u32]>, lo: Rc<[u32]>, offs: Rc<[u32]>, eligible: Rc<[u32]>, node: bool },
}

/// `Column::U32`'s NULL.
pub const U32_NULL: u32 = u32::MAX;
/// A `Column::Text` cell holding a value (`(TEXT_VAL, index into vals)`).
const TEXT_VAL: u32 = u32::MAX;

#[derive(Debug)]
pub enum TextCell {
    Span(u32, u32),
    Val(V),
}

impl Column {
    pub fn text(source: Rc<str>, cells: Vec<TextCell>) -> Column {
        let mut vals = Vec::new();
        let cells = cells
            .into_iter()
            .map(|c| match c {
                TextCell::Span(s, e) => (s, e),
                TextCell::Val(v) => {
                    vals.push(v);
                    (TEXT_VAL, vals.len() as u32 - 1)
                }
            })
            .collect();
        Column::Text { source, cells, vals, cache: std::cell::OnceCell::new() }
    }

    pub fn get(&self, r: u32) -> V {
        match self {
            Column::Vals(v) => v[r as usize].clone(),
            Column::Gather { src, idx, aff } => {
                let s = match idx {
                    Some(i) => i[r as usize],
                    None => r,
                };
                if s == NULL_ROW {
                    return V::Null;
                }
                let v = src.get(s);
                if *aff == Aff::None { v } else { sv::apply_affinity(v, *aff) }
            }
            Column::Chain(parts) => {
                let p = parts.partition_point(|(start, _)| *start <= r) - 1;
                parts[p].1.get(r - parts[p].0)
            }
            Column::Text { source, cells, vals, cache } => match cells[r as usize] {
                (TEXT_VAL, i) => vals[i as usize].clone(),
                (s, e) => {
                    let cache = cache.get_or_init(|| (0..cells.len()).map(|_| std::cell::OnceCell::new()).collect());
                    V::Text(cache[r as usize].get_or_init(|| Rc::from(&source[s as usize..e as usize])).clone())
                }
            },
            Column::Const(v) => v.clone(),
            Column::U32(v) => match v[r as usize] {
                U32_NULL => V::Null,
                x => V::Int(x as i64),
            },
            Column::Sym(v) => match v[r as usize] {
                crate::facts::NONE => V::Null,
                s => V::Text(crate::facts::name(s)),
            },
            Column::Interval { roots, lo, offs, eligible, node } => {
                let k = offs.partition_point(|&o| o <= r) - 1;
                V::Int(if *node { eligible[(lo[k] + (r - offs[k])) as usize] } else { roots[k] } as i64)
            }
        }
    }

    /// `col` with every value converted by `aff` on read (CTAS storage).
    pub fn with_aff(col: &Rc<Column>, aff: Aff) -> Rc<Column> {
        if aff == Aff::None {
            return col.clone();
        }
        match &**col {
            Column::Gather { src, idx, aff: Aff::None } => Rc::new(Column::Gather { src: src.clone(), idx: idx.clone(), aff }),
            _ => Rc::new(Column::Gather { src: col.clone(), idx: None, aff }),
        }
    }

    /// Bytes this column holds itself (values, text payloads, indexes); shared parts counted
    /// once per reference.
    pub fn bytes(&self) -> usize {
        match self {
            Column::Vals(v) => v.len() * std::mem::size_of::<V>() + v.iter().map(|x| if let V::Text(s) = x { s.len() } else { 0 }).sum::<usize>(),
            Column::Gather { idx, .. } => idx.as_ref().map_or(0, |i| i.len() * 4),
            Column::Chain(p) => p.len() * 24,
            Column::Text { cells, vals, cache, .. } => {
                cells.len() * 8 + vals.len() * std::mem::size_of::<V>() + cache.get().map_or(0, |c| c.len() * 16 + c.iter().filter_map(|c| c.get()).map(|s| s.len()).sum::<usize>())
            }
            Column::Const(_) => std::mem::size_of::<V>(),
            Column::U32(v) | Column::Sym(v) => v.len() * 4,
            Column::Interval { roots, lo, offs, eligible, .. } => (roots.len() + lo.len() + offs.len() + eligible.len()) * 4,
        }
    }

    /// Levels of indirection a read goes through.
    fn depth(&self) -> usize {
        match self {
            Column::Vals(_) | Column::Text { .. } | Column::Const(_) | Column::U32(_) | Column::Sym(_) | Column::Interval { .. } => 0,
            Column::Gather { src, .. } => 1 + src.depth(),
            Column::Chain(p) => 1 + p.iter().map(|(_, c)| c.depth()).max().unwrap_or(0),
        }
    }
}

/// Reads deeper than this materialise (a gather over a chain does not compose; recursion could
/// otherwise nest one level per iteration).
const MAX_DEPTH: usize = 5;

/// Composes gather indexes: `idx` over a column that is itself a gather reads its source
/// directly. `memo` shares one composed index among the columns of one source table.
fn gather(src: &Rc<Column>, idx: &Rc<[u32]>, memo: &mut FxHashMap<(usize, usize), Rc<[u32]>>) -> Rc<Column> {
    match &**src {
        Column::Gather { src: s2, idx: i2, aff } => {
            let composed = match i2 {
                None => idx.clone(),
                Some(i2) => memo
                    .entry((Rc::as_ptr(idx) as *const u32 as usize, Rc::as_ptr(i2) as *const u32 as usize))
                    .or_insert_with(|| idx.iter().map(|&k| if k == NULL_ROW { NULL_ROW } else { i2[k as usize] }).collect())
                    .clone(),
            };
            Rc::new(Column::Gather { src: s2.clone(), idx: Some(composed), aff: *aff })
        }
        _ if src.depth() >= MAX_DEPTH => Rc::new(Column::Vals(idx.iter().map(|&k| if k == NULL_ROW { V::Null } else { src.get(k) }).collect())),
        _ => Rc::new(Column::Gather { src: src.clone(), idx: Some(idx.clone()), aff: Aff::None }),
    }
}

/// UNION ALL of columns of `n` rows each. Adjacent gathers of one source merge into one.
fn chain(cols: &[(&Rc<Column>, usize)]) -> Rc<Column> {
    let mut parts: Vec<(u32, Rc<Column>)> = Vec::new();
    let mut start = 0u32;
    let mut push = |parts: &mut Vec<(u32, Rc<Column>)>, s: u32, c: &Rc<Column>| {
        if let (Some((_, last)), Column::Gather { src, idx: Some(i), aff }) = (parts.last_mut(), &**c) {
            if let Column::Gather { src: ls, idx: Some(li), aff: la } = &**last {
                if Rc::ptr_eq(src, ls) && aff == la {
                    let merged: Rc<[u32]> = li.iter().chain(i.iter()).copied().collect();
                    *last = Rc::new(Column::Gather { src: src.clone(), idx: Some(merged), aff: *aff });
                    return;
                }
            }
        }
        parts.push((s, c.clone()));
    };
    for (c, n) in cols {
        if *n == 0 {
            continue;
        }
        match &***c {
            Column::Chain(ps) => ps.iter().for_each(|(s, p)| push(&mut parts, start + s, p)),
            _ => push(&mut parts, start, c),
        }
        start += *n as u32;
    }
    if parts.len() == 1 {
        return parts.pop().unwrap().1;
    }
    if parts.is_empty() {
        return Rc::new(Column::Vals(Vec::new()));
    }
    Rc::new(Column::Chain(parts))
}

/// UNION ALL of tables (each at least `w` columns wide).
fn concat(schema: Schema, w: usize, parts: &[Table]) -> Table {
    let len = parts.iter().map(Table::len).sum();
    let cols = (0..w).map(|c| chain(&parts.iter().map(|t| (&t.cols[c], t.len())).collect::<Vec<_>>())).collect();
    Table { schema, cols, len }
}

/// Set semantics over rows (UNION, recursive UNION): each column's values interned by their
/// distinctness key, rows compared as tuples of interned ids.
struct Distinct {
    ids: Vec<FxHashMap<DKey, u32>>,
    narrow: rustc_hash::FxHashSet<[u32; 4]>,
    wide: rustc_hash::FxHashSet<Box<[u32]>>,
}

impl Distinct {
    fn new(width: usize) -> Distinct {
        Distinct { ids: (0..width).map(|_| FxHashMap::default()).collect(), narrow: Default::default(), wide: Default::default() }
    }
    /// Column `c`'s id for value `v`.
    fn intern(&mut self, c: usize, v: &V) -> u32 {
        let ids = &mut self.ids[c];
        let n = ids.len() as u32;
        *ids.entry(sv::dkey(v)).or_insert(n)
    }
    /// Whether the row of interned ids `key` is new; records it.
    fn insert_key(&mut self, key: &[u32]) -> bool {
        if key.len() <= 4 {
            let mut k = [0u32; 4];
            k[..key.len()].copy_from_slice(key);
            self.narrow.insert(k)
        } else {
            self.wide.insert(key.into())
        }
    }
    /// Whether row `r` of `t` (its first `width` columns) is new; records it.
    fn insert(&mut self, t: &Table, r: usize) -> bool {
        let key: Vec<u32> = (0..self.ids.len()).map(|c| self.intern(c, &t.value(r, c))).collect();
        self.insert_key(&key)
    }
}

/// A relation: columns of equal length. `cols` may be wider than `schema` (ORDER BY keys of a
/// select ride along until the sort).
#[derive(Debug, Default)]
pub struct Table {
    pub schema: Schema,
    pub cols: Vec<Rc<Column>>,
    len: usize,
}

impl Table {
    pub fn new(schema: Schema, cols: Vec<Rc<Column>>, len: usize) -> Table {
        Table { schema, cols, len }
    }
    pub fn empty(schema: Schema) -> Table {
        let cols = schema.cols.iter().map(|_| Rc::new(Column::Vals(Vec::new()))).collect();
        Table { schema, cols, len: 0 }
    }
    /// Rows of `width` = the schema's width (longer rows are cut).
    pub fn from_rows(schema: Schema, rows: Vec<Box<[V]>>) -> Table {
        let w = schema.cols.len();
        let len = rows.len();
        let mut cols: Vec<Vec<V>> = (0..w).map(|_| Vec::with_capacity(len)).collect();
        for r in rows {
            for (c, v) in cols.iter_mut().zip(r.into_vec()) {
                c.push(v);
            }
        }
        Table { schema, cols: cols.into_iter().map(|c| Rc::new(Column::Vals(c))).collect(), len }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn value(&self, r: usize, c: usize) -> V {
        self.cols.get(c).map_or(V::Null, |col| col.get(r as u32))
    }
    /// One row's values over the schema's columns.
    pub fn row(&self, r: usize) -> Vec<V> {
        (0..self.schema.cols.len()).map(|c| self.value(r, c)).collect()
    }
    /// The rows named by `idx`, as views.
    pub fn select(&self, idx: Rc<[u32]>) -> Table {
        let mut memo = FxHashMap::default();
        let len = idx.len();
        Table { schema: self.schema.clone(), cols: self.cols.iter().map(|c| gather(c, &idx, &mut memo)).collect(), len }
    }
    /// The first `w` columns.
    fn truncate(mut self, w: usize) -> Table {
        self.cols.truncate(w);
        self
    }
    pub fn bytes(&self) -> usize {
        self.cols.iter().map(|c| c.bytes()).sum()
    }
}


// ---------------------------------------------------------------------------------------------
// Plan

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Func {
    Coalesce,
    Ifnull,
    Length,
    Lower,
    Upper,
    Trim,
    Ltrim,
    Rtrim,
    Substr,
    Instr,
    Replace,
    Char,
    JsonQuote,
    JsonExtract,
    RxMatches,
    RxExtract,
    RxReplace,
    SourceText,
    Abs,
    Typeof,
    Max,
    Min,
    Iif,
    Nullif,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AggFn {
    Count,
    CountStar,
    Min,
    Max,
    GroupConcat,
    Sum,
    Total,
}

/// A planned literal. Plans are shared by every worker (`Send + Sync`), so a text literal is an
/// index into its statement's `texts`; each execution materialises it once, on first use.
#[derive(Clone, Copy, Debug)]
pub enum Lit {
    Null,
    Int(i64),
    Real(f64),
    Text(u32),
}

#[derive(Clone, Debug)]
pub enum PE {
    Lit(Lit),
    Col { depth: u16, item: u16, col: u16 },
    Agg(u16),
    Win(u16),
    Not(Box<PE>),
    Neg(Box<PE>),
    Cmp(Cmp, Box<PE>, Box<PE>, (Aff, Aff)),
    Arith(char, Box<PE>, Box<PE>),
    Concat(Box<PE>, Box<PE>),
    And(Box<PE>, Box<PE>),
    Or(Box<PE>, Box<PE>),
    Func(Func, Vec<PE>),
    Case { operand: Option<Box<PE>>, aff: (Aff, Aff), whens: Vec<(PE, PE)>, else_: Option<Box<PE>> },
    Cast(Box<PE>, Aff),
    InList { e: Box<PE>, list: Vec<(PE, (Aff, Aff))>, not: bool },
    InSub { e: Box<PE>, sub: usize, not: bool, aff: (Aff, Aff) },
    Exists { sub: usize, not: bool },
    Scalar(usize),
    IsNull(Box<PE>, bool),
    Is(Box<PE>, Box<PE>, bool),
    Like(Box<PE>, Box<PE>, bool),
}

#[derive(Clone, Debug)]
pub(crate) enum Src {
    Global(String),
    Cte(usize),
    Derived(usize),
    JsonEach(PE),
}

#[derive(Clone, Debug)]
pub(crate) struct PItem {
    pub(crate) src: Src,
    pub(crate) width: usize,
    pub(crate) left: bool,
    /// A table function whose argument reads earlier items: evaluated per binding.
    pub(crate) lateral: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Probe {
    pub(crate) cols: Vec<u16>,
    pub(crate) col_affs: Vec<Aff>,
    pub(crate) exprs: Vec<PE>,
    pub(crate) expr_affs: Vec<Aff>,
}

#[derive(Clone, Debug)]
pub(crate) struct AggSpec {
    pub(crate) f: AggFn,
    pub(crate) arg: Option<PE>,
    pub(crate) sep: Option<PE>,
    pub(crate) distinct: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct WinSpec {
    pub(crate) partition: Vec<PE>,
    pub(crate) order: Vec<(PE, bool)>,
}

#[derive(Clone, Debug)]
pub(crate) struct PSelect {
    pub(crate) items: Vec<PItem>,
    pub(crate) probes: Vec<Option<Probe>>,
    /// Per item, the branch probes of an OR conjunct (candidates = union of the lookups).
    pub(crate) or_probes: Vec<Option<Vec<Probe>>>,
    /// Conjuncts tested once all of items[..=i] are bound (inner items; ON of left items).
    pub(crate) level: Vec<Vec<PE>>,
    /// WHERE conjuncts whose last item is a left-joined item i (after null extension).
    pub(crate) post: Vec<Vec<PE>>,
    /// Conjuncts with no item reference.
    pub(crate) konst: Vec<PE>,
    pub(crate) grouped: bool,
    pub(crate) group_by: Vec<PE>,
    pub(crate) aggs: Vec<AggSpec>,
    pub(crate) having: Option<PE>,
    pub(crate) wins: Vec<WinSpec>,
    pub(crate) proj: Vec<PE>,
    /// ORDER BY keys of a simple query, evaluated at projection time.
    pub(crate) order_keys: Vec<PE>,
    pub(crate) distinct: bool,
    pub(crate) out: Schema,
}

#[derive(Clone, Debug)]
pub(crate) enum PBody {
    Select(Box<PSelect>),
    Compound(Box<PBody>, SetOp, Box<PBody>, Schema),
}
impl PBody {
    fn schema(&self) -> &Schema {
        match self {
            PBody::Select(s) => &s.out,
            PBody::Compound(_, _, _, s) => s,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PCte {
    Plain { slot: usize, q: PQuery },
    Recursive { slot: usize, batch: usize, init: Vec<PBody>, rec: Vec<PBody>, distinct: bool, schema: Schema },
}

#[derive(Clone, Debug)]
pub struct PQuery {
    pub(crate) ctes: Vec<PCte>,
    pub(crate) body: PBody,
    /// Sort keys: for a simple select, indices into the extra keys its projection appends;
    /// for a compound, output column indices.
    pub(crate) order: Vec<(usize, bool)>,
    pub(crate) order_on_output: bool,
    pub(crate) limit: Option<PE>,
    pub(crate) offset: Option<PE>,
    pub out: Schema,
}

#[derive(Clone, Debug)]
pub(crate) struct SubPlan {
    pub(crate) q: PQuery,
    pub(crate) cacheable: bool,
}

#[derive(Clone, Debug)]
pub struct Statement {
    pub(crate) root: PQuery,
    pub(crate) subs: Vec<SubPlan>,
    pub(crate) n_slots: usize,
    pub reads: Vec<String>,
    /// Text literals (`Lit::Text`).
    pub(crate) texts: Vec<Box<str>>,
}
impl Statement {
    pub fn schema(&self) -> &Schema {
        &self.root.out
    }
}

pub type Catalog = HashMap<String, Schema>;

struct ScopeItem {
    name: String,
    schema: Schema,
}
struct Scope {
    items: Vec<ScopeItem>,
    /// Result aliases of the select (for alias references in WHERE/GROUP BY/HAVING/ORDER BY).
    aliases: Vec<(String, Expr)>,
    /// Aggregates and windows collected while planning this select's projection.
    aggs: Vec<AggSpec>,
    wins: Vec<WinSpec>,
    allow_agg: bool,
}

struct SubBound {
    scope_depth: usize,
    correlated: bool,
    recursive_ref: bool,
}

pub struct Planner<'c> {
    catalog: &'c Catalog,
    scopes: Vec<Scope>,
    ctes: Vec<(String, usize, Schema, bool)>,
    subs: Vec<SubPlan>,
    n_slots: usize,
    bounds: Vec<SubBound>,
    reads: Vec<String>,
    texts: Vec<Box<str>>,
}

fn lc(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn conjuncts(e: Expr, out: &mut Vec<Expr>) {
    match e {
        Expr::Bin("and", l, r) => {
            conjuncts(*l, out);
            conjuncts(*r, out);
        }
        o => out.push(o),
    }
}

fn disjuncts<'e>(e: &'e PE, out: &mut Vec<&'e PE>) {
    match e {
        PE::Or(l, r) => {
            disjuncts(l, out);
            disjuncts(r, out);
        }
        o => out.push(o),
    }
}

fn pe_conjuncts<'e>(e: &'e PE, out: &mut Vec<&'e PE>) {
    match e {
        PE::And(l, r) => {
            pe_conjuncts(l, out);
            pe_conjuncts(r, out);
        }
        o => out.push(o),
    }
}

/// Adds `c` to `probe` when it is an equality between a column of item `i` and an expression
/// over earlier items, outer rows or constants.
fn probe_eq(c: &PE, i: usize, probe: &mut Probe) -> bool {
    let PE::Cmp(Cmp::Eq, l, r, (al, ar)) = c else { return false };
    let pick = |a: &PE, b: &PE| -> Option<u16> {
        if let PE::Col { depth: 0, item, col } = a {
            if *item as usize == i && max_item(b).is_none_or(|m| m < i) && !refs_item(b, i) && !has_agg(b) {
                return Some(*col);
            }
        }
        None
    };
    if let Some(col) = pick(l, r) {
        probe.cols.push(col);
        probe.col_affs.push(*al);
        probe.exprs.push((**r).clone());
        probe.expr_affs.push(*ar);
        return true;
    }
    if let Some(col) = pick(r, l) {
        probe.cols.push(col);
        probe.col_affs.push(*ar);
        probe.exprs.push((**l).clone());
        probe.expr_affs.push(*al);
        return true;
    }
    false
}

pub fn plan(sql: &str, catalog: &Catalog) -> Result<Statement, String> {
    let q = parse::parse_query(sql)?;
    let mut p = Planner { catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new(), texts: Vec::new() };
    let root = p.query(&q)?;
    let mut seen = std::collections::BTreeSet::new();
    let reads = p.reads.into_iter().filter(|r| seen.insert(r.clone())).collect();
    Ok(Statement { root, subs: p.subs, n_slots: p.n_slots, reads, texts: p.texts })
}

/// Plans a scalar expression over one table's columns (a `deferred` column).
pub fn plan_scalar(sql: &str, schema: &Schema) -> Result<(PE, Statement), String> {
    let e = parse::parse_expr(sql)?;
    let catalog = Catalog::new();
    let mut p = Planner { catalog: &catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new(), texts: Vec::new() };
    p.scopes.push(Scope { items: vec![ScopeItem { name: "__row".into(), schema: schema.clone() }], aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
    let pe = p.expr(&e)?;
    let stmt = Statement { root: PQuery { ctes: vec![], body: PBody::Compound(Box::new(PBody::Select(Box::new(empty_select()))), SetOp::UnionAll, Box::new(PBody::Select(Box::new(empty_select()))), Schema::default()), order: vec![], order_on_output: false, limit: None, offset: None, out: Schema::default() }, subs: p.subs, n_slots: p.n_slots, reads: p.reads, texts: p.texts };
    Ok((pe, stmt))
}

fn empty_select() -> PSelect {
    PSelect { items: vec![], probes: vec![], or_probes: vec![], level: vec![], post: vec![], konst: vec![], grouped: false, group_by: vec![], aggs: vec![], having: None, wins: vec![], proj: vec![], order_keys: vec![], distinct: false, out: Schema::default() }
}

impl<'c> Planner<'c> {
    fn new_slot(&mut self) -> usize {
        self.n_slots += 1;
        self.n_slots - 1
    }

    fn lit(&mut self, v: &V) -> PE {
        PE::Lit(match v {
            V::Null => Lit::Null,
            V::Int(i) => Lit::Int(*i),
            V::Real(f) => Lit::Real(*f),
            V::Text(s) => {
                self.texts.push(Box::from(&**s));
                Lit::Text(self.texts.len() as u32 - 1)
            }
        })
    }

    fn query(&mut self, q: &Query) -> Result<PQuery, String> {
        let cte_mark = self.ctes.len();
        let mut ctes = Vec::new();
        for cte in &q.ctes {
            let self_ref = q.recursive && body_refs(&cte.q.body, &cte.name);
            if self_ref {
                ctes.push(self.recursive_cte(cte)?);
            } else {
                let pq = self.query(&cte.q)?;
                let mut schema = pq.out.clone();
                if let Some(cols) = &cte.cols {
                    if cols.len() != schema.cols.len() {
                        return Err(format!("CTE {} column count", cte.name));
                    }
                    schema.cols = cols.clone();
                }
                let slot = self.new_slot();
                self.ctes.push((lc(&cte.name), slot, schema, false));
                ctes.push(PCte::Plain { slot, q: pq });
            }
        }
        let body = self.body(&q.body, Some(q))?;
        let out = body.schema().clone();
        // ORDER BY: a simple select's keys were appended by `body`; a compound's resolve against
        // its output columns.
        let (order, order_on_output) = match &body {
            PBody::Select(s) => ((0..q.order.len()).map(|i| (s.out.cols.len() + i, q.order[i].1)).collect(), false),
            PBody::Compound(..) => {
                let mut v = Vec::new();
                for (e, desc) in &q.order {
                    let idx = match e {
                        Expr::Col { table: None, name } => out.find(name).ok_or_else(|| format!("ORDER BY {name}"))?,
                        Expr::Lit(V::Int(i)) => (*i as usize).checked_sub(1).ok_or("ORDER BY 0")?,
                        _ => return Err("compound ORDER BY expression unsupported".into()),
                    };
                    v.push((idx, *desc));
                }
                (v, true)
            }
        };
        let limit = q.limit.as_ref().map(|e| self.const_expr(e)).transpose()?;
        let offset = q.offset.as_ref().map(|e| self.const_expr(e)).transpose()?;
        self.ctes.truncate(cte_mark);
        Ok(PQuery { ctes, body, order, order_on_output, limit, offset, out })
    }

    fn const_expr(&mut self, e: &Expr) -> Result<PE, String> {
        self.scopes.push(Scope { items: vec![], aliases: vec![], aggs: vec![], wins: vec![], allow_agg: false });
        let r = self.expr(e);
        self.scopes.pop();
        r
    }

    fn recursive_cte(&mut self, cte: &parse::Cte) -> Result<PCte, String> {
        if !cte.q.order.is_empty() || cte.q.limit.is_some() || !cte.q.ctes.is_empty() {
            return Err("recursive CTE with ORDER BY/LIMIT/WITH unsupported".into());
        }
        let mut arms = Vec::new();
        let mut ops = Vec::new();
        flatten(&cte.q.body, &mut arms, &mut ops);
        let first_rec = arms.iter().position(|a| body_refs(a, &cte.name)).ok_or("recursive CTE without recursive arm")?;
        if first_rec == 0 {
            return Err("recursive CTE must start with a non-recursive arm".into());
        }
        let distinct = match ops[first_rec - 1] {
            SetOp::Union => true,
            SetOp::UnionAll => false,
        };
        if ops.iter().any(|o| *o != ops[first_rec - 1]) {
            return Err("recursive CTE mixing UNION and UNION ALL unsupported".into());
        }
        let mut init = Vec::new();
        for a in &arms[..first_rec] {
            init.push(self.body(a, None)?);
        }
        let mut schema = init[0].schema().clone();
        if let Some(cols) = &cte.cols {
            if cols.len() != schema.cols.len() {
                return Err("recursive CTE column count".into());
            }
            schema.cols = cols.clone();
        }
        let slot = self.new_slot();
        let batch = self.new_slot();
        self.ctes.push((lc(&cte.name), batch, schema.clone(), true));
        let mut rec = Vec::new();
        for a in &arms[first_rec..] {
            if !body_refs(a, &cte.name) {
                return Err("non-recursive arm after a recursive arm".into());
            }
            rec.push(self.body(a, None)?);
        }
        self.ctes.pop();
        self.ctes.push((lc(&cte.name), slot, schema.clone(), false));
        Ok(PCte::Recursive { slot, batch, init, rec, distinct, schema })
    }

    fn body(&mut self, b: &Body, order_of: Option<&Query>) -> Result<PBody, String> {
        match b {
            Body::Select(s) => {
                let order: &[(Expr, bool)] = order_of.map_or(&[], |q| &q.order);
                Ok(PBody::Select(Box::new(self.select(s, order)?)))
            }
            Body::Compound(l, op, r) => {
                let pl = self.body(l, None)?;
                let pr = self.body(r, None)?;
                if pl.schema().cols.len() != pr.schema().cols.len() {
                    return Err("compound arms differ in width".into());
                }
                // SQLite: a compound's column names come from its left-most arm; its affinity is
                // the left-most arm's.
                let schema = pl.schema().clone();
                Ok(PBody::Compound(Box::new(pl), *op, Box::new(pr), schema))
            }
        }
    }

    fn source_schema(&mut self, src: &Source) -> Result<(Src, Schema, String), String> {
        match src {
            Source::Table(name) => {
                let l = lc(name);
                if let Some((_, slot, schema, rec)) = self.ctes.iter().rev().find(|c| c.0 == l) {
                    if *rec {
                        if let Some(b) = self.bounds.last_mut() {
                            b.recursive_ref = true;
                        }
                        for b in self.bounds.iter_mut() {
                            b.recursive_ref = true;
                        }
                    }
                    return Ok((Src::Cte(*slot), schema.clone(), name.clone()));
                }
                let schema = self.catalog.get(&l).ok_or_else(|| format!("no such table: {name}"))?.clone();
                self.reads.push(l);
                Ok((Src::Global(name.clone()), schema, name.clone()))
            }
            Source::Func(f, args) => {
                if !f.eq_ignore_ascii_case("json_each") || args.len() != 1 {
                    return Err(format!("table function {f} unsupported"));
                }
                let arg = self.expr(&args[0])?;
                let schema = Schema {
                    cols: ["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|s| s.to_string()).collect(),
                    affs: vec![Aff::None; 8],
                };
                Ok((Src::JsonEach(arg), schema, f.clone()))
            }
            Source::Sub(q) => {
                let id = self.subquery(q)?;
                let schema = self.subs[id].q.out.clone();
                Ok((Src::Derived(id), schema, String::new()))
            }
        }
    }

    fn subquery(&mut self, q: &Query) -> Result<usize, String> {
        self.bounds.push(SubBound { scope_depth: self.scopes.len(), correlated: false, recursive_ref: false });
        let pq = self.query(q);
        let b = self.bounds.pop().unwrap();
        let pq = pq?;
        if b.recursive_ref {
            for o in self.bounds.iter_mut() {
                o.recursive_ref = true;
            }
        }
        self.subs.push(SubPlan { q: pq, cacheable: !b.correlated && !b.recursive_ref });
        Ok(self.subs.len() - 1)
    }

    fn select(&mut self, s: &parse::Select, order: &[(Expr, bool)]) -> Result<PSelect, String> {
        // FROM items are resolved in the enclosing scope first (derived tables cannot see this
        // select's own items), then pushed one by one.
        self.scopes.push(Scope { items: Vec::new(), aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
        let mut items = Vec::new();
        for fi in &s.from {
            let (src, schema, name) = self.source_schema(&fi.source)?;
            let alias = fi.alias.clone().unwrap_or(name);
            let lateral = matches!(&src, Src::JsonEach(arg) if max_item(arg).is_some());
            items.push(PItem { src, width: schema.cols.len(), left: fi.kind == JoinKind::Left, lateral });
            self.scopes.last_mut().unwrap().items.push(ScopeItem { name: lc(&alias), schema });
        }
        // Result aliases.
        let mut aliases = Vec::new();
        for it in &s.items {
            if let SelItem::Expr { e, alias: Some(a), .. } = it {
                aliases.push((lc(a), e.clone()));
            }
        }
        self.scopes.last_mut().unwrap().aliases = aliases;
        let n = items.len();
        let mut level: Vec<Vec<PE>> = vec![Vec::new(); n];
        let mut post: Vec<Vec<PE>> = vec![Vec::new(); n];
        let mut konst = Vec::new();
        let mut probes: Vec<Option<Probe>> = vec![None; n];
        let mut or_probes: Vec<Option<Vec<Probe>>> = vec![None; n];
        // Conjunct placement.
        let mut placed: Vec<(PE, Option<usize>, bool)> = Vec::new(); // (expr, max item, from ON of a left item)
        for (i, fi) in s.from.iter().enumerate() {
            if let Some(on) = &fi.on {
                let mut cs = Vec::new();
                conjuncts(on.clone(), &mut cs);
                for c in cs {
                    let pe = self.expr(&c)?;
                    if fi.kind == JoinKind::Left {
                        placed.push((pe, Some(i), true));
                    } else {
                        let m = max_item(&pe);
                        placed.push((pe, m, false));
                    }
                }
            }
        }
        if let Some(w) = &s.where_ {
            let mut cs = Vec::new();
            conjuncts(w.clone(), &mut cs);
            for c in cs {
                let pe = self.expr(&c)?;
                let m = max_item(&pe);
                placed.push((pe, m, false));
            }
        }
        for (pe, m, left_on) in placed {
            // A conjunct holding a subquery may read any item: it runs at the last one.
            let m = m.map(|i| i.min(n.saturating_sub(1))).filter(|_| n > 0);
            match m {
                None => konst.push(pe),
                Some(i) if left_on => level[i].push(pe),
                Some(i) if items[i].left => post[i].push(pe),
                Some(i) => level[i].push(pe),
            }
        }
        // Probes: equality between a column of item i and an expression over earlier items,
        // outer rows or constants.
        for i in 0..n {
            if items[i].lateral {
                continue;
            }
            let mut probe = Probe { cols: vec![], col_affs: vec![], exprs: vec![], expr_affs: vec![] };
            let mut rest = Vec::new();
            for c in std::mem::take(&mut level[i]) {
                if !probe_eq(&c, i, &mut probe) {
                    rest.push(c);
                }
            }
            // OR probes (SQLite's OR optimisation): a conjunct `b1 OR b2 OR ...` whose every
            // branch holds such an equality. Candidates are the union of the branches' lookups,
            // each with the plain equalities above too, in table order; the OR conjunct stays in
            // `level[i]` and is re-checked, so the rows are exactly the filtered scan's.
            or_probes[i] = rest.iter().find_map(|c| {
                let mut branches = Vec::new();
                disjuncts(c, &mut branches);
                if branches.len() < 2 {
                    return None;
                }
                branches
                    .into_iter()
                    .map(|b| {
                        let mut cs = Vec::new();
                        pe_conjuncts(b, &mut cs);
                        let mut bp = probe.clone();
                        let found = cs.into_iter().fold(false, |found, c| probe_eq(c, i, &mut bp) || found);
                        found.then_some(bp)
                    })
                    .collect::<Option<Vec<Probe>>>()
            });
            level[i] = rest;
            if !probe.cols.is_empty() {
                probes[i] = Some(probe);
            }
        }
        // Projection.
        self.scopes.last_mut().unwrap().allow_agg = true;
        let mut proj = Vec::new();
        let mut out = Schema::default();
        let n_items = self.scopes.last().unwrap().items.len();
        for it in &s.items {
            match it {
                SelItem::Star => {
                    for ii in 0..n_items {
                        let sc = self.scopes.last().unwrap().items[ii].schema.clone();
                        for (ci, c) in sc.cols.iter().enumerate() {
                            proj.push(PE::Col { depth: 0, item: ii as u16, col: ci as u16 });
                            out.cols.push(c.clone());
                            out.affs.push(sc.affs[ci]);
                        }
                    }
                }
                SelItem::TableStar(t) => {
                    let tl = lc(t);
                    let ii = self.scopes.last().unwrap().items.iter().position(|x| x.name == tl).ok_or_else(|| format!("no such table: {t}"))?;
                    let sc = self.scopes.last().unwrap().items[ii].schema.clone();
                    for (ci, c) in sc.cols.iter().enumerate() {
                        proj.push(PE::Col { depth: 0, item: ii as u16, col: ci as u16 });
                        out.cols.push(c.clone());
                        out.affs.push(sc.affs[ci]);
                    }
                }
                SelItem::Expr { e, alias, text } => {
                    let pe = self.expr(e)?;
                    let name = match (alias, e) {
                        (Some(a), _) => a.clone(),
                        (None, Expr::Col { name, .. }) => {
                            // The source column's declared name.
                            match &pe {
                                PE::Col { depth, item, col } => self.col_name(*depth, *item, *col).unwrap_or(name.clone()),
                                _ => name.clone(),
                            }
                        }
                        (None, _) => text.clone(),
                    };
                    out.affs.push(self.affinity(&pe));
                    out.cols.push(name);
                    proj.push(pe);
                }
            }
        }
        let group_by = s.group_by.iter().map(|g| self.expr(g)).collect::<Result<Vec<_>, _>>()?;
        let having = s.having.as_ref().map(|h| self.expr(h)).transpose()?;
        let mut order_keys = Vec::new();
        for (e, _) in order {
            // A bare result alias or ordinal sorts by that output column.
            let pe = match e {
                Expr::Lit(V::Int(i)) => proj.get((*i as usize).wrapping_sub(1)).cloned().ok_or("ORDER BY ordinal")?,
                Expr::Col { table: None, name } if out.find(name).is_some() && self.resolve(None, name).is_err() => proj[out.find(name).unwrap()].clone(),
                _ => self.expr(e)?,
            };
            order_keys.push(pe);
        }
        let scope = self.scopes.pop().unwrap();
        let grouped = !group_by.is_empty() || !scope.aggs.is_empty();
        Ok(PSelect { items, probes, or_probes, level, post, konst, grouped, group_by, aggs: scope.aggs, having, wins: scope.wins, proj, order_keys, distinct: s.distinct, out })
    }

    fn col_name(&self, depth: u16, item: u16, col: u16) -> Option<String> {
        let s = self.scopes.len().checked_sub(1 + depth as usize)?;
        Some(self.scopes[s].items[item as usize].schema.cols[col as usize].clone())
    }

    fn affinity(&self, pe: &PE) -> Aff {
        match pe {
            PE::Col { depth, item, col } => {
                let s = self.scopes.len() - 1 - *depth as usize;
                self.scopes[s].items[*item as usize].schema.affs[*col as usize]
            }
            PE::Cast(_, a) => *a,
            PE::Scalar(id) => self.subs[*id].q.out.affs.first().copied().unwrap_or(Aff::None),
            _ => Aff::None,
        }
    }

    fn resolve(&mut self, table: Option<&str>, name: &str) -> Result<PE, String> {
        let n = self.scopes.len();
        for d in 0..n {
            let s = n - 1 - d;
            let found = match table {
                Some(t) => {
                    let tl = lc(t);
                    self.scopes[s].items.iter().enumerate().find(|(_, it)| it.name == tl).map(|(i, it)| (i, it.schema.find(name)))
                }
                None => self.scopes[s].items.iter().enumerate().find_map(|(i, it)| it.schema.find(name).map(|c| (i, Some(c)))),
            };
            if let Some((item, col)) = found {
                let col = col.ok_or_else(|| format!("no such column: {}.{name}", table.unwrap_or("")))?;
                for b in self.bounds.iter_mut() {
                    if b.scope_depth > s {
                        b.correlated = true;
                    }
                }
                return Ok(PE::Col { depth: d as u16, item: item as u16, col: col as u16 });
            }
            if table.is_some() && self.scopes[s].items.iter().any(|it| it.name == lc(table.unwrap())) {
                break;
            }
        }
        Err(format!("no such column: {}{name}", table.map(|t| format!("{t}.")).unwrap_or_default()))
    }

    fn cmp_affs(&self, l: &PE, r: &PE) -> (Aff, Aff) {
        sv::comparison_affinity(self.affinity(l), self.affinity(r))
    }

    fn expr(&mut self, e: &Expr) -> Result<PE, String> {
        Ok(match e {
            Expr::Lit(v) => self.lit(v),
            Expr::Col { table, name } => match self.resolve(table.as_deref(), name) {
                Ok(pe) => pe,
                Err(err) => {
                    // A result alias of the current select.
                    if table.is_none() {
                        let alias = self.scopes.last().and_then(|s| s.aliases.iter().find(|(a, _)| *a == lc(name)).map(|(_, e)| e.clone()));
                        if let Some(ae) = alias {
                            return self.expr(&ae);
                        }
                    }
                    return Err(err);
                }
            },
            Expr::Unary(op, x) => {
                let px = self.expr(x)?;
                match *op {
                    "not" => PE::Not(Box::new(px)),
                    "-" => PE::Neg(Box::new(px)),
                    "+" => px,
                    _ => unreachable!(),
                }
            }
            Expr::Bin(op, l, r) => {
                let pl = self.expr(l)?;
                let pr = self.expr(r)?;
                match *op {
                    "and" => PE::And(Box::new(pl), Box::new(pr)),
                    "or" => PE::Or(Box::new(pl), Box::new(pr)),
                    "||" => PE::Concat(Box::new(pl), Box::new(pr)),
                    "+" | "-" | "*" | "/" | "%" => PE::Arith(op.chars().next().unwrap(), Box::new(pl), Box::new(pr)),
                    _ => {
                        let c = match *op {
                            "=" => Cmp::Eq,
                            "!=" => Cmp::Ne,
                            "<" => Cmp::Lt,
                            "<=" => Cmp::Le,
                            ">" => Cmp::Gt,
                            ">=" => Cmp::Ge,
                            o => return Err(format!("operator {o}")),
                        };
                        let a = self.cmp_affs(&pl, &pr);
                        PE::Cmp(c, Box::new(pl), Box::new(pr), a)
                    }
                }
            }
            Expr::Func { name, args, star, distinct, over } => {
                if let Some(w) = over {
                    if name != "row_number" || !args.is_empty() {
                        return Err(format!("window function {name} unsupported"));
                    }
                    let partition = w.partition.iter().map(|p| self.expr(p)).collect::<Result<Vec<_>, _>>()?;
                    let order = w.order.iter().map(|(o, d)| Ok((self.expr(o)?, *d))).collect::<Result<Vec<_>, String>>()?;
                    let sc = self.scopes.last_mut().unwrap();
                    sc.wins.push(WinSpec { partition, order });
                    return Ok(PE::Win((sc.wins.len() - 1) as u16));
                }
                let agg = match (name.as_str(), args.len(), *star) {
                    ("count", 0, true) => Some(AggFn::CountStar),
                    ("count", 1, false) => Some(AggFn::Count),
                    ("min", 1, false) => Some(AggFn::Min),
                    ("max", 1, false) => Some(AggFn::Max),
                    ("group_concat", 1 | 2, false) => Some(AggFn::GroupConcat),
                    ("sum", 1, false) => Some(AggFn::Sum),
                    ("total", 1, false) => Some(AggFn::Total),
                    _ => None,
                };
                if let Some(f) = agg {
                    if !self.scopes.last().is_some_and(|s| s.allow_agg) {
                        return Err(format!("misuse of aggregate {name}"));
                    }
                    // Aggregate arguments are evaluated per input row in this select's scope.
                    let save = self.scopes.last_mut().unwrap().allow_agg;
                    self.scopes.last_mut().unwrap().allow_agg = false;
                    let arg = args.first().map(|a| self.expr(a)).transpose();
                    let sep = args.get(1).map(|a| self.expr(a)).transpose();
                    self.scopes.last_mut().unwrap().allow_agg = save;
                    let sc = self.scopes.last_mut().unwrap();
                    sc.aggs.push(AggSpec { f, arg: arg?, sep: sep?, distinct: *distinct });
                    return Ok(PE::Agg((sc.aggs.len() - 1) as u16));
                }
                if *distinct || *star {
                    return Err(format!("{name}(DISTINCT/*) unsupported"));
                }
                let f = match name.as_str() {
                    "coalesce" => Func::Coalesce,
                    "ifnull" => Func::Ifnull,
                    "length" => Func::Length,
                    "lower" => Func::Lower,
                    "upper" => Func::Upper,
                    "trim" => Func::Trim,
                    "ltrim" => Func::Ltrim,
                    "rtrim" => Func::Rtrim,
                    "substr" | "substring" => Func::Substr,
                    "instr" => Func::Instr,
                    "replace" => Func::Replace,
                    "char" => Func::Char,
                    "json_quote" => Func::JsonQuote,
                    "json_extract" => Func::JsonExtract,
                    "rx_matches" => Func::RxMatches,
                    "rx_extract" => Func::RxExtract,
                    "rx_replace" => Func::RxReplace,
                    "source_text" => Func::SourceText,
                    "abs" => Func::Abs,
                    "typeof" => Func::Typeof,
                    "max" => Func::Max,
                    "min" => Func::Min,
                    "iif" => Func::Iif,
                    "nullif" => Func::Nullif,
                    o => return Err(format!("function {o} unsupported")),
                };
                let pargs = args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
                PE::Func(f, pargs)
            }
            Expr::Case { operand, whens, else_ } => {
                let po = operand.as_ref().map(|o| self.expr(o)).transpose()?;
                let mut pw = Vec::new();
                let mut aff = (Aff::None, Aff::None);
                for (w, t) in whens {
                    let pwv = self.expr(w)?;
                    if let Some(o) = &po {
                        aff = self.cmp_affs(o, &pwv);
                    }
                    pw.push((pwv, self.expr(t)?));
                }
                let pe = else_.as_ref().map(|x| self.expr(x)).transpose()?;
                PE::Case { operand: po.map(Box::new), aff, whens: pw, else_: pe.map(Box::new) }
            }
            Expr::Cast(x, ty) => match (&**x, Aff::of_type(ty)) {
                (Expr::Blob(text), Aff::Text) => PE::Cast(Box::new(self.lit(&V::text(text))), Aff::Text),
                _ => PE::Cast(Box::new(self.expr(x)?), Aff::of_type(ty)),
            },
            Expr::InList { e, list, not } => {
                let pe = self.expr(e)?;
                let mut pl = Vec::new();
                for x in list {
                    let px = self.expr(x)?;
                    // `x IN (list)`: the left operand's affinity governs the comparisons.
                    let la = self.affinity(&pe);
                    let a = if la == Aff::None { (Aff::None, Aff::None) } else { (Aff::None, la) };
                    pl.push((px, a));
                }
                PE::InList { e: Box::new(pe), list: pl, not: *not }
            }
            Expr::InSub { e, q, not } => {
                let pe = self.expr(e)?;
                let id = self.subquery(q)?;
                if self.subs[id].q.out.cols.len() != 1 {
                    return Err("IN subquery must return one column".into());
                }
                let sub_aff = self.subs[id].q.out.affs[0];
                let aff = sv::comparison_affinity(self.affinity(&pe), sub_aff);
                PE::InSub { e: Box::new(pe), sub: id, not: *not, aff }
            }
            Expr::Exists { q, not } => PE::Exists { sub: self.subquery(q)?, not: *not },
            Expr::Sub(q) => PE::Scalar(self.subquery(q)?),
            Expr::IsNull(x, not) => PE::IsNull(Box::new(self.expr(x)?), *not),
            Expr::Is(l, r, not) => PE::Is(Box::new(self.expr(l)?), Box::new(self.expr(r)?), *not),
            Expr::Like(l, r, not) => PE::Like(Box::new(self.expr(l)?), Box::new(self.expr(r)?), *not),
            Expr::Between(..) => return Err("BETWEEN unsupported".into()),
            Expr::Blob(_) => return Err("blob literal outside CAST(.. AS TEXT) unsupported".into()),
        })
    }
}

fn flatten<'a>(b: &'a Body, arms: &mut Vec<&'a Body>, ops: &mut Vec<SetOp>) {
    match b {
        Body::Compound(l, op, r) => {
            flatten(l, arms, ops);
            ops.push(*op);
            flatten(r, arms, ops);
        }
        s => arms.push(s),
    }
}

fn body_refs(b: &Body, name: &str) -> bool {
    match b {
        Body::Select(s) => select_refs(s, name),
        Body::Compound(l, _, r) => body_refs(l, name) || body_refs(r, name),
    }
}
fn query_refs(q: &Query, name: &str) -> bool {
    q.ctes.iter().any(|c| query_refs(&c.q, name)) || body_refs(&q.body, name)
}
fn select_refs(s: &parse::Select, name: &str) -> bool {
    s.from.iter().any(|f| match &f.source {
        Source::Table(t) => t.eq_ignore_ascii_case(name),
        Source::Sub(q) => query_refs(q, name),
        Source::Func(_, a) => a.iter().any(|e| expr_refs(e, name)),
    }) || s.from.iter().any(|f| f.on.as_ref().is_some_and(|e| expr_refs(e, name)))
        || s.where_.as_ref().is_some_and(|e| expr_refs(e, name))
        || s.items.iter().any(|i| matches!(i, SelItem::Expr { e, .. } if expr_refs(e, name)))
}
fn expr_refs(e: &Expr, name: &str) -> bool {
    match e {
        Expr::InSub { e, q, .. } => expr_refs(e, name) || query_refs(q, name),
        Expr::Exists { q, .. } | Expr::Sub(q) => query_refs(q, name),
        Expr::Unary(_, x) | Expr::Cast(x, _) | Expr::IsNull(x, _) => expr_refs(x, name),
        Expr::Bin(_, l, r) | Expr::Is(l, r, _) | Expr::Like(l, r, _) => expr_refs(l, name) || expr_refs(r, name),
        Expr::Func { args, .. } => args.iter().any(|a| expr_refs(a, name)),
        Expr::Case { operand, whens, else_ } => {
            operand.as_ref().is_some_and(|o| expr_refs(o, name))
                || whens.iter().any(|(w, t)| expr_refs(w, name) || expr_refs(t, name))
                || else_.as_ref().is_some_and(|x| expr_refs(x, name))
        }
        Expr::InList { e, list, .. } => expr_refs(e, name) || list.iter().any(|x| expr_refs(x, name)),
        _ => false,
    }
}

fn visit_cols(pe: &PE, f: &mut dyn FnMut(u16, u16, u16)) {
    match pe {
        PE::Col { depth, item, col } => f(*depth, *item, *col),
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => visit_cols(x, f),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
            visit_cols(l, f);
            visit_cols(r, f);
        }
        PE::Func(_, args) => args.iter().for_each(|a| visit_cols(a, f)),
        PE::Case { operand, whens, else_, .. } => {
            if let Some(o) = operand {
                visit_cols(o, f);
            }
            for (w, t) in whens {
                visit_cols(w, f);
                visit_cols(t, f);
            }
            if let Some(x) = else_ {
                visit_cols(x, f);
            }
        }
        PE::InList { e, list, .. } => {
            visit_cols(e, f);
            list.iter().for_each(|(x, _)| visit_cols(x, f));
        }
        PE::InSub { e, .. } => visit_cols(e, f),
        // Subqueries' own outer references are tracked by their correlation; their reference to
        // this select's items is invisible here, so a correlated subquery is placed at the last
        // item (conservative: see `max_item`).
        _ => {}
    }
}

fn has_sub(pe: &PE) -> bool {
    let mut found = false;
    fn walk(pe: &PE, found: &mut bool) {
        match pe {
            PE::Exists { .. } | PE::Scalar(_) | PE::InSub { .. } => *found = true,
            PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => walk(x, found),
            PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
                walk(l, found);
                walk(r, found);
            }
            PE::Func(_, a) => a.iter().for_each(|x| walk(x, found)),
            PE::Case { operand, whens, else_, .. } => {
                if let Some(o) = operand {
                    walk(o, found);
                }
                for (w, t) in whens {
                    walk(w, found);
                    walk(t, found);
                }
                if let Some(x) = else_ {
                    walk(x, found);
                }
            }
            PE::InList { e, list, .. } => {
                walk(e, found);
                list.iter().for_each(|(x, _)| walk(x, found));
            }
            _ => {}
        }
    }
    walk(pe, &mut found);
    found
}

/// The last FROM item (depth 0) an expression reads; a subquery inside counts as reading every
/// item (it may be correlated to any of them).
fn max_item(pe: &PE) -> Option<usize> {
    let mut m: Option<usize> = None;
    visit_cols(pe, &mut |d, i, _| {
        if d == 0 {
            m = Some(m.map_or(i as usize, |x| x.max(i as usize)));
        }
    });
    if has_sub(pe) {
        return Some(usize::MAX);
    }
    m
}
fn refs_item(pe: &PE, i: usize) -> bool {
    let mut r = false;
    visit_cols(pe, &mut |d, it, _| {
        if d == 0 && it as usize == i {
            r = true;
        }
    });
    r
}
/// Whether `pe` reads column `col` of FROM item `i` (depth 0).
fn reads_col(pe: &PE, i: usize, col: usize) -> bool {
    let mut r = false;
    visit_cols(pe, &mut |d, it, c| r |= d == 0 && it as usize == i && c as usize == col);
    r
}
fn has_agg(pe: &PE) -> bool {
    match pe {
        PE::Agg(_) | PE::Win(_) => true,
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => has_agg(x),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => has_agg(l) || has_agg(r),
        PE::Func(_, a) => a.iter().any(has_agg),
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Execution

pub struct Db<'a> {
    pub tables: FxHashMap<String, Rc<Table>>,
    pub source: &'a str,
    pub path: &'a str,
    /// Hash indexes over the stored tables, built once and shared by every statement of the
    /// clause (each entry holds its table, so a table's address is never reused while indexed).
    indexes: RefCell<FxHashMap<(usize, Vec<u16>, Vec<Aff>), (Rc<Table>, Rc<Index>)>>,
}

impl<'a> Db<'a> {
    pub fn new(source: &'a str, path: &'a str) -> Self {
        Db { tables: Default::default(), source, path, indexes: Default::default() }
    }
    /// Drops the stored table `name` (lowercase) and every clause-level index over it, after its
    /// last reader.
    pub fn release(&mut self, name: &str) {
        if let Some(t) = self.tables.remove(name) {
            self.indexes.get_mut().retain(|_, (it, _)| !Rc::ptr_eq(it, &t));
        }
    }
    /// Bytes held by the stored tables' columns and by the clause-level indexes (`FACTS_MEMTRACE`).
    pub fn bytes(&self) -> (usize, usize) {
        (self.tables.values().map(|t| t.bytes()).sum(), self.indexes.borrow().values().map(|(_, ix)| ix.bytes()).sum())
    }
}

/// A hash index of `table` on `cols` (each value compared under its affinity): rows grouped by
/// key, each group's rows ascending and contiguous in `rows`. Keys are not stored: a group is
/// found by its key's hash and confirmed against its first row, so the index costs 4 bytes a row
/// plus ~32 bytes a distinct key. A row with a NULL key part equals nothing and is left out.
struct Index {
    table: Rc<Table>,
    cols: Vec<u16>,
    affs: Vec<Aff>,
    /// Key hash → first group with that hash.
    heads: FxHashMap<u64, u32>,
    groups: Vec<Group>,
    rows: Vec<u32>,
}

struct Group {
    /// A row with the group's key.
    rep: u32,
    start: u32,
    len: u32,
    /// The next group with the same hash (`u32::MAX`: none).
    next: u32,
}

fn key_hash(key: &[Key]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    key.hash(&mut h);
    h.finish()
}

impl Index {
    fn build(table: &Rc<Table>, cols: &[u16], affs: &[Aff]) -> Index {
        let mut ix = Index { table: table.clone(), cols: cols.to_vec(), affs: affs.to_vec(), heads: FxHashMap::default(), groups: Vec::new(), rows: Vec::new() };
        // Pass 1: each row's group (`u32::MAX`: a NULL key part).
        let mut of_row: Vec<u32> = Vec::with_capacity(table.len());
        let mut key = Vec::with_capacity(cols.len());
        for ri in 0..table.len() as u32 {
            if !ix.row_key(ri, &mut key) {
                of_row.push(u32::MAX);
                continue;
            }
            let h = key_hash(&key);
            let g = match ix.find(h, &key) {
                Some(g) => g,
                None => {
                    let g = ix.groups.len() as u32;
                    let next = ix.heads.insert(h, g).unwrap_or(u32::MAX);
                    ix.groups.push(Group { rep: ri, start: 0, len: 0, next });
                    g
                }
            };
            ix.groups[g as usize].len += 1;
            of_row.push(g);
        }
        // Pass 2: counting sort by group, rows ascending within a group.
        let mut at = 0u32;
        for g in ix.groups.iter_mut() {
            g.start = at;
            at += g.len;
            g.len = 0;
        }
        ix.rows = vec![0; at as usize];
        for (ri, &g) in of_row.iter().enumerate() {
            if g != u32::MAX {
                let g = &mut ix.groups[g as usize];
                ix.rows[(g.start + g.len) as usize] = ri as u32;
                g.len += 1;
            }
        }
        ix
    }

    /// Row `r`'s key into `out`; false when a part is NULL.
    fn row_key(&self, r: u32, out: &mut Vec<Key>) -> bool {
        out.clear();
        for (&c, &a) in self.cols.iter().zip(&self.affs) {
            match sv::key(&sv::compare_affinity(&self.table.cols[c as usize].get(r), a)) {
                Some(k) => out.push(k),
                None => return false,
            }
        }
        true
    }

    /// Whether row `r`'s key equals `key`.
    fn row_is(&self, r: u32, key: &[Key]) -> bool {
        self.cols.iter().zip(&self.affs).zip(key).all(|((&c, &a), k)| sv::key(&sv::compare_affinity(&self.table.cols[c as usize].get(r), a)).as_ref() == Some(k))
    }

    fn find(&self, h: u64, key: &[Key]) -> Option<u32> {
        let mut g = *self.heads.get(&h)?;
        while g != u32::MAX {
            let grp = &self.groups[g as usize];
            if self.row_is(grp.rep, key) {
                return Some(g);
            }
            g = grp.next;
        }
        None
    }

    /// The rows whose key equals `key`, ascending.
    fn get(&self, key: &[Key]) -> Option<&[u32]> {
        let g = &self.groups[self.find(key_hash(key), key)? as usize];
        Some(&self.rows[g.start as usize..(g.start + g.len) as usize])
    }

    fn bytes(&self) -> usize {
        self.rows.len() * 4 + self.groups.len() * (std::mem::size_of::<Group>() + 16)
    }
}

/// A join's hash indexes per item: the plain equality probe's, and each OR branch's.
struct Indexes {
    plain: Vec<Option<Rc<Index>>>,
    or: Vec<Option<Vec<Rc<Index>>>>,
}

pub struct Exec<'d, 'a> {
    db: &'d Db<'a>,
    stmt: &'d Statement,
    slots: RefCell<Vec<Option<Rc<Table>>>>,
    /// This statement's indexes by (table, probe).
    indexes: RefCell<FxHashMap<(usize, usize), Rc<Index>>>,
    sub_cache: RefCell<FxHashMap<usize, Rc<SubResult>>>,
    derived: RefCell<FxHashMap<usize, Rc<Table>>>,
    keep: RefCell<Vec<Rc<Table>>>,
    pub error: RefCell<Option<String>>,
    /// The statement's text literals as values, materialised on first use.
    texts: Vec<std::cell::OnceCell<V>>,
}

enum SubResult {
    Exists(bool),
    Scalar(V),
    Set(rustc_hash::FxHashSet<Key>, bool, bool),
}

/// One FROM item's binding: a row of a table (`NULL_ROW`: the null-extended row), or nothing
/// (`t: None`, an item not bound yet). Every column of an unbound or null row reads NULL.
#[derive(Clone, Copy)]
struct RowRef<'r> {
    t: Option<&'r Table>,
    r: u32,
}

const UNBOUND: RowRef<'static> = RowRef { t: None, r: NULL_ROW };

impl RowRef<'_> {
    #[inline]
    fn value(&self, c: u16) -> V {
        match self.t {
            Some(t) if self.r != NULL_ROW => t.cols.get(c as usize).map_or(V::Null, |col| col.get(self.r)),
            _ => V::Null,
        }
    }
}

/// A binding kept past its join (group representatives, window rows): per item, its table and
/// row. Every table is held by the select's `tables` or the execution's `keep` until the
/// execution ends.
#[derive(Clone, Copy)]
struct Kept {
    t: *const Table,
    r: u32,
}

impl Kept {
    fn of(r: &RowRef<'_>) -> Kept {
        Kept { t: r.t.map_or(std::ptr::null(), |t| t as *const Table), r: r.r }
    }
    fn row<'r>(&self) -> RowRef<'r> {
        // SAFETY: `t` is null or points at a table held by the select's `tables` or by `keep`,
        // both alive for as long as the kept binding is used (within one `select_rows`).
        RowRef { t: unsafe { self.t.as_ref() }, r: self.r }
    }
}

/// A select's output under construction: per output column, either the (item, column) it reads
/// — recorded as one row index per binding per item, shared by every column of that item — or a
/// literal (one value for every row), or the values its expression evaluated to.
struct Out<'p> {
    exprs: Vec<&'p PE>,
    src: Vec<Option<(usize, usize)>>,
    konst: Vec<Option<V>>,
    idx: Vec<Option<Vec<u32>>>,
    vals: Vec<Vec<V>>,
    n: usize,
}

impl<'p> Out<'p> {
    fn new(s: &'p PSelect, ex: &Exec<'_, '_>) -> Out<'p> {
        let exprs: Vec<&PE> = s.proj.iter().chain(&s.order_keys).collect();
        let src: Vec<Option<(usize, usize)>> = exprs
            .iter()
            .map(|p| match p {
                PE::Col { depth: 0, item, col } if !s.items[*item as usize].lateral => Some((*item as usize, *col as usize)),
                _ => None,
            })
            .collect();
        let konst = exprs
            .iter()
            .map(|p| match p {
                PE::Lit(_) => {
                    let ctx = EvalCtx { frame: Frame { rows: &[], parent: None }, aggs: None, wins: None };
                    Some(ex.eval(p, &ctx))
                }
                _ => None,
            })
            .collect();
        let mut idx: Vec<Option<Vec<u32>>> = vec![None; s.items.len()];
        for (item, _) in src.iter().flatten() {
            idx[*item] = Some(Vec::new());
        }
        let vals = exprs.iter().map(|_| Vec::new()).collect();
        Out { exprs, src, konst, idx, vals, n: 0 }
    }

    fn push(&mut self, ex: &Exec<'_, '_>, ctx: &EvalCtx<'_>) {
        for (item, ix) in self.idx.iter_mut().enumerate() {
            if let Some(ix) = ix {
                ix.push(ctx.frame.rows[item].r);
            }
        }
        for (j, (src, k)) in self.src.iter().zip(&self.konst).enumerate() {
            if src.is_none() && k.is_none() {
                self.vals[j].push(ex.eval(self.exprs[j], ctx));
            }
        }
        self.n += 1;
    }

    fn finish(self, tables: &[Rc<Table>], schema: Schema) -> Table {
        let idx: Vec<Option<Rc<[u32]>>> = self.idx.into_iter().map(|i| i.map(Rc::from)).collect();
        let mut memo = FxHashMap::default();
        let n = self.n;
        let cols = self
            .src
            .iter()
            .zip(self.konst)
            .zip(self.vals)
            .map(|((src, k), vals)| match (src, k) {
                (Some((item, col)), _) => match tables[*item].cols.get(*col) {
                    Some(c) => gather(c, idx[*item].as_ref().unwrap(), &mut memo),
                    None => Rc::new(Column::Const(V::Null)),
                },
                (None, Some(v)) => Rc::new(Column::Const(v)),
                (None, None) => Rc::new(Column::Vals(vals)),
            })
            .collect();
        Table { schema, cols, len: n }
    }
}

#[derive(Clone, Copy)]
struct Frame<'r> {
    rows: &'r [RowRef<'r>],
    parent: Option<&'r Frame<'r>>,
}

struct EvalCtx<'r> {
    frame: Frame<'r>,
    aggs: Option<&'r [V]>,
    wins: Option<&'r [V]>,
}

/// The plan shape `Exec::closure` evaluates (see there).
struct ClosureShape<'p> {
    seed_body: &'p PBody,
    s: &'p PSelect,
    probe: &'p Probe,
    /// The `node` column, and the `e` column that replaces it.
    nj: usize,
    dst: usize,
    /// The `w` columns the probe keys read.
    from_w: Vec<usize>,
}

fn closure_shape<'p>(batch: usize, init: &'p [PBody], rec: &'p [PBody], distinct: bool, w: usize) -> Option<ClosureShape<'p>> {
    let ([seed_body], [PBody::Select(s)], true) = (init, rec, distinct) else { return None };
    let plain = !s.grouped && s.wins.is_empty() && !s.distinct && s.order_keys.is_empty() && s.konst.is_empty() && s.having.is_none();
    if !plain || s.items.len() != 2 || s.proj.len() != w || s.post.iter().any(|p| !p.is_empty()) {
        return None;
    }
    let (wi, ei) = (&s.items[0], &s.items[1]);
    if !matches!(wi.src, Src::Cte(b) if b == batch) || wi.left || wi.lateral || s.probes[0].is_some() || s.or_probes[0].is_some() {
        return None;
    }
    if ei.left || ei.lateral || s.or_probes[1].is_some() || matches!(ei.src, Src::Cte(b) if b == batch) {
        return None;
    }
    let probe = s.probes[1].as_ref()?;
    // The projection: carried columns in place, and one `e` column as the new node.
    let mut node = None;
    for (j, p) in s.proj.iter().enumerate() {
        match p {
            PE::Col { depth: 0, item: 0, col } if *col as usize == j => {}
            PE::Col { depth: 0, item: 1, col } if node.is_none() => node = Some((j, *col as usize)),
            _ => return None,
        }
    }
    let (nj, dst) = node?;
    // The probe: `e` columns equal to `w` columns; exactly one of them is `w.node`.
    let mut from_w = Vec::new();
    for e in &probe.exprs {
        let PE::Col { depth: 0, item: 0, col } = e else { return None };
        from_w.push(*col as usize);
    }
    if from_w.iter().filter(|&&c| c == nj).count() != 1 {
        return None;
    }
    // Filters may not read `w.node` (so they are functions of the seed and the edge) and
    // hold no subquery.
    if s.level[0].iter().chain(&s.level[1]).any(|c| reads_col(c, 0, nj) || has_sub(c)) {
        return None;
    }
    Some(ClosureShape { seed_body, s, probe, nj, dst, from_w })
}

/// Whether a recursive CTE runs as the closure operator (`Fixpoint{closure}`) rather than
/// level by level; decided by the plan alone, identical for every file.
pub(crate) fn closure_applies(cte: &PCte) -> bool {
    match cte {
        PCte::Recursive { batch, init, rec, distinct, schema, .. } => closure_shape(*batch, init, rec, *distinct, schema.cols.len()).is_some(),
        PCte::Plain { .. } => false,
    }
}

impl<'d, 'a> Exec<'d, 'a> {
    fn new(db: &'d Db<'a>, stmt: &'d Statement) -> Self {
        Exec {
            db,
            stmt,
            slots: RefCell::new(vec![None; stmt.n_slots]),
            indexes: Default::default(),
            sub_cache: Default::default(),
            derived: Default::default(),
            keep: Default::default(),
            error: RefCell::new(None),
            texts: (0..stmt.texts.len()).map(|_| std::cell::OnceCell::new()).collect(),
        }
    }
}

pub fn execute(db: &Db<'_>, stmt: &Statement) -> Result<Table, String> {
    let ex = Exec::new(db, stmt);
    let t = ex.query(&stmt.root, None);
    if let Some(e) = ex.error.borrow_mut().take() {
        return Err(e);
    }
    Ok(t)
}

/// Evaluates a planned scalar expression (deferred column) over every row of `t`.
pub fn eval_scalar_rows(db: &Db<'_>, stmt: &Statement, pe: &PE, t: &Table) -> Result<Vec<V>, String> {
    let ex = Exec::new(db, stmt);
    let mut out = Vec::with_capacity(t.len());
    for r in 0..t.len() {
        let rows = [RowRef { t: Some(t), r: r as u32 }];
        let ctx = EvalCtx { frame: Frame { rows: &rows, parent: None }, aggs: None, wins: None };
        out.push(ex.eval(pe, &ctx));
        if let Some(e) = ex.error.borrow_mut().take() {
            return Err(e);
        }
    }
    Ok(out)
}

impl<'d, 'a> Exec<'d, 'a> {
    fn fail(&self, msg: String) {
        let mut e = self.error.borrow_mut();
        if e.is_none() {
            *e = Some(msg);
        }
    }
    fn failed(&self) -> bool {
        self.error.borrow().is_some()
    }

    fn query(&self, q: &PQuery, outer: Option<&Frame<'_>>) -> Table {
        for cte in &q.ctes {
            match cte {
                PCte::Plain { slot, q } => {
                    let t = self.query(q, outer);
                    self.slots.borrow_mut()[*slot] = Some(Rc::new(t));
                }
                PCte::Recursive { slot, batch, init, rec, distinct, schema } => {
                    let t = match self.closure(*batch, init, rec, *distinct, schema, outer) {
                        Some(t) => t,
                        None => self.recursive(*batch, init, rec, *distinct, schema, outer),
                    };
                    self.slots.borrow_mut()[*slot] = Some(Rc::new(t));
                }
            }
        }
        let t = self.body(&q.body, outer);
        let width = q.out.cols.len();
        let limit = q.limit.as_ref().map(|e| self.const_int(e, outer));
        let offset = q.offset.as_ref().map(|e| self.const_int(e, outer)).unwrap_or(0).max(0) as usize;
        if q.order.is_empty() && limit.is_none_or(|l| l < 0) && offset == 0 {
            return Table { schema: q.out.clone(), ..t }.truncate(width);
        }
        let mut perm: Vec<u32> = (0..t.len() as u32).collect();
        if !q.order.is_empty() {
            // Stable sort on the key values, read once.
            let keys: Vec<(Vec<V>, bool)> = q.order.iter().map(|(i, desc)| ((0..t.len()).map(|r| t.value(r, *i)).collect(), *desc)).collect();
            perm.sort_by(|&a, &b| {
                for (k, desc) in &keys {
                    let o = sv::cmp(&k[a as usize], &k[b as usize]);
                    if o != Ordering::Equal {
                        return if *desc { o.reverse() } else { o };
                    }
                }
                Ordering::Equal
            });
        }
        let start = offset.min(perm.len());
        let end = match limit {
            Some(l) if l >= 0 => start.saturating_add(l as usize).min(perm.len()),
            _ => perm.len(),
        };
        let idx: Rc<[u32]> = perm[start..end].into();
        Table { schema: q.out.clone(), ..t.truncate(width) }.select(idx)
    }

    fn const_int(&self, e: &PE, outer: Option<&Frame<'_>>) -> i64 {
        let empty: [RowRef<'_>; 0] = [];
        let ctx = EvalCtx { frame: Frame { rows: &empty, parent: outer }, aggs: None, wins: None };
        match sv::cast(&self.eval(e, &ctx), Aff::Integer) {
            V::Int(i) => i,
            _ => -1,
        }
    }

    fn body(&self, b: &PBody, outer: Option<&Frame<'_>>) -> Table {
        match b {
            PBody::Select(s) => self.select_rows(s, outer),
            PBody::Compound(l, op, r, schema) => {
                let left = self.body(l, outer);
                let right = self.body(r, outer);
                let w = left.cols.len().min(right.cols.len());
                let cols: Vec<Rc<Column>> = (0..w).map(|c| chain(&[(&left.cols[c], left.len()), (&right.cols[c], right.len())])).collect();
                let all = Table { schema: schema.clone(), cols, len: left.len() + right.len() };
                match op {
                    SetOp::UnionAll => all,
                    SetOp::Union => {
                        let mut seen = Distinct::new(w);
                        let keep: Vec<u32> = (0..all.len()).filter(|&r| seen.insert(&all, r)).map(|r| r as u32).collect();
                        if keep.len() == all.len() { all } else { all.select(keep.into()) }
                    }
                }
            }
        }
    }

    /// A closure: a set-semantics fixpoint whose step joins the previous level `w` to one edge
    /// relation `e` on `e.(keys, src) = w.(keys, node)`, carries every other column of `w` and
    /// replaces `node` by `e.dst`; filters read only carried columns of `w` and columns of `e`.
    /// Recognised on the plan, for any relation. Its rows equal the semi-naive evaluation's, in
    /// the same order (level by level; within a level by the previous row, then the edge
    /// relation's order), without evaluating the step as a select per level: a row is (seed,
    /// node source), carried values are the seed's, and the result columns are gathers.
    ///
    /// Keys: a row's distinctness is `(carried id, node id)` as one `u64` (the carried columns'
    /// tuple interned once per seed, the node value once per seed and per edge row); the edge
    /// lookup for a `(carried probe values, node)` pair is made once and cached.
    /// `None` when the plan is not of that shape.
    fn closure(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, schema: &Schema, outer: Option<&Frame<'_>>) -> Option<Table> {
        let w = schema.cols.len();
        let ClosureShape { seed_body, s, probe, nj, dst, from_w } = closure_shape(batch, init, rec, distinct, w)?;
        let ei = &s.items[1];
        // Seeds: the initial select, set semantics.
        let seeds = self.body(seed_body, outer);
        if self.failed() {
            return Some(Table::empty(schema.clone()));
        }
        let seeds = Table { schema: schema.clone(), ..seeds }.truncate(w);
        let e = self.table_of(&ei.src, outer);
        if self.failed() {
            return Some(Table::empty(schema.clone()));
        }
        let ix = self.index(&e, probe, matches!(ei.src, Src::Global(_)));
        let mut seen = Distinct::new(w);
        // The carried columns' tuples (every column but `node`), interned.
        let mut carried_ids: FxHashMap<Vec<u32>, u32> = FxHashMap::default();
        // The carried probe values (the probe columns read from `w` but `node`), interned.
        let mut probe_ids: FxHashMap<Vec<Key>, u32> = FxHashMap::default();
        let mut rows_seen: rustc_hash::FxHashSet<u64> = Default::default();
        // Per admitted seed: its row, carried id, carried probe id (`u32::MAX`: a NULL part) and
        // node id.
        let mut seed_rows: Vec<u32> = Vec::new();
        let mut seed_carried: Vec<u32> = Vec::new();
        let mut seed_probe: Vec<u32> = Vec::new();
        let mut seed_node: Vec<u32> = Vec::new();
        // Walk rows: (seed, node source: `u32::MAX` = the seed's own node, else an edge row).
        let mut level: Vec<(u32, u32)> = Vec::new();
        let pos_nj = from_w.iter().position(|&c| c == nj).expect("closure probe reads `node`");
        for r in 0..seeds.len() {
            let ids: Vec<u32> = (0..w).map(|c| seen.intern(c, &seeds.value(r, c))).collect();
            let carried: Vec<u32> = ids.iter().enumerate().filter(|&(c, _)| c != nj).map(|(_, &x)| x).collect();
            let n_c = carried_ids.len() as u32;
            let cid = *carried_ids.entry(carried).or_insert(n_c);
            if rows_seen.insert((cid as u64) << 32 | ids[nj] as u64) {
                let mut pk = Vec::with_capacity(from_w.len() - 1);
                let mut null = false;
                for (k, (&c, a)) in from_w.iter().zip(&probe.expr_affs).enumerate() {
                    if k == pos_nj {
                        continue;
                    }
                    match sv::key(&sv::compare_affinity(&seeds.value(r, c), *a)) {
                        Some(x) => pk.push(x),
                        None => null = true,
                    }
                }
                let pid = if null {
                    u32::MAX
                } else {
                    let n_p = probe_ids.len() as u32;
                    *probe_ids.entry(pk).or_insert(n_p)
                };
                level.push((seed_rows.len() as u32, u32::MAX));
                seed_rows.push(r as u32);
                seed_carried.push(cid);
                seed_probe.push(pid);
                seed_node.push(ids[nj]);
            }
        }
        // The probe keys of a carried probe id (inverse of `probe_ids`).
        let mut probe_keys: Vec<Vec<Key>> = vec![Vec::new(); probe_ids.len()];
        for (k, &i) in &probe_ids {
            probe_keys[i as usize] = k.clone();
        }
        // Per edge row: its `dst` value's node id (`u32::MAX`: not yet interned).
        let mut dst_id: Vec<u32> = vec![u32::MAX; e.len()];
        // Per node id: its value and probe key part (the node values met, in id order).
        let mut node_key: FxHashMap<u32, Option<Key>> = FxHashMap::default();
        let node_aff = probe.expr_affs[pos_nj];
        // Edge rows per (carried probe id, node id), looked up once.
        let mut hits_of: FxHashMap<u64, Option<&[u32]>> = FxHashMap::default();
        // A seed expands when `w`'s filters hold on its (carried) values.
        let mut expands: Vec<Option<bool>> = vec![None; seed_rows.len()];
        let mut all: Vec<(u32, u32)> = Vec::new();
        let mut steps = 0usize;
        while !level.is_empty() {
            if self.failed() {
                break;
            }
            steps += level.len();
            if steps > 50_000_000 {
                self.fail("recursive CTE exceeded 50M rows".into());
                break;
            }
            let mut next = Vec::new();
            for &(sr, src) in &level {
                let seed = RowRef { t: Some(&seeds), r: seed_rows[sr as usize] };
                let ok = *expands[sr as usize].get_or_insert_with(|| {
                    let rows = [seed, UNBOUND];
                    let ctx = EvalCtx { frame: Frame { rows: &rows, parent: outer }, aggs: None, wins: None };
                    s.level[0].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
                });
                if !ok {
                    continue;
                }
                let pid = seed_probe[sr as usize];
                if pid == u32::MAX {
                    continue;
                }
                let node = if src == u32::MAX { seed_node[sr as usize] } else { dst_id[src as usize] };
                let hits = *hits_of.entry((pid as u64) << 32 | node as u64).or_insert_with(|| {
                    let nk = node_key.entry(node).or_insert_with(|| {
                        let v = if src == u32::MAX { seeds.value(seed_rows[sr as usize] as usize, nj) } else { e.value(src as usize, dst) };
                        sv::key(&sv::compare_affinity(&v, node_aff))
                    });
                    let nk = nk.clone()?;
                    let mut pk = probe_keys[pid as usize].clone();
                    pk.insert(pos_nj, nk);
                    ix.get(&pk)
                });
                let Some(hits) = hits else { continue };
                for &er in hits {
                    if !s.level[1].is_empty() {
                        let rows = [seed, RowRef { t: Some(&e), r: er }];
                        let ctx = EvalCtx { frame: Frame { rows: &rows, parent: outer }, aggs: None, wins: None };
                        if !s.level[1].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true)) {
                            continue;
                        }
                    }
                    let d = &mut dst_id[er as usize];
                    if *d == u32::MAX {
                        *d = seen.intern(nj, &e.value(er as usize, dst));
                    }
                    if rows_seen.insert((seed_carried[sr as usize] as u64) << 32 | *d as u64) {
                        next.push((sr, er));
                    }
                }
            }
            all.append(&mut level);
            level = next;
        }
        // Columns: carried ones gather the seed rows; the node column chains the seeds' nodes
        // (level 0) and the edges' `dst` (deeper levels).
        let n = all.len();
        let seed_idx: Rc<[u32]> = all.iter().map(|&(sr, _)| seed_rows[sr as usize]).collect();
        let roots = all.iter().take_while(|&&(_, src)| src == u32::MAX).count();
        let first: Rc<[u32]> = seed_idx[..roots].into();
        let edge_idx: Rc<[u32]> = all[roots..].iter().map(|&(_, src)| src).collect();
        let mut memo = FxHashMap::default();
        let cols = (0..w)
            .map(|j| {
                if j == nj {
                    let a = gather(&seeds.cols[j], &first, &mut memo);
                    let b = gather(&e.cols[dst], &edge_idx, &mut memo);
                    chain(&[(&a, roots), (&b, n - roots)])
                } else {
                    gather(&seeds.cols[j], &seed_idx, &mut memo)
                }
            })
            .collect();
        Some(Table { schema: schema.clone(), cols, len: n })
    }

    /// Semi-naive fixpoint, level by level. Each level's rows are views (gathers) over the arms'
    /// sources, so a closure over a relation stores row indexes, not values.
    fn recursive(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, schema: &Schema, outer: Option<&Frame<'_>>) -> Table {
        let width = schema.cols.len();
        let mut seen = Distinct::new(width);
        let mut admit = |t: Table, queue: &mut Vec<Table>| {
            let t = Table { schema: schema.clone(), ..t }.truncate(width);
            let t = if distinct {
                let keep: Vec<u32> = (0..t.len()).filter(|&r| seen.insert(&t, r)).map(|r| r as u32).collect();
                if keep.len() == t.len() { t } else { t.select(keep.into()) }
            } else {
                t
            };
            if !t.is_empty() {
                queue.push(t);
            }
        };
        let mut queue: Vec<Table> = Vec::new();
        for arm in init {
            admit(self.body(arm, outer), &mut queue);
        }
        let mut levels: Vec<Table> = Vec::new();
        let mut steps = 0usize;
        while !queue.is_empty() {
            if self.failed() {
                break;
            }
            let t = Rc::new(concat(schema.clone(), width, &std::mem::take(&mut queue)));
            steps += t.len();
            if steps > 50_000_000 {
                self.fail("recursive CTE exceeded 50M rows".into());
                break;
            }
            self.slots.borrow_mut()[batch] = Some(t.clone());
            // Indexes over a batch table are per batch.
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            for arm in rec {
                admit(self.body(arm, outer), &mut queue);
            }
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            self.slots.borrow_mut()[batch] = None;
            levels.push(Rc::try_unwrap(t).unwrap_or_else(|rc| Table { schema: rc.schema.clone(), cols: rc.cols.clone(), len: rc.len }));
        }
        concat(schema.clone(), width, &levels)
    }

    fn table_of(&self, src: &Src, outer: Option<&Frame<'_>>) -> Rc<Table> {
        match src {
            Src::Global(name) => match self.db.tables.get(&lc(name)) {
                Some(t) => t.clone(),
                None => {
                    self.fail(format!("table {name} missing at run time"));
                    Rc::new(Table::default())
                }
            },
            Src::Cte(slot) => match &self.slots.borrow()[*slot] {
                Some(t) => t.clone(),
                None => {
                    self.fail("CTE not materialised".into());
                    Rc::new(Table::default())
                }
            },
            Src::Derived(id) => {
                let sub = &self.stmt.subs[*id];
                if sub.cacheable {
                    if let Some(t) = self.derived.borrow().get(id) {
                        return t.clone();
                    }
                }
                let t = Rc::new(self.query(&sub.q, outer));
                if sub.cacheable {
                    self.derived.borrow_mut().insert(*id, t.clone());
                }
                t
            }
            Src::JsonEach(arg) => {
                let empty: [RowRef<'_>; 0] = [];
                let ctx = EvalCtx { frame: Frame { rows: &empty, parent: outer }, aggs: None, wins: None };
                let v = self.eval(arg, &ctx);
                Rc::new(self.json_each_table(v))
            }
        }
    }

    fn json_each_table(&self, v: V) -> Table {
        let schema = Schema {
            cols: ["key", "value", "type", "atom", "id", "parent", "fullkey", "path"].iter().map(|s| s.to_string()).collect(),
            affs: vec![Aff::None; 8],
        };
        let mut rows = Vec::new();
        if let V::Text(s) = v {
            match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(serde_json::Value::Array(items)) => {
                    for (i, it) in items.iter().enumerate() {
                        rows.push(json_each_row(V::Int(i as i64), it, &format!("$[{i}]")));
                    }
                }
                Ok(serde_json::Value::Object(map)) => {
                    for (k, it) in map.iter() {
                        rows.push(json_each_row(V::text(k), it, &format!("$.{k}")));
                    }
                }
                Ok(other) => rows.push(json_each_row(V::Null, &other, "$")),
                Err(e) => self.fail(format!("malformed JSON: {e}")),
            }
        }
        Table::from_rows(schema, rows)
    }

    /// The hash index of `t` on the probe's columns. A stored table's (`global`) index is built
    /// once per clause; any other table's once per statement.
    fn index(&self, t: &Rc<Table>, p: &Probe, global: bool) -> Rc<Index> {
        let k = (Rc::as_ptr(t) as usize, p as *const Probe as usize);
        if let Some(ix) = self.indexes.borrow().get(&k) {
            return ix.clone();
        }
        let dk = (k.0, p.cols.clone(), p.col_affs.clone());
        let shared = if global { self.db.indexes.borrow().get(&dk).map(|(_, ix)| ix.clone()) } else { None };
        let ix = shared.unwrap_or_else(|| {
            let ix = Rc::new(Index::build(t, &p.cols, &p.col_affs));
            if global {
                self.db.indexes.borrow_mut().insert(dk, (t.clone(), ix.clone()));
            } else {
                self.keep.borrow_mut().push(t.clone());
            }
            ix
        });
        self.indexes.borrow_mut().insert(k, ix.clone());
        ix
    }

    /// The FROM items' tables (`None` after a failure).
    fn item_tables(&self, s: &PSelect, outer: Option<&Frame<'_>>) -> Option<Vec<Rc<Table>>> {
        let tables: Vec<Rc<Table>> = s.items.iter().map(|it| self.table_of(&it.src, outer)).collect();
        if self.failed() { None } else { Some(tables) }
    }

    /// Runs the join pipeline of `s` over its item tables, calling `sink` with each binding that
    /// passes every conjunct.
    fn join<'t>(&self, s: &PSelect, tables: &'t [Rc<Table>], outer: Option<&Frame<'_>>, sink: &mut dyn FnMut(&Frame<'_>) -> ControlFlow<()>) {
        let mut rows: Vec<RowRef<'t>> = vec![UNBOUND; s.items.len()];
        // Constant conjuncts.
        {
            let ctx = EvalCtx { frame: Frame { rows: &rows, parent: outer }, aggs: None, wins: None };
            for c in &s.konst {
                if sv::truth(&self.eval(c, &ctx)) != Some(true) {
                    return;
                }
            }
        }
        let indexes = Indexes {
            plain: s.probes.iter().zip(tables).zip(&s.items).map(|((p, t), it)| p.as_ref().map(|p| self.index(t, p, matches!(it.src, Src::Global(_))))).collect(),
            or: s.or_probes.iter().zip(tables).zip(&s.items).map(|((b, t), it)| b.as_ref().map(|b| b.iter().map(|p| self.index(t, p, matches!(it.src, Src::Global(_)))).collect())).collect(),
        };
        let _ = self.level(s, 0, tables, &indexes, &mut rows, outer, sink);
    }

    /// The probe's key for the current binding; `None` when a key part is NULL (no row equals).
    fn probe_key(&self, p: &Probe, ctx: &EvalCtx<'_>) -> Option<Vec<Key>> {
        p.exprs.iter().zip(&p.expr_affs).map(|(e, a)| sv::key(&sv::compare_affinity(&self.eval(e, ctx), *a))).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn level<'t>(
        &self,
        s: &PSelect,
        i: usize,
        tables: &'t [Rc<Table>],
        indexes: &Indexes,
        rows: &mut Vec<RowRef<'t>>,
        outer: Option<&Frame<'_>>,
        sink: &mut dyn FnMut(&Frame<'_>) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if self.failed() {
            return ControlFlow::Break(());
        }
        if i == s.items.len() {
            let f = Frame { rows: rows.as_slice(), parent: outer };
            return sink(&f);
        }
        let item = &s.items[i];
        let lateral_table: Rc<Table>;
        let t: &'t Table = if item.lateral {
            let Src::JsonEach(arg) = &item.src else { unreachable!() };
            let v = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                self.eval(arg, &ctx)
            };
            lateral_table = Rc::new(self.json_each_table(v));
            let ptr = Rc::as_ptr(&lateral_table);
            if s.grouped || !s.wins.is_empty() {
                self.keep.borrow_mut().push(lateral_table.clone());
            }
            // SAFETY: grouped and window selects keep bindings (`Kept`) past this level, so `keep`
            // holds the table until the execution ends. Otherwise every binding of this item lives
            // only below this level: `rows[i]` is reset before `lateral_table` drops, and outputs
            // copy a lateral item's values (`Out` gathers only non-lateral items).
            unsafe { &*ptr }
        } else {
            &tables[i]
        };
        // Candidate rows, in table order.
        let or_cands: Vec<u32>;
        let cands: Option<&[u32]> = {
            let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
            match (&s.or_probes[i], &indexes.or[i], &s.probes[i], &indexes.plain[i]) {
                (Some(branches), Some(ixs), _, _) => {
                    let mut c: Vec<u32> = Vec::new();
                    for (p, ix) in branches.iter().zip(ixs) {
                        if let Some(hits) = self.probe_key(p, &ctx).and_then(|k| ix.get(&k)) {
                            c.extend_from_slice(hits);
                        }
                    }
                    c.sort_unstable();
                    c.dedup();
                    or_cands = c;
                    Some(&or_cands)
                }
                (_, _, Some(p), Some(ix)) => Some(self.probe_key(p, &ctx).and_then(|k| ix.get(&k)).map_or(&[][..], |h| &h[..])),
                _ => None,
            }
        };
        let n = cands.map_or(t.len(), |c| c.len());
        let mut matched = false;
        for k in 0..n {
            let r = cands.map_or(k as u32, |c| c[k]);
            rows[i] = RowRef { t: Some(t), r };
            let ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.level[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if !ok {
                continue;
            }
            matched = true;
            let post_ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.post[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if post_ok {
                self.level(s, i + 1, tables, indexes, rows, outer, sink)?;
            }
        }
        if item.left && !matched {
            rows[i] = RowRef { t: Some(t), r: NULL_ROW };
            let post_ok = {
                let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
                s.post[i].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true))
            };
            if post_ok {
                self.level(s, i + 1, tables, indexes, rows, outer, sink)?;
            }
        }
        rows[i] = UNBOUND;
        ControlFlow::Continue(())
    }

    /// Output of a select: the projection, then the ORDER BY keys as extra columns. A projected
    /// column of a (non-lateral) FROM item is a gather view of that item's column.
    fn select_rows(&self, s: &PSelect, outer: Option<&Frame<'_>>) -> Table {
        let Some(tables) = self.item_tables(s, outer) else { return Table::empty(s.out.clone()) };
        let mut out = Out::new(s, self);
        if s.grouped {
            // Groups in first-seen order: key -> (agg states, representative binding).
            let mut groups: FxHashMap<Vec<DKey>, usize> = FxHashMap::default();
            let mut states: Vec<(Vec<AggState>, Vec<Kept>)> = Vec::new();
            self.join(s, &tables, outer, &mut |f| {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                let key: Vec<DKey> = s.group_by.iter().map(|g| sv::dkey(&self.eval(g, &ctx))).collect();
                let gi = *groups.entry(key).or_insert_with(|| {
                    states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new()));
                    states.len() - 1
                });
                let (st, rep) = &mut states[gi];
                let mut extremum_updated = false;
                for (a, spec) in st.iter_mut().zip(&s.aggs) {
                    let v = spec.arg.as_ref().map(|e| self.eval(e, &ctx));
                    let sep = spec.sep.as_ref().map(|e| self.eval(e, &ctx));
                    if a.update(spec, v, sep) {
                        extremum_updated = true;
                    }
                }
                // Bare columns read the row a lone min()/max() chose, else the last row.
                let lone_extremum = s.aggs.len() == 1 && matches!(s.aggs[0].f, AggFn::Min | AggFn::Max);
                if rep.is_empty() || !lone_extremum || extremum_updated {
                    rep.clear();
                    rep.extend(f.rows.iter().map(Kept::of));
                }
                ControlFlow::Continue(())
            });
            if s.group_by.is_empty() && states.is_empty() {
                states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new()));
            }
            for (st, rep) in states {
                let aggs: Vec<V> = st.into_iter().zip(&s.aggs).map(|(a, spec)| a.finish(spec)).collect();
                let refs: Vec<RowRef<'_>> = if rep.is_empty() { vec![UNBOUND; s.items.len()] } else { rep.iter().map(Kept::row).collect() };
                let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: Some(&aggs), wins: None };
                if let Some(h) = &s.having {
                    if sv::truth(&self.eval(h, &ctx)) != Some(true) {
                        continue;
                    }
                }
                out.push(self, &ctx);
            }
        } else if !s.wins.is_empty() {
            // Window functions: keep each binding, number them, then project.
            let w_items = s.items.len();
            let mut bound: Vec<Kept> = Vec::new();
            let mut nb = 0usize;
            self.join(s, &tables, outer, &mut |f| {
                bound.extend(f.rows.iter().map(Kept::of));
                nb += 1;
                ControlFlow::Continue(())
            });
            let binding = |bi: usize| -> Vec<RowRef<'_>> { bound[bi * w_items..(bi + 1) * w_items].iter().map(Kept::row).collect() };
            let mut wins = vec![vec![V::Null; s.wins.len()]; nb];
            for (wi, w) in s.wins.iter().enumerate() {
                let mut keyed: Vec<(Vec<V>, Vec<V>, usize)> = (0..nb)
                    .map(|bi| {
                        let refs = binding(bi);
                        let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: None };
                        (w.partition.iter().map(|e| self.eval(e, &ctx)).collect(), w.order.iter().map(|(e, _)| self.eval(e, &ctx)).collect(), bi)
                    })
                    .collect();
                keyed.sort_by(|a, b| {
                    for (x, y) in a.0.iter().zip(&b.0) {
                        let o = sv::cmp(x, y);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    for (k, (x, y)) in a.1.iter().zip(&b.1).enumerate() {
                        let o = sv::cmp(x, y);
                        if o != Ordering::Equal {
                            return if w.order[k].1 { o.reverse() } else { o };
                        }
                    }
                    a.2.cmp(&b.2)
                });
                let mut n = 0i64;
                let mut prev: Option<&Vec<V>> = None;
                for (p, _, bi) in &keyed {
                    let same = prev.is_some_and(|q| q.iter().zip(p).all(|(x, y)| sv::cmp(x, y) == Ordering::Equal));
                    n = if same { n + 1 } else { 1 };
                    prev = Some(p);
                    wins[*bi][wi] = V::Int(n);
                }
            }
            for (bi, w) in wins.iter().enumerate() {
                let refs = binding(bi);
                let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: Some(w) };
                out.push(self, &ctx);
            }
        } else {
            self.join(s, &tables, outer, &mut |f| {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                out.push(self, &ctx);
                ControlFlow::Continue(())
            });
        }
        let t = out.finish(&tables, s.out.clone());
        if s.distinct {
            let mut seen = rustc_hash::FxHashSet::default();
            let w = s.proj.len();
            let keep: Vec<u32> = (0..t.len()).filter(|&r| seen.insert((0..w).map(|c| sv::dkey(&t.value(r, c))).collect::<Vec<_>>())).map(|r| r as u32).collect();
            if keep.len() != t.len() {
                return t.select(keep.into());
            }
        }
        t
    }

    fn sub(&self, id: usize, ctx: &EvalCtx<'_>, want: SubWant) -> Rc<SubResult> {
        let sub = &self.stmt.subs[id];
        if sub.cacheable {
            if let Some(r) = self.sub_cache.borrow().get(&id) {
                return r.clone();
            }
        }
        let frame = ctx.frame;
        let r = match want {
            SubWant::Exists => {
                let mut found = false;
                match &sub.q.body {
                    PBody::Select(s) if !s.grouped && s.wins.is_empty() && sub.q.ctes.is_empty() && sub.q.limit.is_none() => {
                        if let Some(tables) = self.item_tables(s, Some(&frame)) {
                            self.join(s, &tables, Some(&frame), &mut |_| {
                                found = true;
                                ControlFlow::Break(())
                            });
                        }
                    }
                    _ => {
                        found = !self.query(&sub.q, Some(&frame)).is_empty();
                    }
                }
                SubResult::Exists(found)
            }
            SubWant::Scalar => {
                let t = self.query(&sub.q, Some(&frame));
                SubResult::Scalar(if t.is_empty() { V::Null } else { t.value(0, 0) })
            }
            SubWant::Set(aff) => {
                let t = self.query(&sub.q, Some(&frame));
                let mut set = rustc_hash::FxHashSet::default();
                let mut has_null = false;
                for r in 0..t.len() {
                    match sv::key(&sv::compare_affinity(&t.value(r, 0), aff)) {
                        Some(k) => {
                            set.insert(k);
                        }
                        None => has_null = true,
                    }
                }
                SubResult::Set(set, has_null, !t.is_empty())
            }
        };
        let r = Rc::new(r);
        if sub.cacheable {
            self.sub_cache.borrow_mut().insert(id, r.clone());
        }
        r
    }

    fn col(&self, ctx: &EvalCtx<'_>, depth: u16, item: u16, col: u16) -> V {
        let mut f = &ctx.frame;
        for _ in 0..depth {
            match f.parent {
                Some(p) => f = p,
                None => return V::Null,
            }
        }
        f.rows.get(item as usize).map_or(V::Null, |r| r.value(col))
    }

    fn eval(&self, pe: &PE, ctx: &EvalCtx<'_>) -> V {
        match pe {
            PE::Lit(l) => match *l {
                Lit::Null => V::Null,
                Lit::Int(i) => V::Int(i),
                Lit::Real(f) => V::Real(f),
                Lit::Text(i) => self.texts[i as usize].get_or_init(|| V::text(&self.stmt.texts[i as usize])).clone(),
            },
            PE::Col { depth, item, col } => self.col(ctx, *depth, *item, *col),
            PE::Agg(i) => ctx.aggs.and_then(|a| a.get(*i as usize)).cloned().unwrap_or(V::Null),
            PE::Win(i) => ctx.wins.and_then(|a| a.get(*i as usize)).cloned().unwrap_or(V::Null),
            PE::Not(x) => match sv::truth(&self.eval(x, ctx)) {
                None => V::Null,
                Some(b) => V::bool(!b),
            },
            PE::Neg(x) => match sv::to_numeric(&self.eval(x, ctx)) {
                None => V::Null,
                Some(V::Int(i)) => i.checked_neg().map_or(V::Real(-(i as f64)), V::Int),
                Some(V::Real(f)) => V::Real(-f),
                Some(_) => V::Null,
            },
            PE::Cmp(op, l, r, aff) => {
                let lv = self.eval(l, ctx);
                let rv = self.eval(r, ctx);
                match sv::compare(&lv, &rv, *aff) {
                    None => V::Null,
                    Some(o) => V::bool(match op {
                        Cmp::Eq => o == Ordering::Equal,
                        Cmp::Ne => o != Ordering::Equal,
                        Cmp::Lt => o == Ordering::Less,
                        Cmp::Le => o != Ordering::Greater,
                        Cmp::Gt => o == Ordering::Greater,
                        Cmp::Ge => o != Ordering::Less,
                    }),
                }
            }
            PE::Arith(op, l, r) => sv::arith(*op, &self.eval(l, ctx), &self.eval(r, ctx)),
            PE::Concat(l, r) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                match (sv::to_text(&a), sv::to_text(&b)) {
                    (Some(x), Some(y)) => {
                        let mut s = String::with_capacity(x.len() + y.len());
                        s.push_str(&x);
                        s.push_str(&y);
                        V::Text(Rc::from(s))
                    }
                    _ => V::Null,
                }
            }
            PE::And(l, r) => {
                let a = sv::truth(&self.eval(l, ctx));
                if a == Some(false) {
                    return V::Int(0);
                }
                let b = sv::truth(&self.eval(r, ctx));
                match (a, b) {
                    (_, Some(false)) => V::Int(0),
                    (Some(true), Some(true)) => V::Int(1),
                    _ => V::Null,
                }
            }
            PE::Or(l, r) => {
                let a = sv::truth(&self.eval(l, ctx));
                if a == Some(true) {
                    return V::Int(1);
                }
                let b = sv::truth(&self.eval(r, ctx));
                match (a, b) {
                    (_, Some(true)) => V::Int(1),
                    (Some(false), Some(false)) => V::Int(0),
                    _ => V::Null,
                }
            }
            PE::Func(f, args) => self.func(*f, args, ctx),
            PE::Case { operand, aff, whens, else_ } => {
                let ov = operand.as_ref().map(|o| self.eval(o, ctx));
                for (w, t) in whens {
                    let hit = match &ov {
                        Some(o) => sv::compare(o, &self.eval(w, ctx), *aff) == Some(Ordering::Equal),
                        None => sv::truth(&self.eval(w, ctx)) == Some(true),
                    };
                    if hit {
                        return self.eval(t, ctx);
                    }
                }
                else_.as_ref().map_or(V::Null, |e| self.eval(e, ctx))
            }
            PE::Cast(x, a) => sv::cast(&self.eval(x, ctx), *a),
            PE::InList { e, list, not } => {
                let v = self.eval(e, ctx);
                if v.is_null() {
                    return if list.is_empty() { V::bool(*not) } else { V::Null };
                }
                let mut null = false;
                for (x, aff) in list {
                    let xv = self.eval(x, ctx);
                    match sv::compare(&v, &xv, *aff) {
                        Some(Ordering::Equal) => return V::bool(!*not),
                        None => null = true,
                        _ => {}
                    }
                }
                if null { V::Null } else { V::bool(*not) }
            }
            PE::InSub { e, sub, not, aff } => {
                let v = self.eval(e, ctx);
                let r = self.sub(*sub, ctx, SubWant::Set(aff.1));
                let SubResult::Set(set, has_null, nonempty) = &*r else { unreachable!() };
                if !nonempty {
                    return V::bool(*not);
                }
                let Some(k) = sv::key(&sv::compare_affinity(&v, aff.0)) else { return V::Null };
                if set.contains(&k) {
                    V::bool(!*not)
                } else if *has_null {
                    V::Null
                } else {
                    V::bool(*not)
                }
            }
            PE::Exists { sub, not } => {
                let r = self.sub(*sub, ctx, SubWant::Exists);
                let SubResult::Exists(b) = &*r else { unreachable!() };
                V::bool(*b != *not)
            }
            PE::Scalar(sub) => {
                let r = self.sub(*sub, ctx, SubWant::Scalar);
                let SubResult::Scalar(v) = &*r else { unreachable!() };
                v.clone()
            }
            PE::IsNull(x, not) => V::bool(self.eval(x, ctx).is_null() != *not),
            PE::Is(l, r, not) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                let eq = match (&a, &b) {
                    (V::Null, V::Null) => true,
                    (V::Null, _) | (_, V::Null) => false,
                    _ => sv::cmp(&a, &b) == Ordering::Equal,
                };
                V::bool(eq != *not)
            }
            PE::Like(l, r, not) => {
                let (a, b) = (self.eval(l, ctx), self.eval(r, ctx));
                match (sv::to_text(&a), sv::to_text(&b)) {
                    (Some(t), Some(p)) => V::bool(sv::like(&p, &t) != *not),
                    _ => V::Null,
                }
            }
        }
    }

    fn regex(&self, pattern: &str) -> Option<Rc<regex::Regex>> {
        match crate::rx::local(pattern) {
            Ok(r) => Some(r),
            Err(e) => {
                self.fail(format!("regex: {e}"));
                None
            }
        }
    }

    fn rx_text(&self, v: &V) -> Option<Rc<str>> {
        match v {
            V::Null => Some(Rc::from("")),
            V::Text(s) => Some(s.clone()),
            _ => {
                self.fail("rx_* text argument must be text".into());
                None
            }
        }
    }

    fn func(&self, f: Func, args: &[PE], ctx: &EvalCtx<'_>) -> V {
        let a = |i: usize| self.eval(&args[i], ctx);
        match f {
            Func::Coalesce | Func::Ifnull => {
                for x in args {
                    let v = self.eval(x, ctx);
                    if !v.is_null() {
                        return v;
                    }
                }
                V::Null
            }
            Func::Length => match a(0) {
                V::Null => V::Null,
                V::Text(s) => V::Int(s.chars().count() as i64),
                v => V::Int(sv::to_text(&v).map_or(0, |s| s.chars().count() as i64)),
            },
            Func::Lower | Func::Upper => match sv::to_text(&a(0)) {
                None => V::Null,
                Some(s) => V::text(&if f == Func::Lower { s.to_ascii_lowercase() } else { s.to_ascii_uppercase() }),
            },
            Func::Trim | Func::Ltrim | Func::Rtrim => {
                let Some(s) = sv::to_text(&a(0)) else { return V::Null };
                let chars = if args.len() > 1 {
                    match sv::to_text(&a(1)) {
                        Some(c) => Some(c),
                        None => return V::Null,
                    }
                } else {
                    None
                };
                V::text(&sv::trim(&s, chars.as_deref(), f != Func::Rtrim, f != Func::Ltrim))
            }
            Func::Substr => {
                let Some(s) = sv::to_text(&a(0)) else { return V::Null };
                let st = match sv::cast(&a(1), Aff::Integer) {
                    V::Int(i) => i,
                    _ => return V::Null,
                };
                let len = if args.len() > 2 {
                    match sv::cast(&a(2), Aff::Integer) {
                        V::Int(i) => Some(i),
                        _ => return V::Null,
                    }
                } else {
                    None
                };
                V::text(&sv::substr(&s, st, len))
            }
            Func::Instr => {
                let (h, n) = (a(0), a(1));
                match (sv::to_text(&h), sv::to_text(&n)) {
                    (Some(h), Some(n)) => match h.find(&*n) {
                        Some(b) => V::Int(h[..b].chars().count() as i64 + 1),
                        None => V::Int(0),
                    },
                    _ => V::Null,
                }
            }
            Func::Replace => {
                let (s, from, to) = (a(0), a(1), a(2));
                match (sv::to_text(&s), sv::to_text(&from), sv::to_text(&to)) {
                    (Some(s), Some(from), Some(to)) => {
                        if from.is_empty() {
                            V::Text(s)
                        } else {
                            V::text(&s.replace(&*from, &to))
                        }
                    }
                    _ => V::Null,
                }
            }
            Func::Char => {
                let mut s = String::new();
                for i in 0..args.len() {
                    if let V::Int(c) = sv::cast(&a(i), Aff::Integer) {
                        if let Some(ch) = char::from_u32(c as u32) {
                            s.push(ch);
                        }
                    }
                }
                V::text(&s)
            }
            Func::JsonQuote => sv::json_quote(&a(0)),
            Func::JsonExtract => {
                let (j, p) = (a(0), a(1));
                match (sv::to_text(&j), sv::to_text(&p)) {
                    (Some(j), Some(p)) => match sv::json_extract(&j, &p) {
                        Ok(v) => v,
                        Err(e) => {
                            self.fail(e);
                            V::Null
                        }
                    },
                    _ => V::Null,
                }
            }
            Func::RxMatches => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let V::Text(p) = a(1) else {
                    self.fail("rx pattern must be text".into());
                    return V::Null;
                };
                match self.regex(&p) {
                    Some(r) => V::bool(r.is_match(&t)),
                    None => V::Null,
                }
            }
            Func::RxExtract => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let V::Text(p) = a(1) else {
                    self.fail("rx pattern must be text".into());
                    return V::Null;
                };
                let g = match a(2) {
                    V::Int(g) if g >= 0 => g as usize,
                    _ => {
                        self.fail("rx_extract group must be a non-negative integer".into());
                        return V::Null;
                    }
                };
                let Some(r) = self.regex(&p) else { return V::Null };
                if g >= r.captures_len() {
                    self.fail("unknown regex group".into());
                    return V::Null;
                }
                match r.captures(&t).and_then(|c| c.get(g)) {
                    Some(m) => V::text(m.as_str()),
                    None => V::Null,
                }
            }
            Func::RxReplace => {
                let Some(t) = self.rx_text(&a(0)) else { return V::Null };
                let (V::Text(p), V::Text(by)) = (a(1), a(2)) else {
                    self.fail("rx_replace pattern and replacement must be text".into());
                    return V::Null;
                };
                let Some(r) = self.regex(&p) else { return V::Null };
                V::text(&r.replace_all(&t, &*by))
            }
            Func::SourceText => {
                let (p, s, e) = (a(0), a(1), a(2));
                let V::Text(p) = p else {
                    if !p.is_null() {
                        self.fail("source_text path must be text".into());
                    }
                    return V::Null;
                };
                if &*p != self.db.path {
                    return V::Null;
                }
                match (s, e) {
                    (V::Int(s), V::Int(e)) if s >= 0 && e >= s => match self.db.source.get(s as usize..e as usize) {
                        Some(t) => V::text(t),
                        None => V::Null,
                    },
                    _ => V::Null,
                }
            }
            Func::Abs => match sv::to_numeric(&a(0)) {
                None => V::Null,
                Some(V::Int(i)) => V::Int(i.abs()),
                Some(V::Real(f)) => V::Real(f.abs()),
                _ => V::Null,
            },
            Func::Typeof => V::text(match a(0) {
                V::Null => "null",
                V::Int(_) => "integer",
                V::Real(_) => "real",
                V::Text(_) => "text",
            }),
            Func::Max | Func::Min => {
                let mut best: Option<V> = None;
                for i in 0..args.len() {
                    let v = a(i);
                    if v.is_null() {
                        return V::Null;
                    }
                    best = Some(match best {
                        None => v,
                        Some(b) => {
                            let o = sv::cmp(&v, &b);
                            if (f == Func::Max && o == Ordering::Greater) || (f == Func::Min && o == Ordering::Less) { v } else { b }
                        }
                    });
                }
                best.unwrap_or(V::Null)
            }
            Func::Iif => {
                if sv::truth(&a(0)) == Some(true) { a(1) } else { a(2) }
            }
            Func::Nullif => {
                let (x, y) = (a(0), a(1));
                if sv::compare(&x, &y, (Aff::None, Aff::None)) == Some(Ordering::Equal) { V::Null } else { x }
            }
        }
    }
}

fn json_each_row(key: V, v: &serde_json::Value, path: &str) -> Box<[V]> {
    let ty = match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(true) => "true",
        serde_json::Value::Bool(false) => "false",
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() { "integer" } else { "real" }
        }
        serde_json::Value::String(_) => "text",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    let value = sv::json_value_to_sql(v);
    let atom = match v {
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => V::Null,
        _ => value.clone(),
    };
    vec![key, value, V::text(ty), atom, V::Null, V::Null, V::text(path), V::text("$")].into_boxed_slice()
}

#[derive(Clone, Copy)]
enum SubWant {
    Exists,
    Scalar,
    Set(Aff),
}

#[derive(Default)]
struct AggState {
    count: i64,
    best: Option<V>,
    concat: Option<String>,
    sum_i: Option<i64>,
    sum_f: Option<f64>,
    seen: Option<rustc_hash::FxHashSet<DKey>>,
}

impl AggState {
    /// Folds one value; true when a min/max took this row.
    fn update(&mut self, spec: &AggSpec, v: Option<V>, sep: Option<V>) -> bool {
        if spec.f == AggFn::CountStar {
            self.count += 1;
            return false;
        }
        let v = v.unwrap_or(V::Null);
        if v.is_null() {
            return false;
        }
        if spec.distinct {
            let set = self.seen.get_or_insert_default();
            if !set.insert(sv::dkey(&v)) {
                return false;
            }
        }
        match spec.f {
            AggFn::Count => self.count += 1,
            AggFn::Min | AggFn::Max => {
                let better = match &self.best {
                    None => true,
                    Some(b) => {
                        let o = sv::cmp(&v, b);
                        (spec.f == AggFn::Max && o == Ordering::Greater) || (spec.f == AggFn::Min && o == Ordering::Less)
                    }
                };
                if better {
                    self.best = Some(v);
                    return true;
                }
            }
            AggFn::GroupConcat => {
                let t = sv::to_text(&v).unwrap();
                match &mut self.concat {
                    None => self.concat = Some(t.to_string()),
                    Some(c) => {
                        let s = match &sep {
                            Some(V::Null) => String::new(),
                            Some(x) => sv::to_text(x).map(|t| t.to_string()).unwrap_or_default(),
                            None => ",".into(),
                        };
                        c.push_str(&s);
                        c.push_str(&t);
                    }
                }
            }
            AggFn::Sum | AggFn::Total => match sv::to_numeric(&v) {
                Some(V::Int(i)) if self.sum_f.is_none() => match self.sum_i.unwrap_or(0).checked_add(i) {
                    Some(s) => self.sum_i = Some(s),
                    None => self.sum_f = Some(self.sum_i.unwrap_or(0) as f64 + i as f64),
                },
                Some(V::Int(i)) => self.sum_f = Some(self.sum_f.unwrap() + i as f64),
                Some(V::Real(f)) => {
                    self.sum_f = Some(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64) + f);
                }
                _ => {}
            },
            AggFn::CountStar => {}
        }
        false
    }
    fn finish(self, spec: &AggSpec) -> V {
        match spec.f {
            AggFn::Count | AggFn::CountStar => V::Int(self.count),
            AggFn::Min | AggFn::Max => self.best.unwrap_or(V::Null),
            AggFn::GroupConcat => self.concat.map_or(V::Null, |c| V::text(&c)),
            AggFn::Sum => match (self.sum_i, self.sum_f) {
                (_, Some(f)) => V::Real(f),
                (Some(i), None) => V::Int(i),
                (None, None) => V::Null,
            },
            AggFn::Total => V::Real(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64)),
        }
    }
}
