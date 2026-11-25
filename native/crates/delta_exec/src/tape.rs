//! A document's tape views, read where the tape lies (decision 0353: a view is a read of the tape, not a relation;
//! nothing makes its rows). Each view's rows begin with the document's id, since node ids are the document's own
//! (L9):
//!
//! - `kind(d, n, k)`: node `n`'s kind, a symbol;
//! - `field(d, p, f, c)`: `c` is `p`'s child in field `f`;
//! - `child(d, p, c, i)`: `c` is `p`'s `i`-th child, counting named children that are no comment;
//! - `text(d, n, s)`: `n`'s source text, a symbol;
//! - `value(d, n, v)`: a literal's value: a string's unescaped contents (a symbol), a number's value;
//! - `doc(d, path)`: the document's path.
//!
//! A row is given to its reader in a chunk the reader reuses, never stored.

use crate::sym::Symbols;
use rerail_rr_syntax::tape::{FIELD_NAMES, FIRST_ANONYMOUS, KIND_NAMES, Kind, entry_kind_field};
use rerail_tape_format::f16::F16;
use rerail_tape_format::tape::Tape as _;
use rustc_hash::FxHashMap;

/// The views, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Kind,
    Field,
    Child,
    Text,
    Value,
    Doc,
}

impl View {
    pub const ALL: [(View, &'static str, usize); 6] =
        [(View::Kind, "kind", 3), (View::Field, "field", 4), (View::Child, "child", 4), (View::Text, "text", 3), (View::Value, "value", 3), (View::Doc, "doc", 2)];

    pub fn arity(self) -> usize {
        View::ALL.iter().find(|v| v.0 == self).expect("a view").2
    }
}

/// The kinds' and fields' names as symbols, both ways: resolved once, so a view compares ids.
pub struct Names {
    kind: Vec<i64>,
    field: Vec<i64>,
    kind_of: FxHashMap<i64, u16>,
    field_of: FxHashMap<i64, u16>,
}

impl Names {
    pub fn new(sym: &Symbols) -> Names {
        let kind: Vec<i64> = KIND_NAMES.iter().map(|n| sym.intern(n)).collect();
        let field: Vec<i64> = FIELD_NAMES.iter().map(|n| sym.intern(n)).collect();
        let kind_of = kind.iter().enumerate().skip(1).map(|(i, &s)| (s, i as u16)).collect();
        let field_of = field.iter().enumerate().skip(1).map(|(i, &s)| (s, i as u16)).collect();
        Names { kind, field, kind_of, field_of }
    }

    pub fn kind(&self, k: u16) -> i64 {
        self.kind.get(k as usize).copied().unwrap_or(0)
    }

    pub fn field(&self, f: u16) -> i64 {
        self.field.get(f as usize).copied().unwrap_or(0)
    }
}

/// A tape's reads: a node's kind, field, children and text, where the tape lies.
pub struct Reads<'t> {
    r: F16<'t>,
    src: &'t str,
}

impl<'t> Reads<'t> {
    pub fn of(text: &'t crate::db::Text) -> Reads<'t> {
        let r = text.tape.reader(rerail_rr_syntax::tape::lang()).expect("an rr tape read with the rr table");
        Reads { r, src: &text.src }
    }

    pub fn len(&self) -> usize {
        self.r.len()
    }

    pub fn is_empty(&self) -> bool {
        self.r.len() == 0
    }

    /// Node `i`'s kind and field, and whether it is an extra (a comment).
    pub fn node(&self, i: usize) -> (u16, u16, bool) {
        let e = self.r.table().entry(self.r.code(i));
        let (k, f) = entry_kind_field(e);
        (k, f, e.flags & rerail_tape_format::CODE_EXTRA != 0)
    }

    pub fn parent(&self, i: usize) -> Option<usize> {
        self.r.parent(i)
    }

    /// Node `i`'s children, in order.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.r.subtree_end(i);
        std::iter::successors(self.r.first_child(i), move |&c| self.r.next_sibling(c, end))
    }

    /// Whether node `i` is counted by `child`: named, and no comment.
    pub fn counted(&self, i: usize) -> bool {
        let (k, _, extra) = self.node(i);
        !extra && k < FIRST_ANONYMOUS
    }

