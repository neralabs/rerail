//! Planner and executor for the SQLite SELECT subset. Plans are fixed by the SQL text: joins run
//! in FROM order as index nested loops; equality conjuncts between a FROM item's column and
//! earlier items (or outer rows, or constants) probe a hash index built once per table; every
//! other conjunct is applied at the earliest level where its columns are bound. Correlated
//! subqueries probe the same indexes; uncorrelated ones run once per statement execution.
use super::parse::{self, Body, Expr, JoinKind, Query, SelItem, SetOp, Source};
use super::value::{self as sv, Aff, DKey, Key, V};
use crate::counts::Phys;
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
    /// R16, a text node id `kind || ':' || start || ':' || end` projected from three columns,
    /// kept as its parts: every row reads that concatenation's value, but keys and comparisons
    /// read the parts (`Cell::Tid`), and nothing per row is stored or formatted unless the text
    /// itself is read.
    Tid { kind: Rc<Column>, start: Rc<Column>, end: Rc<Column> },
    /// R16, a node id column whose parts are a symbol and two offsets on every row, stored flat
    /// (`facts::NONE` kind: a NULL id): 12 bytes a row, read without indirection.
    TidFlat { kind: Vec<u32>, start: Vec<u32>, end: Vec<u32> },
    /// R19, a column of a recursive walk whose rows each read one row of one source column:
    /// row `r` is `srcs[tag[r]]` at row `row[r]` (`PICK_NULL`: NULL). 6 bytes a row.
    Pick { srcs: Rc<[Rc<Column>]>, tag: Vec<u16>, row: Vec<u32> },
}

/// `Column::U32`'s NULL.
pub const U32_NULL: u32 = u32::MAX;
/// `Column::Pick`'s NULL tag.
const PICK_NULL: u16 = u16::MAX;
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
            Column::Tid { kind, start, end } => {
                let parts = [kind.get(r), start.get(r), end.get(r)];
                let [Some(k), Some(s), Some(e)] = parts.map(|v| sv::to_text(&v)) else { return V::Null };
                V::text(&format!("{k}:{s}:{e}"))
            }
            Column::TidFlat { kind, start, end } => match kind[r as usize] {
                crate::facts::NONE => V::Null,
                k => tid_text(&TidKind::Sym(k), start[r as usize] as i64, end[r as usize] as i64, V::text),
            },
            Column::Pick { srcs, tag, row } => match tag[r as usize] {
                PICK_NULL => V::Null,
                t => srcs[t as usize].get(row[r as usize]),
            },
        }
    }

    /// `col` with every value converted by `aff` on read (CTAS storage).
    pub fn with_aff(col: &Rc<Column>, aff: Aff) -> Rc<Column> {
        if aff == Aff::None {
            return col.clone();
        }
        match &**col {
            Column::Gather { src, idx, aff: Aff::None } => Rc::new(Column::Gather { src: src.clone(), idx: idx.clone(), aff }),
            // A node id is NULL or a text holding `:`, which no affinity converts (a well-formed
            // number has no `:`; text affinity leaves a text alone).
            Column::Tid { .. } | Column::TidFlat { .. } => col.clone(),
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
            Column::Tid { .. } => 0,
            Column::TidFlat { kind, .. } => kind.len() * 12,
            Column::Pick { tag, .. } => tag.len() * 6,
        }
    }

    /// Levels of indirection a read goes through.
    fn depth(&self) -> usize {
        match self {
            Column::Vals(_) | Column::Text { .. } | Column::Const(_) | Column::U32(_) | Column::Sym(_) | Column::Interval { .. } | Column::TidFlat { .. } => 0,
            Column::Gather { src, .. } => 1 + src.depth(),
            Column::Chain(p) => 1 + p.iter().map(|(_, c)| c.depth()).max().unwrap_or(0),
            Column::Tid { kind, start, end } => 1 + kind.depth().max(start.depth()).max(end.depth()),
            Column::Pick { srcs, .. } => 1 + srcs.iter().map(|c| c.depth()).max().unwrap_or(0),
        }
    }

    /// Row `r`'s value as a borrowed cell: what `get` returns, without cloning or allocating
    /// (a node's text is a slice of the source; a symbol stays an id).
    pub(crate) fn cell(&self, r: u32) -> Cell<'_> {
        match self {
            Column::Vals(v) => vcell(&v[r as usize]),
            Column::Gather { src, idx, aff } => {
                let s = match idx {
                    Some(i) => i[r as usize],
                    None => r,
                };
                if s == NULL_ROW {
                    return Cell::Null;
                }
                let c = src.cell(s);
                if *aff == Aff::None { c } else { cell_aff(c, *aff) }
            }
            Column::Chain(parts) => {
                let p = parts.partition_point(|(start, _)| *start <= r) - 1;
                parts[p].1.cell(r - parts[p].0)
            }
            Column::Text { source, cells, vals, .. } => match cells[r as usize] {
                (TEXT_VAL, i) => vcell(&vals[i as usize]),
                (s, e) => Cell::Text(&source[s as usize..e as usize]),
            },
            Column::Const(v) => vcell(v),
            Column::U32(v) => match v[r as usize] {
                U32_NULL => Cell::Null,
                x => Cell::Int(x as i64),
            },
            Column::Sym(v) => match v[r as usize] {
                crate::facts::NONE => Cell::Null,
                s => Cell::Sym(s),
            },
            Column::Interval { .. } => vcell_own(self.get(r)),
            Column::Tid { kind, start, end } => match (kind.cell(r), start.cell(r), end.cell(r)) {
                (Cell::Null, _, _) | (_, Cell::Null, _) | (_, _, Cell::Null) => Cell::Null,
                (Cell::Text(k), Cell::Int(s), Cell::Int(e)) => Cell::Tid(TidKind::Text(k), s, e),
                (Cell::RcT(k), Cell::Int(s), Cell::Int(e)) => Cell::Tid(TidKind::Text(k), s, e),
                (Cell::Sym(k), Cell::Int(s), Cell::Int(e)) => Cell::Tid(TidKind::Sym(k), s, e),
                _ => Cell::Own(self.get(r)),
            },
            Column::TidFlat { kind, start, end } => match kind[r as usize] {
                crate::facts::NONE => Cell::Null,
                k => Cell::Tid(TidKind::Sym(k), start[r as usize] as i64, end[r as usize] as i64),
            },
            Column::Pick { srcs, tag, row } => match tag[r as usize] {
                PICK_NULL => Cell::Null,
                t => srcs[t as usize].cell(row[r as usize]),
            },
        }
    }

    /// A node id over `n` rows of `kind`, `start` and `end` (R16): flat when every row's parts are
    /// a kind naming a symbol and two offsets in `u32` (or a NULL part), else kept as its parts.
    /// A kind read as text (a repository clause's spilled rows store kinds as values) is flat
    /// when the text is an existing symbol's name: each distinct text is looked up once, and no
    /// symbol is created (the symbol table holds the grammar's kinds; other texts stay lazy).
    fn tid(kind: Rc<Column>, start: Rc<Column>, end: Rc<Column>, n: usize) -> Column {
        let (mut k, mut s, mut e) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let off = |c: Cell<'_>| match c {
            Cell::Int(i) if (0..U32_NULL as i64).contains(&i) => Some(Some(i as u32)),
            Cell::Null => Some(None),
            _ => None,
        };
        let mut syms: FxHashMap<String, Option<u32>> = FxHashMap::default();
        let mut sym_of = |t: &str| match syms.get(t) {
            Some(y) => *y,
            None => *syms.entry(t.to_owned()).or_insert(crate::facts::lookup(t)),
        };
        for r in 0..n as u32 {
            let (kc, sc, ec) = (kind.cell(r), off(start.cell(r)), off(end.cell(r)));
            let (Some(sc), Some(ec)) = (sc, ec) else { return Column::Tid { kind, start, end } };
            let y = match (kc, sc, ec) {
                (Cell::Null, _, _) | (_, None, _) | (_, _, None) => None,
                (Cell::Sym(y), _, _) => Some(y),
                (Cell::Text(t), _, _) => sym_of(t),
                (Cell::RcT(t), _, _) => sym_of(t),
                _ => None,
            };
            match (y, sc, ec) {
                (Some(y), Some(a), Some(b)) => {
                    k.push(y);
                    s.push(a);
                    e.push(b);
                }
                (None, Some(_), Some(_)) if !matches!(kind.cell(r), Cell::Null) => return Column::Tid { kind, start, end },
                _ => {
                    k.push(crate::facts::NONE);
                    s.push(0);
                    e.push(0);
                }
            }
        }
        Column::TidFlat { kind: k, start: s, end: e }
    }

    /// The value every row holds, when the column's structure shows it is one value (a constant,
    /// gathered without null rows, or chained from equal constants). Asked of non-empty columns.
    fn konst(&self) -> Option<V> {
        match self {
            Column::Const(v) => Some(v.clone()),
            Column::Gather { src, idx, aff } => {
                let v = src.konst()?;
                if idx.as_ref().is_some_and(|i| i.contains(&NULL_ROW)) {
                    return None;
                }
                Some(sv::apply_affinity(v, *aff))
            }
            Column::Chain(parts) => {
                let first = parts.first()?.1.konst()?;
                for (_, p) in &parts[1..] {
                    if sv::dkey(&p.konst()?) != sv::dkey(&first) || std::mem::discriminant(&p.konst()?) != std::mem::discriminant(&first) {
                        return None;
                    }
                }
                Some(first)
            }
            _ => None,
        }
    }
}

/// A value read without cloning (`Column::cell`): text borrowed from its column, a symbol by
/// id (its name read only where bytes are compared), numbers by value, a text node id by its
/// parts (`Column::Tid`), and `Own` where a storage affinity converted the value.
#[derive(Clone, Debug)]
pub(crate) enum Cell<'c> {
    Null,
    Int(i64),
    Real(f64),
    Text(&'c str),
    /// Text shared by its column (a stored value): owning it is a count bump.
    RcT(&'c Rc<str>),
    Sym(u32),
    /// The text `kind:start:end`.
    Tid(TidKind<'c>, i64, i64),
    Own(V),
}

/// The kind part of a text node id: borrowed text, a symbol, or owned text.
#[derive(Clone, Debug)]
pub(crate) enum TidKind<'c> {
    Text(&'c str),
    Sym(u32),
    Own(Rc<str>),
}

impl TidKind<'_> {
    fn with<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        match self {
            TidKind::Text(s) => f(s),
            TidKind::Sym(y) => f(&crate::facts::name(*y)),
            TidKind::Own(s) => f(s),
        }
    }
    fn owned(&self) -> TidKind<'static> {
        match self {
            TidKind::Text(s) => TidKind::Own(Rc::from(*s)),
            TidKind::Sym(y) => TidKind::Sym(*y),
            TidKind::Own(s) => TidKind::Own(s.clone()),
        }
    }
    fn eq(&self, o: &TidKind<'_>) -> bool {
        match (self, o) {
            (TidKind::Sym(a), TidKind::Sym(b)) => a == b,
            _ => self.with(|a| o.with(|b| a == b)),
        }
    }
}

/// The text of node id `kind:start:end` (`value::to_text` of each part, joined by `:`), passed
/// to `f` from a stack buffer (a heap string only for a kind longer than the buffer).
fn tid_text<R>(k: &TidKind<'_>, s: i64, e: i64, f: impl FnOnce(&str) -> R) -> R {
    fn push_int(buf: &mut [u8], at: &mut usize, v: i64) {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut u = v.unsigned_abs();
        loop {
            digits[n] = b'0' + (u % 10) as u8;
            n += 1;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        if v < 0 {
            buf[*at] = b'-';
            *at += 1;
        }
        for d in digits[..n].iter().rev() {
            buf[*at] = *d;
            *at += 1;
        }
    }
    k.with(|k| {
        let mut buf = [0u8; 160];
        if k.len() > 160 - 2 * 21 - 2 {
            return f(&format!("{k}:{s}:{e}"));
        }
        buf[..k.len()].copy_from_slice(k.as_bytes());
        let mut at = k.len();
        buf[at] = b':';
        at += 1;
        push_int(&mut buf, &mut at, s);
        buf[at] = b':';
        at += 1;
        push_int(&mut buf, &mut at, e);
        // SAFETY: a UTF-8 kind, ASCII separators and ASCII digits.
        f(unsafe { std::str::from_utf8_unchecked(&buf[..at]) })
    })
}

impl Cell<'_> {
    fn to_v(&self) -> V {
        match self {
            Cell::Null => V::Null,
            Cell::Int(i) => V::Int(*i),
            Cell::Real(f) => V::Real(*f),
            Cell::Text(s) => V::text(s),
            Cell::RcT(s) => V::Text((*s).clone()),
            Cell::Sym(y) => V::Text(crate::facts::name(*y)),
            Cell::Tid(k, s, e) => tid_text(k, *s, *e, V::text),
            Cell::Own(v) => v.clone(),
        }
    }
}

fn vcell(v: &V) -> Cell<'_> {
    match v {
        V::Null => Cell::Null,
        V::Int(i) => Cell::Int(*i),
        V::Real(f) => Cell::Real(*f),
        V::Text(s) => Cell::RcT(s),
    }
}

/// Whether a numeric affinity may turn text `s` into a number: `value::text_to_number` reads
/// only a text whose first character after leading spaces is a sign, a digit or `.`; any other
/// text stays text.
fn maybe_numeric(s: &str) -> bool {
    s.trim_start_matches([' ', '\t', '\n', '\r', '\x0c', '\x0b']).as_bytes().first().is_some_and(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
}

/// `value::apply_affinity` on a cell (a `Gather`'s storage affinity). The cases it leaves
/// unchanged stay borrowed; every other case is the owned conversion itself.
fn cell_aff(c: Cell<'_>, aff: Aff) -> Cell<'_> {
    let unchanged = match (&c, aff) {
        (_, Aff::None) | (Cell::Null, _) => true,
        (Cell::Text(_) | Cell::RcT(_) | Cell::Sym(_) | Cell::Tid(..), Aff::Text) => true,
        (Cell::Int(_), Aff::Numeric | Aff::Integer) | (Cell::Real(_), Aff::Real) => true,
        (Cell::Text(_) | Cell::RcT(_) | Cell::Sym(_) | Cell::Tid(..), Aff::Numeric | Aff::Integer | Aff::Real) => !text_may_be_numeric(&c),
        _ => false,
    };
    if unchanged { c } else { Cell::Own(sv::apply_affinity(c.to_v(), aff)) }
}

/// `maybe_numeric` of a text cell (a node id's text starts with its kind; an empty or blank
/// kind is followed by `:`, which is not numeric).
fn text_may_be_numeric(c: &Cell<'_>) -> bool {
    match c {
        Cell::Text(s) => maybe_numeric(s),
        Cell::RcT(s) => maybe_numeric(s),
        Cell::Sym(y) => maybe_numeric(&crate::facts::name(*y)),
        Cell::Tid(k, ..) => k.with(maybe_numeric),
        _ => true,
    }
}

/// `value::compare_affinity` on a cell, borrowed where it leaves the value unchanged.
fn cell_cmp_aff(c: Cell<'_>, aff: Aff) -> Cell<'_> {
    let unchanged = match (&c, aff) {
        (_, Aff::None) | (Cell::Null, _) => true,
        (Cell::Own(_), _) => false,
        (Cell::Int(_) | Cell::Real(_), Aff::Text) => false,
        (_, Aff::Text) => true,
        (Cell::Text(_) | Cell::RcT(_) | Cell::Sym(_) | Cell::Tid(..), _) => !text_may_be_numeric(&c),
        (Cell::Int(_) | Cell::Real(_), _) => true,
    };
    if unchanged { c } else { vcell_own(sv::compare_affinity(&c.to_v(), aff)) }
}

fn vcell_own(v: V) -> Cell<'static> {
    match v {
        V::Null => Cell::Null,
        V::Int(i) => Cell::Int(i),
        V::Real(f) => Cell::Real(f),
        t => Cell::Own(t),
    }
}

/// An equality key part (`value::key` of a compared value), borrowed where the column holds
/// it: integers (and integral reals), other reals by bits, text by bytes (a symbol by id and a
/// node id by its parts; each compares and hashes as its text).
#[derive(Clone, Debug)]
pub(crate) enum KCell<'c> {
    Int(i64),
    Real(u64),
    Text(&'c str),
    RcT(&'c Rc<str>),
    Sym(u32),
    Tid(TidKind<'c>, i64, i64),
    Own(Rc<str>),
}

/// `value::key(value::compare_affinity(c, aff))` on a cell; `None` for NULL.
fn cell_key(c: Cell<'_>, aff: Aff) -> Option<KCell<'_>> {
    Some(match cell_cmp_aff(c, aff) {
        Cell::Null => return None,
        Cell::Int(i) => KCell::Int(i),
        Cell::Real(f) => real_key(f),
        Cell::Text(s) => KCell::Text(s),
        Cell::RcT(s) => KCell::RcT(s),
        Cell::Sym(y) => KCell::Sym(y),
        Cell::Tid(k, s, e) => KCell::Tid(k, s, e),
        Cell::Own(v) => KCell::of(sv::key(&v)?),
    })
}

fn real_key(f: f64) -> KCell<'static> {
    match sv::key(&V::Real(f)) {
        Some(Key::Int(i)) => KCell::Int(i),
        Some(Key::Real(b)) => KCell::Real(b),
        _ => unreachable!("a real's key is a number"),
    }
}

/// A probe's key parts: up to four inline, more on the heap.
pub(crate) enum KeyBuf<'a> {
    Inline(usize, [KCell<'a>; 4]),
    Heap(Vec<KCell<'a>>),
}

impl<'a> std::ops::Deref for KeyBuf<'a> {
    type Target = [KCell<'a>];
    fn deref(&self) -> &[KCell<'a>] {
        match self {
            KeyBuf::Inline(n, a) => &a[..*n],
            KeyBuf::Heap(v) => v,
        }
    }
}

/// Combines key part hashes (FxHash's step).
#[inline]
fn mix(h: u64, part: u64) -> u64 {
    (h.rotate_left(5) ^ part).wrapping_mul(0xf135_7aea_2e62_a9c5)
}

fn raw_hash(b: &str) -> u64 {
    use std::hash::Hasher;
    let mut h = rustc_hash::FxHasher::default();
    h.write(b.as_bytes());
    mix(3, h.finish())
}

/// The hash of a text key. A text of node-id form `kind:start:end` (both offsets canonical
/// decimal integers, as `value::to_text` writes them) hashes by its parts, so a node id kept as
/// parts (R16) hashes without formatting its text; the hash is still a function of the bytes.
fn text_hash(s: &str) -> u64 {
    match split_tid(s) {
        Some((k, a, b)) => tid_hash(raw_hash(k), a, b),
        None => raw_hash(s),
    }
}

fn tid_hash(kind: u64, s: i64, e: i64) -> u64 {
    mix(mix(mix(4, kind), s as u64), e as u64)
}

/// `kind:start:end` split at its last two `:` when both offsets are canonical decimal integers
/// (no leading zero but `0`, no `-0`, within `i64`).
fn split_tid(s: &str) -> Option<(&str, i64, i64)> {
    fn int_back(b: &[u8]) -> Option<(usize, i64)> {
        // The canonical integer ending `b`: (its start, value).
        let mut i = b.len();
        while i > 0 && b[i - 1].is_ascii_digit() {
            i -= 1;
        }
        let digits = &b[i..];
        if digits.is_empty() || digits.len() > 19 || (digits.len() > 1 && digits[0] == b'0') {
            return None;
        }
        let mut v: i64 = 0;
        for d in digits {
            v = v.checked_mul(10)?.checked_add((d - b'0') as i64)?;
        }
        if i > 0 && b[i - 1] == b'-' {
            if v == 0 {
                return None;
            }
            return Some((i - 1, -v));
        }
        Some((i, v))
    }
    let b = s.as_bytes();
    if !b.last()?.is_ascii_digit() {
        return None;
    }
    let (i, e) = int_back(b)?;
    if i == 0 || b[i - 1] != b':' {
        return None;
    }
    let (j, st) = int_back(&b[..i - 1])?;
    if j == 0 || b[j - 1] != b':' {
        return None;
    }
    Some((&s[..j - 1], st, e))
}

thread_local! {
    /// Per symbol, (`raw_hash`, `text_hash`) of its name, once computed.
    static SYM_HASH: std::cell::RefCell<Vec<Option<(u64, u64)>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A symbol's (`raw_hash`, `text_hash`) of its name.
fn sym_hash(y: u32) -> (u64, u64) {
    SYM_HASH.with(|c| {
        if let Some(Some(h)) = c.borrow().get(y as usize) {
            return *h;
        }
        let name = crate::facts::name(y);
        let h = (raw_hash(&name), text_hash(&name));
        let mut c = c.borrow_mut();
        if c.len() <= y as usize {
            c.resize(y as usize + 1, None);
        }
        c[y as usize] = Some(h);
        h
    })
}

impl KCell<'_> {
    fn of(k: Key) -> KCell<'static> {
        match k {
            Key::Int(i) => KCell::Int(i),
            Key::Real(b) => KCell::Real(b),
            Key::Text(s) => KCell::Own(s),
        }
    }
    fn owned(&self) -> KCell<'static> {
        match self {
            KCell::Int(i) => KCell::Int(*i),
            KCell::Real(b) => KCell::Real(*b),
            KCell::Text(s) => KCell::Own(Rc::from(*s)),
            KCell::RcT(s) => KCell::Own((*s).clone()),
            KCell::Sym(y) => KCell::Sym(*y),
            KCell::Tid(k, s, e) => KCell::Tid(k.owned(), *s, *e),
            KCell::Own(s) => KCell::Own(s.clone()),
        }
    }
    /// The hash of the key part; equal parts (`eq`) hash equally whatever their form.
    fn hash(&self) -> u64 {
        match self {
            KCell::Int(i) => mix(1, *i as u64),
            KCell::Real(b) => mix(2, *b),
            KCell::Sym(y) => sym_hash(*y).1,
            KCell::Tid(k, s, e) => tid_hash(
                match k {
                    TidKind::Sym(y) => sym_hash(*y).0,
                    TidKind::Text(t) => raw_hash(t),
                    TidKind::Own(t) => raw_hash(t),
                },
                *s,
                *e,
            ),
            _ => self.with_text(text_hash),
        }
    }
    /// `Key` equality: numbers by value, texts by bytes. Two node ids are equal exactly when
    /// their parts are: `kind:start:end` with decimal `start` and `end` (no `:` in either) is
    /// split back by its last two `:`.
    fn eq(&self, o: &KCell<'_>) -> bool {
        match (self, o) {
            (KCell::Int(a), KCell::Int(b)) => a == b,
            (KCell::Real(a), KCell::Real(b)) => a == b,
            (KCell::Int(_) | KCell::Real(_), _) | (_, KCell::Int(_) | KCell::Real(_)) => false,
            (KCell::Sym(a), KCell::Sym(b)) => a == b,
            (KCell::Tid(ka, sa, ea), KCell::Tid(kb, sb, eb)) => sa == sb && ea == eb && ka.eq(kb),
            _ => self.with_text(|a| o.with_text(|b| a == b)),
        }
    }
    fn with_text<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        match self {
            KCell::Text(s) => f(s),
            KCell::RcT(s) => f(s),
            KCell::Own(s) => f(s),
            KCell::Sym(y) => f(&crate::facts::name(*y)),
            KCell::Tid(k, s, e) => tid_text(k, *s, *e, f),
            KCell::Int(_) | KCell::Real(_) => unreachable!("not a text key"),
        }
    }
}

/// `value::compare` on cells: `None` when either is NULL.
fn compare_cells(a: Cell<'_>, b: Cell<'_>, aff: (Aff, Aff)) -> Option<Ordering> {
    if matches!(a, Cell::Null) || matches!(b, Cell::Null) {
        return None;
    }
    Some(cmp_cells(&cell_cmp_aff(a, aff.0), &cell_cmp_aff(b, aff.1)))
}

/// `value::cmp` on cells: NULL < numbers < text (bytes).
fn cmp_cells(a: &Cell<'_>, b: &Cell<'_>) -> Ordering {
    fn class(c: &Cell<'_>) -> u8 {
        match c {
            Cell::Null | Cell::Own(V::Null) => 0,
            Cell::Int(_) | Cell::Real(_) | Cell::Own(V::Int(_) | V::Real(_)) => 1,
            Cell::Text(_) | Cell::RcT(_) | Cell::Sym(_) | Cell::Tid(..) | Cell::Own(V::Text(_)) => 2,
        }
    }
    fn with_text<R>(c: &Cell<'_>, f: impl FnOnce(&str) -> R) -> R {
        match c {
            Cell::Text(s) => f(s),
            Cell::RcT(s) => f(s),
            Cell::Sym(y) => f(&crate::facts::name(*y)),
            Cell::Own(V::Text(s)) => f(s),
            Cell::Tid(k, s, e) => tid_text(k, *s, *e, f),
            _ => unreachable!("not a text"),
        }
    }
    match (a, b) {
        (Cell::Int(x), Cell::Int(y)) => x.cmp(y),
        (Cell::Sym(x), Cell::Sym(y)) if x == y => Ordering::Equal,
        _ => match (class(a), class(b)) {
            (2, 2) => with_text(a, |x| with_text(b, |y| x.as_bytes().cmp(y.as_bytes()))),
            (ca, cb) if ca != cb => ca.cmp(&cb),
            _ => sv::cmp(&a.to_v(), &b.to_v()),
        },
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
        // A gathered node id is the node id of the gathered parts (a null row's parts are NULL,
        // and so is its id).
        Column::Tid { kind, start, end } => Rc::new(Column::Tid { kind: gather(kind, idx, memo), start: gather(start, idx, memo), end: gather(end, idx, memo) }),
        // A gathered pick is the pick of the gathered rows (a null row reads NULL).
        Column::Pick { srcs, tag, row } => Rc::new(Column::Pick {
            srcs: srcs.clone(),
            tag: idx.iter().map(|&k| if k == NULL_ROW { PICK_NULL } else { tag[k as usize] }).collect(),
            row: idx.iter().map(|&k| if k == NULL_ROW { 0 } else { row[k as usize] }).collect(),
        }),
        _ if src.depth() >= MAX_DEPTH => Rc::new(Column::Vals(idx.iter().map(|&k| if k == NULL_ROW { V::Null } else { src.get(k) }).collect())),
        _ => Rc::new(Column::Gather { src: src.clone(), idx: Some(idx.clone()), aff: Aff::None }),
    }
}

/// UNION ALL of columns of `n` rows each. Adjacent gathers of one source merge into one; a
/// union of node ids (R16) is the node id of the unions of their parts, so the parts stay
/// shallow gathers through unions and recursion levels.
fn chain(cols: &[(&Rc<Column>, usize)]) -> Rc<Column> {
    let live: Vec<&(&Rc<Column>, usize)> = cols.iter().filter(|(_, n)| *n > 0).collect();
    if live.len() > 1 && live.iter().all(|(c, _)| matches!(&***c, Column::Tid { .. })) {
        let part = |j: usize| {
            let ps: Vec<(&Rc<Column>, usize)> = live
                .iter()
                .map(|(c, n)| match &***c {
                    Column::Tid { kind, start, end } => ([kind, start, end][j], *n),
                    _ => unreachable!("every part is a node id"),
                })
                .collect();
            chain(&ps)
        };
        return Rc::new(Column::Tid { kind: part(0), start: part(1), end: part(2) });
    }
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

/// Set semantics over rows (UNION, recursive UNION, DISTINCT): each column's values interned by
/// their distinctness key (`value::dkey`: NULLs equal, numbers by value, texts by bytes), read
/// as key cells; rows compared as tuples of interned ids.
struct Distinct {
    ids: Vec<Interner>,
    narrow: rustc_hash::FxHashSet<[u32; 4]>,
    wide: rustc_hash::FxHashSet<Box<[u32]>>,
    /// `slot`'s rows: key → slot (keys of at most four columns inline).
    narrow_slots: FxHashMap<[u32; 4], u32>,
    wide_slots: FxHashMap<Box<[u32]>, u32>,
}

/// Ids for distinctness keys (`None`: NULL), in first-seen order; each key stored once, owned.
#[derive(Default)]
struct Interner {
    heads: FxHashMap<u64, u32>,
    keys: Vec<(Option<KCell<'static>>, u32)>,
}

impl Interner {
    fn id(&mut self, k: Option<KCell<'_>>) -> u32 {
        let h = k.as_ref().map_or(0, KCell::hash);
        let mut i = self.heads.get(&h).copied().unwrap_or(u32::MAX);
        while i != u32::MAX {
            let (rep, next) = &self.keys[i as usize];
            let same = match (rep, &k) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq(b),
                _ => false,
            };
            if same {
                return i;
            }
            i = *next;
        }
        let id = self.keys.len() as u32;
        let next = self.heads.insert(h, id).unwrap_or(u32::MAX);
        self.keys.push((k.map(|k| k.owned()), next));
        id
    }
    /// The id of a value's distinctness key.
    fn of(&mut self, c: Cell<'_>) -> u32 {
        self.id(cell_key(c, Aff::None))
    }
}

impl Distinct {
    fn new(width: usize) -> Distinct {
        Distinct { ids: (0..width).map(|_| Interner::default()).collect(), narrow: Default::default(), wide: Default::default(), narrow_slots: Default::default(), wide_slots: Default::default() }
    }
    /// Column `c`'s id for value `v`.
    fn intern(&mut self, c: usize, v: Cell<'_>) -> u32 {
        self.ids[c].of(v)
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
        let w = self.ids.len();
        if w <= 4 {
            let mut key = [0u32; 4];
            for (c, k) in key.iter_mut().enumerate().take(w) {
                *k = self.intern(c, t.cell(r, c));
            }
            return self.narrow.insert(key);
        }
        let key: Vec<u32> = (0..w).map(|c| self.intern(c, t.cell(r, c))).collect();
        self.insert_key(&key)
    }
    /// The slot of row `r`'s key (its first `width` columns): an earlier equal row's, or `next`,
    /// recorded, for a new key.
    fn slot(&mut self, t: &Table, r: usize, next: u32) -> u32 {
        let w = self.ids.len();
        if w <= 4 {
            let mut key = [0u32; 4];
            for (c, k) in key.iter_mut().enumerate().take(w) {
                *k = self.intern(c, t.cell(r, c));
            }
            return *self.narrow_slots.entry(key).or_insert(next);
        }
        let key: Box<[u32]> = (0..w).map(|c| self.intern(c, t.cell(r, c))).collect();
        *self.wide_slots.entry(key).or_insert(next)
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
    /// Row `r`'s value in column `c` as a borrowed cell.
    pub(crate) fn cell(&self, r: usize, c: usize) -> Cell<'_> {
        self.cols.get(c).map_or(Cell::Null, |col| col.cell(r as u32))
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
    /// SQLite's `printf` (`sv::printf`: `%d %i %s %f %%`, others refused by name at run time).
    Printf,
    /// SQLite's `json_array`: an argument that is itself a JSON function's result (JSON subtype)
    /// is embedded as JSON, any other as its JSON value (`json_elem`).
    JsonArray,
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
    /// SQLite's `json_group_array`: every value, NULLs included, in input order; `[]` over none.
    JsonGroupArray,
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

/// R14 (`docs/facts/REWRITES.md`): comparison conjuncts `col OP e` (`OP` one of `<`, `<=`, `>`,
/// `>=`) on one column of item `i`, `e` reading earlier items or outer rows. Candidates are the
/// rows whose comparison value (`compare_affinity(col, col_aff)`) lies between the bounds in
/// `value::cmp`'s total order, found by binary search in a sorted index (within the equality
/// probe's group, if any), then put back in table order. The conjuncts stay in `level` and are
/// re-checked.
#[derive(Clone, Debug)]
pub(crate) struct RangeProbe {
    pub(crate) col: u16,
    pub(crate) col_aff: Aff,
    /// The lower bound: `(e, e's comparison affinity, strict)`.
    pub(crate) lo: Option<(PE, Aff, bool)>,
    /// The upper bound, likewise.
    pub(crate) hi: Option<(PE, Aff, bool)>,
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
    /// `None`: `row_number()`; `Some((max, arg))`: `min`/`max(arg)` over the whole partition.
    pub(crate) func: Option<(bool, PE)>,
}

#[derive(Clone, Debug)]
pub(crate) struct PSelect {
    pub(crate) items: Vec<PItem>,
    pub(crate) probes: Vec<Option<Probe>>,
    /// Per item, the branch probes of an OR conjunct (candidates = union of the lookups).
    pub(crate) or_probes: Vec<Option<Vec<Probe>>>,
    /// Per item, its range probe (R14).
    pub(crate) ranges: Vec<Option<RangeProbe>>,
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
    /// Per output column, its typing as a compound arm (`Planner::arm_col`).
    pub(crate) arm: Vec<ArmCol>,
    /// A grouped select's bare columns (`(item, column)` its select list, `HAVING` and `ORDER
    /// BY` read outside aggregates, through their subqueries) that must agree within a group:
    /// empty with one `min()`/`max()` (`extremum`). SQLite reads them from an arbitrary row of
    /// the group, so two distinct values (storage class and value) are a run-time refusal.
    pub(crate) bare: Vec<(u16, u16)>,
    /// Exactly one `min()`/`max()` (others alongside; `same_call`s are one): the representative
    /// is the row it took (the group's last while it took none), as SQLite documents.
    pub(crate) extremum: bool,
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
    /// `tree`: R18/R19 may run (not under `FACTS_REWRITES=0`); each checks its side conditions
    /// per execution and falls back to the operator below it.
    /// `real`: the columns typed REAL, whose integers every arm's rows read as reals (then
    /// neither the closure nor R18/R19 runs).
    Recursive { slot: usize, batch: usize, init: Vec<PBody>, rec: Vec<PBody>, distinct: bool, tree: bool, schema: Schema, real: Vec<u16> },
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
    /// Per output column, its type as a FROM subquery, CTE or statement sees it (`query_ty`;
    /// `out`'s affinities are these types' IR affinities).
    pub(crate) ty: Vec<Ty>,
    /// The right-most arm's first column's type (a scalar or `IN` subquery's comparisons).
    pub(crate) first: Ty,
    /// Whether the body is a compound (its values need not match its types).
    pub(crate) compound: bool,
    /// Columns whose integers read as reals (`real_cols`): set where the query is a FROM
    /// subquery or a CTE.
    pub(crate) real: Vec<u16>,
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
    /// Per column, its type (BLOB for a column declared without a type: above an expression's
    /// none in a comparison).
    ty: Vec<Ty>,
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
    ctes: Vec<(String, usize, Schema, bool, Vec<Ty>)>,
    subs: Vec<SubPlan>,
    n_slots: usize,
    bounds: Vec<SubBound>,
    reads: Vec<String>,
    texts: Vec<Box<str>>,
    /// Whether rewrites are planned: R14 range probes, R18/R19 on recursive CTEs (off under
    /// `FACTS_REWRITES=0`).
    rewrites: bool,
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

/// `c` as one bound of an R14 range probe on item `i`: `(col, col_aff, bound, bound_aff, lower,
/// strict)`. The bound reads only earlier items or outer rows (never item `i`, an aggregate or a
/// subquery) and reads at least one of them: a constant bound would sort the table to save one
/// scan.
fn probe_range(c: &PE, i: usize) -> Option<(u16, Aff, PE, Aff, bool, bool)> {
    let PE::Cmp(op, l, r, (al, ar)) = c else { return None };
    let varies = |b: &PE| {
        let mut outer = false;
        visit_cols(b, &mut |d, _, _| outer |= d > 0);
        outer || max_item(b).is_some()
    };
    let pick = |a: &PE, b: &PE| -> Option<u16> {
        if let PE::Col { depth: 0, item, col } = a {
            if *item as usize == i && max_item(b).is_none_or(|m| m < i) && !refs_item(b, i) && !has_agg(b) && varies(b) {
                return Some(*col);
            }
        }
        None
    };
    // (lower, strict) of `col OP bound`.
    let side = match op {
        Cmp::Gt => (true, true),
        Cmp::Ge => (true, false),
        Cmp::Lt => (false, true),
        Cmp::Le => (false, false),
        Cmp::Eq | Cmp::Ne => return None,
    };
    if let Some(col) = pick(l, r) {
        return Some((col, *al, (**r).clone(), *ar, side.0, side.1));
    }
    // `bound OP col`: the column is on the other side of the inequality.
    pick(r, l).map(|col| (col, *ar, (**l).clone(), *al, !side.0, side.1))
}

pub fn plan(sql: &str, catalog: &Catalog) -> Result<Statement, String> {
    plan_with(sql, catalog, crate::lowered::rewrites())
}

/// `plan` with the planner's rewrites (R14, R18, R19) on or off.
pub(crate) fn plan_with(sql: &str, catalog: &Catalog, rewrites: bool) -> Result<Statement, String> {
    let q = parse::parse_query(sql)?;
    let mut p = Planner { catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new(), texts: Vec::new(), rewrites };
    let root = p.query(&q)?;
    let mut seen = std::collections::BTreeSet::new();
    let reads = p.reads.into_iter().filter(|r| seen.insert(r.clone())).collect();
    Ok(Statement { root, subs: p.subs, n_slots: p.n_slots, reads, texts: p.texts })
}

/// [`plan`] for a stage, whose rows are stored as SQLite's `CREATE TABLE … AS` stores them
/// ([`stage_checked`]).
pub fn plan_stage(sql: &str, catalog: &Catalog) -> Result<Statement, String> {
    stage_checked(plan(sql, catalog)?)
}

/// A stage's statement, refused when a column has FLEXNUM type, as the front end refuses it
/// (`front::sql::lower_stage`): CTAS stores it converting texts to numbers and keeping integral
/// reals, which no declared affinity expresses.
pub(crate) fn stage_checked(stmt: Statement) -> Result<Statement, String> {
    if let Some(i) = stmt.root.ty.iter().position(|t| *t == Ty::Flex) {
        return Err(format!("stage column {}: FLEXNUM affinity (a compound whose left-most arm CASTs to a numeric type, or a column of one) stored by CREATE TABLE … AS", stmt.root.out.cols[i]));
    }
    Ok(stmt)
}

/// Plans a scalar expression over one table's columns (a `deferred` column).
pub fn plan_scalar(sql: &str, schema: &Schema) -> Result<(PE, Statement), String> {
    let e = parse::parse_expr(sql)?;
    let catalog = Catalog::new();
    let mut p = Planner { catalog: &catalog, scopes: Vec::new(), ctes: Vec::new(), subs: Vec::new(), n_slots: 0, bounds: Vec::new(), reads: Vec::new(), texts: Vec::new(), rewrites: crate::lowered::rewrites() };
    p.scopes.push(Scope { items: vec![ScopeItem { name: "__row".into(), schema: schema.clone(), ty: stored_types(schema) }], aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
    let pe = p.expr(&e)?;
    let stmt = Statement { root: PQuery { ctes: vec![], body: PBody::Compound(Box::new(PBody::Select(Box::new(empty_select()))), SetOp::UnionAll, Box::new(PBody::Select(Box::new(empty_select()))), Schema::default()), order: vec![], order_on_output: false, limit: None, offset: None, out: Schema::default(), ty: vec![], first: Ty::None, compound: true, real: vec![] }, subs: p.subs, n_slots: p.n_slots, reads: p.reads, texts: p.texts };
    Ok((pe, stmt))
}

fn empty_select() -> PSelect {
    PSelect { items: vec![], probes: vec![], or_probes: vec![], ranges: vec![], level: vec![], post: vec![], konst: vec![], grouped: false, group_by: vec![], aggs: vec![], having: None, wins: vec![], proj: vec![], order_keys: vec![], distinct: false, out: Schema::default(), arm: vec![], bare: vec![], extremum: false }
}

/// An expression's SQLite 3.53.2 affinity (`sqlite3ExprAffinity`), as `rerail_facts::front::sql`
/// tracks it: `None` is an expression's no affinity; `Blob` a stored column's with no declared
/// type (or `CAST(… AS BLOB)`, or a compound column demoted by [`settle`]), which ranks above it
/// in a comparison; `Flex` (FLEXNUM) is numeric in a comparison and converts nothing on read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Ty {
    None,
    Blob,
    Text,
    Numeric,
    Integer,
    Real,
    Flex,
}

impl Ty {
    /// The IR affinity (BLOB and none are both `Aff::None`; FLEXNUM is numeric).
    fn aff(self) -> Aff {
        match self {
            Ty::None | Ty::Blob => Aff::None,
            Ty::Text => Aff::Text,
            Ty::Numeric | Ty::Flex => Aff::Numeric,
            Ty::Integer => Aff::Integer,
            Ty::Real => Aff::Real,
        }
    }
    /// An expression of IR affinity `a` (`CAST` targets: `BLOB` is `Blob`).
    fn of(a: Aff) -> Ty {
        match a {
            Aff::None => Ty::None,
            Aff::Text => Ty::Text,
            Aff::Numeric => Ty::Numeric,
            Aff::Integer => Ty::Integer,
            Aff::Real => Ty::Real,
        }
    }
    /// `sqlite3ExprDataType` of a column, subquery or `CAST` of this affinity: the storage
    /// classes it may hold (0x01 numeric, 0x02 text, 0x04 blob).
    fn data_type(self) -> u8 {
        match self {
            Ty::Numeric | Ty::Integer | Ty::Real | Ty::Flex => 0x05,
            Ty::Text => 0x06,
            Ty::None | Ty::Blob => 0x07,
        }
    }
}

/// A stored table's columns' types: a column whose declared type gives no affinity (none
/// declared, or a CTAS column whose expression had none) has BLOB affinity.
fn stored_types(schema: &Schema) -> Vec<Ty> {
    schema.affs.iter().map(|a| if *a == Aff::None { Ty::Blob } else { Ty::of(*a) }).collect()
}

/// The comparison affinities of two operands (`sqlite3CompareAffinity`): `comparison_affinity`,
/// except that a TEXT operand compared with a BLOB one converts nothing (both have an affinity,
/// neither numeric), where with an expression of no affinity it converts that expression.
fn cmp_ty(a: Ty, b: Ty) -> (Aff, Aff) {
    if (a == Ty::Text && b == Ty::Blob) || (b == Ty::Text && a == Ty::Blob) {
        return (Aff::None, Aff::None);
    }
    sv::comparison_affinity(a.aff(), b.aff())
}

/// The comparison affinities when an operand is a compound subquery (scalar or `IN`), whose
/// values need not match its affinity: SQLite applies the comparison's one affinity to both
/// operands (`front::sql::cmp_aff_both`).
fn cmp_ty_both(a: Ty, b: Ty) -> (Aff, Aff) {
    let x = if a.aff().numeric_class() || b.aff().numeric_class() {
        Aff::Numeric
    } else {
        match cmp_ty(a, b) {
            (Aff::None, y) => y,
            (x, _) => x,
        }
    };
    (x, x)
}

/// One output column of one select (a compound's arm): its expression's type, its
/// `sqlite3ExprDataType` and whether it is a `CAST`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ArmCol {
    ty: Ty,
    dt: u8,
    cast: bool,
}

/// `sqlite3SubqueryColumnTypes`' first phase over one column of a compound's arms (text order):
/// the first arm's type; while it is none and another arm follows, the data types of the arms
/// passed over accumulate and the next arm's type is taken. The type, the arm it came from and
/// the accumulated data types. Mutant 33 stops at the left-most arm.
fn walk_arms(arms: &[ArmCol]) -> (Ty, usize, u8) {
    let (mut ty, mut j, mut m) = (arms[0].ty, 0, 0u8);
    while ty <= Ty::None && j + 1 < arms.len() && !crate::mutant::on(33) {
        m |= arms[j].dt;
        j += 1;
        ty = arms[j].ty;
    }
    (ty, j, m)
}

/// The second phase: a TEXT or numeric type of a compound (`post`: the arms as typed after the
/// first phase) becomes BLOB when an arm after `j` may hold a number (TEXT) or a text (numeric);
/// a numeric one whose left-most arm is a `CAST` becomes FLEXNUM.
fn settle(ty: Ty, j: usize, mut m: u8, post: &[ArmCol]) -> Ty {
    if ty < Ty::Text || post.len() < 2 {
        return ty;
    }
    for a in &post[j + 1..] {
        m |= a.dt;
    }
    let mut ty = ty;
    if (ty == Ty::Text && m & 0x01 != 0) || (ty >= Ty::Numeric && m & 0x02 != 0) {
        ty = Ty::Blob;
    }
    if ty >= Ty::Numeric && post[0].cast {
        ty = Ty::Flex;
    }
    ty
}

/// A body's arms (its selects in text order).
fn body_arms<'b>(b: &'b PBody, out: &mut Vec<&'b [ArmCol]>) {
    match b {
        PBody::Select(s) => out.push(&s.arm),
        PBody::Compound(l, _, r, _) => {
            body_arms(l, out);
            body_arms(r, out);
        }
    }
}

/// A query's column types as a FROM subquery, CTE or statement sees them
/// (`sqlite3SubqueryColumnTypes` over its arms), and the type of the right-most arm's first
/// column (what a scalar subquery or `IN (SELECT …)` compares with: SQLite reads a compound's
/// last select).
fn query_ty(arms: &[&[ArmCol]]) -> (Vec<Ty>, Ty) {
    let w = arms[0].len();
    let cols = (0..w)
        .map(|i| {
            let col: Vec<ArmCol> = arms.iter().map(|a| a[i]).collect();
            let (ty, j, m) = walk_arms(&col);
            settle(ty, j, m, &col)
        })
        .collect();
    (cols, arms.last().and_then(|a| a.first()).map_or(Ty::None, |c| c.ty))
}

/// The columns of a compound read as a FROM subquery or CTE that convert integers to reals
/// (`OP_RealAffinity` on a REAL column; texts and reals unchanged).
fn real_cols(compound: bool, ty: &[Ty]) -> Vec<u16> {
    if !compound {
        return Vec::new();
    }
    (0..ty.len() as u16).filter(|&i| ty[i as usize] == Ty::Real).collect()
}

/// `t` with the integers of columns `real` read as reals (`real_cols`).
fn real_read(mut t: Table, real: &[u16]) -> Table {
    for &c in real {
        let Some(col) = t.cols.get(c as usize) else { continue };
        if (0..t.len as u32).any(|r| matches!(col.get(r), V::Int(_))) {
            let vals = (0..t.len as u32)
                .map(|r| match col.get(r) {
                    V::Int(i) => V::Real(i as f64),
                    v => v,
                })
                .collect();
            t.cols[c as usize] = Rc::new(Column::Vals(vals));
        }
    }
    t
}

/// Whether two cells are the same value of the same storage class (NULL is NULL): a bare
/// column's agreement within a group (the reference's `identical`).
fn cells_identical(a: &Cell<'_>, b: &Cell<'_>) -> bool {
    fn class(c: &Cell<'_>) -> u8 {
        match c {
            Cell::Null | Cell::Own(V::Null) => 0,
            Cell::Int(_) | Cell::Own(V::Int(_)) => 1,
            Cell::Real(_) | Cell::Own(V::Real(_)) => 2,
            Cell::Text(_) | Cell::RcT(_) | Cell::Sym(_) | Cell::Tid(..) | Cell::Own(V::Text(_)) => 3,
        }
    }
    match (a, b) {
        (Cell::Tid(k1, s1, e1), Cell::Tid(k2, s2, e2)) => s1 == s2 && e1 == e2 && k1.eq(k2),
        _ => match (class(a), class(b)) {
            (0, 0) => true,
            (1, 1) => matches!((a.to_v(), b.to_v()), (V::Int(x), V::Int(y)) if x == y),
            (2, 2) => matches!((a.to_v(), b.to_v()), (V::Real(x), V::Real(y)) if x == y),
            (3, 3) => cmp_cells(a, b) == Ordering::Equal,
            _ => false,
        },
    }
}

/// A bound row's column as a cell (NULL for an unbound or null-extended row).
fn row_cell<'r>(r: &RowRef<'r>, col: u16) -> Cell<'r> {
    match r.t {
        Some(t) if r.r != NULL_ROW => t.cols.get(col as usize).map_or(Cell::Null, |c| c.cell(r.r)),
        _ => Cell::Null,
    }
}

/// Calls `f(item, col)` for each column of the select `d` scope levels out (`d = 0`: the select
/// `pe` belongs to) that `pe` reads, through its subqueries (`subs`): a derived table, a CTE body
/// or a compound arm stays on its level, a subquery expression is one level in (the reference's
/// `ir::cols_read`).
fn cols_read_at(pe: &PE, d: u16, subs: &[SubPlan], f: &mut dyn FnMut(u16, u16)) {
    match pe {
        PE::Col { depth, item, col } => {
            if *depth == d {
                f(*item, *col)
            }
        }
        PE::Lit(_) | PE::Agg(_) | PE::Win(_) => {}
        PE::Not(x) | PE::Neg(x) | PE::Cast(x, _) | PE::IsNull(x, _) => cols_read_at(x, d, subs, f),
        PE::Cmp(_, l, r, _) | PE::Arith(_, l, r) | PE::Concat(l, r) | PE::And(l, r) | PE::Or(l, r) | PE::Is(l, r, _) | PE::Like(l, r, _) => {
            cols_read_at(l, d, subs, f);
            cols_read_at(r, d, subs, f);
        }
        PE::Func(_, a) => a.iter().for_each(|x| cols_read_at(x, d, subs, f)),
        PE::Case { operand, whens, else_, .. } => {
            if let Some(o) = operand {
                cols_read_at(o, d, subs, f);
            }
            for (w, t) in whens {
                cols_read_at(w, d, subs, f);
                cols_read_at(t, d, subs, f);
            }
            if let Some(x) = else_ {
                cols_read_at(x, d, subs, f);
            }
        }
        PE::InList { e, list, .. } => {
            cols_read_at(e, d, subs, f);
            list.iter().for_each(|(x, _)| cols_read_at(x, d, subs, f));
        }
        PE::InSub { e, sub, .. } => {
            cols_read_at(e, d, subs, f);
            query_reads_at(&subs[*sub].q, d + 1, subs, f);
        }
        PE::Exists { sub, .. } | PE::Scalar(sub) => query_reads_at(&subs[*sub].q, d + 1, subs, f),
    }
}

/// [`cols_read_at`] over every expression of a query on level `d`.
fn query_reads_at(q: &PQuery, d: u16, subs: &[SubPlan], f: &mut dyn FnMut(u16, u16)) {
    fn body(b: &PBody, d: u16, subs: &[SubPlan], f: &mut dyn FnMut(u16, u16)) {
        let s = match b {
            PBody::Select(s) => s,
            PBody::Compound(l, _, r, _) => {
                body(l, d, subs, f);
                return body(r, d, subs, f);
            }
        };
        for it in &s.items {
            match &it.src {
                Src::JsonEach(arg) => cols_read_at(arg, d, subs, f),
                Src::Derived(id) => query_reads_at(&subs[*id].q, d, subs, f),
                Src::Global(_) | Src::Cte(_) => {}
            }
        }
        let mut e = |pe: &PE| cols_read_at(pe, d, subs, f);
        for p in s.probes.iter().flatten().chain(s.or_probes.iter().flatten().flatten()) {
            p.exprs.iter().for_each(&mut e);
        }
        for r in s.ranges.iter().flatten() {
            r.lo.iter().chain(&r.hi).for_each(|(x, ..)| e(x));
        }
        s.level.iter().chain(&s.post).flatten().chain(&s.konst).chain(&s.group_by).for_each(&mut e);
        for a in &s.aggs {
            a.arg.iter().chain(&a.sep).for_each(&mut e);
        }
        s.having.iter().for_each(&mut e);
        for w in &s.wins {
            w.partition.iter().chain(w.order.iter().map(|(x, _)| x)).chain(w.func.iter().map(|(_, x)| x)).for_each(&mut e);
        }
        s.proj.iter().chain(&s.order_keys).for_each(&mut e);
    }
    for cte in &q.ctes {
        match cte {
            PCte::Plain { q, .. } => query_reads_at(q, d, subs, f),
            PCte::Recursive { init, rec, .. } => init.iter().chain(rec).for_each(|b| body(b, d, subs, f)),
        }
    }
    body(&q.body, d, subs, f);
    for x in q.limit.iter().chain(&q.offset) {
        cols_read_at(x, d, subs, f);
    }
}

/// Two aggregate calls SQLite folds into one: the same call, or the same function over the same
/// column (any other two count as two; the reference's rule).
fn same_call(a: &AggSpec, b: &AggSpec) -> bool {
    std::ptr::eq(a, b) || (a.f == b.f && a.distinct == b.distinct && matches!((&a.arg, &b.arg), (Some(PE::Col { depth: d1, item: i1, col: c1 }), Some(PE::Col { depth: d2, item: i2, col: c2 })) if d1 == d2 && i1 == i2 && c1 == c2))
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
                // A compound CTE's REAL columns read integers as reals (`real_cols`).
                let mut pq = self.query(&cte.q)?;
                pq.real = real_cols(pq.compound, &pq.ty);
                let mut schema = pq.out.clone();
                if let Some(cols) = &cte.cols {
                    if cols.len() != schema.cols.len() {
                        return Err(format!("CTE {} column count", cte.name));
                    }
                    schema.cols = cols.clone();
                }
                let slot = self.new_slot();
                self.ctes.push((lc(&cte.name), slot, schema, false, pq.ty.clone()));
                ctes.push(PCte::Plain { slot, q: pq });
            }
        }
        let body = self.body(&q.body, Some(q))?;
        // Column types over the arms (`query_ty`); a column's affinity is its type's.
        let (ty, first) = {
            let mut arms = Vec::new();
            body_arms(&body, &mut arms);
            query_ty(&arms)
        };
        let mut out = body.schema().clone();
        out.affs = ty.iter().map(|t| t.aff()).collect();
        let compound = matches!(body, PBody::Compound(..));
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
        Ok(PQuery { ctes, body, order, order_on_output, limit, offset, out, ty, first, compound, real: Vec::new() })
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
        let steps = &arms[first_rec..];
        if !steps.iter().all(|a| body_refs(a, &cte.name)) {
            return Err("non-recursive arm after a recursive arm".into());
        }
        // Column types over every arm (SQLite 3.53.2 types a recursive CTE as a compound),
        // column by column while the steps' references to the CTE read its columns as typed so
        // far: an earlier column's final type, a later one's none; the column itself none while
        // the arms are walked (`walk_arms`), then the walk's type (`settle`). The trial plans of
        // the steps are dropped (`trial_steps`).
        let mut init_arms: Vec<Vec<ArmCol>> = Vec::new();
        for b in &init {
            let mut a = Vec::new();
            body_arms(b, &mut a);
            init_arms.extend(a.into_iter().map(<[ArmCol]>::to_vec));
        }
        let w = schema.cols.len();
        let mut ty = vec![Ty::None; w];
        for i in 0..w {
            let now = |ty: &[Ty], own: Ty| (0..w).map(|j| if j < i { ty[j] } else if j == i { own } else { Ty::None }).collect::<Vec<_>>();
            let col = |post: &[Vec<ArmCol>]| init_arms.iter().chain(post).map(|a| a[i]).collect::<Vec<_>>();
            let walked = self.trial_steps(&cte.name, batch, &schema, &now(&ty, Ty::None), steps)?;
            let (t, j, m) = walk_arms(&col(&walked));
            ty[i] = if t >= Ty::Text {
                let post = self.trial_steps(&cte.name, batch, &schema, &now(&ty, t), steps)?;
                settle(t, j, m, &col(&post))
            } else {
                t
            };
        }
        schema.affs = ty.iter().map(|t| t.aff()).collect();
        let (rec, _) = self.plan_steps(&cte.name, batch, &schema, &ty, steps)?;
        self.ctes.push((lc(&cte.name), slot, schema.clone(), false, ty.clone()));
        // Every arm's rows read through the REAL columns (the step reads the previous level
        // converted too); the closure and R18/R19 do not convert, so they do not run then.
        let real = real_cols(true, &ty);
        Ok(PCte::Recursive { slot, batch, init, rec, distinct, tree: self.rewrites, schema, real })
    }

    /// A recursive CTE's step arms planned with the CTE's columns typed `ty` (in scope as the
    /// batch `batch` while they plan): their bodies and their arms' column typing.
    fn plan_steps(&mut self, name: &str, batch: usize, schema: &Schema, ty: &[Ty], steps: &[&Body]) -> Result<(Vec<PBody>, Vec<Vec<ArmCol>>), String> {
        let mut schema = schema.clone();
        schema.affs = ty.iter().map(|t| t.aff()).collect();
        let w = schema.cols.len();
        self.ctes.push((lc(name), batch, schema, true, ty.to_vec()));
        let mut planned = || -> Result<(Vec<PBody>, Vec<Vec<ArmCol>>), String> {
            let (mut bodies, mut arms) = (Vec::new(), Vec::new());
            for a in steps {
                let b = self.body(a, None)?;
                if b.schema().cols.len() != w {
                    return Err("recursive arm width".into());
                }
                let mut ac = Vec::new();
                body_arms(&b, &mut ac);
                arms.extend(ac.into_iter().map(<[ArmCol]>::to_vec));
                bodies.push(b);
            }
            Ok((bodies, arms))
        };
        let r = planned();
        self.ctes.pop();
        r
    }

    /// [`plan_steps`]' arm typing, its plans dropped: the subqueries, slots, texts and reads it
    /// added are removed again.
    fn trial_steps(&mut self, name: &str, batch: usize, schema: &Schema, ty: &[Ty], steps: &[&Body]) -> Result<Vec<Vec<ArmCol>>, String> {
        let mark = (self.subs.len(), self.n_slots, self.texts.len(), self.reads.len());
        let r = self.plan_steps(name, batch, schema, ty, steps);
        self.subs.truncate(mark.0);
        self.n_slots = mark.1;
        self.texts.truncate(mark.2);
        self.reads.truncate(mark.3);
        r.map(|(_, arms)| arms)
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
                // SQLite: a compound's column names come from its left-most arm; its column types
                // from every arm (`query_ty`).
                let schema = pl.schema().clone();
                Ok(PBody::Compound(Box::new(pl), *op, Box::new(pr), schema))
            }
        }
    }

    fn source_schema(&mut self, src: &Source) -> Result<(Src, Schema, Vec<Ty>, String), String> {
        match src {
            Source::Table(name) => {
                let l = lc(name);
                if let Some((_, slot, schema, rec, ty)) = self.ctes.iter().rev().find(|c| c.0 == l) {
                    if *rec {
                        if let Some(b) = self.bounds.last_mut() {
                            b.recursive_ref = true;
                        }
                        for b in self.bounds.iter_mut() {
                            b.recursive_ref = true;
                        }
                    }
                    return Ok((Src::Cte(*slot), schema.clone(), ty.clone(), name.clone()));
                }
                let schema = self.catalog.get(&l).ok_or_else(|| format!("no such table: {name}"))?.clone();
                self.reads.push(l);
                let ty = stored_types(&schema);
                Ok((Src::Global(name.clone()), schema, ty, name.clone()))
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
                Ok((Src::JsonEach(arg), schema, vec![Ty::None; 8], f.clone()))
            }
            Source::Sub(q) => {
                // SQLite scoping: a FROM subquery sees the enclosing select's outer scopes, never
                // this select's own (sibling) items, and runs with the enclosing select's outer
                // frame (`table_of`), so its outer references count depth from there. The
                // select's partial scope is hidden while the subquery is planned (mutant 22 keeps
                // it visible).
                let own = (!crate::mutant::on(22)).then(|| self.scopes.pop().expect("source_schema runs inside a select scope"));
                let id = self.subquery(q);
                self.scopes.extend(own);
                let id = id?;
                // A compound's REAL columns read integers as reals (`real_cols`).
                let q = &mut self.subs[id].q;
                q.real = real_cols(q.compound, &q.ty);
                Ok((Src::Derived(id), q.out.clone(), q.ty.clone(), String::new()))
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
        // FROM items are pushed one by one; a derived table is planned with this select's scope
        // hidden (`source_schema`), a lateral `json_each` sees the items before it.
        self.scopes.push(Scope { items: Vec::new(), aliases: Vec::new(), aggs: Vec::new(), wins: Vec::new(), allow_agg: false });
        let mut items = Vec::new();
        for fi in &s.from {
            let (src, schema, ty, name) = self.source_schema(&fi.source)?;
            let alias = fi.alias.clone().unwrap_or(name);
            let lateral = matches!(&src, Src::JsonEach(arg) if max_item(arg).is_some());
            items.push(PItem { src, width: schema.cols.len(), left: fi.kind == JoinKind::Left, lateral });
            self.scopes.last_mut().unwrap().items.push(ScopeItem { name: lc(&alias), schema, ty });
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
        let mut ranges: Vec<Option<RangeProbe>> = vec![None; n];
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
            // R14: comparison conjuncts on one column of the item, bounded by earlier items or
            // outer rows, become a range probe (within the equality probe's group). They stay in
            // `level[i]` and are re-checked. A column bounded on both sides is preferred.
            if self.rewrites && or_probes[i].is_none() {
                let mut cands: Vec<RangeProbe> = Vec::new();
                for c in &rest {
                    let Some((col, col_aff, e, aff, lower, strict)) = probe_range(c, i) else { continue };
                    let k = match cands.iter().position(|r| r.col == col && r.col_aff == col_aff) {
                        Some(k) => k,
                        None => {
                            cands.push(RangeProbe { col, col_aff, lo: None, hi: None });
                            cands.len() - 1
                        }
                    };
                    let slot = if lower { &mut cands[k].lo } else { &mut cands[k].hi };
                    if slot.is_none() {
                        *slot = Some((e, aff, strict));
                    }
                }
                let pick = cands.iter().position(|r| r.lo.is_some() && r.hi.is_some()).or((!cands.is_empty()).then_some(0));
                ranges[i] = pick.map(|k| cands.swap_remove(k));
            }
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
        let arm = proj.iter().map(|p| self.arm_col(p)).collect();
        let scope = self.scopes.pop().unwrap();
        let grouped = !group_by.is_empty() || !scope.aggs.is_empty();
        // Bare columns (the decider's ruling, SQLite 3.53.2): with one min()/max() the
        // representative is its row; otherwise the columns read outside aggregates must agree
        // within a group. Mutant 31 drops that check.
        let extrema: Vec<&AggSpec> = scope.aggs.iter().filter(|a| matches!(a.f, AggFn::Min | AggFn::Max)).collect();
        let extremum = !extrema.is_empty() && extrema.iter().all(|b| same_call(extrema[0], b));
        let mut bare: Vec<(u16, u16)> = Vec::new();
        if grouped && !extremum && !crate::mutant::on(31) {
            for pe in proj.iter().chain(&order_keys).chain(&having) {
                cols_read_at(pe, 0, &self.subs, &mut |i, c| {
                    if !bare.contains(&(i, c)) {
                        bare.push((i, c));
                    }
                });
            }
        }
        Ok(PSelect { items, probes, or_probes, ranges, level, post, konst, grouped, group_by, aggs: scope.aggs, having, wins: scope.wins, proj, order_keys, distinct: s.distinct, out, arm, bare, extremum })
    }

    fn col_name(&self, depth: u16, item: u16, col: u16) -> Option<String> {
        let s = self.scopes.len().checked_sub(1 + depth as usize)?;
        Some(self.scopes[s].items[item as usize].schema.cols[col as usize].clone())
    }

    fn affinity(&self, pe: &PE) -> Aff {
        self.ty(pe).aff()
    }

    /// `pe`'s type (`sqlite3ExprAffinity`): a column's (BLOB for one with no declared type,
    /// `stored_types`; a subquery's or CTE's per `query_ty`), a `CAST`'s target (`BLOB`: BLOB),
    /// a scalar subquery's right-most arm's first column's; none for any other expression.
    fn ty(&self, pe: &PE) -> Ty {
        match pe {
            PE::Col { depth, item, col } => {
                let s = self.scopes.len() - 1 - *depth as usize;
                self.scopes[s].items[*item as usize].ty.get(*col as usize).copied().unwrap_or(Ty::None)
            }
            PE::Cast(_, Aff::None) => Ty::Blob,
            PE::Cast(_, a) => Ty::of(*a),
            PE::Scalar(id) => self.subs[*id].q.first,
            _ => Ty::None,
        }
    }

    /// `sqlite3ExprDataType`: the storage classes `pe` may hold (0x01 numeric, 0x02 text, 0x04
    /// blob; 0 for NULL), as the front end computes it.
    fn data_type(&self, pe: &PE) -> u8 {
        match pe {
            PE::Lit(Lit::Null) => 0,
            PE::Lit(Lit::Text(_)) => 0x02,
            PE::Lit(_) => 0x01,
            PE::Col { .. } | PE::Cast(..) | PE::Scalar(_) => self.ty(pe).data_type(),
            PE::Concat(..) => 0x06,
            PE::Case { whens, else_, .. } => whens.iter().map(|(_, t)| t).chain(else_.as_deref()).fold(0, |m, x| m | self.data_type(x)),
            PE::Agg(_) | PE::Win(_) | PE::Func(..) => 0x07,
            PE::Not(_) | PE::Neg(_) | PE::Cmp(..) | PE::Arith(..) | PE::And(..) | PE::Or(..) | PE::InList { .. } | PE::InSub { .. } | PE::Exists { .. } | PE::IsNull(..) | PE::Is(..) | PE::Like(..) => 0x01,
        }
    }

    /// A select output column's typing as a compound arm.
    fn arm_col(&self, pe: &PE) -> ArmCol {
        ArmCol { ty: self.ty(pe), dt: self.data_type(pe), cast: matches!(pe, PE::Cast(..)) }
    }

    /// Whether `pe` is a scalar subquery over a compound (its value need not match its type).
    fn loose(&self, pe: &PE) -> bool {
        matches!(pe, PE::Scalar(id) if self.subs[*id].q.compound)
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
        if self.loose(l) || self.loose(r) { cmp_ty_both(self.ty(l), self.ty(r)) } else { cmp_ty(self.ty(l), self.ty(r)) }
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
                    // `row_number()`; `min`/`max` over a partition without ORDER BY (the frame is
                    // the whole partition; with ORDER BY it would be a running frame, refused).
                    let func = match (name.as_str(), args.len(), *star || *distinct, w.order.is_empty()) {
                        ("row_number", 0, false, _) => None,
                        ("min" | "max", 1, false, true) => Some((name == "max", self.expr(&args[0])?)),
                        _ => return Err(format!("window function {name} unsupported")),
                    };
                    let partition = w.partition.iter().map(|p| self.expr(p)).collect::<Result<Vec<_>, _>>()?;
                    let order = w.order.iter().map(|(o, d)| Ok((self.expr(o)?, *d))).collect::<Result<Vec<_>, String>>()?;
                    let sc = self.scopes.last_mut().unwrap();
                    sc.wins.push(WinSpec { partition, order, func });
                    return Ok(PE::Win((sc.wins.len() - 1) as u16));
                }
                let agg = match (name.as_str(), args.len(), *star) {
                    ("count", 0, true) => Some(AggFn::CountStar),
                    ("count", 1, false) => Some(AggFn::Count),
                    ("min", 1, false) => Some(AggFn::Min),
                    ("max", 1, false) => Some(AggFn::Max),
                    ("group_concat", 1 | 2, false) => Some(AggFn::GroupConcat),
                    ("json_group_array", 1, false) => Some(AggFn::JsonGroupArray),
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
                    let (arg, sep) = (arg?, sep?);
                    if f == AggFn::JsonGroupArray && matches!(arg, Some(PE::Func(Func::JsonExtract, _))) {
                        return Err("json_group_array over json_extract (JSON subtype by value) unsupported".into());
                    }
                    sc.aggs.push(AggSpec { f, arg, sep, distinct: *distinct });
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
                    "printf" => Func::Printf,
                    "json_array" => Func::JsonArray,
                    o => return Err(format!("function {o} unsupported")),
                };
                let pargs = args.iter().map(|a| self.expr(a)).collect::<Result<Vec<_>, _>>()?;
                // SQLite passes a JSON function's result to another with its JSON subtype:
                // `json_array` embeds it (json_subtype); `json_quote` would return it unquoted and
                // `json_extract`'s subtype depends on its value — both refused as arguments here.
                if matches!(f, Func::JsonArray | Func::JsonQuote) && pargs.iter().any(|p| matches!(p, PE::Func(Func::JsonExtract, _)) || (f == Func::JsonQuote && json_subtype(p))) {
                    return Err(format!("{name} over a JSON function's result (JSON subtype) unsupported"));
                }
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
                // The right-most arm's first column's type; a compound (or a compound scalar
                // on the left) compares with one affinity on both sides.
                let q = &self.subs[id].q;
                let aff = if q.compound || self.loose(&pe) { cmp_ty_both(self.ty(&pe), q.first) } else { cmp_ty(self.ty(&pe), q.first) };
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
///
/// Keys are read as borrowed cells (`Column::cell`, `cell_key`): no value is cloned or
/// allocated per row. A key column that holds one value on every row (`Column::konst`, e.g. a
/// partition's `__path`) is checked once per lookup instead of hashed per row.
struct Index {
    table: Rc<Table>,
    /// Key positions read per row: (position in the key, column, affinity).
    var: Vec<(usize, u16, Aff)>,
    /// Key positions whose column holds one non-NULL value on every row: (position, its key).
    /// A constant NULL leaves the index empty.
    konst: Vec<(usize, KCell<'static>)>,
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

impl Index {
    fn build(table: &Rc<Table>, cols: &[u16], affs: &[Aff]) -> Index {
        let mut ix = Index { table: table.clone(), var: Vec::new(), konst: Vec::new(), heads: FxHashMap::default(), groups: Vec::new(), rows: Vec::new() };
        let n = table.len();
        if n == 0 {
            return ix;
        }
        for (p, (&c, &a)) in cols.iter().zip(affs).enumerate() {
            // R17: a constant key column is checked once per lookup (not under FACTS_REWRITES=0).
            match crate::lowered::rewrites().then(|| table.cols[c as usize].konst()).flatten() {
                Some(v) => match cell_key(vcell(&v), a) {
                    Some(k) => ix.konst.push((p, k.owned())),
                    // Every row has a NULL key part.
                    None => return ix,
                },
                None => ix.var.push((p, c, a)),
            }
        }
        // Pass 1: each row's group (`u32::MAX`: a NULL key part).
        let mut of_row: Vec<u32> = Vec::with_capacity(n);
        let mut cur: Vec<KCell<'_>> = Vec::with_capacity(ix.var.len());
        'rows: for ri in 0..n as u32 {
            cur.clear();
            let mut h = 0u64;
            for &(_, c, a) in &ix.var {
                match cell_key(table.cols[c as usize].cell(ri), a) {
                    Some(k) => {
                        h = mix(h, k.hash());
                        cur.push(k);
                    }
                    None => {
                        of_row.push(u32::MAX);
                        continue 'rows;
                    }
                }
            }
            let mut g = ix.heads.get(&h).copied().unwrap_or(u32::MAX);
            while g != u32::MAX {
                let grp = &ix.groups[g as usize];
                if ix.rep_is(grp.rep, &cur) {
                    break;
                }
                g = grp.next;
            }
            if g == u32::MAX {
                g = ix.groups.len() as u32;
                let next = ix.heads.insert(h, g).unwrap_or(u32::MAX);
                ix.groups.push(Group { rep: ri, start: 0, len: 0, next });
            }
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

    /// Whether row `r`'s key equals `cur` (one part per `var` entry, in order).
    fn rep_is(&self, r: u32, cur: &[KCell<'_>]) -> bool {
        self.var.iter().zip(cur).all(|(&(_, c, a), k)| cell_key(self.table.cols[c as usize].cell(r), a).is_some_and(|x| x.eq(k)))
    }

    /// The group whose key equals `key` (one part per key column).
    fn group(&self, key: &[KCell<'_>]) -> Option<u32> {
        // Mutant 26 (R17, dropped side condition): the constant key parts are not compared.
        if !crate::mutant::on(26) && !self.konst.iter().all(|(p, k)| key[*p].eq(k)) {
            return None;
        }
        let h = self.var.iter().fold(0u64, |h, &(p, _, _)| mix(h, key[p].hash()));
        let mut g = *self.heads.get(&h)?;
        while g != u32::MAX {
            let grp = &self.groups[g as usize];
            if self.var.iter().all(|&(p, c, a)| cell_key(self.table.cols[c as usize].cell(grp.rep), a).is_some_and(|x| x.eq(&key[p]))) {
                return Some(g);
            }
            g = grp.next;
        }
        None
    }

    /// The rows whose key equals `key`, ascending.
    fn get(&self, key: &[KCell<'_>]) -> Option<&[u32]> {
        let g = &self.groups[self.group(key)? as usize];
        Some(&self.rows[g.start as usize..(g.start + g.len) as usize])
    }

    /// Row `r`'s key part at position `p` (`None` when it is NULL: then `r` is in no group).
    fn part(&self, r: u32, p: usize) -> Option<KCell<'_>> {
        if let Some((_, k)) = self.konst.iter().find(|(q, _)| *q == p) {
            return Some(k.clone());
        }
        let &(_, c, a) = self.var.iter().find(|(q, _, _)| *q == p)?;
        cell_key(self.table.cols[c as usize].cell(r), a)
    }

    fn bytes(&self) -> usize {
        self.rows.len() * 4 + self.groups.len() * (std::mem::size_of::<Group>() + 16)
    }
}

/// R14's sorted index of a table on one column's comparison values (`compare_affinity(v, aff)`),
/// within each group of the item's equality index (one group without one): per group, the rows
/// with a non-NULL value sorted by (value in `value::cmp` order, row), and whether that order is
/// the table's (then a slice of it is already in table order).
struct RangeIndex {
    /// Per group: (start, len, sorted rows ascending).
    groups: Vec<(u32, u32, bool)>,
    rows: Vec<u32>,
    keys: Vec<V>,
}

impl RangeIndex {
    /// `None` when a value is a NaN (`value::cmp` is then not a total order): the item runs
    /// without the range probe.
    fn build(table: &Table, eq: Option<&Index>, col: u16, aff: Aff) -> Option<RangeIndex> {
        let all: Vec<u32>;
        let groups: Vec<&[u32]> = match eq {
            Some(ix) => ix.groups.iter().map(|g| &ix.rows[g.start as usize..(g.start + g.len) as usize]).collect(),
            None => {
                all = (0..table.len() as u32).collect();
                vec![&all[..]]
            }
        };
        let column = &table.cols[col as usize];
        let mut out = RangeIndex { groups: Vec::with_capacity(groups.len()), rows: Vec::new(), keys: Vec::new() };
        let mut buf: Vec<(V, u32)> = Vec::new();
        for g in groups {
            buf.clear();
            for &r in g {
                let v = column.get(r);
                // NULL compares to nothing: such a row never satisfies a comparison.
                if v.is_null() {
                    continue;
                }
                let k = sv::compare_affinity(&v, aff);
                if matches!(k, V::Real(f) if f.is_nan()) {
                    return None;
                }
                buf.push((k, r));
            }
            buf.sort_by(|a, b| sv::cmp(&a.0, &b.0).then(a.1.cmp(&b.1)));
            let ascending = buf.windows(2).all(|w| w[0].1 < w[1].1);
            out.groups.push((out.rows.len() as u32, buf.len() as u32, ascending));
            for (k, r) in buf.drain(..) {
                out.rows.push(r);
                out.keys.push(k);
            }
        }
        Some(out)
    }

    /// Group `g`'s positions `[a, b)` in `rows` whose value lies within the bounds (`(value,
    /// strict)`; `None`: unbounded on that side), and whether they are in table order.
    fn span(&self, g: usize, lo: Option<(&V, bool)>, hi: Option<(&V, bool)>) -> (usize, usize, bool) {
        let (start, len, ascending) = self.groups[g];
        let keys = &self.keys[start as usize..(start + len) as usize];
        let a = match lo {
            // Mutant 23 (R14, wrong bound): `>=` taken as `>`, rows equal to the bound lost.
            Some((v, strict)) if strict || crate::mutant::on(23) => keys.partition_point(|k| sv::cmp(k, v) != Ordering::Greater),
            Some((v, _)) => keys.partition_point(|k| sv::cmp(k, v) == Ordering::Less),
            None => 0,
        };
        let b = match hi {
            Some((v, true)) => keys.partition_point(|k| sv::cmp(k, v) == Ordering::Less),
            Some((v, false)) => keys.partition_point(|k| sv::cmp(k, v) != Ordering::Greater),
            None => keys.len(),
        };
        (start as usize + a, start as usize + b.max(a), ascending)
    }
}

/// A join's indexes per item: the plain equality probe's, each OR branch's, and the range
/// probe's (R14; `None` also when its column holds a NaN).
struct Indexes {
    plain: Vec<Option<Rc<Index>>>,
    or: Vec<Option<Vec<Rc<Index>>>>,
    range: Vec<Option<Rc<RangeIndex>>>,
}

pub struct Exec<'d, 'a> {
    db: &'d Db<'a>,
    stmt: &'d Statement,
    slots: RefCell<Vec<Option<Rc<Table>>>>,
    /// This statement's indexes by (table, probe).
    indexes: RefCell<FxHashMap<(usize, usize), Rc<Index>>>,
    /// This statement's range indexes by (table, range probe).
    range_indexes: RefCell<FxHashMap<(usize, usize), Option<Rc<RangeIndex>>>>,
    sub_cache: RefCell<FxHashMap<usize, Rc<SubResult>>>,
    derived: RefCell<FxHashMap<usize, Rc<Table>>>,
    keep: RefCell<Vec<Rc<Table>>>,
    pub error: RefCell<Option<String>>,
    /// The statement's text literals as values, materialised on first use.
    texts: Vec<std::cell::OnceCell<V>>,
    /// Physical work counts of this execution (`crate::counts`), flushed when it ends.
    ph: crate::counts::Cells,
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
/// text node id over three such columns (R16, `tid_parts`), or a literal (one value for every
/// row), or the values its expression evaluated to.
struct Out<'p> {
    exprs: Vec<&'p PE>,
    src: Vec<Option<(usize, usize)>>,
    tid: Vec<Option<[(usize, usize); 3]>>,
    konst: Vec<Option<V>>,
    idx: Vec<Option<Vec<u32>>>,
    vals: Vec<Vec<V>>,
    n: usize,
}

/// R16: the projection `a || ':' || b || ':' || c` of three columns of non-lateral FROM items
/// (the policies' text node ids `kind || ':' || start || ':' || end`), as (item, column) parts.
fn tid_parts(p: &PE, s: &PSelect, texts: &[Box<str>]) -> Option<[(usize, usize); 3]> {
    let col = |e: &PE| match e {
        PE::Col { depth: 0, item, col } if !s.items[*item as usize].lateral => Some((*item as usize, *col as usize)),
        _ => None,
    };
    // Mutant 25 (R16, dropped side condition): any text literal taken for the `':'` separator.
    let colon = |e: &PE| matches!(e, PE::Lit(Lit::Text(i)) if &*texts[*i as usize] == ":" || crate::mutant::on(25));
    let PE::Concat(l, c) = p else { return None };
    let PE::Concat(l, sep2) = &**l else { return None };
    let PE::Concat(l, b) = &**l else { return None };
    let PE::Concat(a, sep1) = &**l else { return None };
    if !(colon(sep1) && colon(sep2)) || !crate::lowered::rewrites() {
        return None;
    }
    Some([col(a)?, col(b)?, col(c)?])
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
        let tid: Vec<Option<[(usize, usize); 3]>> = exprs.iter().map(|p| tid_parts(p, s, &ex.stmt.texts)).collect();
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
        for (item, _) in src.iter().flatten().chain(tid.iter().flatten().flatten()) {
            idx[*item] = Some(Vec::new());
        }
        let vals = exprs.iter().map(|_| Vec::new()).collect();
        Out { exprs, src, tid, konst, idx, vals, n: 0 }
    }

    fn push(&mut self, ex: &Exec<'_, '_>, ctx: &EvalCtx<'_>) {
        for (item, ix) in self.idx.iter_mut().enumerate() {
            if let Some(ix) = ix {
                ix.push(ctx.frame.rows[item].r);
            }
        }
        for (j, ((src, k), tid)) in self.src.iter().zip(&self.konst).zip(&self.tid).enumerate() {
            if src.is_none() && k.is_none() && tid.is_none() {
                self.vals[j].push(ex.eval(self.exprs[j], ctx));
            }
        }
        self.n += 1;
    }

    fn finish(self, tables: &[Rc<Table>], schema: Schema) -> Table {
        let idx: Vec<Option<Rc<[u32]>>> = self.idx.into_iter().map(|i| i.map(Rc::from)).collect();
        let mut memo = FxHashMap::default();
        let n = self.n;
        let mut read = |(item, col): (usize, usize)| match tables[item].cols.get(col) {
            Some(c) => gather(c, idx[item].as_ref().unwrap(), &mut memo),
            None => Rc::new(Column::Const(V::Null)),
        };
        let cols = self
            .src
            .iter()
            .zip(self.tid)
            .zip(self.konst)
            .zip(self.vals)
            .map(|(((src, tid), k), vals)| match (src, tid, k) {
                (Some(p), _, _) => read(*p),
                (None, Some([a, b, c]), _) => Rc::new(Column::tid(read(a), read(b), read(c), n)),
                (None, None, Some(v)) => Rc::new(Column::Const(v)),
                (None, None, None) => Rc::new(Column::Vals(vals)),
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

/// The closure's table from its walk rows `all` (per row: the admitted seed, and the edge row
/// whose `dst` is its node, `u32::MAX` for the seed's own node; level 0 first): carried columns
/// gather the seed rows; the node column chains the seeds' nodes (level 0) and the edges' `dst`
/// (deeper levels).
fn closure_table(schema: &Schema, seeds: &Table, e: &Table, (nj, dst): (usize, usize), seed_rows: &[u32], all: &[(u32, u32)]) -> Table {
    let w = schema.cols.len();
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
    Table { schema: schema.clone(), cols, len: n }
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

/// R19: an output column of a bag recursion's step as a reference: a FROM item's column at the
/// bound row, the previous level's row's column (`Carry`), a constant, or a `CASE` choosing
/// among such. `slot` numbers the item columns and literals (their source per level).
enum WalkRef<'p> {
    Item { item: u16, col: u16, slot: usize },
    Carry(u16),
    Null,
    Lit { pe: &'p PE, slot: usize },
    Case { operand: Option<&'p PE>, aff: (Aff, Aff), whens: Vec<(&'p PE, WalkRef<'p>)>, else_: Box<WalkRef<'p>> },
}

/// `pe` as a reference form over the step `s` (`None`: it is not one), numbering slots.
fn walk_ref<'p>(pe: &'p PE, s: &PSelect, slots: &mut usize) -> Option<WalkRef<'p>> {
    fn slot(slots: &mut usize) -> usize {
        *slots += 1;
        *slots - 1
    }
    Some(match pe {
        PE::Col { depth: 0, item: 0, col } => WalkRef::Carry(*col),
        PE::Col { depth: 0, item, col } if !s.items[*item as usize].lateral => WalkRef::Item { item: *item, col: *col, slot: slot(slots) },
        PE::Lit(Lit::Null) => WalkRef::Null,
        PE::Lit(_) => WalkRef::Lit { pe, slot: slot(slots) },
        PE::Case { operand, aff, whens, else_ } => {
            if operand.as_deref().is_some_and(has_sub) || whens.iter().any(|(w, _)| has_sub(w)) {
                return None;
            }
            let mut ws = Vec::with_capacity(whens.len());
            for (w, t) in whens {
                ws.push((w, walk_ref(t, s, slots)?));
            }
            let else_ = match else_ {
                Some(e) => walk_ref(e, s, slots)?,
                None => WalkRef::Null,
            };
            WalkRef::Case { operand: operand.as_deref(), aff: *aff, whens: ws, else_: Box::new(else_) }
        }
        // Mutant 29 (R19, dropped side condition): an arithmetic column taken for a reference to
        // its left operand.
        PE::Arith(_, l, _) if crate::mutant::on(29) => walk_ref(l, s, slots)?,
        _ => return None,
    })
}

/// P3b's forest-walk bound (`docs/facts/REWRITES.md` W1): a bag recursion whose one step reads
/// the recursion as its first item, probes one edge relation `E` (item 1) with columns of the
/// recursion (the walk key, at positions `key`, on `E`'s columns `from`), then only looks rows up
/// (items 2..), and projects back into the walk key columns of `E` (`to`, through the lookups'
/// equalities). Every derivation moves along one `E` row from `from` to `to`. When, on a content,
/// that relation is a forest (acyclic; every node with at most one parent, or at most one child)
/// and every lookup finds at most one row, every initial row's derivations follow distinct paths
/// of that forest: at most its depth `d` going up, at most its subtree going down (with distinct
/// initial keys, `n·d` over all of them). `Exec::check_walk` checks this per content.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WalkBound {
    pub(crate) key: Vec<u16>,
    pub(crate) from: Vec<u16>,
    pub(crate) to: Vec<u16>,
}

/// The recursive arm extends a path: some output column is a concatenation reading the same
/// column of the recursion's row (`path || step`). Its rows are paths of the graph it walks, which
/// only the visited test or set semantics keep simple: simple-path enumeration, bounded by the
/// number of simple paths (exponential in general; docs/facts/PHASE-3.md § P3b).
pub(crate) fn extends_path(batch: usize, rec: &[PBody]) -> bool {
    fn reads(e: &PE, item: u16, c: u16) -> bool {
        match e {
            PE::Col { depth: 0, item: i, col } => *i == item && *col == c,
            PE::Concat(l, r) => reads(l, item, c) || reads(r, item, c),
            _ => false,
        }
    }
    rec.iter().any(|b| {
        let PBody::Select(s) = b else { return false };
        let Some(item) = s.items.iter().position(|i| matches!(i.src, Src::Cte(x) if x == batch)) else { return false };
        s.proj.iter().enumerate().any(|(j, e)| matches!(e, PE::Concat(..)) && reads(e, item as u16, j as u16))
    })
}

/// M1, a shrinking text (`docs/facts/REWRITES.md`): the one recursive arm reads only the
/// recursion's row and replaces its column `c` by a substring of itself (`substr(c, …)`).
/// Checked per round by `Exec::recursive`: the round's longest `c` is shorter than the previous
/// round's, so the rounds are at most the initial longest text (the content's size).
pub(crate) fn shrinking_text(batch: usize, rec: &[PBody]) -> Option<u16> {
    let [PBody::Select(s)] = rec else { return None };
    if s.items.len() != 1 || !matches!(s.items[0].src, Src::Cte(x) if x == batch) || s.grouped || !s.aggs.is_empty() {
        return None;
    }
    s.proj.iter().enumerate().find_map(|(c, e)| match e {
        PE::Func(Func::Substr, a) if matches!(a.first(), Some(PE::Col { depth: 0, item: 0, col }) if *col as usize == c) => Some(c as u16),
        _ => None,
    })
}

/// M2, a counter up to a value of the input: the one recursive arm probes `E` (item 1) with the
/// recursion's column `c` plus a positive constant and writes back `E`'s probed column (or
/// `c + k`), so `c` strictly increases through values `E` holds: at most `|E|` rounds. Returns
/// `E`'s probed columns (a key checked per content, K1, bounds each round's rows).
pub(crate) fn counter_up(batch: usize, rec: &[PBody]) -> Option<(usize, Vec<u16>)> {
    let [PBody::Select(s)] = rec else { return None };
    if s.items.len() < 2 || !matches!(s.items[0].src, Src::Cte(x) if x == batch) || s.items[1..].iter().any(|i| matches!(i.src, Src::Cte(x) if x == batch)) {
        return None;
    }
    let p = s.probes.get(1)?.as_ref()?;
    p.exprs.iter().zip(&p.cols).find_map(|(x, &e)| {
        let PE::Arith('+', l, r) = x else { return None };
        let (PE::Col { depth: 0, item: 0, col: c }, PE::Lit(Lit::Int(k))) = (&**l, &**r) else { return None };
        let back = s.proj.get(*c as usize)?;
        let writes = matches!(back, PE::Col { depth: 0, item: 1, col } if *col == e) || matches!(back, PE::Arith('+', l2, r2) if matches!(**l2, PE::Col { depth: 0, item: 0, col } if col == *c) && matches!(**r2, PE::Lit(Lit::Int(k2)) if k2 > 0));
        (*k > 0 && writes).then(|| (*c as usize, p.cols.clone()))
    })
}

pub(crate) fn walk_bound(batch: usize, rec: &[PBody]) -> Option<WalkBound> {
    let [PBody::Select(s)] = rec else { return None };
    let plain = !s.grouped && s.aggs.is_empty() && s.wins.is_empty() && !s.distinct && s.having.is_none();
    if !plain || s.items.len() < 2 || !matches!(s.items[0].src, Src::Cte(b) if b == batch) {
        return None;
    }
    if s.items.iter().any(|i| i.left || i.lateral || matches!(i.src, Src::JsonEach(_))) || s.items[1..].iter().any(|i| matches!(i.src, Src::Cte(b) if b == batch)) {
        return None;
    }
    // Every later item is reached by an equality probe without affinity conversion.
    let probe = |k: usize| s.probes.get(k).and_then(Option::as_ref).filter(|p| s.or_probes.get(k).is_none_or(Option::is_none) && s.ranges.get(k).is_none_or(Option::is_none) && p.col_affs.iter().chain(&p.expr_affs).all(|a| *a == Aff::None));
    let e = probe(1)?;
    if (2..s.items.len()).any(|k| probe(k).is_none()) {
        return None;
    }
    let key: Vec<u16> = e.exprs.iter().map(|x| match x {
        PE::Col { depth: 0, item: 0, col } => Some(*col),
        _ => None,
    }).collect::<Option<_>>()?;
    // An output column as a column of `E`, through the lookups' equalities; a column of the
    // recursion carried unchanged is the `E` column it was probed against.
    fn to_edge(s: &PSelect, e: &Probe, x: &PE, fuel: usize) -> Option<u16> {
        let PE::Col { depth: 0, item, col } = x else { return None };
        match *item {
            0 => e.exprs.iter().position(|y| matches!(y, PE::Col { depth: 0, item: 0, col: c } if c == col)).map(|j| e.cols[j]),
            1 => Some(*col),
            i if fuel > 0 => {
                let p = s.probes.get(i as usize)?.as_ref()?;
                let j = p.cols.iter().position(|c| c == col)?;
                to_edge(s, e, &p.exprs[j], fuel - 1)
            }
            _ => None,
        }
    }
    let to: Vec<u16> = key.iter().map(|&c| s.proj.get(c as usize).and_then(|x| to_edge(s, e, x, s.items.len()))).collect::<Option<_>>()?;
    if to == e.cols {
        return None;
    }
    Some(WalkBound { key, from: e.cols.clone(), to })
}

/// The plan shape `Exec::walk` (R19) evaluates: a bag recursion with one initial select and one
/// step select reading the recursion once, as its first FROM item (a scan: not left-joined, not
/// lateral), not grouped, windowed, `DISTINCT` or ordered, every output column a reference form.
/// Returns the initial body, the step, its references and their slot count.
fn walk_shape<'p>(batch: usize, init: &'p [PBody], rec: &'p [PBody], distinct: bool, w: usize) -> Option<(&'p PBody, &'p PSelect, Vec<WalkRef<'p>>, usize)> {
    let ([init], [PBody::Select(s)], false) = (init, rec, distinct) else { return None };
    let plain = !s.grouped && s.wins.is_empty() && !s.distinct && s.order_keys.is_empty() && s.having.is_none();
    if !plain || s.items.is_empty() || s.proj.len() != w {
        return None;
    }
    let i0 = &s.items[0];
    if !matches!(i0.src, Src::Cte(b) if b == batch) || i0.left || i0.lateral || s.items[1..].iter().any(|it| matches!(it.src, Src::Cte(b) if b == batch)) {
        return None;
    }
    let mut slots = 0;
    let refs = s.proj.iter().map(|p| walk_ref(p, s, &mut slots)).collect::<Option<Vec<_>>>()?;
    Some((init, s, refs, slots))
}

/// Whether a recursive CTE runs as the closure operator (`Fixpoint{closure}`) rather than
/// level by level; decided by the plan alone, identical for every file.
pub(crate) fn closure_applies(cte: &PCte) -> bool {
    match cte {
        PCte::Recursive { batch, init, rec, distinct, schema, real, .. } => real.is_empty() && closure_shape(*batch, init, rec, *distinct, schema.cols.len()).is_some(),
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
            range_indexes: Default::default(),
            sub_cache: Default::default(),
            derived: Default::default(),
            keep: Default::default(),
            error: RefCell::new(None),
            texts: (0..stmt.texts.len()).map(|_| std::cell::OnceCell::new()).collect(),
            ph: Default::default(),
        }
    }
}

pub fn execute(db: &Db<'_>, stmt: &Statement) -> Result<Table, String> {
    let ex = Exec::new(db, stmt);
    let t = ex.query(&stmt.root, None);
    ex.ph.flush();
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
    ex.ph.flush();
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

    /// Records a fixpoint's admitted rows per round (`crate::counts`, algebraic) when it runs
    /// outside every expression subquery (`outer` is `None`), where the reference evaluates it once
    /// too. Every round is counted up to the first that admits nothing, as semi-naive evaluation
    /// runs it (an operator that stops a round early still ends on that empty round).
    fn fix_rounds(&self, outer: Option<&Frame<'_>>, mut rounds: Vec<u64>) {
        if outer.is_none() && !self.failed() {
            if rounds.last().is_some_and(|&n| n > 0) {
                rounds.push(0);
            }
            crate::counts::fix(&rounds);
        }
    }

    /// G3b.3: one round of a top-level fixpoint (where `fix_rounds` counts it) against the
    /// clause's algebraic budget (`rerail_facts::budget`); over it, the execution fails.
    fn spend_round(&self, outer: Option<&Frame<'_>>, rows: u64) {
        if outer.is_none() {
            if let Err(e) = rerail_facts::budget::round(rows) {
                self.fail(e);
            }
        }
    }

    fn query(&self, q: &PQuery, outer: Option<&Frame<'_>>) -> Table {
        for cte in &q.ctes {
            match cte {
                PCte::Plain { slot, q } => {
                    let t = self.query(q, outer);
                    self.slots.borrow_mut()[*slot] = Some(Rc::new(t));
                }
                PCte::Recursive { slot, batch, init, rec, distinct, tree, schema, real } => {
                    // W1: a bag recursion of the walk shape is checked against its forest bound
                    // before it runs. Where the bound does not hold the recursion runs as
                    // written, under the clause's algebraic budget (its kill switch), and the
                    // content is counted (`bounds_unproven`): never a different answer.
                    if let (false, Some(wb), [PBody::Select(s)]) = (*distinct, walk_bound(*batch, rec), &rec[..]) {
                        if self.check_walk(&wb, init, s, outer).is_err() {
                            self.ph.add(Phys::BoundsUnproven, 1);
                        }
                    }
                    let t = if !real.is_empty() {
                        self.recursive(*batch, init, rec, *distinct, schema, real, outer)
                    } else {
                        match self.closure(*batch, init, rec, *distinct, *tree, schema, outer) {
                            Some(t) => t,
                            None => match tree.then(|| self.walk(*batch, init, rec, *distinct, schema, outer)).flatten() {
                                Some(t) => t,
                                None => self.recursive(*batch, init, rec, *distinct, schema, real, outer),
                            },
                        }
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
            return real_read(Table { schema: q.out.clone(), ..t }.truncate(width), &q.real);
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
        real_read(Table { schema: q.out.clone(), ..t.truncate(width) }.select(idx), &q.real)
    }

    fn const_int(&self, e: &PE, outer: Option<&Frame<'_>>) -> i64 {
        let empty: [RowRef<'_>; 0] = [];
        let ctx = EvalCtx { frame: Frame { rows: &empty, parent: outer }, aggs: None, wins: None };
        match sv::cast(&self.eval(e, &ctx), Aff::Integer) {
            V::Int(i) => i,
            _ => -1,
        }
    }

    /// A body. `UNION` keeps one row per distinctness class, in first-seen position, and the row
    /// SQLite keeps for it (the decider's ruling on equal values of different storage classes,
    /// `2` and `2.0`): SQLite 3.53 (the native engine's) keeps the class's first row in the
    /// right-most arm holding it, so a binary `UNION` takes the right arm's first row of a class
    /// the right arm holds and the left side's (itself so chosen) otherwise; with or without the
    /// compound's ORDER BY (probed against the bundled SQLite in `key_cells_match_sqlite`).
    /// `DISTINCT` and a recursive `UNION` keep the first row, as SQLite does.
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
                        let nl = left.len() as u32;
                        let mut seen = Distinct::new(w);
                        let mut rep: Vec<u32> = Vec::new();
                        for r in 0..all.len() as u32 {
                            let s = seen.slot(&all, r as usize, rep.len() as u32) as usize;
                            if s == rep.len() {
                                rep.push(r);
                            } else if rep[s] < nl && r >= nl {
                                rep[s] = r;
                            }
                        }
                        if rep.len() == all.len() { all } else { all.select(rep.into()) }
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
    /// tuple interned once per seed, the node value once per seed and per edge row). The edge
    /// lookup reads the row's own node value under the probe's affinity, interned as a node key
    /// (two values of one distinctness class, `5` and `5.0`, key differently under TEXT
    /// affinity), and is made once per `(carried probe values, node key)` and cached.
    ///
    /// R18 (`tree`, `docs/facts/REWRITES.md`): when the walk is a forest, `tree_walk` emits the
    /// same rows in the same order without the distinctness check and without a lookup per row.
    /// `None` when the plan is not of the closure's shape.
    #[allow(clippy::too_many_arguments)]
    fn closure(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, tree: bool, schema: &Schema, outer: Option<&Frame<'_>>) -> Option<Table> {
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
        let pos_nj = from_w.iter().position(|&c| c == nj).expect("closure probe reads `node`");
        // R18 (not under `FACTS_REWRITES=0`; no filter on the edge row): the forest walk, when
        // its side conditions hold on this execution's rows.
        if tree && s.level[1].is_empty() {
            if let Some((seed_rows, all)) = self.tree_walk(s, &seeds, &e, &ix, probe, &from_w, (nj, dst, pos_nj), outer) {
                return Some(closure_table(schema, &seeds, &e, (nj, dst), &seed_rows, &all));
            }
        }
        let mut seen = Distinct::new(w);
        // The carried columns' tuples (every column but `node`), interned.
        let mut carried_ids: FxHashMap<Vec<u32>, u32> = FxHashMap::default();
        // The carried probe values (the probe columns read from `w` but `node`), interned: per
        // id its key parts, found by their combined hash.
        let mut probe_keys: Vec<Vec<KCell<'static>>> = Vec::new();
        let mut probe_heads: FxHashMap<u64, Vec<u32>> = FxHashMap::default();
        let mut rows_seen: rustc_hash::FxHashSet<u64> = Default::default();
        let node_aff = probe.expr_affs[pos_nj];
        // Node values as probe key parts (`compare_affinity` under the probe's affinity),
        // interned; `u32::MAX` is NULL.
        let mut node_keys = Interner::default();
        // Per admitted seed: its row, carried id, carried probe id (`u32::MAX`: a NULL part) and
        // node key id.
        let mut seed_rows: Vec<u32> = Vec::new();
        let mut seed_carried: Vec<u32> = Vec::new();
        let mut seed_probe: Vec<u32> = Vec::new();
        let mut seed_key: Vec<u32> = Vec::new();
        // Walk rows: (seed, node source: `u32::MAX` = the seed's own node, else an edge row).
        let mut level: Vec<(u32, u32)> = Vec::new();
        for r in 0..seeds.len() {
            let ids: Vec<u32> = (0..w).map(|c| seen.intern(c, seeds.cell(r, c))).collect();
            let carried: Vec<u32> = ids.iter().enumerate().filter(|&(c, _)| c != nj).map(|(_, &x)| x).collect();
            let n_c = carried_ids.len() as u32;
            let cid = *carried_ids.entry(carried).or_insert(n_c);
            if rows_seen.insert((cid as u64) << 32 | ids[nj] as u64) {
                let mut pk: Vec<KCell<'static>> = Vec::with_capacity(from_w.len() - 1);
                let mut null = false;
                for (k, (&c, a)) in from_w.iter().zip(&probe.expr_affs).enumerate() {
                    if k == pos_nj {
                        continue;
                    }
                    match cell_key(seeds.cell(r, c), *a) {
                        Some(x) => pk.push(x.owned()),
                        None => null = true,
                    }
                }
                let pid = if null {
                    u32::MAX
                } else {
                    let h = pk.iter().fold(0u64, |h, k| mix(h, k.hash()));
                    let cands = probe_heads.entry(h).or_default();
                    match cands.iter().find(|&&i| probe_keys[i as usize].iter().zip(&pk).all(|(a, b)| a.eq(b))) {
                        Some(&i) => i,
                        None => {
                            cands.push(probe_keys.len() as u32);
                            probe_keys.push(pk);
                            probe_keys.len() as u32 - 1
                        }
                    }
                };
                level.push((seed_rows.len() as u32, u32::MAX));
                seed_rows.push(r as u32);
                seed_carried.push(cid);
                seed_probe.push(pid);
                seed_key.push(cell_key(seeds.cell(r, nj), node_aff).map_or(u32::MAX, |k| node_keys.id(Some(k))));
            }
        }
        // Per edge row: its `dst` value's node id and node key id (`UNSET`: not yet interned; a
        // key `u32::MAX`: NULL).
        const UNSET: u32 = u32::MAX - 1;
        let mut dst_id: Vec<u32> = vec![UNSET; e.len()];
        let mut dst_key: Vec<u32> = vec![UNSET; e.len()];
        // Edge rows per (carried probe id, node key id), looked up once.
        let mut hits_of: FxHashMap<u64, Option<&[u32]>> = FxHashMap::default();
        // A seed expands when `w`'s filters hold on its (carried) values.
        let mut expands: Vec<Option<bool>> = vec![None; seed_rows.len()];
        let mut all: Vec<(u32, u32)> = Vec::new();
        let mut steps = 0usize;
        let mut rounds = vec![level.len() as u64];
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
                let nk = if src == u32::MAX {
                    seed_key[sr as usize]
                } else {
                    let k = &mut dst_key[src as usize];
                    if *k == UNSET {
                        *k = cell_key(e.cell(src as usize, dst), node_aff).map_or(u32::MAX, |c| node_keys.id(Some(c)));
                    }
                    *k
                };
                if nk == u32::MAX {
                    continue;
                }
                let hits = *hits_of.entry((pid as u64) << 32 | nk as u64).or_insert_with(|| {
                    let mut pk: Vec<KCell<'_>> = probe_keys[pid as usize].clone();
                    pk.insert(pos_nj, node_keys.keys[nk as usize].0.clone().expect("a non-NULL node key"));
                    self.ph.add(Phys::Probes, 1);
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
                    if *d == UNSET {
                        *d = seen.intern(nj, e.cell(er as usize, dst));
                    }
                    if rows_seen.insert((seed_carried[sr as usize] as u64) << 32 | *d as u64) {
                        next.push((sr, er));
                    }
                }
            }
            all.append(&mut level);
            rounds.push(next.len() as u64);
            level = next;
        }
        self.fix_rounds(outer, rounds);
        Some(closure_table(schema, &seeds, &e, (nj, dst), &seed_rows, &all))
    }

    /// R18, tree closure (`docs/facts/REWRITES.md`): the closure's admitted seed rows and walk
    /// rows when its walk is a forest, `None` (the closure runs as above) otherwise.
    ///
    /// Side conditions, checked on this execution's rows: (s) each carried tuple (every column
    /// but the node, by distinctness class) has one seed. On the edge rows in the probe's index
    /// (no NULL key part) whose key without the node part some seed's carried probe values
    /// select: (a) within one such carried probe group no two edge rows have the same `dst`
    /// (distinctness class), (b) no two edge rows' `dst` keys select the same group of children
    /// (each node has at most one parent), (c) the parent relation is acyclic (every row is
    /// reached from a group without a parent), and (d) no seed reaches an edge row whose `dst` is
    /// the seed's own node. Then every walk row of a seed is distinct, so the distinctness check
    /// never removes a row.
    ///
    /// Order: the closure emits level by level, each level's rows by the previous level's row,
    /// then by edge row. Within one seed that is the order of the edge-rank paths from the seed,
    /// which is the preorder of a depth-first numbering visiting children in edge-row order,
    /// restricted to one depth. So the edge rows are numbered once in that preorder (each
    /// subtree an interval), bucketed by depth, and level `j` of a seed is the slice of the
    /// bucket for its depth inside the seed's interval; seeds in their order within a level.
    /// Per seed this costs a few hash lookups; per walk row, a copy.
    #[allow(clippy::too_many_arguments)]
    fn tree_walk(&self, s: &PSelect, seeds: &Table, e: &Table, ix: &Index, probe: &Probe, from_w: &[usize], (nj, dst, pos_nj): (usize, usize, usize), outer: Option<&Frame<'_>>) -> Option<(Vec<u32>, Vec<(u32, u32)>)> {
        const NONE: u32 = u32::MAX;
        /// A value's distinctness key (`value::dkey` as a key cell), its hash, and equality.
        fn ident(c: Cell<'_>) -> Option<KCell<'_>> {
            cell_key(c, Aff::None)
        }
        fn ihash(k: &Option<KCell<'_>>) -> u64 {
            k.as_ref().map_or(0x5bd1_e995, KCell::hash)
        }
        fn same(a: &Option<KCell<'_>>, b: &Option<KCell<'_>>) -> bool {
            match (a, b) {
                (None, None) => true,
                (Some(x), Some(y)) => x.eq(y),
                _ => false,
            }
        }
        let w = seeds.cols.len();
        let node_aff = probe.expr_affs[pos_nj];
        let np = from_w.len();
        // Seeds, set semantics; (s).
        let mut rows: Vec<u32> = Vec::new();
        let mut carried_head: FxHashMap<u64, u32> = FxHashMap::default();
        let mut carried_next: Vec<u32> = Vec::new();
        'seeds: for r in 0..seeds.len() {
            let hc = (0..w).filter(|&c| c != nj).fold(0u64, |h, c| mix(h, ihash(&ident(seeds.cell(r, c)))));
            let mut i = carried_head.get(&hc).copied().unwrap_or(NONE);
            while i != NONE {
                let a = rows[i as usize] as usize;
                if (0..w).filter(|&c| c != nj).all(|c| same(&ident(seeds.cell(a, c)), &ident(seeds.cell(r, c)))) {
                    if same(&ident(seeds.cell(a, nj)), &ident(seeds.cell(r, nj))) {
                        continue 'seeds;
                    }
                    return None;
                }
                i = carried_next[i as usize];
            }
            carried_next.push(carried_head.insert(hc, rows.len() as u32).unwrap_or(NONE));
            rows.push(r as u32);
        }
        // Carried probe ids: per admitted seed, its probe key without the node part, interned.
        let mut probe_keys: Vec<Vec<KCell<'_>>> = Vec::new();
        let mut probe_heads: FxHashMap<u64, u32> = FxHashMap::default();
        let mut probe_next: Vec<u32> = Vec::new();
        let find = |pk: &[KCell<'_>], probe_keys: &[Vec<KCell<'_>>], probe_heads: &FxHashMap<u64, u32>, probe_next: &[u32]| {
            let h = pk.iter().fold(0u64, |h, k| mix(h, k.hash()));
            let mut i = probe_heads.get(&h).copied().unwrap_or(NONE);
            while i != NONE && !probe_keys[i as usize].iter().zip(pk).all(|(a, b)| a.eq(b)) {
                i = probe_next[i as usize];
            }
            (h, i)
        };
        let mut seed_pid: Vec<u32> = Vec::with_capacity(rows.len());
        let mut pk: Vec<KCell<'_>> = Vec::with_capacity(np);
        'pids: for &r in &rows {
            pk.clear();
            for (k, (&c, a)) in from_w.iter().zip(&probe.expr_affs).enumerate() {
                if k != pos_nj {
                    match cell_key(seeds.cell(r as usize, c), *a) {
                        Some(x) => pk.push(x),
                        None => {
                            seed_pid.push(NONE);
                            continue 'pids;
                        }
                    }
                }
            }
            let (h, mut i) = find(&pk, &probe_keys, &probe_heads, &probe_next);
            if i == NONE {
                i = probe_keys.len() as u32;
                probe_next.push(probe_heads.insert(h, i).unwrap_or(NONE));
                probe_keys.push(pk.clone());
            }
            seed_pid.push(i);
        }
        let ng = ix.groups.len();
        let rows_of = |g: u32| {
            let grp = &ix.groups[g as usize];
            &ix.rows[grp.start as usize..(grp.start + grp.len) as usize]
        };
        // Each group's carried probe id: its key without the node part, among the seeds'.
        let mut pid_of = vec![NONE; ng];
        let mut gk: Vec<KCell<'_>> = Vec::with_capacity(np);
        for (g, grp) in ix.groups.iter().enumerate() {
            gk.clear();
            for p in (0..np).filter(|&p| p != pos_nj) {
                gk.push(ix.part(grp.rep, p)?);
            }
            pid_of[g] = find(&gk, &probe_keys, &probe_heads, &probe_next).1;
        }
        // (a) and (b): per edge row its carried probe id and its children's group; per group its
        // parent row; per (carried probe id, `dst` class) its row.
        let mut group_parent = vec![NONE; ng];
        let mut child_group = vec![NONE; e.len()];
        let mut row_pid = vec![NONE; e.len()];
        let mut dst_head: FxHashMap<u64, u32> = FxHashMap::default();
        let mut dst_next = vec![NONE; e.len()];
        let find_dst = |pid: u32, id: &Option<KCell<'_>>, row_pid: &[u32], dst_head: &FxHashMap<u64, u32>, dst_next: &[u32]| {
            let h = mix(pid as u64, ihash(id));
            let mut i = dst_head.get(&h).copied().unwrap_or(NONE);
            while i != NONE && !(row_pid[i as usize] == pid && same(&ident(e.cell(i as usize, dst)), id)) {
                i = dst_next[i as usize];
            }
            (h, i)
        };
        let mut key: Vec<KCell<'_>> = Vec::with_capacity(np);
        for g in 0..ng as u32 {
            let pid = pid_of[g as usize];
            if pid == NONE {
                continue;
            }
            for &r in rows_of(g) {
                let c = e.cell(r as usize, dst);
                let id = ident(c.clone());
                let (h, twin) = find_dst(pid, &id, &row_pid, &dst_head, &dst_next);
                // Mutant 27 (R18, dropped side condition): (a) is not checked.
                if twin != NONE && !crate::mutant::on(27) {
                    return None;
                }
                row_pid[r as usize] = pid;
                dst_next[r as usize] = dst_head.insert(h, r).unwrap_or(NONE);
                let Some(d) = cell_key(c, node_aff) else { continue };
                key.clear();
                key.extend(probe_keys[pid as usize].iter().cloned());
                key.insert(pos_nj, d);
                if let Some(cg) = ix.group(&key) {
                    if group_parent[cg as usize] != NONE {
                        return None;
                    }
                    group_parent[cg as usize] = r;
                    child_group[r as usize] = cg;
                }
            }
        }
        // Preorder numbering from the groups without a parent; (c): every row numbered.
        let mut pre = vec![NONE; e.len()];
        let mut end = vec![0u32; e.len()];
        let mut order: Vec<u32> = Vec::new();
        let mut depth: Vec<u32> = Vec::new();
        let mut span = vec![(0u32, 0u32); ng];
        let mut stack: Vec<(u32, u32)> = Vec::new();
        for g in 0..ng as u32 {
            if pid_of[g as usize] == NONE || group_parent[g as usize] != NONE {
                continue;
            }
            let lo = order.len() as u32;
            for &r in rows_of(g) {
                pre[r as usize] = order.len() as u32;
                order.push(r);
                depth.push(0);
                stack.push((r, 0));
                while let Some(&(x, i)) = stack.last() {
                    let cg = child_group[x as usize];
                    let kids = if cg == NONE { &[][..] } else { rows_of(cg) };
                    if (i as usize) < kids.len() {
                        stack.last_mut().expect("non-empty").1 += 1;
                        let c = kids[i as usize];
                        if pre[c as usize] != NONE {
                            return None;
                        }
                        pre[c as usize] = order.len() as u32;
                        order.push(c);
                        depth.push(stack.len() as u32);
                        stack.push((c, 0));
                    } else {
                        end[x as usize] = order.len() as u32;
                        stack.pop();
                    }
                }
            }
            span[g as usize] = (lo, order.len() as u32);
        }
        if (0..ng as u32).any(|g| pid_of[g as usize] != NONE && rows_of(g).iter().any(|&r| pre[r as usize] == NONE)) {
            return None;
        }
        // Preorder positions bucketed by depth, ascending within a bucket.
        let max_depth = depth.iter().copied().max().map_or(0, |d| d as usize + 1);
        let mut bucket = vec![0u32; max_depth + 1];
        for &d in &depth {
            bucket[d as usize + 1] += 1;
        }
        for d in 1..bucket.len() {
            bucket[d] += bucket[d - 1];
        }
        let mut by_depth = vec![0u32; order.len()];
        let mut fill = bucket.clone();
        for (p, &d) in depth.iter().enumerate() {
            by_depth[fill[d as usize] as usize] = p as u32;
            fill[d as usize] += 1;
        }
        // Per seed: the interval of its descendants and their first depth.
        let mut all: Vec<(u32, u32)> = (0..rows.len() as u32).map(|sr| (sr, NONE)).collect();
        let mut active: Vec<(u32, u32, u32, u32)> = Vec::new();
        for (sr, &r) in rows.iter().enumerate() {
            if !s.level[0].is_empty() {
                let seed = [RowRef { t: Some(seeds), r }, UNBOUND];
                let ctx = EvalCtx { frame: Frame { rows: &seed, parent: outer }, aggs: None, wins: None };
                if !s.level[0].iter().all(|c| sv::truth(&self.eval(c, &ctx)) == Some(true)) {
                    continue;
                }
            }
            let pid = seed_pid[sr];
            if pid == NONE {
                continue;
            }
            let node = seeds.cell(r as usize, nj);
            let Some(nk) = cell_key(node.clone(), node_aff) else { continue };
            key.clear();
            key.extend(probe_keys[pid as usize].iter().cloned());
            key.insert(pos_nj, nk);
            self.ph.add(Phys::Probes, 1);
            let Some(g) = ix.group(&key) else { continue };
            let (lo, hi, d) = match group_parent[g as usize] {
                NONE => (span[g as usize].0, span[g as usize].1, 0),
                p => (pre[p as usize] + 1, end[p as usize], depth[pre[p as usize] as usize] + 1),
            };
            // (d)
            let twin = find_dst(pid, &ident(node), &row_pid, &dst_head, &dst_next).1;
            if twin != NONE && (lo..hi).contains(&pre[twin as usize]) {
                return None;
            }
            active.push((sr as u32, lo, hi, d));
        }
        // Level by level: per active seed its slice of the next depth's bucket.
        let mut rounds = vec![rows.len() as u64];
        while !active.is_empty() && all.len() <= 50_000_000 {
            let mut next = Vec::with_capacity(active.len());
            let before = all.len();
            for &(sr, lo, hi, d) in &active {
                if d as usize >= max_depth {
                    continue;
                }
                let b = &by_depth[bucket[d as usize] as usize..bucket[d as usize + 1] as usize];
                let (a, z) = (b.partition_point(|&p| p < lo), b.partition_point(|&p| p < hi));
                if a == z {
                    continue;
                }
                all.extend(b[a..z].iter().map(|&p| (sr, order[p as usize])));
                next.push((sr, lo, hi, d + 1));
            }
            rounds.push((all.len() - before) as u64);
            active = next;
        }
        if all.len() > 50_000_000 {
            self.fail("recursive CTE exceeded 50M rows".into());
        }
        self.fix_rounds(outer, rounds);
        #[cfg(test)]
        tests::TREE_WALKS.with(|c| c.set(c.get() + 1));
        Some((rows, all))
    }

    /// R19, bag recursion as a reference walk (`docs/facts/REWRITES.md`): the semi-naive
    /// evaluation of a recursion of `walk_shape`, level by level through the step's own join
    /// (`join`: the same probes, filters and candidate order as the select it is), but each
    /// output row stored as one (source, row) pair per column instead of evaluated values: an
    /// item's column is that item's bound row, a carried column copies the previous level's
    /// pair for that column, a literal is a constant source, and a `CASE` evaluates its
    /// conditions on the binding and takes its chosen result's pair. Every level and the result
    /// are `Column::Pick` columns over level 0's columns and the step's item columns, so no value
    /// is materialised (text node ids stay parts) and readers of the result read the sources.
    /// Rows and order are the generic fixpoint's: the same join per level over a level table
    /// holding the same values, the same 50M-row bound. `None` when the plan is not of the
    /// shape (or a level needs more than `PICK_NULL` sources).
    fn walk(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, schema: &Schema, outer: Option<&Frame<'_>>) -> Option<Table> {
        let w = schema.cols.len();
        let (init, s, refs, n_slots) = walk_shape(batch, init, rec, distinct, w)?;
        let t0 = self.body(init, outer);
        if self.failed() {
            return Some(Table::empty(schema.clone()));
        }
        let t0 = Table { schema: schema.clone(), ..t0 }.truncate(w);
        // Sources: level 0's columns, then the item columns (by column) and literals the step
        // reads.
        let mut srcs: Vec<Rc<Column>> = t0.cols.clone();
        let mut src_of: FxHashMap<usize, u16> = FxHashMap::default();
        let mut slot_src = vec![PICK_NULL; n_slots];
        let mut lits: Vec<Option<u16>> = vec![None; n_slots];
        let n0 = t0.len();
        let mut tag: Vec<Vec<u16>> = (0..w).map(|j| vec![j as u16; n0]).collect();
        let mut row: Vec<Vec<u32>> = (0..w).map(|_| (0..n0 as u32).collect()).collect();
        let (mut lo, mut hi, mut steps) = (0usize, n0, 0usize);
        let mut rounds = vec![n0 as u64];
        // The budget's rounds so far, restored if the walk gives up (the fixpoint runs again).
        let mark = rerail_facts::budget::live();
        self.spend_round(outer, n0 as u64);
        let mut slot_refs: Vec<&WalkRef<'_>> = Vec::with_capacity(n_slots);
        fn collect<'r, 'p>(r: &'r WalkRef<'p>, out: &mut Vec<&'r WalkRef<'p>>) {
            match r {
                WalkRef::Item { .. } | WalkRef::Lit { .. } => out.push(r),
                WalkRef::Case { whens, else_, .. } => {
                    whens.iter().for_each(|(_, t)| collect(t, out));
                    collect(else_, out);
                }
                WalkRef::Carry(_) | WalkRef::Null => {}
            }
        }
        refs.iter().for_each(|r| collect(r, &mut slot_refs));
        while lo < hi {
            if self.failed() {
                break;
            }
            steps += hi - lo;
            if steps > 50_000_000 {
                self.fail("recursive CTE exceeded 50M rows".into());
                break;
            }
            let snap: Rc<[Rc<Column>]> = srcs.as_slice().into();
            let lt = Rc::new(Table { schema: schema.clone(), cols: (0..w).map(|j| Rc::new(Column::Pick { srcs: snap.clone(), tag: tag[j][lo..hi].to_vec(), row: row[j][lo..hi].to_vec() })).collect(), len: hi - lo });
            self.slots.borrow_mut()[batch] = Some(lt.clone());
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&lt) as usize);
            let Some(tables) = self.item_tables(s, outer) else { break };
            for r in &slot_refs {
                match r {
                    WalkRef::Item { item, col, slot } => {
                        slot_src[*slot] = match tables[*item as usize].cols.get(*col as usize) {
                            Some(c) => *src_of.entry(Rc::as_ptr(c) as *const u8 as usize).or_insert_with(|| {
                                srcs.push(c.clone());
                                srcs.len() as u16 - 1
                            }),
                            None => PICK_NULL,
                        }
                    }
                    WalkRef::Lit { pe, slot } => {
                        slot_src[*slot] = *lits[*slot].get_or_insert_with(|| {
                            let ctx = EvalCtx { frame: Frame { rows: &[], parent: None }, aggs: None, wins: None };
                            srcs.push(Rc::new(Column::Const(self.eval(pe, &ctx))));
                            srcs.len() as u16 - 1
                        })
                    }
                    _ => {}
                }
            }
            if srcs.len() >= PICK_NULL as usize {
                self.slots.borrow_mut()[batch] = None;
                rerail_facts::budget::set_live(mark);
                return None;
            }
            let mut next_tag: Vec<Vec<u16>> = vec![Vec::new(); w];
            let mut next_row: Vec<Vec<u32>> = vec![Vec::new(); w];
            self.join(s, &tables, outer, &mut |f: &Frame<'_>| {
                for (j, r) in refs.iter().enumerate() {
                    let (t, x) = self.pick(r, f, &slot_src, lo, &tag, &row);
                    next_tag[j].push(t);
                    next_row[j].push(x);
                }
                ControlFlow::Continue(())
            });
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&lt) as usize);
            self.slots.borrow_mut()[batch] = None;
            for j in 0..w {
                tag[j].append(&mut next_tag[j]);
                row[j].append(&mut next_row[j]);
            }
            (lo, hi) = (hi, tag.first().map_or(hi, Vec::len));
            rounds.push((hi - lo) as u64);
            self.spend_round(outer, (hi - lo) as u64);
        }
        self.fix_rounds(outer, rounds);
        #[cfg(test)]
        tests::WALKS.with(|c| c.set(c.get() + 1));
        let snap: Rc<[Rc<Column>]> = srcs.as_slice().into();
        let len = tag.first().map_or(0, Vec::len);
        let cols = tag.into_iter().zip(row).map(|(tag, row)| Rc::new(Column::Pick { srcs: snap.clone(), tag, row })).collect();
        Some(Table { schema: schema.clone(), cols, len })
    }

    /// R19: a reference form's (source, row) for the binding `f`; `lo` is the previous level's
    /// first row in `tag`/`row`.
    fn pick(&self, r: &WalkRef<'_>, f: &Frame<'_>, slot_src: &[u16], lo: usize, tag: &[Vec<u16>], row: &[Vec<u32>]) -> (u16, u32) {
        match r {
            WalkRef::Item { item, slot, .. } => match f.rows[*item as usize].r {
                NULL_ROW => (PICK_NULL, 0),
                x => (slot_src[*slot], x),
            },
            WalkRef::Carry(c) => {
                let p = lo + f.rows[0].r as usize;
                (tag[*c as usize][p], row[*c as usize][p])
            }
            WalkRef::Null => (PICK_NULL, 0),
            WalkRef::Lit { slot, .. } => (slot_src[*slot], 0),
            WalkRef::Case { operand, aff, whens, else_ } => {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                let ov = operand.map(|o| self.eval(o, &ctx));
                for (wexp, t) in whens {
                    let hit = match &ov {
                        Some(o) => sv::compare(o, &self.eval(wexp, &ctx), *aff) == Some(Ordering::Equal),
                        None => sv::truth(&self.eval(wexp, &ctx)) == Some(true),
                    };
                    if hit {
                        return self.pick(t, f, slot_src, lo, tag, row);
                    }
                }
                self.pick(else_, f, slot_src, lo, tag, row)
            }
        }
    }

    /// W1 (`walk_bound`), checked on this execution's rows: the edge relation of a bag recursion
    /// of the walk shape is a forest over its walk key (acyclic, every node with at most one
    /// parent or every node with at most one child), every lookup after it finds at most one row,
    /// and, going down (a node may have several children), the initial rows' walk keys are
    /// distinct. `Err` names what does not hold: the recursion's bound is not proven on this
    /// content, which is refused.
    fn check_walk(&self, wb: &WalkBound, init: &[PBody], s: &PSelect, outer: Option<&Frame<'_>>) -> Result<(), String> {
        let keyof = |t: &Table, r: usize, cols: &[u16]| -> Option<Vec<DKey>> {
            let k: Vec<DKey> = cols.iter().map(|&c| sv::dkey(&t.value(r, c as usize))).collect();
            (!k.contains(&DKey::Null)).then_some(k)
        };
        let e = self.table_of(&s.items[1].src, outer);
        let mut ids: FxHashMap<Vec<DKey>, u32> = FxHashMap::default();
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for r in 0..e.len() {
            let (Some(a), Some(b)) = (keyof(&e, r, &wb.from), keyof(&e, r, &wb.to)) else { continue };
            let next = ids.len() as u32;
            let a = *ids.entry(a).or_insert(next);
            let next = ids.len() as u32;
            let b = *ids.entry(b).or_insert(next);
            edges.push((a, b));
        }
        let n = ids.len();
        let (mut out, mut inn) = (vec![0u32; n], vec![0u32; n]);
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b) in &edges {
            out[a as usize] += 1;
            inn[b as usize] += 1;
            adj[a as usize].push(b);
        }
        let up = out.iter().all(|&x| x <= 1);
        if !up && !inn.iter().all(|&x| x <= 1) {
            return Err("its edge relation is not a forest over the walk key (a node with two parents, and a node with two children)".into());
        }
        // Mutant 59 (W1): the acyclicity check is skipped.
        if !crate::mutant::on(59) {
            let mut deg = inn;
            let mut stack: Vec<u32> = (0..n as u32).filter(|&v| deg[v as usize] == 0).collect();
            let mut done = 0;
            while let Some(v) = stack.pop() {
                done += 1;
                for &w in &adj[v as usize] {
                    deg[w as usize] -= 1;
                    if deg[w as usize] == 0 {
                        stack.push(w);
                    }
                }
            }
            if done < n {
                return Err("its edge relation has a cycle over the walk key".into());
            }
        }
        for k in 2..s.items.len() {
            let Some(p) = s.probes[k].as_ref() else { continue };
            let t = self.table_of(&s.items[k].src, outer);
            let mut seen = rustc_hash::FxHashSet::default();
            if (0..t.len()).filter_map(|r| keyof(&t, r, &p.cols)).any(|kk| !seen.insert(kk)) {
                return Err(format!("lookup item {k} finds two rows for one key"));
            }
        }
        if !up {
            let mut seen = rustc_hash::FxHashSet::default();
            for arm in init {
                let t = self.body(arm, outer);
                if (0..t.len()).filter_map(|r| keyof(&t, r, &wb.key)).any(|kk| !seen.insert(kk)) {
                    return Err("two initial rows walk down from one key".into());
                }
            }
        }
        Ok(())
    }

    /// Semi-naive fixpoint, level by level. Each level's rows are views (gathers) over the arms'
    /// sources, so a closure over a relation stores row indexes, not values. Each arm's rows read
    /// the integers of the CTE's REAL columns (`real`) as reals.
    #[allow(clippy::too_many_arguments)]
    fn recursive(&self, batch: usize, init: &[PBody], rec: &[PBody], distinct: bool, schema: &Schema, real: &[u16], outer: Option<&Frame<'_>>) -> Table {
        let width = schema.cols.len();
        let mut seen = Distinct::new(width);
        let mut admit = |t: Table, queue: &mut Vec<Table>| {
            let t = real_read(Table { schema: schema.clone(), ..t }.truncate(width), real);
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
        let queued = |q: &[Table]| q.iter().map(|t| t.len() as u64).sum::<u64>();
        let mut rounds = vec![queued(&queue)];
        self.spend_round(outer, rounds[0]);
        let mut steps = 0usize;
        // M1: a shrinking text's longest value must shrink every round (counted where it does not).
        let shrink = if distinct { None } else { shrinking_text(batch, rec) };
        let (mut longest, mut shrink_ok): (Option<usize>, bool) = (None, true);
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
            if let (Some(c), true) = (shrink, shrink_ok) {
                let lens: Option<Vec<usize>> = (0..t.len()).map(|r| match t.value(r, c as usize) {
                    V::Text(s) => Some(s.len()),
                    V::Null => Some(0),
                    _ => None,
                }).collect();
                let m = lens.map(|l| l.into_iter().max().unwrap_or(0));
                if m.is_none() || longest.is_some_and(|p| m.unwrap() >= p) {
                    shrink_ok = false;
                    self.ph.add(Phys::BoundsUnproven, 1);
                }
                longest = m;
            }
            self.slots.borrow_mut()[batch] = Some(t.clone());
            // Indexes over a batch table are per batch.
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            for arm in rec {
                admit(self.body(arm, outer), &mut queue);
            }
            rounds.push(queued(&queue));
            self.spend_round(outer, *rounds.last().unwrap());
            self.indexes.borrow_mut().retain(|k, _| k.0 != Rc::as_ptr(&t) as usize);
            self.slots.borrow_mut()[batch] = None;
            levels.push(Rc::try_unwrap(t).unwrap_or_else(|rc| Table { schema: rc.schema.clone(), cols: rc.cols.clone(), len: rc.len }));
        }
        self.fix_rounds(outer, rounds);
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
            self.ph.add(Phys::IndexBuilds, 1);
            self.ph.add(Phys::IndexRows, t.len() as u64);
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

    /// R14's range index of `t` for range probe `r` (within `eq`'s groups), built once per
    /// statement; `None` when the column holds a NaN.
    fn range_index(&self, t: &Rc<Table>, r: &RangeProbe, eq: Option<&Index>) -> Option<Rc<RangeIndex>> {
        let k = (Rc::as_ptr(t) as usize, r as *const RangeProbe as usize);
        if let Some(ix) = self.range_indexes.borrow().get(&k) {
            return ix.clone();
        }
        let ix = RangeIndex::build(t, eq, r.col, r.col_aff).map(Rc::new);
        self.ph.add(Phys::RangeBuilds, 1);
        self.ph.add(Phys::RangeRows, t.len() as u64);
        // The key holds the table's address: keep the table alive while the statement runs.
        self.keep.borrow_mut().push(t.clone());
        self.range_indexes.borrow_mut().insert(k, ix.clone());
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
        let plain: Vec<Option<Rc<Index>>> = s.probes.iter().zip(tables).zip(&s.items).map(|((p, t), it)| p.as_ref().map(|p| self.index(t, p, matches!(it.src, Src::Global(_))))).collect();
        let range = s.ranges.iter().zip(tables).zip(&plain).map(|((r, t), eq)| r.as_ref().and_then(|r| self.range_index(t, r, eq.as_deref()))).collect();
        let indexes = Indexes {
            or: s.or_probes.iter().zip(tables).zip(&s.items).map(|((b, t), it)| b.as_ref().map(|b| b.iter().map(|p| self.index(t, p, matches!(it.src, Src::Global(_)))).collect())).collect(),
            plain,
            range,
        };
        let _ = self.level(s, 0, tables, &indexes, &mut rows, outer, sink);
    }

    /// The probe's key for the current binding; `None` when a key part is NULL (no row equals).
    fn probe_key<'s>(&'s self, p: &Probe, ctx: &'s EvalCtx<'_>) -> Option<KeyBuf<'s>> {
        if p.exprs.len() <= 4 {
            let mut a = [KCell::Int(0), KCell::Int(0), KCell::Int(0), KCell::Int(0)];
            for (j, (e, aff)) in p.exprs.iter().zip(&p.expr_affs).enumerate() {
                a[j] = self.key_of(e, *aff, ctx)?;
            }
            return Some(KeyBuf::Inline(p.exprs.len(), a));
        }
        p.exprs.iter().zip(&p.expr_affs).map(|(e, a)| self.key_of(e, *a, ctx)).collect::<Option<Vec<_>>>().map(KeyBuf::Heap)
    }

    /// `value::key(value::compare_affinity(e, aff))`, borrowed when `e` is a column or literal.
    fn key_of<'s>(&'s self, e: &PE, aff: Aff, ctx: &'s EvalCtx<'_>) -> Option<KCell<'s>> {
        match self.cell_of(e, ctx) {
            Some(c) => cell_key(c, aff),
            None => sv::key(&sv::compare_affinity(&self.eval(e, ctx), aff)).map(KCell::of),
        }
    }

    /// A column's or literal's value as a borrowed cell; `None` for any other expression.
    fn cell_of<'s>(&'s self, e: &PE, ctx: &'s EvalCtx<'_>) -> Option<Cell<'s>> {
        Some(match e {
            PE::Col { depth, item, col } => {
                let mut f = &ctx.frame;
                for _ in 0..*depth {
                    match f.parent {
                        Some(p) => f = p,
                        None => return Some(Cell::Null),
                    }
                }
                match f.rows.get(*item as usize) {
                    Some(RowRef { t: Some(t), r }) if *r != NULL_ROW => t.cols.get(*col as usize).map_or(Cell::Null, |c| c.cell(*r)),
                    _ => Cell::Null,
                }
            }
            PE::Lit(Lit::Null) => Cell::Null,
            PE::Lit(Lit::Int(i)) => Cell::Int(*i),
            PE::Lit(Lit::Real(f)) => Cell::Real(*f),
            PE::Lit(Lit::Text(i)) => vcell(self.texts[*i as usize].get_or_init(|| V::text(&self.stmt.texts[*i as usize]))),
            _ => return None,
        })
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
            self.ph.add(Phys::RowsJoined, 1);
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
        let range_cands: Vec<u32>;
        let cands: Option<&[u32]> = {
            let ctx = EvalCtx { frame: Frame { rows: rows.as_slice(), parent: outer }, aggs: None, wins: None };
            match (&s.or_probes[i], &indexes.or[i], &s.probes[i], &indexes.plain[i]) {
                // Mutant 55 (the historical OR-probe regression, G3b.2): the item is scanned and
                // the OR re-checked on every row (same rows, same order; quadratic work).
                (Some(branches), Some(ixs), _, _) if !crate::mutant::on(55) => {
                    let mut c: Vec<u32> = Vec::new();
                    for (p, ix) in branches.iter().zip(ixs) {
                        self.ph.add(Phys::Probes, 1);
                        if let Some(hits) = self.probe_key(p, &ctx).and_then(|k| ix.get(&k)) {
                            c.extend_from_slice(hits);
                        }
                    }
                    c.sort_unstable();
                    c.dedup();
                    or_cands = c;
                    Some(&or_cands)
                }
                (_, _, p, eq) if indexes.range[i].is_some() => {
                    // R14: the equality group (if any), then the rows within the bounds.
                    let (rp, rix) = (s.ranges[i].as_ref().expect("range probe"), indexes.range[i].as_ref().unwrap());
                    self.ph.add(Phys::Probes, 1 + (p.is_some() && eq.is_some()) as u64);
                    let group = match (p, eq) {
                        (Some(p), Some(ix)) => self.probe_key(p, &ctx).and_then(|k| ix.group(&k)),
                        _ => Some(0),
                    };
                    // `None`: a NULL bound (no row compares true); `Some(None)`: unbounded (absent,
                    // or a NaN, which the re-checked conjunct decides).
                    let bound = |b: &Option<(PE, Aff, bool)>| -> Option<Option<(V, bool)>> {
                        let Some((e, aff, strict)) = b else { return Some(None) };
                        let v = self.eval(e, &ctx);
                        if v.is_null() {
                            return None;
                        }
                        let k = sv::compare_affinity(&v, *aff);
                        Some((!matches!(k, V::Real(f) if f.is_nan())).then_some((k, *strict)))
                    };
                    match (group, bound(&rp.lo), bound(&rp.hi)) {
                        (Some(g), Some(lo), Some(hi)) => {
                            let (a, b, ascending) = rix.span(g as usize, lo.as_ref().map(|(v, st)| (v, *st)), hi.as_ref().map(|(v, st)| (v, *st)));
                            if ascending {
                                Some(&rix.rows[a..b])
                            } else {
                                let mut c = rix.rows[a..b].to_vec();
                                c.sort_unstable();
                                range_cands = c;
                                Some(&range_cands)
                            }
                        }
                        _ => Some(&[][..]),
                    }
                }
                (_, _, Some(p), Some(ix)) => {
                    self.ph.add(Phys::Probes, 1);
                    Some(self.probe_key(p, &ctx).and_then(|k| ix.get(&k)).map_or(&[][..], |h| &h[..]))
                }
                _ => None,
            }
        };
        let n = cands.map_or(t.len(), |c| c.len());
        self.ph.add(Phys::RowsScanned, n as u64);
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
            // Groups in first-seen order: key (interned ids of the group values' distinctness
            // keys) -> (agg states, representative binding, whether the one min()/max() took a
            // row of the group).
            let mut gids: Vec<Interner> = s.group_by.iter().map(|_| Interner::default()).collect();
            let mut groups: FxHashMap<Vec<u32>, usize> = FxHashMap::default();
            let mut states: Vec<(Vec<AggState>, Vec<Kept>, bool)> = Vec::new();
            self.join(s, &tables, outer, &mut |f| {
                let ctx = EvalCtx { frame: *f, aggs: None, wins: None };
                let key: Vec<u32> = s
                    .group_by
                    .iter()
                    .zip(gids.iter_mut())
                    .map(|(g, ids)| match self.cell_of(g, &ctx) {
                        Some(c) => ids.of(c),
                        None => ids.of(vcell_own(self.eval(g, &ctx))),
                    })
                    .collect();
                let gi = *groups.entry(key).or_insert_with(|| {
                    states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new(), false));
                    states.len() - 1
                });
                let (st, rep, picked) = &mut states[gi];
                let mut took = false;
                for (a, spec) in st.iter_mut().zip(&s.aggs) {
                    let v = spec.arg.as_ref().map(|e| self.eval(e, &ctx));
                    let sep = spec.sep.as_ref().map(|e| self.eval(e, &ctx));
                    took |= a.update(spec, v, sep);
                }
                // Bare columns (SQLite 3.53.2, the decider's ruling): with one min()/max() the
                // row it took last (each row while it took none: every argument NULL so far);
                // otherwise the last row, every bare column agreeing with the rows before.
                let take = if rep.is_empty() {
                    true
                } else if s.extremum {
                    took || !*picked
                } else {
                    if let Some(&(i, c)) = s.bare.iter().find(|&&(i, c)| !cells_identical(&row_cell(&rep[i as usize].row(), c), &row_cell(&f.rows[i as usize], c))) {
                        let (a, b) = (row_cell(&rep[i as usize].row(), c).to_v(), row_cell(&f.rows[i as usize], c).to_v());
                        self.fail(format!("bare column: column {c} of FROM item {i} takes two distinct values in one group ({a:?}, {b:?}), and SQLite reads an arbitrary row"));
                        return ControlFlow::Break(());
                    }
                    true
                };
                *picked |= took;
                if take {
                    rep.clear();
                    rep.extend(f.rows.iter().map(Kept::of));
                }
                ControlFlow::Continue(())
            });
            if self.failed() {
                return Table::empty(s.out.clone());
            }
            if s.group_by.is_empty() && states.is_empty() {
                states.push((s.aggs.iter().map(|_| AggState::default()).collect(), Vec::new(), false));
            }
            for (st, rep, _) in states {
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
                // Each binding's key cells (partition, then order), borrowed from the tables the
                // bindings name (held by `tables` or `keep` until the select ends). A key on a
                // column that holds one value on every row of an inner-joined item is equal for
                // every binding and is left out.
                let constant = |e: &PE| match e {
                    PE::Col { depth: 0, item, col } => {
                        let it = &s.items[*item as usize];
                        crate::lowered::rewrites() && nb > 0 && !it.left && !it.lateral && tables[*item as usize].cols.get(*col as usize).is_some_and(|c| c.konst().is_some())
                    }
                    _ => false,
                };
                let part: Vec<&PE> = w.partition.iter().filter(|e| !constant(e)).collect();
                let order: Vec<(&PE, bool)> = w.order.iter().filter(|(e, _)| !constant(e)).map(|(e, d)| (e, *d)).collect();
                let nk = part.len() + order.len();
                let mut kc: Vec<Cell<'_>> = Vec::with_capacity(nb * nk);
                for bi in 0..nb {
                    let refs = binding(bi);
                    let ctx = EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: None };
                    for e in part.iter().copied().chain(order.iter().map(|(e, _)| *e)) {
                        kc.push(match e {
                            PE::Col { depth: 0, item, col } if !s.items[*item as usize].lateral => {
                                let k = bound[bi * w_items + *item as usize];
                                let RowRef { t, r } = k.row();
                                match t {
                                    Some(t) if r != NULL_ROW => t.cols.get(*col as usize).map_or(Cell::Null, |c| c.cell(r)),
                                    _ => Cell::Null,
                                }
                            }
                            _ => vcell_own(self.eval(e, &ctx)),
                        });
                    }
                }
                let key = |bi: usize| &kc[bi * nk..(bi + 1) * nk];
                let np = part.len();
                let mut keyed: Vec<usize> = (0..nb).collect();
                keyed.sort_by(|&a, &b| {
                    let (ka, kb) = (key(a), key(b));
                    for j in 0..nk {
                        let o = cmp_cells(&ka[j], &kb[j]);
                        if o != Ordering::Equal {
                            return if j >= np && order[j - np].1 { o.reverse() } else { o };
                        }
                    }
                    a.cmp(&b)
                });
                let same = |a: usize, b: usize| key(a)[..np].iter().zip(&key(b)[..np]).all(|(x, y)| cmp_cells(x, y) == Ordering::Equal);
                if let Some((max, arg)) = &w.func {
                    // The aggregate over each partition (its rows are consecutive in `keyed`):
                    // NULLs skipped, NULL when none is left (as the aggregate `min`/`max`).
                    let mut start = 0;
                    while start < keyed.len() {
                        let mut end = start + 1;
                        while end < keyed.len() && same(keyed[start], keyed[end]) {
                            end += 1;
                        }
                        let mut best: Option<V> = None;
                        for bi in &keyed[start..end] {
                            let refs = binding(*bi);
                            let v = self.eval(arg, &EvalCtx { frame: Frame { rows: &refs, parent: outer }, aggs: None, wins: None });
                            if v.is_null() {
                                continue;
                            }
                            let better = best.as_ref().is_none_or(|b| {
                                let o = sv::cmp(&v, b);
                                if *max { o == Ordering::Greater } else { o == Ordering::Less }
                            });
                            if better {
                                best = Some(v);
                            }
                        }
                        let best = best.unwrap_or(V::Null);
                        for bi in &keyed[start..end] {
                            wins[*bi][wi] = best.clone();
                        }
                        start = end;
                    }
                    continue;
                }
                let mut n = 0i64;
                let mut prev: Option<usize> = None;
                for &bi in &keyed {
                    n = if prev.is_some_and(|q| same(q, bi)) { n + 1 } else { 1 };
                    prev = Some(bi);
                    wins[bi][wi] = V::Int(n);
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
        self.ph.add(Phys::RowsOut, t.len() as u64);
        if s.distinct {
            let mut seen = Distinct::new(s.proj.len());
            let keep: Vec<u32> = (0..t.len()).filter(|&r| seen.insert(&t, r)).map(|r| r as u32).collect();
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
        self.ph.add(Phys::SubqueryRuns, 1);
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
                let o = match (self.cell_of(l, ctx), self.cell_of(r, ctx)) {
                    (Some(a), Some(b)) => compare_cells(a, b, *aff),
                    _ => {
                        let lv = self.eval(l, ctx);
                        let rv = self.eval(r, ctx);
                        sv::compare(&lv, &rv, *aff)
                    }
                };
                match o {
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
                let held: V;
                let v = match self.cell_of(e, ctx) {
                    Some(c) => c,
                    None => {
                        held = self.eval(e, ctx);
                        vcell(&held)
                    }
                };
                if matches!(v, Cell::Null) {
                    return if list.is_empty() { V::bool(*not) } else { V::Null };
                }
                let mut null = false;
                for (x, aff) in list {
                    let xv: V;
                    let xc = match self.cell_of(x, ctx) {
                        Some(c) => c,
                        None => {
                            xv = self.eval(x, ctx);
                            vcell(&xv)
                        }
                    };
                    match compare_cells(v.clone(), xc, *aff) {
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
            PE::IsNull(x, not) => V::bool(self.cell_of(x, ctx).map_or_else(|| self.eval(x, ctx).is_null(), |c| matches!(c, Cell::Null)) != *not),
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
            Func::Printf => {
                let Some(fmt) = (!args.is_empty()).then(|| a(0)).and_then(|f| sv::to_text(&f)) else { return V::Null };
                let vals: Vec<V> = (1..args.len()).map(|i| a(i)).collect();
                match sv::printf(&fmt, &vals) {
                    Ok(s) => V::text(&s),
                    Err(e) => {
                        self.fail(e);
                        V::Null
                    }
                }
            }
            Func::JsonArray => {
                let mut s = String::from("[");
                for (i, x) in args.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    json_elem(&self.eval(x, ctx), json_subtype(x), &mut s);
                }
                s.push(']');
                V::text(&s)
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
    /// `json_group_array`'s elements so far, rendered.
    json: Option<String>,
}

/// Whether a JSON function's argument carries SQLite's JSON subtype: it is the direct result of
/// a JSON-producing function (`json_array`, `json_quote`). A `json_extract` result's subtype
/// depends on its value; it is refused at plan time where a JSON function reads it.
fn json_subtype(e: &PE) -> bool {
    matches!(e, PE::Func(Func::JsonArray | Func::JsonQuote, _))
}

/// One JSON array element as SQLite renders it: JSON-subtype text as is, NULL, an integer, a
/// real as SQLite's text of it, text quoted (`json_quote`).
fn json_elem(v: &V, subtype: bool, out: &mut String) {
    match v {
        V::Text(t) if subtype => out.push_str(t),
        V::Null => out.push_str("null"),
        V::Int(i) => out.push_str(&i.to_string()),
        V::Real(f) => out.push_str(&sv::real_to_text(*f)),
        V::Text(_) => {
            if let V::Text(q) = sv::json_quote(v) {
                out.push_str(&q);
            }
        }
    }
}

impl AggState {
    /// Folds one value; true when a min/max took this row.
    fn update(&mut self, spec: &AggSpec, v: Option<V>, sep: Option<V>) -> bool {
        if spec.f == AggFn::CountStar {
            self.count += 1;
            return false;
        }
        let v = v.unwrap_or(V::Null);
        if spec.f == AggFn::JsonGroupArray {
            // NULLs are elements too.
            if spec.distinct && !self.seen.get_or_insert_default().insert(sv::dkey(&v)) {
                return false;
            }
            let s = self.json.get_or_insert_with(String::new);
            if !s.is_empty() {
                s.push(',');
            }
            json_elem(&v, spec.arg.as_ref().is_some_and(json_subtype), s);
            return false;
        }
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
            // SQLite's sum() adds an integer, or a text reading as one, as an integer
            // (`sqlite3_value_numeric_type`); anything else makes the sum a REAL (the reference's
            // rule: `'12abc'` adds 12.0, `'abc'` 0.0).
            AggFn::Sum | AggFn::Total => match sv::compare_affinity(&v, Aff::Numeric) {
                V::Int(i) if self.sum_f.is_none() => match self.sum_i.unwrap_or(0).checked_add(i) {
                    Some(s) => self.sum_i = Some(s),
                    None => self.sum_f = Some(self.sum_i.unwrap_or(0) as f64 + i as f64),
                },
                _ => {
                    let x = match sv::to_numeric(&v) {
                        Some(V::Int(i)) => i as f64,
                        Some(V::Real(f)) => f,
                        _ => 0.0,
                    };
                    self.sum_f = Some(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64) + x);
                }
            },
            AggFn::CountStar | AggFn::JsonGroupArray => {}
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
            AggFn::JsonGroupArray => V::text(&format!("[{}]", self.json.unwrap_or_default())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(db: &Db<'_>, catalog: &Catalog, sql: &str) -> Result<Vec<Vec<V>>, String> {
        let stmt = plan(sql, catalog)?;
        let t = execute(db, &stmt)?;
        Ok((0..t.len()).map(|r| t.row(r)).collect())
    }

    thread_local! {
        /// Executions R18 ran (`Exec::tree_walk` returned rows).
        pub(super) static TREE_WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        /// Executions R19 ran (`Exec::walk` returned rows).
        pub(super) static WALKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Random graphs for the recursive-CTE rewrites: `nd(p, id, kind, par)` (nodes, `par` the
    /// parent's id) and `ed(p, src, dst, ord)` (edges), over one or two partitions `p`. Shapes:
    /// forests (most rounds), forests with extra edges (shared children, cycles, self loops),
    /// random graphs; duplicate rows, NULL ids, ids as integers, texts, numeric texts and reals,
    /// under random column affinities; rows in random order.
    /// `acyclic`: every edge goes from a lower to a higher node number (bag recursions end).
    fn graph_db(rnd: &mut dyn FnMut(u64) -> u64, acyclic: bool) -> (Catalog, Db<'static>, String) {
        let affs = [Aff::None, Aff::Integer, Aff::Text];
        let form = rnd(4);
        let val = |rnd: &mut dyn FnMut(u64) -> u64, i: u64| match form {
            0 => V::Int(i as i64),
            1 => V::text(&format!("n{i}")),
            2 => V::text(&format!("k:{i}:{}", i + 1)),
            _ => match rnd(4) {
                0 => V::text(&i.to_string()),
                1 => V::Real(i as f64),
                _ => V::Int(i as i64),
            },
        };
        let shape = rnd(6);
        let mut nd: Vec<Box<[V]>> = Vec::new();
        let mut ed: Vec<Box<[V]>> = Vec::new();
        for p in if rnd(3) == 0 { &["f", "g"][..] } else { &["f"][..] } {
            let n = 1 + rnd(14);
            let mut edges: Vec<(u64, u64)> = Vec::new();
            let mut parent = vec![None; n as usize];
            if shape < 5 {
                for i in 1..n {
                    if rnd(5) != 0 {
                        let q = rnd(i);
                        parent[i as usize] = Some(q);
                        edges.push((q, i));
                    }
                }
            }
            if shape >= 3 {
                for _ in 0..1 + rnd(4) {
                    let (a, b) = (rnd(n), rnd(n));
                    if !acyclic || a < b {
                        edges.push((a, b));
                    }
                }
            }
            for (a, b) in edges.clone() {
                if rnd(8) == 0 {
                    edges.push((a, b));
                }
            }
            // Random order.
            for i in (1..edges.len()).rev() {
                edges.swap(i, rnd(i as u64 + 1) as usize);
            }
            for (a, b) in edges {
                let (src, dst) = (if rnd(25) == 0 { V::Null } else { val(rnd, a) }, if rnd(25) == 0 { V::Null } else { val(rnd, b) });
                ed.push(vec![V::text(p), src, dst, V::Int(rnd(3) as i64)].into_boxed_slice());
            }
            let mut order: Vec<u64> = (0..n).collect();
            if rnd(2) == 0 {
                for i in (1..order.len()).rev() {
                    order.swap(i, rnd(i as u64 + 1) as usize);
                }
            }
            for i in order {
                let par = parent[i as usize].map_or(V::Null, |q| val(rnd, q));
                let row = vec![V::text(p), if rnd(30) == 0 { V::Null } else { val(rnd, i) }, V::text(["a", "b", "c"][rnd(3) as usize]), par].into_boxed_slice();
                if rnd(10) == 0 {
                    nd.push(row.clone());
                }
                nd.push(row);
            }
        }
        let mut catalog = Catalog::new();
        let mut db = Db::new("", "");
        let mut desc = format!("form {form} shape {shape}; ");
        for (name, cols, rows) in [("nd", ["p", "id", "kind", "par"], nd), ("ed", ["p", "src", "dst", "ord"], ed)] {
            let a: Vec<Aff> = cols.iter().enumerate().map(|(i, _)| if i == 0 || i == 2 && name == "nd" { Aff::Text } else if i == 3 && name == "ed" { Aff::Integer } else { affs[rnd(3) as usize] }).collect();
            let rows: Vec<Box<[V]>> = rows.into_iter().map(|r| r.iter().zip(&a).map(|(v, a)| sv::apply_affinity(v.clone(), *a)).collect()).collect();
            desc.push_str(&format!("{name} {a:?} {:?}; ", rows.iter().map(|r| r.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>()));
            let schema = Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: a };
            catalog.insert(name.into(), schema.clone());
            db.tables.insert(name.into(), Rc::new(Table::from_rows(schema, rows)));
        }
        (catalog, db, desc)
    }

    /// R18 (tree closure): every closure returns the same rows in the same order with the
    /// rewrite as without (the closure operator's distinctness walk), over `graph_db`'s forests,
    /// DAGs, cycles, duplicate and NULL rows and mixed-type ids: seeds one per carried tuple or
    /// several (fallback), with a seed filter, with an edge filter (no R18), with and without a
    /// partition key. Kills mutant 27.
    #[test]
    fn rewrite_tree_closure_equals_closure() {
        let mut seed = 0x51f1_5ee3_a17b_9c4du64;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let queries = [
            "WITH RECURSIVE walk(p, root, node) AS (SELECT p, id, id FROM nd UNION SELECT w.p, w.root, e.dst FROM walk w JOIN ed e ON e.p = w.p AND e.src = w.node) SELECT * FROM walk",
            "WITH RECURSIVE walk(p, root, node, x) AS (SELECT p, id, id, kind <> 'a' FROM nd UNION SELECT w.p, w.root, e.dst, w.x FROM walk w JOIN ed e ON e.p = w.p AND e.src = w.node WHERE w.x) SELECT p, root, node FROM walk",
            "WITH RECURSIVE walk(p, node) AS (SELECT p, id FROM nd WHERE kind = 'a' UNION SELECT w.p, e.dst FROM walk w JOIN ed e ON e.p = w.p AND e.src = w.node) SELECT * FROM walk",
            "WITH RECURSIVE walk(root, node) AS (SELECT id, id FROM nd UNION SELECT w.root, e.dst FROM walk w JOIN ed e ON e.src = w.node) SELECT * FROM walk",
            "WITH RECURSIVE walk(root, node) AS (SELECT id, id FROM nd UNION SELECT w.root, e.dst FROM walk w JOIN ed e ON e.src = w.node AND e.ord > 0) SELECT * FROM walk",
            "WITH RECURSIVE walk(p, root, node) AS (SELECT p, par, par FROM nd WHERE kind <> 'c' UNION SELECT w.p, w.root, e.dst FROM walk w JOIN ed e ON e.p = w.p AND e.src = w.node) SELECT * FROM walk",
        ];
        let before = TREE_WALKS.with(|c| c.get());
        for round in 0..400 {
            let (catalog, db, desc) = graph_db(&mut rnd, false);
            for q in queries {
                let rows = |st: &Statement| {
                    let t = execute(&db, st).unwrap();
                    format!("{:?}", (0..t.len()).map(|r| t.row(r)).collect::<Vec<_>>())
                };
                let (with, without) = (plan_with(q, &catalog, true).unwrap(), plan_with(q, &catalog, false).unwrap());
                assert_eq!(rows(&with), rows(&without), "round {round}: {q}; {desc}");
            }
        }
        let fired = TREE_WALKS.with(|c| c.get()) - before;
        assert!(fired >= 400, "R18 ran {fired} times");
    }

    /// R19 (bag recursion as a reference walk): every bag recursion returns the same rows in the
    /// same order with the rewrite as without (the generic level-by-level fixpoint), over
    /// `graph_db`'s acyclic graphs (forests, shared children, duplicate edges, NULL and mixed-type
    /// ids): the ownership shape (searched `CASE` over the child and the carried columns), plain
    /// carries, a simple `CASE` mixing literals, NULL and an item column, an `ON` filter, a LEFT
    /// JOIN's null-extended item, and a step column that is not a reference form (no R19).
    /// Kills mutant 29.
    #[test]
    fn rewrite_walk_equals_levels() {
        let mut seed = 0x2d35_8dcc_aa6c_78a5u64;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let queries = [
            "WITH RECURSIVE owned(p, id, fn, cls, top) AS (SELECT n.p, n.id, CASE WHEN n.kind = 'a' THEN n.id END, CASE WHEN n.kind = 'b' THEN n.id END, CASE WHEN n.kind = 'a' THEN n.id END FROM nd n LEFT JOIN nd q ON q.p = n.p AND q.id = n.par WHERE q.id IS NULL UNION ALL SELECT c.p, c.id, CASE WHEN c.kind = 'a' THEN c.id ELSE o.fn END, CASE WHEN c.kind = 'b' THEN c.id ELSE o.cls END, CASE WHEN o.top IS NULL AND c.kind = 'a' THEN c.id ELSE o.top END FROM owned o JOIN ed e ON e.p = o.p AND e.src = o.id JOIN nd c ON c.p = e.p AND c.id = e.dst) SELECT * FROM owned",
            "WITH RECURSIVE r(p, root, node) AS (SELECT p, id, id FROM nd UNION ALL SELECT r.p, r.root, e.dst FROM r JOIN ed e ON e.p = r.p AND e.src = r.node) SELECT * FROM r",
            "WITH RECURSIVE r(p, node, k) AS (SELECT p, id, kind FROM nd WHERE kind <> 'c' UNION ALL SELECT r.p, e.dst, CASE r.k WHEN 'a' THEN 'x' WHEN 'b' THEN NULL ELSE e.ord END FROM r JOIN ed e ON e.p = r.p AND e.src = r.node AND e.ord > 0) SELECT * FROM r",
            "WITH RECURSIVE r(node, kid, kind) AS (SELECT id, NULL, kind FROM nd UNION ALL SELECT e.dst, c.id, c.kind FROM r JOIN ed e ON e.src = r.node LEFT JOIN nd c ON c.id = e.dst AND c.kind = 'a') SELECT * FROM r",
            "WITH RECURSIVE r(p, node, d) AS (SELECT p, id, 0 FROM nd UNION ALL SELECT r.p, e.dst, r.d + 1 FROM r JOIN ed e ON e.p = r.p AND e.src = r.node WHERE r.d < 3) SELECT * FROM r",
        ];
        let before = WALKS.with(|c| c.get());
        let rounds = 300;
        for round in 0..rounds {
            let (catalog, db, desc) = graph_db(&mut rnd, true);
            for q in queries {
                let rows = |st: &Statement| {
                    let t = execute(&db, st).unwrap();
                    format!("{:?}", (0..t.len()).map(|r| t.row(r)).collect::<Vec<_>>())
                };
                let (with, without) = (plan_with(q, &catalog, true).unwrap(), plan_with(q, &catalog, false).unwrap());
                assert_eq!(rows(&with), rows(&without), "round {round}: {q}; {desc}");
            }
        }
        let fired = WALKS.with(|c| c.get()) - before;
        assert!(fired >= 4 * rounds, "R19 ran {fired} times");
    }

    /// W1 (`walk_bound`, `Exec::check_walk`): a bag recursion of the walk shape over an edge
    /// relation that is a forest is admitted (bound `n·d`); over a 3-cycle, where every node has
    /// one parent and one child, the check must fail (the walk never ends), and so must it over
    /// two parents and two children. Kills mutant 59 (the acyclicity check skipped).
    #[test]
    fn w1_walk_check_refuses_cycles() {
        let q = "WITH RECURSIVE r(p, node) AS (SELECT p, id FROM nd UNION ALL SELECT r.p, e.dst FROM r JOIN ed e ON e.p = r.p AND e.src = r.node) SELECT * FROM r";
        let check = |edges: &[(i64, i64)]| {
            let mut catalog = Catalog::new();
            let mut db = Db::new("", "");
            let nd: Vec<Box<[V]>> = vec![vec![V::text("f"), V::Int(1)].into_boxed_slice()];
            let ed: Vec<Box<[V]>> = edges.iter().map(|&(a, b)| vec![V::text("f"), V::Int(a), V::Int(b)].into_boxed_slice()).collect();
            for (name, cols, rows) in [("nd", vec!["p", "id"], nd), ("ed", vec!["p", "src", "dst"], ed)] {
                let schema = Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: vec![Aff::None; cols.len()] };
                catalog.insert(name.into(), schema.clone());
                db.tables.insert(name.into(), Rc::new(Table::from_rows(schema, rows)));
            }
            let st = plan(q, &catalog).unwrap();
            let Some(PCte::Recursive { init, rec, batch, .. }) = st.root.ctes.first() else { panic!("a recursive CTE") };
            let wb = walk_bound(*batch, rec).expect("the walk shape");
            let [PBody::Select(s)] = &rec[..] else { panic!("one step") };
            Exec::new(&db, &st).check_walk(&wb, init, s, None)
        };
        assert_eq!(check(&[(1, 2), (2, 3), (1, 4)]), Ok(()));
        assert!(check(&[(1, 2), (2, 3), (3, 1)]).is_err_and(|e| e.contains("cycle")), "a 3-cycle is not a forest");
        assert!(check(&[(1, 2), (3, 2), (2, 4), (2, 5)]).is_err());
    }

    /// A FROM subquery's outer references resolve and evaluate as in SQLite: past the enclosing
    /// select (whose FROM siblings it cannot see) to that select's outer rows, at every nesting
    /// depth. Expected rows are SQLite 3.51's for the same statements and tables. Kills mutant 22.
    #[test]
    fn derived_table_correlation_follows_sqlite_scoping() {
        let mut catalog = Catalog::new();
        let mut db = Db::new("", "");
        let schema = |cols: &[&str]| Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: vec![Aff::None; cols.len()] };
        let (t, i) = (V::text, V::Int);
        for (name, s, rows) in [
            ("d", schema(&["id"]), vec![vec![t("a")], vec![t("b")], vec![t("c")]]),
            ("p", schema(&["sym", "name", "pos"]), vec![vec![t("a"), t("y"), i(2)], vec![t("a"), t("x"), i(1)], vec![t("b"), t("z"), i(1)]]),
        ] {
            catalog.insert(name.into(), s.clone());
            db.tables.insert(name.into(), Rc::new(Table::from_rows(s, rows.into_iter().map(Vec::into_boxed_slice).collect())));
        }
        // `V` has no `PartialEq` (SQL equality is not structural); rows compare by their Debug text.
        let per_id = |sql: &str| format!("{:?}", run(&db, &catalog, sql).unwrap());
        let expect = |a: V, b: V, c: V| format!("{:?}", [[t("a"), a], [t("b"), b], [t("c"), c]]);
        // The traits stage's shape (public_api_triples): a scalar subquery over a derived table
        // correlated to the outermost row.
        assert_eq!(
            per_id("SELECT d.id, (SELECT json_group_array(name) FROM (SELECT name FROM p WHERE p.sym=d.id ORDER BY p.pos)) FROM d ORDER BY d.id"),
            expect(t(r#"["x","y"]"#), t(r#"["z"]"#), t("[]")),
        );
        // The derived table as a later FROM item: earlier siblings do not shift its depth.
        assert_eq!(
            per_id("SELECT d.id, (SELECT count(*) FROM d AS s, (SELECT name FROM p WHERE p.sym = d.id) AS q WHERE s.id <> d.id) FROM d ORDER BY d.id"),
            expect(i(4), i(2), i(0)),
        );
        // An unqualified name skips the invisible sibling `s` and binds the outer row.
        assert_eq!(
            per_id("SELECT d.id, (SELECT count(*) FROM d AS s, (SELECT name FROM p WHERE p.sym = id) AS q) FROM d ORDER BY d.id"),
            expect(i(6), i(3), i(0)),
        );
        // Derived tables nested two deep.
        assert_eq!(
            per_id("SELECT d.id, (SELECT json_group_array(n) FROM (SELECT n FROM (SELECT p.name AS n, p.pos AS k FROM p WHERE p.sym = d.id) ORDER BY k)) FROM d ORDER BY d.id"),
            expect(t(r#"["x","y"]"#), t(r#"["z"]"#), t("[]")),
        );
        // A FROM sibling is not in scope (SQLite: `no such column: s.id`).
        assert!(plan("SELECT (SELECT count(*) FROM d AS s, (SELECT 1 FROM p WHERE p.sym = s.id)) FROM d", &catalog).is_err());
    }

    fn ranges_in(q: &PQuery) -> usize {
        fn body(b: &PBody) -> usize {
            match b {
                PBody::Select(s) => s.ranges.iter().flatten().count(),
                PBody::Compound(l, _, r, _) => body(l) + body(r),
            }
        }
        let ctes: usize = q.ctes.iter().map(|c| match c {
            PCte::Plain { q, .. } => ranges_in(q),
            PCte::Recursive { init, rec, .. } => init.iter().chain(rec).map(body).sum(),
        }).sum();
        ctes + body(&q.body)
    }

    /// R14 (range probe): every statement returns the same rows in the same order with range
    /// probes as without (the filtered scan), over random tables mixing integers, integral and
    /// fractional reals, numeric and non-numeric texts and NULLs under every column affinity;
    /// bounds of both directions, strict and not, one- and two-sided, under an equality group, a
    /// LEFT JOIN's ON and in correlated subqueries. Kills mutant 23.
    #[test]
    fn rewrite_range_probe_equals_scan() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let affs = [Aff::None, Aff::Integer, Aff::Text, Aff::Real, Aff::Numeric];
        let queries = [
            "SELECT a.k, a.lo, b.v FROM a JOIN b ON b.k = a.k AND b.v >= a.lo AND b.v <= a.hi",
            "SELECT a.k, b.v FROM a JOIN b ON b.v > a.lo AND b.v < a.hi",
            "SELECT a.k, b.v FROM a, b WHERE a.lo <= b.v AND a.hi > b.v AND b.k = a.k",
            "SELECT a.k, b.v FROM a JOIN b ON b.v >= a.lo",
            "SELECT a.k, b.v FROM a JOIN b ON a.hi < b.v",
            "SELECT a.k, b.v FROM a LEFT JOIN b ON b.k = a.k AND b.v >= a.lo AND b.v < a.hi",
            "SELECT a.k, (SELECT count(*) FROM b WHERE b.v >= a.lo AND b.v <= a.hi), EXISTS(SELECT 1 FROM b WHERE b.k = a.k AND b.v > a.hi) FROM a",
            "SELECT a.k, b.v, c.v FROM a JOIN b ON b.v >= a.lo JOIN c ON c.v <= b.v AND c.v >= a.lo",
        ];
        let mut fired = 0;
        for round in 0..60 {
            let mut catalog = Catalog::new();
            let mut db = Db::new("", "");
            let value = |rnd: &mut dyn FnMut(u64) -> u64| match rnd(9) {
                0 => V::Null,
                1 | 2 | 3 => V::Int(rnd(12) as i64 - 3),
                4 => V::Real(rnd(12) as f64 - 3.0),
                5 => V::Real((rnd(24) as f64 - 6.0) / 2.0),
                6 => V::text(&(rnd(12) as i64 - 3).to_string()),
                7 => V::text(&format!("{}.5", rnd(9))),
                _ => V::text(["a", "b", "", "x1", "1e1"][rnd(5) as usize]),
            };
            for (name, cols) in [("a", &["k", "lo", "hi"][..]), ("b", &["k", "v"][..]), ("c", &["k", "v"][..])] {
                let schema = Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: cols.iter().map(|_| affs[rnd(5) as usize]).collect() };
                let rows: Vec<Box<[V]>> = (0..rnd(14)).map(|_| cols.iter().map(|_| value(&mut rnd)).collect()).collect();
                catalog.insert(name.into(), schema.clone());
                db.tables.insert(name.into(), Rc::new(Table::from_rows(schema, rows)));
            }
            for q in queries {
                let with = plan_with(q, &catalog, true).unwrap();
                let without = plan_with(q, &catalog, false).unwrap();
                assert_eq!(ranges_in(&without.root) + without.subs.iter().map(|s| ranges_in(&s.q)).sum::<usize>(), 0);
                fired += ranges_in(&with.root) + with.subs.iter().map(|s| ranges_in(&s.q)).sum::<usize>();
                let rows = |st: &Statement| {
                    let t = execute(&db, st).unwrap();
                    format!("{:?}", (0..t.len()).map(|r| t.row(r)).collect::<Vec<_>>())
                };
                assert_eq!(rows(&with), rows(&without), "round {round}: {q}");
            }
        }
        assert!(fired >= 60 * queries.len(), "range probes planned {fired}");
    }

    /// Borrowed cells and key cells (`Column::cell`, `cell_key`, `Index`, `compare_cells`):
    /// equality joins, constant key columns, IN lists, comparisons and correlated probes return
    /// SQLite's rows. Random tables mix integers, integral and fractional reals, numeric and
    /// non-numeric texts and NULLs under every declared affinity (none declared: BLOB), stored in
    /// each representation the engine reads (values, CTAS gathers, symbols, integer fact columns,
    /// source spans, constants, chains). Rows compare with their storage classes, so a `UNION`
    /// or `DISTINCT` keeping another of two equal values (`2`, `2.0`) than SQLite fails. Covers
    /// the decider's two SQLite rulings (BLOB affinity of untyped columns, UNION representative).
    #[test]
    fn key_cells_match_sqlite() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let affs = [(Aff::None, ""), (Aff::Integer, "INT"), (Aff::Text, "TEXT"), (Aff::Real, "REAL"), (Aff::Numeric, "NUMERIC")];
        let queries = [
            "SELECT a.k, b.k FROM a JOIN b ON b.k = a.k",
            "SELECT a.k, b.v FROM a JOIN b ON b.p = a.p AND b.k = a.v",
            "SELECT a.k, b.v FROM a JOIN b ON b.p = a.p",
            "SELECT a.k, b.k FROM a, b WHERE a.v = b.v AND a.k <> b.k",
            "SELECT a.k, b.k FROM a LEFT JOIN b ON b.k = a.v",
            "SELECT a.k, b.k FROM a JOIN b ON b.k = CAST(a.v AS TEXT)",
            "SELECT a.k, b.k FROM a JOIN b ON b.v = a.k + 0",
            "SELECT a.k, b.k FROM a JOIN b ON a.k < b.v",
            "SELECT a.k FROM a WHERE a.k IN ('1', 1, 1.5, 'a', '', a.v)",
            "SELECT a.k FROM a WHERE a.v NOT IN ('x1', 2, 2.0)",
            "SELECT a.k, a.v FROM a WHERE a.v = '1' OR a.v = 1 OR a.v > 'a' OR a.k >= 2.5",
            "SELECT a.k, EXISTS(SELECT 1 FROM b WHERE b.k = a.k AND b.p = a.p) FROM a",
            "SELECT a.k, (SELECT count(*) FROM b WHERE b.v = a.k) FROM a",
            "SELECT b.k, b.p FROM b WHERE b.p = 'p' OR b.p = 7",
            // R16 node ids: projected, joined to node ids and to plain text, deduplicated.
            "WITH t AS (SELECT kd || ':' || s || ':' || e AS id, kd FROM c) SELECT t.id, u.id FROM t JOIN t AS u ON u.id = t.id AND u.kd <> 'a'",
            "WITH t AS (SELECT kd || ':' || s || ':' || e AS id FROM c) SELECT id, count(*) FROM t GROUP BY id",
            "SELECT DISTINCT kd || ':' || s || ':' || e FROM c",
            "WITH t AS (SELECT kd || ':' || s || ':' || e AS id FROM c), u AS (SELECT kd || ':' || e || ':' || s AS id FROM c) SELECT t.id FROM t JOIN u ON u.id = t.id",
            "SELECT kd || '/' || s || '/' || e, kd || ':' || s || '' || e FROM c",
            "WITH t AS (SELECT kd || ':' || s || ':' || e AS id FROM c UNION ALL SELECT kd || ':' || e || ':' || s FROM c WHERE s > 0), u AS (SELECT CAST(t.id AS TEXT) AS id FROM t) SELECT t.id, u.id FROM t LEFT JOIN u ON u.id = t.id",
            "WITH t AS (SELECT k || ':' || v || ':' || p AS id, k FROM a), u AS (SELECT k || ':' || v || ':' || p AS id, v FROM b) SELECT t.id, u.id, t.id = u.id, t.id < u.id FROM t JOIN u ON u.id = t.id OR u.v = t.k",
            "WITH t AS (SELECT k || ':' || v || ':' || p AS id FROM a) SELECT t.id, b.k FROM t JOIN b ON b.k = t.id",
            "WITH t AS (SELECT k || ':' || v || ':' || p AS id FROM a) SELECT t.id, b.k FROM b JOIN t ON t.id = b.k || ''",
            "WITH RECURSIVE w(id, n) AS (SELECT k || ':' || v || ':' || p, 0 FROM a UNION SELECT id, n + 1 FROM w WHERE n < 2) SELECT id, n FROM w",
            "SELECT DISTINCT k || ':' || v || ':' || p FROM a",
            // UNION representative (ruling 2): equal values of different storage classes (`2`,
            // `2.0`) within and across columns of different affinities, with and without the
            // compound's ORDER BY, after UNION ALL. Rulings (c) (bare columns) and (d) (compound
            // column types) have their own differentials (`bare_columns_match_sqlite`,
            // `compound_types_match_sqlite`); the compounds here are top-level.
            "SELECT v FROM a UNION SELECT v FROM a WHERE k IS NOT NULL",
            "SELECT v FROM a UNION SELECT v FROM b",
            "SELECT k, v FROM a UNION SELECT v, k FROM b UNION SELECT p, v FROM a",
            "SELECT v FROM a UNION SELECT k FROM b ORDER BY 1",
            "SELECT v FROM a UNION ALL SELECT k FROM b UNION SELECT v FROM b",
            "SELECT k FROM b UNION ALL SELECT v FROM a UNION SELECT k FROM b ORDER BY 1 DESC",
            // Columns without a declared type (BLOB affinity, ruling 1) compared with typed
            // columns, with expressions of no affinity, through a CTE, IN (SELECT), CASE and CAST.
            "WITH t AS (SELECT v, v || '' AS s, k + 0 AS n FROM a) SELECT t.v, b.v FROM t JOIN b ON b.v = t.s",
            "WITH t AS (SELECT v FROM a) SELECT t.v, b.k FROM t JOIN b ON b.k = t.v",
            "SELECT a.k FROM a WHERE a.v IN (SELECT k FROM b)",
            "SELECT a.k, b.k, CASE a.v WHEN b.k THEN 1 ELSE 0 END FROM a, b",
            "SELECT a.k, b.k FROM a, b WHERE CAST(a.v AS TEXT) = b.v",
            "SELECT a.k, b.k FROM a, b WHERE a.v = b.v || ''",
            "SELECT DISTINCT a.k, a.p FROM a",
            "SELECT a.v, count(*), min(a.k) FROM a GROUP BY a.v",
            "SELECT k || ':' || v || ':' || p, count(*) FROM a GROUP BY k || ':' || v || ':' || p",
            "SELECT a.k, row_number() OVER (PARTITION BY a.p, a.v ORDER BY a.k) FROM a",
            "SELECT a.k, a.v, row_number() OVER (PARTITION BY a.p ORDER BY a.p, a.v DESC, a.k) FROM a",
        ];
        let render = |v: &V| match v {
            V::Null => "N".to_string(),
            V::Int(i) => format!("I{i}"),
            V::Real(f) => format!("R{f}"),
            V::Text(s) => format!("T{s}"),
        };
        for round in 0..80 {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            let mut catalog = Catalog::new();
            let mut db = Db::new("", "");
            let mut desc = String::new();
            let value = |rnd: &mut dyn FnMut(u64) -> u64| match rnd(9) {
                0 => V::Null,
                1 | 2 | 3 => V::Int(rnd(6) as i64),
                4 => V::Real(rnd(6) as f64),
                5 => V::Real(rnd(12) as f64 / 2.0),
                6 => V::text(&rnd(6).to_string()),
                7 => V::text(&format!("{}.5", rnd(3))),
                _ => V::text(["a", "b", "", "x1", "1e1", " 2"][rnd(6) as usize]),
            };
            for name in ["a", "b"] {
                let cols = ["k", "v", "p"];
                // Declared types; `b`'s may be none (BLOB affinity, ruling 1). `a`'s are always
                // declared: an untyped column holds `3` and `3.0` alike, so a GROUP BY over it
                // could refuse a bare column (`bare_columns_match_sqlite` covers that).
                let decl: Vec<(Aff, &str)> = cols.iter().map(|_| affs[if name == "b" { rnd(5) as usize } else { 1 + rnd(4) as usize }]).collect();
                let n = rnd(12) as usize;
                let konst = [V::Null, V::text("p"), V::Int(7), V::text("7")][rnd(4) as usize].clone();
                let raw: Vec<Vec<V>> = (0..n).map(|_| vec![value(&mut rnd), value(&mut rnd), konst.clone()]).collect();
                conn.execute(&format!("CREATE TABLE {name}({})", cols.iter().zip(&decl).map(|(c, (_, t))| format!("{c} {t}")).collect::<Vec<_>>().join(", ")), []).unwrap();
                for r in &raw {
                    let sql_v = |v: &V| match v {
                        V::Null => rusqlite::types::Value::Null,
                        V::Int(i) => rusqlite::types::Value::Integer(*i),
                        V::Real(f) => rusqlite::types::Value::Real(*f),
                        V::Text(s) => rusqlite::types::Value::Text(s.to_string()),
                    };
                    conn.execute(&format!("INSERT INTO {name} VALUES (?1, ?2, ?3)"), rusqlite::params_from_iter(r.iter().map(sql_v))).unwrap();
                }
                // The engine's columns: each in a random representation reading the stored value.
                let mut source = String::new();
                let ecols: Vec<Rc<Column>> = (0..3)
                    .map(|c| {
                        let aff = decl[c].0;
                        let stored: Vec<V> = raw.iter().map(|r| sv::apply_affinity(r[c].clone(), aff)).collect();
                        let texts = stored.iter().all(|v| matches!(v, V::Text(_) | V::Null));
                        let small = stored.iter().all(|v| matches!(v, V::Int(i) if (0..1000).contains(i)) || v.is_null());
                        match rnd(6) {
                            0 => Column::with_aff(&Rc::new(Column::Vals(raw.iter().map(|r| r[c].clone()).collect())), aff),
                            1 if texts => Rc::new(Column::Sym(stored.iter().map(|v| if let V::Text(s) = v { crate::facts::intern(s) } else { crate::facts::NONE }).collect())),
                            2 if small => Rc::new(Column::U32(stored.iter().map(|v| if let V::Int(i) = v { *i as u32 } else { U32_NULL }).collect())),
                            3 if texts => {
                                let cells = stored
                                    .iter()
                                    .map(|v| match v {
                                        V::Text(s) => {
                                            let s0 = source.len() as u32;
                                            source.push_str(s);
                                            TextCell::Span(s0, source.len() as u32)
                                        }
                                        _ => TextCell::Val(V::Null),
                                    })
                                    .collect();
                                Rc::new(Column::text(Rc::from(source.as_str()), cells))
                            }
                            4 if c == 2 && n > 0 => Rc::new(Column::Const(stored[0].clone())),
                            5 if n > 1 => {
                                let h = n / 2;
                                let (x, y) = (Rc::new(Column::Vals(stored[..h].to_vec())), Rc::new(Column::Vals(stored[h..].to_vec())));
                                chain(&[(&x, h), (&y, n - h)])
                            }
                            _ => Rc::new(Column::Vals(stored)),
                        }
                    })
                    .collect();
                let schema = Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: decl.iter().map(|d| d.0).collect() };
                desc.push_str(&format!("{name}: {:?} {:?}; ", decl.iter().map(|d| d.1).collect::<Vec<_>>(), ecols.iter().map(|c| format!("{c:?}").chars().take(60).collect::<String>()).collect::<Vec<_>>()));
                catalog.insert(name.into(), schema.clone());
                db.tables.insert(name.into(), Rc::new(Table::new(schema, ecols, n)));
            }
            // `c`: node-id parts as the policies store them (a kind as a symbol or a source span,
            // offsets as integer fact columns or values), from small domains so that ids of
            // different kinds share offsets.
            {
                let n = rnd(10) as usize;
                let rows: Vec<(Option<&str>, i64, i64)> = (0..n).map(|_| ([None, Some("a"), Some("b"), Some("ab")][rnd(4) as usize], rnd(3) as i64, rnd(3) as i64)).collect();
                conn.execute("CREATE TABLE c(kd TEXT, s INT, e INT)", []).unwrap();
                for (k, s, e) in &rows {
                    conn.execute("INSERT INTO c VALUES (?1, ?2, ?3)", rusqlite::params![k, s, e]).unwrap();
                }
                let kd = if rnd(2) == 0 {
                    Column::Sym(rows.iter().map(|r| r.0.map_or(crate::facts::NONE, crate::facts::intern)).collect())
                } else {
                    let source: String = rows.iter().filter_map(|r| r.0).collect();
                    let mut at = 0u32;
                    let cells = rows
                        .iter()
                        .map(|r| match r.0 {
                            Some(k) => {
                                at += k.len() as u32;
                                TextCell::Span(at - k.len() as u32, at)
                            }
                            None => TextCell::Val(V::Null),
                        })
                        .collect();
                    Column::text(Rc::from(source.as_str()), cells)
                };
                let mut ints = |f: &dyn Fn(&(Option<&str>, i64, i64)) -> i64| if rnd(2) == 0 { Column::U32(rows.iter().map(|r| f(r) as u32).collect()) } else { Column::Vals(rows.iter().map(|r| V::Int(f(r))).collect()) };
                let ecols = vec![Rc::new(kd), Rc::new(ints(&|r| r.1)), Rc::new(ints(&|r| r.2))];
                let schema = Schema { cols: vec!["kd".into(), "s".into(), "e".into()], affs: vec![Aff::Text, Aff::Integer, Aff::Integer] };
                catalog.insert("c".into(), schema.clone());
                db.tables.insert("c".into(), Rc::new(Table::new(schema, ecols, n)));
            }
            for q in queries {
                // A GROUP BY over a concatenation whose parts disagree within a group (a NULL
                // part makes every key NULL) is refused (ruling (c), `bare_columns_match_sqlite`).
                let t = match execute(&db, &plan(q, &catalog).unwrap()) {
                    Ok(t) => t,
                    Err(e) if e.starts_with("bare column") => continue,
                    Err(e) => panic!("round {round}: {q}: {e}; {desc}"),
                };
                let mut got: Vec<String> = (0..t.len()).map(|r| t.row(r).iter().map(render).collect::<Vec<_>>().join(",")).collect();
                let mut stmt = conn.prepare(q).unwrap();
                let w = stmt.column_count();
                let mut want: Vec<String> = stmt
                    .query_map([], |row| {
                        Ok((0..w)
                            .map(|i| {
                                render(&match row.get::<_, rusqlite::types::Value>(i).unwrap() {
                                    rusqlite::types::Value::Null => V::Null,
                                    rusqlite::types::Value::Integer(i) => V::Int(i),
                                    rusqlite::types::Value::Real(f) => V::Real(f),
                                    rusqlite::types::Value::Text(s) => V::text(&s),
                                    rusqlite::types::Value::Blob(_) => unreachable!(),
                                })
                            })
                            .collect::<Vec<_>>()
                            .join(","))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                got.sort();
                want.sort();
                assert_eq!(got, want, "round {round}: {q}; {desc}");
            }
        }
    }

    /// One random database three ways: in the bundled SQLite (3.53.2), as the engine's tables and
    /// as `rerail_facts::reference` reads them (through the front end's lowering).
    struct Three {
        conn: rusqlite::Connection,
        catalog: Catalog,
        db: Db<'static>,
        front: rerail_facts::front::sql::Catalog,
        rows: Vec<Vec<Vec<V>>>,
        desc: String,
    }

    fn render_row(r: &[V]) -> String {
        r.iter()
            .map(|v| match v {
                V::Null => "N".to_string(),
                V::Int(i) => format!("I{i}"),
                V::Real(f) => format!("R{f}"),
                V::Text(s) => format!("T{s}"),
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn sorted(rows: impl Iterator<Item = Vec<V>>) -> Vec<String> {
        let mut v: Vec<String> = rows.map(|r| render_row(&r)).collect();
        v.sort();
        v
    }

    impl Three {
        /// Tables `a`, `b` (`k`, `v`, `p` of random declared types, none included; values mixing
        /// integers, integral and fractional reals, numeric and other texts, NULLs; `p` from a
        /// small domain, so groups on it hold several rows).
        fn random(rnd: &mut dyn FnMut(u64) -> u64) -> Three {
            let mut t = Three { conn: rusqlite::Connection::open_in_memory().unwrap(), catalog: Catalog::new(), db: Db::new("", ""), front: Default::default(), rows: Vec::new(), desc: String::new() };
            let types = ["", "INT", "TEXT", "REAL", "NUMERIC"];
            for name in ["a", "b"] {
                let decl: Vec<&str> = (0..3).map(|_| types[rnd(5) as usize]).collect();
                let value = |rnd: &mut dyn FnMut(u64) -> u64| match rnd(9) {
                    0 => V::Null,
                    1..=3 => V::Int(rnd(4) as i64),
                    4 => V::Real(rnd(4) as f64),
                    5 => V::Real(rnd(8) as f64 / 2.0),
                    6 => V::text(&rnd(4).to_string()),
                    7 => V::text(&format!("{}.5", rnd(3))),
                    _ => V::text(["a", "b", "", "x1", "1e0", " 2"][rnd(6) as usize]),
                };
                let raw: Vec<Vec<V>> = (0..rnd(8))
                    .map(|_| {
                        let p = [V::Null, V::Int(1), V::text("1"), V::Real(1.0), V::text("p")][rnd(5) as usize].clone();
                        vec![value(rnd), value(rnd), p]
                    })
                    .collect();
                t.conn.execute(&format!("CREATE TABLE {name}(k {}, v {}, p {})", decl[0], decl[1], decl[2]), []).unwrap();
                for r in &raw {
                    let sql_v = |v: &V| match v {
                        V::Null => rusqlite::types::Value::Null,
                        V::Int(i) => rusqlite::types::Value::Integer(*i),
                        V::Real(f) => rusqlite::types::Value::Real(*f),
                        V::Text(s) => rusqlite::types::Value::Text(s.to_string()),
                    };
                    t.conn.execute(&format!("INSERT INTO {name} VALUES (?1, ?2, ?3)"), rusqlite::params_from_iter(r.iter().map(sql_v))).unwrap();
                }
                let affs: Vec<Aff> = decl.iter().map(|d| Aff::of_type(d)).collect();
                let stored: Vec<Vec<V>> = raw.iter().map(|r| r.iter().zip(&affs).map(|(v, a)| sv::apply_affinity(v.clone(), *a)).collect()).collect();
                let cols = ["k", "v", "p"];
                let schema = Schema { cols: cols.iter().map(|c| c.to_string()).collect(), affs: affs.clone() };
                let ecols = (0..3).map(|c| Rc::new(Column::Vals(stored.iter().map(|r| r[c].clone()).collect()))).collect();
                t.catalog.insert(name.into(), schema.clone());
                t.db.tables.insert(name.into(), Rc::new(Table::new(schema, ecols, stored.len())));
                let fschema = cols.iter().zip(&affs).map(|(c, a)| rerail_facts::ir::Column::new(c, *a)).collect();
                t.front.insert(name, rerail_facts::ir::AnySource::Input(t.rows.len() as u32), fschema);
                t.desc.push_str(&format!("{name}{decl:?} {:?}; ", raw.iter().map(|r| render_row(r)).collect::<Vec<_>>()));
                t.rows.push(stored);
            }
            t
        }

        fn sqlite(&self, sql: &str) -> Vec<String> {
            let mut stmt = self.conn.prepare(sql).unwrap_or_else(|e| panic!("SQLite: {sql}: {e}"));
            let w = stmt.column_count();
            let rows: Vec<Vec<V>> = stmt
                .query_map([], |row| {
                    Ok((0..w)
                        .map(|i| match row.get::<_, rusqlite::types::Value>(i).unwrap() {
                            rusqlite::types::Value::Null => V::Null,
                            rusqlite::types::Value::Integer(i) => V::Int(i),
                            rusqlite::types::Value::Real(f) => V::Real(f),
                            rusqlite::types::Value::Text(s) => V::text(&s),
                            rusqlite::types::Value::Blob(_) => unreachable!("no blob values"),
                        })
                        .collect())
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            sorted(rows.into_iter())
        }

        /// `CREATE TABLE … AS sql`, read back.
        fn sqlite_ctas(&self, sql: &str) -> Vec<String> {
            self.conn.execute(&format!("CREATE TABLE ctas AS {sql}"), []).unwrap();
            let r = self.sqlite("SELECT * FROM ctas");
            self.conn.execute("DROP TABLE ctas", []).unwrap();
            r
        }

        /// The engine's rows; `stage`: planned as a stage and stored as `rel::store` stores it.
        /// The rewrites (R14, R18, R19) have their own differentials and stay off here: under
        /// their mutants (29: `n + 1` read as `n`) the recursive queries would not end.
        fn engine(&self, sql: &str, stage: bool) -> Result<Vec<String>, String> {
            let stmt = plan_with(sql, &self.catalog, false)?;
            let stmt = if stage { stage_checked(stmt)? } else { stmt };
            let t = execute(&self.db, &stmt)?;
            let schema = stmt.schema();
            let cols: Vec<Rc<Column>> = t.cols.iter().take(schema.cols.len()).zip(&schema.affs).map(|(c, a)| if stage { Column::with_aff(c, *a) } else { c.clone() }).collect();
            let t = Table::new(schema.clone(), cols, t.len());
            Ok(sorted((0..t.len()).map(|r| t.row(r))))
        }

        /// The reference's rows (statement or stage).
        fn reference(&self, sql: &str, stage: bool) -> Result<Vec<String>, String> {
            use rerail_facts::front::sql::{lower_query, lower_stage};
            use rerail_facts::ir::{AnySource, Stage, StageDef};
            use rerail_facts::reference::{Evaluator, evaluate_stages};
            let q = parse::parse_query(sql)?;
            let src = |s: &AnySource| match s {
                AnySource::Input(i) => Ok(self.rows[*i as usize].clone()),
                s => Err(format!("unexpected source {s:?}")),
            };
            if stage {
                let (rel, schema) = lower_stage(&q, &self.front, &mut 0)?;
                let (done, refusal) = evaluate_stages(&[Stage { name: "s".into(), schema, def: StageDef::Query { rel } }], |_| None, &src);
                return match refusal {
                    Some(e) => Err(e),
                    None => Ok(sorted(done[0].iter().cloned())),
                };
            }
            let (rel, _) = lower_query(&q, &self.front, &mut 0)?;
            Ok(sorted(Evaluator::new(&src).rel(&rel, &[])?.into_iter()))
        }

        /// Checks `sql` (`stage`: as a stage): the engine gives the reference's result exactly,
        /// rows or a refusal of the same kind (`bare column`, `FLEXNUM`), and (`sqlite`) SQLite's
        /// rows whenever it gives rows. Returns whether it refused.
        fn check(&self, sql: &str, stage: bool, sqlite: bool, failures: &mut Vec<String>) -> bool {
            let (got, want) = (self.engine(sql, stage), self.reference(sql, stage));
            let kind = |e: &str| if e.starts_with("bare column") { "bare" } else if e.contains("FLEXNUM") { "flexnum" } else { "other" };
            let agree = match (&got, &want) {
                (Ok(g), Ok(w)) => g == w,
                (Err(g), Err(w)) => kind(g) == kind(w) && kind(g) != "other",
                _ => false,
            };
            let lite = if stage { self.sqlite_ctas(sql) } else { self.sqlite(sql) };
            if !agree || (sqlite && got.as_ref().is_ok_and(|g| *g != lite)) {
                let sqlite = lite;
                failures.push(format!("{sql}\n  engine    {got:?}\n  reference {want:?}\n  SQLite    {sqlite:?}\n  {}", self.desc));
            }
            got.is_err()
        }
    }

    fn xorshift(mut seed: u64) -> impl FnMut(u64) -> u64 {
        move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        }
    }

    /// The decider's ruling (c) on aggregate bare columns against SQLite 3.53.2 and the
    /// reference: with exactly one `min()`/`max()` (others alongside; the same call twice is
    /// one) the row it took, every argument NULL the group's last; otherwise a bare column read
    /// by the select list, `HAVING` or `ORDER BY` (through a correlated subquery too) with two
    /// distinct values in a group is refused (`bare column`), and agreeing ones are SQLite's.
    /// Mutant 31 (the check dropped) fails it.
    #[test]
    fn bare_columns_match_sqlite() {
        let queries = [
            "SELECT a.p, a.k, min(a.v) FROM a GROUP BY a.p",
            "SELECT a.p, a.k, max(a.v), count(*) FROM a GROUP BY a.p",
            "SELECT a.p, a.k, count(*), max(a.v), sum(a.k), group_concat(a.k) FROM a GROUP BY a.p",
            "SELECT a.k, a.v, min(a.k) FROM a",
            "SELECT a.p, a.k, min(a.v), min(a.v) FROM a GROUP BY a.p",
            "SELECT a.p, a.k FROM a GROUP BY a.p HAVING min(a.v) IS NOT NULL",
            "SELECT b.p, b.v, max(b.k), sum(b.k) FROM b GROUP BY b.p HAVING max(b.k) IS NOT NULL",
            // The one min()'s row among join rows tied on it: the engine and the reference take
            // the first in text-order nested-loop order, SQLite the first in its join order (it
            // may put `b` outside), so this one is compared with the reference only (reported
            // to the decider, phase-2-s8-perfile.md § 10).
            "SELECT a.p, b.v, min(a.k) FROM a, b WHERE b.p = a.p GROUP BY a.p",
            "SELECT a.p, b.k, max(b.v) FROM a LEFT JOIN b ON b.k = a.k GROUP BY a.p",
            "SELECT a.v, count(*) FROM a GROUP BY a.v",
            "SELECT a.p, a.k FROM a GROUP BY a.p",
            "SELECT a.p, a.k, min(a.v), max(a.v) FROM a GROUP BY a.p",
            "SELECT a.p, a.k, min(a.v), min(a.k) FROM a GROUP BY a.p",
            "SELECT k || ':' || v, count(*) FROM a GROUP BY k || ':' || v",
            "SELECT a.p, count(*) FROM a GROUP BY a.p HAVING a.k IS NOT NULL",
            "SELECT a.p, count(*) FROM a GROUP BY a.p ORDER BY a.k",
            "SELECT a.k, count(*) FROM a",
            "SELECT a.p, (SELECT count(*) FROM b WHERE b.k = a.k) FROM a GROUP BY a.p",
            "SELECT a.p, count(*) FROM a JOIN b ON b.p = a.p GROUP BY a.p",
            "SELECT t.x, t.k, count(*) FROM (SELECT p AS x, k FROM a) AS t GROUP BY t.x",
        ];
        let mut rnd = xorshift(0x5851_f42d_4c95_7f2d);
        let (mut failures, mut refused, mut total) = (Vec::new(), 0usize, 0usize);
        for _ in 0..150 {
            let t = Three::random(&mut rnd);
            for q in queries {
                total += 1;
                refused += t.check(q, false, !q.contains("FROM a, b"), &mut failures) as usize;
            }
        }
        let qs: std::collections::BTreeSet<&str> = failures.iter().filter_map(|f| f.lines().next()).collect();
        assert!(failures.is_empty(), "{} of {total} differ, queries {qs:#?}; first:\n{}", failures.len(), failures.iter().take(4).cloned().collect::<Vec<_>>().join("\n"));
        // Both outcomes occur: rows compared and refusals.
        assert!(refused > 0 && refused * 2 < total, "refused {refused} of {total}");
    }

    /// The decider's ruling (d) on compound column types against SQLite 3.53.2 and the
    /// reference: FROM subqueries and CTEs (recursive ones typed over all arms) walk the arms
    /// past none, demote to BLOB, read REAL columns' integers as reals; a `CAST` in the
    /// left-most arm compares numerically and converts nothing, a stage holding such a column
    /// is refused (FLEXNUM); a compound in `IN (SELECT …)` or a scalar subquery compares with the
    /// right-most arm's type on both sides. Mutant 33 (the walk stops at the left-most arm)
    /// fails it.
    #[test]
    fn compound_types_match_sqlite() {
        let queries: &[(&str, bool)] = &[
            ("SELECT x FROM (SELECT v AS x FROM a UNION ALL SELECT k FROM b)", false),
            ("SELECT x, x = '1', x = 1 FROM (SELECT v AS x FROM a UNION ALL SELECT k FROM b)", false),
            ("SELECT x FROM (SELECT 1 AS x UNION ALL SELECT v FROM a)", false),
            ("SELECT x, x = '1' FROM (SELECT k + 0 AS x FROM a UNION SELECT v FROM b)", false),
            ("SELECT x FROM (SELECT NULL AS x UNION ALL SELECT k FROM a UNION ALL SELECT 2)", false),
            ("WITH t AS (SELECT k AS x FROM a UNION SELECT v FROM b) SELECT x, x = '1', x = 1 + 0 FROM t", false),
            ("WITH t AS (SELECT 1 AS x UNION ALL SELECT v FROM a) SELECT t.x, b.k FROM t JOIN b ON b.k = t.x", false),
            ("SELECT t.x, b.v FROM (SELECT v AS x FROM a UNION ALL SELECT 'a' UNION ALL SELECT k FROM b) AS t, b WHERE t.x = b.v", false),
            ("SELECT x, x = '1' FROM (SELECT CAST(v AS REAL) AS x FROM a UNION ALL SELECT k FROM b)", false),
            ("SELECT x, x = b.v FROM (SELECT CAST(k AS INT) AS x FROM a UNION ALL SELECT 4.0) AS t, b", false),
            ("SELECT x FROM (SELECT k + 0 AS x FROM a UNION ALL SELECT v FROM b UNION ALL SELECT p FROM a)", false),
            ("SELECT x FROM (SELECT x FROM (SELECT v AS x FROM a UNION ALL SELECT k FROM b) WHERE x IS NOT NULL)", false),
            ("SELECT x, y FROM (SELECT CAST(k AS TEXT) AS x, v AS y FROM a UNION ALL SELECT 1, 'z' FROM b) WHERE x = y", false),
            ("WITH RECURSIVE w(x, n) AS (SELECT v, 0 FROM a UNION ALL SELECT x, n + 1 FROM w WHERE n < 2) SELECT x, n, x = '1', x = 1 + 0 FROM w", false),
            ("WITH RECURSIVE w(x, n) AS (SELECT 1, 0 UNION ALL SELECT a.v, n + 1 FROM w, a WHERE n < 1) SELECT x, n, x = '1' FROM w", false),
            ("WITH RECURSIVE w(x, y, n) AS (SELECT k, v, 0 FROM a UNION ALL SELECT y, x, n + 1 FROM w WHERE n < 2) SELECT x, y, n, x = 1 + 0, y = 1 + 0 FROM w", false),
            ("WITH RECURSIVE w(x, n) AS (SELECT k, 0 FROM a UNION SELECT b.v, n + 1 FROM w, b WHERE n < 2) SELECT x, n FROM w", false),
            ("SELECT a.k, a.v IN (SELECT k FROM b UNION ALL SELECT v FROM b) FROM a", false),
            ("SELECT a.k, a.v = (SELECT k FROM b UNION ALL SELECT v FROM b) FROM a", false),
            ("SELECT a.k, (SELECT k FROM b UNION ALL SELECT v FROM b) = a.v FROM a", false),
            ("SELECT a.k, a.v IN (SELECT v FROM b UNION SELECT 1) FROM a", false),
            ("SELECT v AS x FROM a UNION ALL SELECT k FROM b", true),
            ("SELECT 1 AS x UNION ALL SELECT v FROM a", true),
            ("SELECT k AS x, v AS y FROM a UNION SELECT v, 'z' FROM b", true),
            ("SELECT CAST(v AS INT) AS x FROM a UNION ALL SELECT 4.0", true),
            ("SELECT x FROM (SELECT CAST(v AS INT) AS x FROM a UNION ALL SELECT k FROM b)", true),
            ("SELECT v AS x, v || '' AS y, CAST(k AS REAL) AS z FROM a", true),
        ];
        let mut rnd = xorshift(0x2545_f491_4f6c_dd1d);
        let (mut failures, mut total) = (Vec::new(), 0usize);
        for _ in 0..150 {
            let t = Three::random(&mut rnd);
            for (q, stage) in queries {
                total += 1;
                t.check(q, *stage, true, &mut failures);
            }
        }
        assert!(failures.is_empty(), "{} of {total} differ; first:\n{}", failures.len(), failures.iter().take(4).cloned().collect::<Vec<_>>().join("\n"));
    }
}