    /// Node `i`'s source text.
    pub fn text(&self, i: usize) -> &'t str {
        &self.src[self.r.start(i) as usize..self.r.end(i) as usize]
    }

    /// A literal's value: a string's unescaped contents, interned; a number's value. `None` for any other node.
    pub fn value(&self, i: usize, sym: &Symbols) -> Option<i64> {
        let (k, _, _) = self.node(i);
        let t = self.text(i);
        if k == Kind::String as u16 {
            let body = t.strip_prefix('"').and_then(|b| b.strip_suffix('"'))?;
            let s = rerail_rr_syntax::syntax::unescape(body).ok()?;
            Some(sym.intern(&s))
        } else if k == Kind::Number as u16 {
            t.parse::<i64>().ok()
        } else {
            None
        }
    }
}

/// Every row of view `v` over document `d`'s tape `t` that agrees with `bound` (`Some` columns), given to `each`.
/// A bound node is read directly; an unbound one is every node.
pub fn rows(v: View, d: i64, path: i64, t: &Reads<'_>, sym: &Symbols, names: &Names, bound: &[Option<i64>], each: &mut dyn FnMut(&[i64])) {
    let fits = |row: &[i64]| row.iter().zip(bound).all(|(x, b)| b.is_none_or(|b| b == *x));
    let node_ok = |n: i64| n >= 0 && (n as usize) < t.len();
    let nodes = |b: Option<i64>| -> Box<dyn Iterator<Item = usize>> {
        match b {
            Some(n) if node_ok(n) => Box::new(std::iter::once(n as usize)),
            Some(_) => Box::new(std::iter::empty()),
            None => Box::new(0..t.len()),
        }
    };
    match v {
        View::Doc => {
            let row = [d, path];
            if fits(&row) {
                each(&row);
            }
        }
        View::Kind => {
            let want = bound[2].map(|k| names.kind_of.get(&k).copied());
            if want == Some(None) {
                return;
            }
            for n in nodes(bound[1]) {
                let (k, _, _) = t.node(n);
                if want.is_some_and(|w| w != Some(k)) {
                    continue;
                }
                each(&[d, n as i64, names.kind(k)]);
            }
        }
        View::Field => {
            let want = bound[2].map(|f| names.field_of.get(&f).copied());
            if want == Some(None) {
                return;
            }
            let mut put = |p: usize, c: usize| {
                let (_, f, _) = t.node(c);
                if f != 0 && want.is_none_or(|w| w == Some(f)) {
                    let row = [d, p as i64, names.field(f), c as i64];
                    if fits(&row) {
                        each(&row);
                    }
                }
            };
            match (bound[1], bound[3]) {
                (Some(p), _) if node_ok(p) => t.children(p as usize).for_each(|c| put(p as usize, c)),
                (Some(_), _) => {}
                (None, Some(c)) if node_ok(c) => {
                    if let Some(p) = t.parent(c as usize) {
                        put(p, c as usize)
                    }
                }
                (None, Some(_)) => {}
                (None, None) => (0..t.len()).for_each(|p| t.children(p).for_each(|c| put(p, c))),
            }
        }
        View::Child => {
            let mut put = |p: usize| {
                for (i, c) in t.children(p).filter(|&c| t.counted(c)).enumerate() {
                    let row = [d, p as i64, c as i64, i as i64];
                    if fits(&row) {
                        each(&row);
                    }
                }
            };
            match (bound[1], bound[2]) {
                (Some(p), _) if node_ok(p) => put(p as usize),
                (Some(_), _) => {}
                (None, Some(c)) if node_ok(c) => {
                    if let Some(p) = t.parent(c as usize) {
                        put(p)
                    }
                }
                (None, Some(_)) => {}
                (None, None) => (0..t.len()).for_each(put),
            }
        }
        View::Text => {
            for n in nodes(bound[1]) {
                // A symbol made of the node's text: a derived fact, written once into the symbols' block.
                let row = [d, n as i64, sym.intern(t.text(n))];
                if fits(&row) {
                    each(&row);
                }
            }
        }
        View::Value => {
            for n in nodes(bound[1]) {
                if let Some(v) = t.value(n, sym) {
                    let row = [d, n as i64, v];
                    if fits(&row) {
                        each(&row);
                    }
                }
            }
        }
    }
}
