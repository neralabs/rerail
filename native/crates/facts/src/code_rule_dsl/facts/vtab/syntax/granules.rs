//! Node ids per definition (decisions 0083 §3, 0094 §2): the default numbering. A node id's low 32 bits are a
//! granule's slot and an index inside the granule, so an edit renumbers the nodes of the granules it touches and no
//! other's. `RERAIL_NODE_IDS=preorder` numbers nodes in preorder instead, to compare.
//!
//! ```text
//! 31            18 17                 0
//! [  granule slot ][ index in granule  ]
//!   hash of the       the granule's nodes in preorder, then its tokens
//!   granule's key
//! ```
//!
//! **Granules.** A document is cut into granules, the same way in every language:
//!
//! - **Definitions** start a granule wherever they are, so the granule is the innermost one: a function, a method, a
//!   class, an interface, a Rust `impl`, a SQL `CREATE`. A wrapper that only adds to one (a Python decorator, an
//!   `export`, a SQL statement around its `CREATE`) is part of its granule.
//! - **Statements of a container** are granules of their own: the document's top level, and a class-like body (a
//!   Python class's block, a JavaScript class body, a PHP or Rust declaration list, a namespace's block). So a class
//!   header and its fields are apart from its methods, and an edit at module level renumbers that statement only.
//! - Anything else belongs to the granule it is in: a function's body, a JSX tree inside its component, an
//!   anonymous callback.
//!
//! **Keys.** A granule's key is its parent granule's key, its kind and its name, and an ordinal among siblings of
//! the same kind and name (`a.py/class_definition:C/function_definition:m#0`). The name is:
//! - a definition's own name (its `name` field; a Rust `impl`'s trait and type; a SQL object's reference);
//! - the names a declaration binds (`const App = () => …`, `X = 1`), so a component or constant keeps its granule
//!   when its value is edited;
//! - an import's text;
//! - for any other statement, a digest of its own text with nested granules cut out, so an unchanged statement keeps
//!   its granule wherever it moves.
//!
//! **Slots.** A granule's home slot is a hash of its key. Clashes are settled in key order: granules are placed by
//! (home, key), each in the first free slot from its home. So the slots are a function of the set of keys, not of
//! the order the definitions appear in, and adding one moves at most a granule it clashes with. Slot 0 is the
//! document's own (its root); the top `SPAN_SLOTS` slots are its spans' (its text, lines and tokens, which are
//! positions, in order), and the `EXTRA_SLOTS` below them `Extra`'s spans' (`ExtraSpan`: core.rerail's source, lines,
//! comments and matches made beside the projection), a slot range for each kind.
//!
//! **Overflow** never falls back to preorder: up to `SPAN_SLOTS << INDEX_BITS` positions (4 M), each `ExtraSpan`
//! kind's capacity (a document past either fails with a named error, never wraps) and any number of nodes. A granule with more than `INDEX` nodes and tokens goes on in further
//! slots, keyed by its key and a count, placed after every granule's first slot (nodes first, then tokens), so a
//! token's continuation never moves a node. A document with more granules than slots folds its statement granules
//! into their parents, then, past that, every granule into its root.
//!
//! **Embedded documents** (guests: SQL in a string, a Vue file's script) are numbered by the same rules in their own
//! grammar. A guest's document key is a hash of its host node's id (`corpus::guests`), so it is as stable as that.

use crate::code_rule_dsl::facts::core::FactsIn;
use rerail_blocks::frame::{self, Frame, FrameVec, InlineVec};
use rerail_blocks::vector::Vector;
use super::cells::{Block, Cell, Cells};
use super::ranks::{Item, Ranks};
use super::{Dialect, Projected};
use crate::code_rule_dsl::facts::core::Facts;
use std::sync::Arc;

pub(super) const SLOT_BITS: u32 = rerail_clif::ids::SLOT_BITS;
pub const INDEX_BITS: u32 = rerail_clif::ids::INDEX_BITS;
// A kept column splits a node id at its granule (decision 0456 K2): the bit is this one.
const _: () = assert!(INDEX_BITS == rerail_blocks::packed::ID_SPLIT, "a kept id column splits at the granule's index bits");
/// Indexes in one slot.
pub const INDEX: u32 = 1 << INDEX_BITS;
const SLOTS: u32 = 1 << SLOT_BITS;
/// The top slots, from `SLOTS - 1` down, hold a document's spans: up to `SPAN_SLOTS << INDEX_BITS` of them.
pub(super) const SPAN_SLOTS: u32 = rerail_clif::ids::SPAN_SLOTS;
/// The positions (`span_low`) a document may have: past them a run fails (`super::Batch::build`), never wraps.
pub const POSITIONS: u64 = SPAN_SLOTS as u64 * INDEX as u64;
/// The first slot of the spans'.
const SPAN_FIRST: u32 = SLOTS - SPAN_SLOTS;
/// Slots of `Extra`'s spans (`ExtraSpan`): one for the lines, one for the comments, `MATCH_SLOTS` for the matches,
/// below the positions'. Sixteen in all, so the granules' slots (and every node's id) are as they were.
const MATCH_SLOTS: u32 = 14;
pub const EXTRA_SLOTS: u32 = 2 + MATCH_SLOTS;
const EXTRA_FIRST: u32 = SPAN_FIRST - EXTRA_SLOTS;
/// Slots granules take: `1..EXTRA_FIRST`.
const GRANULE_SLOTS: u32 = EXTRA_FIRST - 1;

/// The kinds of `Extra`'s spans (core.rerail's `span`s made beside the projection: `super::Extra::document`), each in slots
/// of its own between the granules' and the positions', so no node, position or other kind has its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraSpan {
    /// The whole source (index 0) and its lines (`src_line`, from 1).
    Line,
    /// The comments no node is, from 1.
    Comment,
    /// Each `match_all`/`match_group`/`rx_span` match or group, from 0.
    Match,
}

impl ExtraSpan {
    const fn first(self) -> u32 {
        match self {
            ExtraSpan::Line => 0,
            ExtraSpan::Comment => 1,
            ExtraSpan::Match => 2,
        }
    }
    /// How many spans of the kind a document may have.
    pub const fn capacity(self) -> u32 {
        match self {
            ExtraSpan::Match => MATCH_SLOTS * INDEX,
            _ => INDEX,
        }
    }
}

/// The low bits of span `i` of `kind`, or `None` past the kind's capacity.
pub fn extra_low(kind: ExtraSpan, i: u32) -> Option<u32> {
    (i < kind.capacity()).then(|| ((EXTRA_FIRST + kind.first() + i / INDEX) << INDEX_BITS) | (i % INDEX))
}

/// Whether an id's low bits are one of `Extra`'s spans' (`extra_low`): past the granules', below the positions'.
pub fn is_extra(low: u32) -> bool {
    (EXTRA_FIRST..SPAN_FIRST).contains(&(low >> INDEX_BITS))
}

/// The comment number (from 1) of a `Comment` span's low bits, as `extra_low(ExtraSpan::Comment, i)` made them.
pub fn comment_of(low: u32) -> Option<u32> {
    let first = EXTRA_FIRST + ExtraSpan::Comment.first();
    (low >> INDEX_BITS == first).then_some(low % INDEX)
}

/// Definitions, in every grammar the engine reads (Python, JavaScript, TypeScript and TSX, PHP, Rust, SQL).
fn is_def(kind: &str) -> bool {
    matches!(
        kind,
        // Python; PHP's functions
        "function_definition" | "class_definition"
        // JavaScript, TypeScript, TSX; PHP's classes, interfaces and enums
        | "function_declaration" | "generator_function_declaration" | "class_declaration" | "abstract_class_declaration"
        | "method_definition" | "interface_declaration" | "type_alias_declaration" | "enum_declaration"
        | "internal_module" | "function_signature" | "abstract_method_signature"
        // PHP
        | "method_declaration" | "trait_declaration"
        // Rust
        | "function_item" | "function_signature_item" | "struct_item" | "enum_item" | "union_item" | "trait_item"
        | "impl_item" | "mod_item" | "macro_definition" | "const_item" | "static_item" | "type_item"
        // SQL
        | "create_table" | "create_view" | "create_materialized_view" | "create_function" | "create_procedure"
        | "create_index" | "create_trigger" | "create_type" | "create_schema" | "create_sequence" | "create_domain"
        | "create_policy" | "create_foreign_table"
    )
}

/// Wrappers that only add to the definition inside them: a granule with it.
fn is_wrapper(kind: &str) -> bool {
    matches!(kind, "decorated_definition" | "export_statement" | "statement")
}

/// Bodies whose statements are granules, when their parent is one of `is_container_owner`'s.
fn is_body(kind: &str) -> bool {
    matches!(kind, "block" | "class_body" | "declaration_list" | "statement_block" | "compound_statement")
}

/// Definitions whose body is a container: class-like ones and namespaces, not functions.
fn is_container_owner(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition" | "class_declaration" | "abstract_class_declaration" | "class" | "internal_module"
            | "module" | "trait_declaration" | "interface_declaration" | "enum_declaration" | "namespace_definition"
            | "impl_item" | "trait_item" | "mod_item"
    )
}

/// What kind `kind` is to the granules (`DEF`, `WRAPPER`, `BODY`, `OWNER`): the names above, made into a table by
/// kind code once per grammar (`Dialect::granule_kind`), so cutting reads a byte for each node.
pub fn kind_flags(kind: &str) -> u8 {
    (u8::from(is_def(kind)) * DEF) | (u8::from(is_wrapper(kind)) * WRAPPER) | (u8::from(is_body(kind)) * BODY) | (u8::from(is_container_owner(kind)) * OWNER) | (u8::from(super::core_rr::is_spread(kind)) * SPREAD) | (u8::from(super::core_rr::is_comment(kind)) * COMMENT)
}
pub const DEF: u8 = 1;
pub const WRAPPER: u8 = 2;
pub const BODY: u8 = 4;
pub const OWNER: u8 = 8;
/// A spread element or argument (`core_rr::is_spread`): no granule's, the `spread` view's (`Item::Spread`).
pub const SPREAD: u8 = 16;
/// A comment (`core_rr::is_comment`): no lexeme (`ranks::Item::Lexeme`).
pub const COMMENT: u8 = 32;

/// Declarations named by what they bind.
fn binds(kind: &str) -> bool {
    matches!(kind, "lexical_declaration" | "variable_declaration" | "expression_statement" | "const_declaration" | "property_declaration" | "field_definition" | "public_field_definition" | "let_declaration")
}

fn is_import(kind: &str) -> bool {
    matches!(kind, "import_statement" | "import_from_statement" | "future_import_statement" | "use_declaration" | "namespace_use_declaration" | "extern_crate_declaration")
}

/// A document's granules, in the numbering's frame (decision 0687): each node's granule, each granule's first node
/// and key, and once placed (`place`) each granule's slots.
pub(super) struct Cut<'f> {
    /// Each local node's granule, as an index into `starts`.
    pub(super) of: FrameVec<'f, u32>,
    /// Each granule's first node (its start), in preorder; the root's is 0.
    pub(super) starts: FrameVec<'f, u32>,
    pub(super) keys: Keys<'f>,
    /// Each local node's parent (`Projected::parent`), read in the cut's walk.
    pub(super) up: FrameVec<'f, u32>,
    /// Granule `g`'s slots are `slots[at[g]..at[g + 1]]`, once placed (`place`): its first, then its continuations.
    pub(super) at: FrameVec<'f, u32>,
    pub(super) slots: FrameVec<'f, u32>,
}

/// Texts written one after another in one array of the frame: text `i` is `text[at[i]..at[i + 1]]`.
pub(super) struct Keys<'f> {
    text: FrameVec<'f, u8>,
    at: FrameVec<'f, u32>,
}

impl<'f> Keys<'f> {
    fn new(fr: &'f Frame, n: usize, bytes: usize) -> Self {
        let mut at = fr.vec_with(n + 1);
        at.push(0);
        Keys { text: fr.vec_with(bytes), at }
    }

    fn len(&self) -> usize {
        self.at.len() - 1
    }

    fn get(&self, i: usize) -> &[u8] {
        &self.text[self.at[i] as usize..self.at[i + 1] as usize]
    }

    /// Ends the text being written (`text[at[len]..]`) as text `len`.
    fn end(&mut self) {
        self.at.push(self.text.len() as u32);
    }
}

/// Appends `s`.
fn put(out: &mut FrameVec<'_, u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
}

/// Appends `n` in decimal.
fn put_dec(out: &mut FrameVec<'_, u8>, n: u32) {
    let mut d = [0u8; 10];
    let mut i = d.len();
    let mut n = n;
    loop {
        i -= 1;
        d[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&d[i..]);
}

/// `out[from..]` (UTF-8) with its whitespace runs made one space and none at either end, as
/// `split_whitespace().join(" ")`; kept when that is 1 to 160 bytes, else removed. Whether it was kept.
fn short_from(out: &mut FrameVec<'_, u8>, from: usize) -> bool {
    let s = &mut out[from..];
    let (mut r, mut w, mut space) = (0, 0, false);
    while r < s.len() {
        // SAFETY: `s[r..]` starts at a character of the UTF-8 text: what is written lies before `r` (`w <= r`).
        let c = unsafe { std::str::from_utf8_unchecked(&s[r..]) }.chars().next().expect("a character at r");
        let n = c.len_utf8();
        if c.is_whitespace() {
            space = w > 0;
        } else {
            if space {
                s[w] = b' ';
                w += 1;
                space = false;
            }
            s.copy_within(r..r + n, w);
            w += n;
        }
        r += n;
    }
    let keep = w > 0 && w <= 160;
    out.truncate(if keep { from + w } else { from });
    keep
}

/// How far a document's granules are folded (more granules than slots).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Fold {
    None,
    Statements,
    All,
}

/// The ids of a projection's nodes and tokens, and its nodes' order keys, as arrays: what the granule table answers in
/// place, made only to check and measure it (`number`'s `reference`).
#[derive(Default)]
pub struct Reference {
    /// Each node's, then each token's, id's low 32 bits.
    pub low: Vector<u32>,
    /// Each node's order key.
    pub order: Vector<u32>,
}

/// The granule table of `p` (`Table`): what answers the ids of its nodes and of `tokens` (each token's node, in
/// preorder of the tokens) and its nodes' order keys in place. With `reference`, those ids and keys as arrays too,
/// to check the table against.
///
/// One operation (decision 0687): its working arrays are its frame's, and the table's columns are written into the
/// projection's block.
pub fn number(p: &Projected, dialect: &Dialect, tokens: &[u32], mut reference: Option<&mut Reference>) -> Table {
    for fold in [Fold::None, Fold::Statements, Fold::All] {
        let table = frame::op(|fr| {
            let mut cut = cut(fr, p, dialect, fold);
            let g = cut.starts.len();
            let nodes = fr.zeroed::<u32>(g);
            for &o in cut.of.iter() {
                nodes[o as usize] += 1;
            }
            let mut all = fr.vec_from(&*nodes);
            for &t in tokens {
                all[cut.of[t as usize] as usize] += 1;
            }
            let continued: u32 = all.iter().map(|&n| n.saturating_sub(1) / INDEX).sum();
            if g as u32 - 1 + continued > GRANULE_SLOTS && fold < Fold::All {
                return None;
            }
            let _span = crate::code_rule_dsl::logic::profile::span(|| "granule slots".into());
            place(fr, &mut cut, nodes, &all);
            if let Some(r) = reference.as_deref_mut() {
                r.low = reference_low(fr, &cut, tokens);
                r.order = order(fr, &cut.of);
            }
            let mut table = Table::new(fr, p, &cut, nodes, &all);
            table.whole = fold == Fold::All;
            Some(table)
        });
        if let Some(table) = table {
            return table;
        }
    }
    unreachable!("a document folded whole fits")
}

/// Bits of an order key's index inside its granule; the rest are its run's rank (`order`).
const ORDER_INDEX_BITS: u32 = rerail_clif::ids::ORDER_INDEX_BITS;

/// Each node's order key (`node_order`'s low 32 bits, decision 0121): the rank of its *run*, a stretch of nodes
/// consecutive in preorder that are all one granule's, then its index among its granule's nodes. Runs follow one another
/// in preorder and a granule's nodes are in preorder in it, so the keys are in preorder; but an edit inside a granule
/// changes neither the runs (granules nested in it start where they did among its nodes) nor another granule's
/// indexes, so no other granule's keys move, where a preorder index moves every node after the edit. An edit that
/// adds or removes a granule moves the ranks after it. A document with more runs, or a granule with more nodes, than
/// the bits hold is ordered by preorder index (one scheme per document, so its keys still compare).
fn order(fr: &Frame, of: &[u32]) -> Vector<u32> {
    let index = fr.zeroed::<u32>(of.iter().max().map_or(1, |&m| m as usize + 1));
    let mut rank = 0u32;
    let mut keys = rerail_blocks::vector::Vector::with_capacity(of.len());
    for (l, &g) in of.iter().enumerate() {
        if l > 0 && g != of[l - 1] {
            rank += 1;
        }
        let i = &mut index[g as usize];
        if rank >> (32 - ORDER_INDEX_BITS) != 0 || *i >> ORDER_INDEX_BITS != 0 {
            return (0..of.len() as u32).collect();
        }
        keys.push(rank << ORDER_INDEX_BITS | *i);
        *i += 1;
    }
    keys
}

/// Each node's granule, unfolded (`tests::node_order_keys_census`): an index in preorder of the granules' starts.
#[cfg(any(test, feature = "test-hooks"))]
pub fn granule_of(p: &Projected, dialect: &Dialect) -> Vector<u32> {
    frame::op(|fr| cut(fr, p, dialect, Fold::None).of.iter().copied().collect())
}

/// The granules of `p`, folded as `fold` says, in the frame `fr`.
fn cut<'f>(fr: &'f Frame, p: &Projected, dialect: &Dialect, fold: Fold) -> Cut<'f> {
    let f = p.f();
    let n = p.len() as usize;
    let kind = |l: usize| dialect.name(f, p.node(l as u32));
    let starts_span = crate::code_rule_dsl::logic::profile::span(|| "granule starts".into());
    // The numbering's working state, one walk of the nodes: each node's parent and what its kind is to the granules.
    let mut up = fr.vec_with::<u32>(n);
    up.resize(n, super::NONE);
    let flags = fr.zeroed::<u8>(n);
    for (l, x, q) in p.walk() {
        up[l as usize] = q;
        flags[l as usize] = dialect.granule_kind(dialect.code(f, x));
    }
    let is = |l: usize, flag: u8| flags[l] & flag != 0;
    // Whether each node starts a granule (a definition, or a statement of a container).
    let start = fr.zeroed::<u8>(n);
    // A wrapper's definition is in the wrapper's granule.
    let claimed = fr.zeroed::<u8>(n);
    if fold != Fold::All {
        for l in 1..n {
            let q = up[l] as usize;
            let owner = |q: usize| {
                let o = up[q] as usize;
                is(o, OWNER) && (start[o] != 0 || claimed[o] != 0)
            };
            let in_container = q == 0 || (is(q, BODY) && up[q] != super::NONE && owner(q));
            if is(l, DEF) && claimed[l] == 0 {
                start[l] = 1;
            } else if in_container && fold == Fold::None {
                start[l] = 1;
            }
            // A wrapper claims its first definition child, which then starts no granule of its own: wherever it is,
            // so a definition's decorators are its granule's in a function's body too (`units`).
            if is(l, WRAPPER)
                && let Some(c) = p.children(l as u32).find(|&c| is(c as usize, DEF)) {
                    claimed[c as usize] = 1;
                    start[l] = 1;
                }
            if claimed[l] != 0 {
                start[l] = 0;
            }
        }
    }
    let mut of = fr.vec_with::<u32>(n);
    of.resize(n, 0);
    let mut starts = fr.vec_with::<u32>(start.iter().filter(|&&s| s != 0).count() + 1);
    starts.push(0);
    for l in 1..n {
        of[l] = if start[l] != 0 {
            starts.push(l as u32);
            starts.len() as u32 - 1
        } else {
            of[up[l] as usize]
        };
    }
    drop(starts_span);
    let _keys_span = crate::code_rule_dsl::logic::profile::span(|| "granule keys".into());
    // Keys, parents first (preorder), each its parent's key, kind and name (its base) and an ordinal among the
    // granules of the same base before it. `seen` is an open table of the bases: (the base's first byte + 1, its
    // length, how many so far).
    let g = starts.len();
    let seen = fr.zeroed::<[u32; 3]>((2 * g).next_power_of_two());
    let mask = seen.len() - 1;
    let mut keys = Keys::new(fr, g, 64 * g);
    keys.end();
    for i in 1..g {
        let l = starts[i] as usize;
        let parent = of[up[l] as usize] as usize;
        let from = keys.text.len();
        let (a, b) = (keys.at[parent] as usize, keys.at[parent + 1] as usize);
        keys.text.resize(from + (b - a), 0);
        keys.text.copy_within(a..b, from);
        put(&mut keys.text, "/");
        put(&mut keys.text, kind(l));
        put(&mut keys.text, ":");
        name(p, dialect, &of, &up, &starts, i, &mut keys.text);
        let base = &keys.text[from..];
        let mut h = rustc_hash::FxHasher::default();
        std::hash::Hasher::write(&mut h, base);
        let mut s = std::hash::Hasher::finish(&h) as usize & mask;
        let ord = loop {
            let e = &mut seen[s];
            if e[0] == 0 {
                *e = [from as u32 + 1, base.len() as u32, 1];
                break 0;
            }
            let (at, len) = (e[0] as usize - 1, e[1] as usize);
            if keys.text[at..at + len] == *base {
                e[2] += 1;
                break e[2] - 1;
            }
            s = (s + 1) & mask;
        };
        put(&mut keys.text, "#");
        put_dec(&mut keys.text, ord);
        keys.end();
    }
    Cut { of, starts, keys, up, at: fr.vec(), slots: fr.vec() }
}

/// Writes granule `g`'s name into `out`: what it defines or binds, an import's text, or its own text's digest.
fn name(p: &Projected, dialect: &Dialect, of: &[u32], up: &[u32], starts: &[u32], g: usize, out: &mut FrameVec<'_, u8>) {
    let f = p.f();
    let l = starts[g];
    let id = p.node(l);
    let kind = dialect.name(f, id);
    // A wrapper is named by its definition, or by the declaration it exports.
    let inner = |want: fn(&str) -> bool| f.children(id).iter().copied().find(|&c| f.named(c) && want(dialect.name(f, c)));
    let def = if is_wrapper(kind) { inner(is_def).or_else(|| inner(binds)).unwrap_or(id) } else { id };
    let text_of = |field: &str| dialect.field_child(f, def, field).map(|c| f.text(c));
    let from = out.len();
    let named = match dialect.name(f, def) {
        "impl_item" => {
            put(out, text_of("trait").unwrap_or(""));
            put(out, " for ");
            put(out, text_of("type").unwrap_or(""));
            short_from(out, from)
        }
        k if k.starts_with("create_") => match f.children(def).iter().copied().find(|&c| dialect.name(f, c) == "object_reference") {
            Some(c) => {
                put(out, f.text(c));
                short_from(out, from)
            }
            None => false,
        },
        k if is_def(k) => match text_of("name") {
            Some(t) => {
                put(out, t);
                short_from(out, from)
            }
            None => false,
        },
        k if is_import(k) => {
            put(out, f.text(def));
            short_from(out, from)
        }
        k if binds(k) => bound(f, dialect, def, out) && short_from(out, from),
        _ => false,
    };
    if named || dialect.name(f, def).ends_with("_element") {
        // A Vue file's template, script and style: by kind and ordinal (no name).
        return;
    }
    own_digest(p, of, up, starts, g, out);
}

/// Writes the names a declaration binds into `out`: its declarators' names, or an assignment's targets, joined by
/// commas. Whether it binds any (an assignment with no target binds none: nothing is written).
fn bound(f: FactsIn<'_>, dialect: &Dialect, id: u32, out: &mut FrameVec<'_, u8>) -> bool {
    let from = out.len();
    let mut names = 0;
    let mut name = |out: &mut FrameVec<'_, u8>, s: &str| {
        if names > 0 {
            put(out, ",");
        }
        put(out, s);
        names += 1;
    };
    let mut stack: InlineVec<u32, 32> = InlineVec::default();
    stack.push(id);
    // A declaration's declarators, an expression statement's assignment, a field's property name: a short walk.
    while let Some(x) = stack.pop() {
        for &c in f.children(x).iter().rev() {
            if !f.named(c) {
                continue;
            }
            match dialect.name(f, c) {
                "variable_declarator" | "const_element" | "property_element" => {
                    if let Some(n) = dialect.field_child(f, c, "name").or_else(|| f.children(c).iter().copied().find(|&k| f.named(k))) {
                        name(out, f.text(n));
                    }
                }
                "assignment" | "augmented_assignment" => match dialect.field_child(f, c, "left") {
                    Some(left) => name(out, f.text(left)),
                    None => {
                        out.truncate(from);
                        return false;
                    }
                },
                "property_identifier" | "private_property_identifier" | "identifier" if dialect.name(f, x).contains("field") => {
                    name(out, f.text(c));
                    break;
                }
                "variable_name" if dialect.name(f, x) == "property_declaration" => name(out, f.text(c)),
                "pattern" | "identifier" if dialect.name(f, x) == "let_declaration" => {
                    name(out, f.text(c));
                    break;
                }
                _ => {}
            }
        }
    }
    names > 0
}

/// Writes a digest of granule `g`'s own text into `out`: its source with each nested granule cut out (standing for
/// its kind), 16 hex digits.
fn own_digest(p: &Projected, of: &[u32], up: &[u32], starts: &[u32], g: usize, out: &mut FrameVec<'_, u8>) {
    use sha2::{Digest, Sha256};
    let f = p.f();
    let l = starts[g] as usize;
    let id = p.node(l as u32) as usize;
    let (start, end) = (f.start(id as u32) as usize, f.end(id as u32) as usize);
    let mut h = Sha256::new();
    let mut at = start;
    // Local ids are in preorder: the subtree's are those ranked below its end.
    let until = p.ranks().rank(Item::Node, f.subtree_end(id as u32)) as usize;
    for x in l + 1..until {
        // A nested granule's start whose parent is in this one: cut it out.
        if of[x] as usize != g && of[up[x] as usize] as usize == g {
            let c = p.node(x as u32) as usize;
            h.update(&f.source.as_bytes()[at..f.start(c as u32) as usize]);
            h.update([0]);
            at = f.end(c as u32) as usize;
        }
    }
    h.update(&f.source.as_bytes()[at.min(end)..end]);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in &h.finalize()[..8] {
        out.extend_from_slice(&[HEX[(b >> 4) as usize], HEX[(b & 15) as usize]]);
    }
}

fn home(key: &[u8]) -> u32 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(key);
    (u64::from_le_bytes(d[..8].try_into().unwrap()) % u64::from(GRANULE_SLOTS)) as u32 + 1
}

/// Slots for each of `keys`, each from its home in (home, key) order, into `out` (by the key's index); `taken` marks
/// those already used.
fn settle(fr: &Frame, keys: &Keys<'_>, taken: &mut [u8], out: &mut [u32]) {
    let order = fr.take::<[u32; 2]>(keys.len());
    for (i, o) in order.iter_mut().enumerate() {
        *o = [home(keys.get(i)), i as u32];
    }
    order.sort_unstable_by(|a, b| a[0].cmp(&b[0]).then_with(|| keys.get(a[1] as usize).cmp(keys.get(b[1] as usize))));
    for &[h, i] in order.iter() {
        let mut s = h;
        while taken[s as usize] != 0 {
            s = if s == GRANULE_SLOTS { 1 } else { s + 1 };
        }
        taken[s as usize] = 1;
        out[i as usize] = s;
    }
}

/// Places the granules: each one's slots (`cut.at`, `cut.slots`), its first, then its continuations. `nodes` and
/// `all` are each granule's nodes, and its nodes and tokens.
fn place<'f>(fr: &'f Frame, cut: &mut Cut<'f>, nodes: &[u32], all: &[u32]) {
    let g = cut.starts.len();
    // Each granule's slots: its first, then the nodes' continuations (indexes past `INDEX`), then the tokens', so a
    // token never moves a node.
    let need = |n: u32| (n.max(1) - 1) / INDEX + 1;
    let mut at = fr.vec_with::<u32>(g + 1);
    at.push(0);
    for &n in all {
        at.push(at[at.len() - 1] + need(n));
    }
    let mut slots = fr.vec_with::<u32>(at[g] as usize);
    slots.resize(at[g] as usize, 0);
    let taken = fr.zeroed::<u8>(SPAN_FIRST as usize);
    taken[0] = 1;
    // The root's first slot is 0; the others' are settled by their keys.
    fr.sub(|fr| {
        let first = fr.take::<u32>(g);
        let mut keys = Keys::new(fr, g, cut.keys.text.len());
        // `keys.get(i)` is granule `i + 1`'s key.
        for i in 1..g {
            keys.text.extend_from_slice(cut.keys.get(i));
            keys.end();
        }
        settle(fr, &keys, taken, &mut first[1..]);
        for i in 1..g {
            slots[at[i] as usize] = first[i];
        }
    });
    for pass in 0..2u32 {
        fr.sub(|fr| {
            // Continuation `c` of granule `i` is keyed by its key, a nul, its pass and `c`.
            let span = |i: usize| if pass == 0 { (1, need(nodes[i])) } else { (need(nodes[i]), need(all[i])) };
            let count: usize = (0..g).map(|i| { let (have, want) = span(i); want.saturating_sub(have) as usize }).sum();
            let mut who = fr.vec_with::<[u32; 2]>(count);
            let mut keys = Keys::new(fr, count, count * (cut.keys.text.len() / g + 16));
            for i in 0..g {
                let (have, want) = span(i);
                for c in have..want {
                    keys.text.extend_from_slice(cut.keys.get(i));
                    put(&mut keys.text, "\0+");
                    put_dec(&mut keys.text, pass);
                    put(&mut keys.text, ".");
                    put_dec(&mut keys.text, c);
                    keys.end();
                    who.push([i as u32, c]);
                }
            }
            let got = fr.take::<u32>(who.len());
            settle(fr, &keys, taken, got);
            for (&[i, c], &s) in who.iter().zip(got.iter()) {
                slots[(at[i as usize] + c) as usize] = s;
            }
        });
    }
    cut.at = at;
    cut.slots = slots;
}

/// Each node's, then each token's, id's low bits, from the placed slots: the reference the table is checked against.
fn reference_low(fr: &Frame, cut: &Cut<'_>, tokens: &[u32]) -> Vector<u32> {
    let next = fr.zeroed::<u32>(cut.starts.len());
    let owners = cut.of.iter().copied().chain(tokens.iter().map(|&t| cut.of[t as usize]));
    owners
        .map(|o| {
            let i = next[o as usize];
            next[o as usize] += 1;
            cut.slots[(cut.at[o as usize] + i / INDEX) as usize] << INDEX_BITS | (i % INDEX)
        })
        .collect()
}

/// The low bits of span `i` (past the nodes and tokens): the top slots, down from the last.
pub(super) fn span_low(i: u32) -> u32 {
    let slot = (SLOTS - 1).saturating_sub((i >> INDEX_BITS).min(SPAN_SLOTS - 1));
    slot << INDEX_BITS | (i % INDEX)
}

/// The span index of low bits in the spans' slots, or `None` for a node's.
pub fn span_of(low: u32) -> Option<u32> {
    let slot = low >> INDEX_BITS;
    (slot >= SPAN_FIRST).then(|| (SLOTS - 1 - slot) << INDEX_BITS | (low % INDEX))
}

/// What a document's numbering came to (for measuring the bit widths): granules, slots past the first, the most
/// nodes and tokens in one granule, and how far it folded.
#[cfg(any(test, feature = "test-hooks"))]
pub fn census(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> (usize, usize, u32, u8) {
    for (i, fold) in [Fold::None, Fold::Statements, Fold::All].into_iter().enumerate() {
        let done = frame::op(|fr| {
            let cut = cut(fr, p, dialect, fold);
            let size = fr.zeroed::<u32>(cut.starts.len());
            for &o in cut.of.iter().chain(tokens.iter().map(|&t| &cut.of[t as usize])) {
                size[o as usize] += 1;
            }
            let continued: u32 = size.iter().map(|&n| n.saturating_sub(1) / INDEX).sum();
            (cut.starts.len() as u32 - 1 + continued <= GRANULE_SLOTS || fold == Fold::All)
                .then(|| (cut.starts.len(), continued as usize, size.iter().copied().max().unwrap_or(0), i as u8))
        });
        if let Some(c) = done {
            return c;
        }
    }
    unreachable!()
}

/// A document's granule table (decision 0353 step 2): with the projection's rank index (`Ranks`), what answers a tape
/// node's id, and the tape node of an id, with no per-node array. Written once where the granules are numbered, a
/// derived fact of the tape.
///
/// A granule is its start's subtree less its nested granules' subtrees. So a node's granule is the innermost one
/// whose subtree holds it, and its index is its rank among the granule's numbered nodes: those from the granule's
/// start to it, less those of the granules nested between (their counts' prefix sums). A token's is the granule's
/// node count and its rank among its tokens.
pub struct Table {
    /// Its columns, one after another in one region of the projection's block: column `c` is
    /// `cells[cols[c]..cols[c + 1]]` (the `Col`s below).
    cells: Cells<u32>,
    cols: [u32; COLS + 1],
    /// The order keys are preorder indexes: the runs or a granule's nodes outgrow the key's bits (`order`).
    preorder_keys: bool,
    /// Every granule folded into the root (`Fold::All`): no definition starts one, so a node's definition unit is
    /// climbed on the tree (`units`).
    whole: bool,
}

/// The table's columns (`Table::cells`), each of `u32`s or of items of `u32`s:
/// - `STARTS`, `ENDS`, `PARENTS`: each granule's start and its subtree's end (tape ids), and its parent granule
///   (`u32::MAX` for the root's), in preorder of the starts.
/// - `NODES`, `TOKENS`: the numbered nodes and tokens of granules `..g`, prefix sums, one longer than the granules.
/// - `AT`, `SLOTS`: granule `g`'s slots are `slots[at[g]..at[g + 1]]`, its first, then its continuations.
/// - `BY_SLOT`: `(slot, granule, continuation)`, by slot.
/// - `FIRST_KID`, `KIDS`: granule `g`'s child granules are `kids[first_kid[g]..first_kid[g + 1]]`, in order, each
///   with the numbered nodes and tokens of `g` before it: the stretches of `g` between them, found by bisection.
/// - `RUNS`: the run rank of each stretch of each granule (`order`): granule `g`'s stretch `s` (after `s` of its
///   child granules) is `runs[first_kid[g] + g + s]`, `u32::MAX` where the stretch holds no node.
/// - `RUN_AT`: each run's granule and stretch, by rank.
/// - `RANK_STARTS`, `RANK_ENDS` (nodes, then tokens): each granule's start and end as ranks, by item. With them a
///   numbered item's id is read from its number alone, no select.
/// - `JUMPS` (nodes, then tokens): by item, the last granule starting at or before item number `JUMP * b`, for each
///   `b`: a number's granule is searched between two of them (`num_granule`), not among all the granules. 4 bytes a
///   `JUMP` items.
const STARTS: usize = 0;
const ENDS: usize = 1;
const PARENTS: usize = 2;
const NODES: usize = 3;
const TOKENS: usize = 4;
const AT: usize = 5;
const SLOTS_COL: usize = 6;
const BY_SLOT: usize = 7;
const FIRST_KID: usize = 8;
const KIDS: usize = 9;
const RUNS: usize = 10;
const RUN_AT: usize = 11;
const RANK_STARTS: usize = 12;
const RANK_ENDS: usize = 14;
const JUMPS: usize = 16;
const COLS: usize = 18;

/// The items one of `Table::jumps` stands for.
const JUMP: u32 = rerail_clif::ids::JUMP;

/// A child granule, and its parent's own numbered nodes and tokens before its start.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Kid {
    granule: u32,
    before: [u32; 2],
}

// SAFETY: `#[repr(C)]`, three `u32`s: every byte a named field, no address.
unsafe impl rerail_blocks::Pod for Kid {}

/// `v`'s `u32`s.
fn words<T: rerail_blocks::Pod>(v: &[T]) -> &[u32] {
    const { assert!(size_of::<T>() % 4 == 0 && align_of::<T>() == 4) };
    // SAFETY: `T` is plain data of whole `u32`s, aligned as one.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v) / 4) }
}

/// `w` as `T`s (`words`' inverse).
fn items<T: rerail_blocks::Pod>(w: &[u32]) -> &[T] {
    const { assert!(size_of::<T>() % 4 == 0 && align_of::<T>() == 4) };
    // SAFETY: `T` is plain data of whole `u32`s, aligned as one: every pattern of them is a `T`.
    unsafe { std::slice::from_raw_parts(w.as_ptr().cast(), w.len() * 4 / size_of::<T>()) }
}

impl Table {
    fn col(&self, c: usize) -> &[u32] {
        &self.cells[self.cols[c] as usize..self.cols[c + 1] as usize]
    }
    fn starts(&self) -> &[u32] {
        self.col(STARTS)
    }
    fn ends(&self) -> &[u32] {
        self.col(ENDS)
    }
    fn parents(&self) -> &[u32] {
        self.col(PARENTS)
    }
    fn nodes(&self) -> &[u32] {
        self.col(NODES)
    }
    fn tokens(&self) -> &[u32] {
        self.col(TOKENS)
    }
    fn at(&self) -> &[u32] {
        self.col(AT)
    }
    fn slots(&self) -> &[u32] {
        self.col(SLOTS_COL)
    }
    fn by_slot(&self) -> &[[u32; 3]] {
        items(self.col(BY_SLOT))
    }
    fn first_kid(&self) -> &[u32] {
        self.col(FIRST_KID)
    }
    fn kids(&self) -> &[Kid] {
        items(self.col(KIDS))
    }
    fn runs(&self) -> &[u32] {
        self.col(RUNS)
    }
    fn run_at(&self) -> &[[u32; 2]] {
        items(self.col(RUN_AT))
    }

    /// The table of `cut`, placed, whose granules have `nodes` nodes and `all` nodes and tokens: computed in the
    /// frame, then written once, its columns one region of the projection's block.
    fn new(fr: &Frame, p: &Projected, cut: &Cut<'_>, nodes: &[u32], all: &[u32]) -> Self {
        let (f, r) = (p.f(), p.ranks());
        // A projection with no node (a withheld document) numbers nothing: no granule.
        let g = if p.len() == 0 { 0 } else { cut.starts.len() };
        let starts = fr.take::<u32>(g);
        for (s, &l) in starts.iter_mut().zip(&cut.starts[..g]) {
            *s = p.node(l);
        }
        let starts = &*starts;
        let ends = &*fr.collect(starts.iter().map(|&s| f.subtree_end(s)));
        let parents = &*fr.collect((0..g).map(|i| if i == 0 { u32::MAX } else { cut.of[cut.up[cut.starts[i] as usize] as usize] }));
        let prefix = |c: &dyn Fn(usize) -> u32| {
            let mut a = 0;
            fr.collect((0..=g).map(|i| {
                if i > 0 {
                    a += c(i - 1);
                }
                a
            }))
        };
        let node_sums = prefix(&|i| nodes[i]);
        let token_sums = prefix(&|i| all[i] - nodes[i]);
        let mut by_slot = fr.vec_with::<[u32; 3]>(cut.at[g] as usize);
        for i in 0..g {
            for (c, &slot) in cut.slots[cut.at[i] as usize..cut.at[i + 1] as usize].iter().enumerate() {
                by_slot.push([slot, i as u32, c as u32]);
            }
        }
        by_slot.sort_unstable();
        // Each granule's children (parents come first in preorder, so a counting pass places them), and its own
        // items before each: the stretch before it, and the stretches before that.
        let count = fr.zeroed::<u32>(g + 1);
        for &q in parents.iter().skip(1) {
            count[q as usize + 1] += 1;
        }
        let first_kid = prefix(&|i| count[i + 1]);
        let kids = fr.zeroed::<Kid>(g.saturating_sub(1));
        let mut fill = fr.vec_from(&first_kid[..]);
        for (h, &q) in parents.iter().enumerate().skip(1) {
            kids[fill[q as usize] as usize].granule = h as u32;
            fill[q as usize] += 1;
        }
        for q in 0..g {
            let (mut pos, mut own) = (starts[q], [0u32; 2]);
            for kid in &mut kids[first_kid[q] as usize..first_kid[q + 1] as usize] {
                let h = kid.granule as usize;
                for item in [Item::Node, Item::Token] {
                    own[item as usize] += r.rank(item, starts[h]) - r.rank(item, pos);
                }
                kid.before = own;
                pos = ends[h];
            }
        }
        // The runs (`order`): a run starts where a node's granule is not the one before it. A granule's stretch is how
        // many of its child granules started before the node.
        let runs = fr.take::<u32>(g + kids.len());
        runs.fill(u32::MAX);
        let passed = fr.zeroed::<u32>(g);
        let mut run_at = fr.vec::<[u32; 2]>();
        let mut next = 1;
        for (l, &q) in cut.of.iter().enumerate().take(if g == 0 { 0 } else { cut.of.len() }) {
            if next < g && cut.starts[next] as usize == l {
                passed[parents[next] as usize] += 1;
                next += 1;
            }
            if l == 0 || q != cut.of[l - 1] {
                let (q, s) = (q as usize, passed[q as usize]);
                runs[first_kid[q] as usize + q + s as usize] = run_at.len() as u32;
                run_at.push([q as u32, s]);
            }
        }
        let preorder_keys = run_at.len() as u64 > 1 << (32 - ORDER_INDEX_BITS) || nodes[..g].iter().any(|&n| n > 1 << ORDER_INDEX_BITS);
        let ranked = |at: &[u32], item: Item| fr.collect(at.iter().map(|&x| r.rank(item, x)));
        let rank_starts = [ranked(starts, Item::Node), ranked(starts, Item::Token)];
        let rank_ends = [ranked(ends, Item::Node), ranked(ends, Item::Token)];
        let jumped = |at: &[u32], item: Item| {
            fr.collect((0..=r.count(item) / JUMP).map(|q| at.partition_point(|&s| s <= q * JUMP).saturating_sub(1) as u32))
        };
        let jumps = [jumped(&rank_starts[0], Item::Node), jumped(&rank_starts[1], Item::Token)];
        // Written once into the projection's block, beside its rank index (decision 0353 step 2), where the device's
        // reads will find them too.
        let columns: [&[u32]; COLS] = [
            starts,
            ends,
            parents,
            &node_sums,
            &token_sums,
            &cut.at[..=g],
            &cut.slots[..cut.at[g] as usize],
            words(&by_slot),
            &first_kid,
            words(kids),
            runs,
            words(&run_at),
            &rank_starts[0],
            &rank_starts[1],
            &rank_ends[0],
            &rank_ends[1],
            &jumps[0],
            &jumps[1],
        ];
        let mut cols = [0u32; COLS + 1];
        for (c, col) in columns.iter().enumerate() {
            cols[c + 1] = cols[c] + col.len() as u32;
        }
        let cells = Cells::build(r.block(), cols[COLS] as usize, |out| {
            for (c, col) in columns.iter().enumerate() {
                // copy: derived (the table's columns, computed in the numbering's frame and written once here)
                out[cols[c] as usize..cols[c + 1] as usize].copy_from_slice(col);
            }
            cols[COLS] as usize
        })
        .expect("a batch's block holds its projection");
        Table { cells, cols, preorder_keys, whole: false }
    }

    /// Points the projection's descriptor's table runs at these columns (`rerail_clif::ids::Numbering`), so the reads
    /// written once find them: the table the projection keeps (`DefIds`), as it is built. A table numbered again (a
    /// reference, a census) is not described: dropped, it would leave the runs at columns that are gone.
    pub(super) fn describe(&self, r: &Ranks) {
        use rerail_clif::ids::Run;
        let d = r.desc_mut();
        let set = |run: *mut Run, c: &[u32]| unsafe { Run::set(run, c.as_ptr(), c.len()) };
        // SAFETY: the projection is being built, so the descriptor is its builder's alone; the columns are cells of
        // its block, which live as long as the projection.
        unsafe {
            (*d).granules = self.starts().len() as u32;
            (*d).preorder_keys = u32::from(self.preorder_keys);
            set(&raw mut (*d).starts, self.starts());
            set(&raw mut (*d).ends, self.ends());
            set(&raw mut (*d).parents, self.parents());
            set(&raw mut (*d).nodes, self.nodes());
            set(&raw mut (*d).tokens, self.tokens());
            set(&raw mut (*d).at, self.at());
            set(&raw mut (*d).slots, self.slots());
            Run::set(&raw mut (*d).by_slot, self.by_slot().as_ptr(), self.by_slot().len());
            set(&raw mut (*d).first_kid, self.first_kid());
            Run::set(&raw mut (*d).kids, self.kids().as_ptr(), self.kids().len());
            set(&raw mut (*d).runs, self.runs());
            Run::set(&raw mut (*d).run_at, self.run_at().as_ptr(), self.run_at().len());
            for i in 0..2 {
                set(&raw mut (*d).rank_starts[i], self.col(RANK_STARTS + i));
                set(&raw mut (*d).rank_ends[i], self.col(RANK_ENDS + i));
                set(&raw mut (*d).jumps[i], self.col(JUMPS + i));
            }
        }
    }

    /// The low bits of `item` number `k`'s id (a node's local id; a token's number among the tokens): no select
    /// (`Numbering::low_of`; the CLIF's `num_low_of`).
    #[inline]
    pub fn low_of(&self, r: &Ranks, item: Item, k: u32) -> Option<u32> {
        // SAFETY: the projection's descriptor, whose runs point at this table and `r`.
        let low = unsafe { r.numbering().low_of(item as u32, k) };
        (low != rerail_clif::ids::NONE).then_some(low)
    }

    /// `low_of`, the granule searched from `hint` (`Numbering::low_of_near`): probes in key order.
    #[inline]
    pub fn low_of_near(&self, r: &Ranks, item: Item, k: u32, hint: &mut u32) -> Option<u32> {
        // SAFETY: as `low_of`.
        let low = unsafe { r.numbering().low_of_near(item as u32, k, hint) };
        (low != rerail_clif::ids::NONE).then_some(low)
    }

    /// The innermost granule whose subtree holds tape node `x`, its start included (`None` outside the document's).
    pub fn granule_of(&self, x: u32) -> Option<usize> {
        let mut g = self.starts().partition_point(|&s| s <= x).checked_sub(1)?;
        while self.ends()[g] <= x {
            let up = self.parents()[g];
            if up == u32::MAX {
                return None;
            }
            g = up as usize;
        }
        Some(g)
    }
    /// Granule `g`'s start (a tape node) and its parent granule (`None` for the root's).
    pub fn start_up(&self, g: usize) -> (u32, Option<usize>) {
        (self.starts()[g], (self.parents()[g] != u32::MAX).then_some(self.parents()[g] as usize))
    }
    /// Whether every granule is folded into the root (`Fold::All`): no definition starts one.
    pub fn whole(&self) -> bool {
        self.whole
    }

    /// The item whose id's low 32 bits are `low` (`low_of`'s inverse): what it is and its number (`num_number_of`).
    #[inline]
    pub fn number_of(&self, r: &Ranks, low: u32) -> Option<(Item, u32)> {
        // SAFETY: as `low_of`.
        let (item, k) = unsafe { r.numbering().number_of(low) }?;
        Some((if item == 0 { Item::Node } else { Item::Token }, k))
    }

    /// Node number `l`'s order key (`order`), from its number alone (`num_order_of`).
    #[inline]
    pub fn order_of(&self, r: &Ranks, l: u32) -> Option<u32> {
        // SAFETY: as `low_of`.
        let key = unsafe { r.numbering().order_of(l) };
        (key != rerail_clif::ids::NONE).then_some(key)
    }

    /// The node number whose order key is `key` (`order_of`'s inverse, `num_number_of_order`).
    #[inline]
    pub fn number_of_order(&self, r: &Ranks, key: u32) -> Option<u32> {
        // SAFETY: as `low_of`.
        let l = unsafe { r.numbering().number_of_order(key) };
        (l != rerail_clif::ids::NONE).then_some(l)
    }

    /// Node `x`'s order key (`node_order`'s low 32 bits, `order`): its run's rank, then its index in its granule.
    pub fn order(&self, r: &Ranks, x: u32) -> Option<u32> {
        self.order_of(r, r.local(Item::Node, x)?)
    }

    /// The tape node whose order key is `key` (`order`'s inverse).
    pub fn node_of_order(&self, r: &Ranks, key: u32) -> Option<u32> {
        r.select(Item::Node, self.number_of_order(r, key)?)
    }

    /// Bytes the table holds.
    pub fn bytes(&self) -> usize {
        4 * (self.starts().len() * 3 + self.nodes().len() * 2 + self.at().len() + self.slots().len() + 3 * self.by_slot().len() + self.first_kid().len() + self.runs().len() + 2 * self.run_at().len() + 4 * self.starts().len() + self.col(JUMPS).len() + self.col(JUMPS + 1).len())
            + std::mem::size_of::<Kid>() * self.kids().len()
    }

    /// The most nodes and tokens of one granule.
    pub fn largest(&self) -> u32 {
        (0..self.starts().len()).map(|g| self.nodes()[g + 1] - self.nodes()[g] + self.tokens()[g + 1] - self.tokens()[g]).max().unwrap_or(0)
    }

    /// The granules.
    pub fn len(&self) -> usize {
        self.starts().len()
    }
    pub fn is_empty(&self) -> bool {
        self.starts().is_empty()
    }

    /// Tape node `x`'s id's low 32 bits, and what it is; `None` for a node the projection does not number.
    pub fn id(&self, r: &Ranks, x: u32) -> Option<(u32, Item)> {
        let item = if r.is(Item::Node, x) {
            Item::Node
        } else if r.is(Item::Token, x) {
            Item::Token
        } else {
            return None;
        };
        Some((self.low_of(r, item, r.rank(item, x))?, item))
    }

    /// Granule `g`'s item `index`'s low bits.
    #[inline]
    fn low(&self, g: usize, index: u32) -> u32 {
        self.slots()[(self.at()[g] + index / INDEX) as usize] << INDEX_BITS | (index % INDEX)
    }

    /// The tape node whose id's low 32 bits are `low` (`id`'s inverse), and what it is.
    pub fn node(&self, r: &Ranks, low: u32) -> Option<(u32, Item)> {
        let (item, k) = self.number_of(r, low)?;
        Some((r.select(item, k)?, item))
    }

    /// Every numbered node and token of the document, in preorder, with its id's low bits: one walk over the rank
    /// index's set bits that carries the innermost granule and its running counts.
    pub fn scan(&self, r: &Ranks, mut out: impl FnMut(u32, u32, Item)) {
        // The open granules, innermost last: (granule, its nodes so far, its tokens so far).
        let mut open: Vec<(usize, u32, u32)> = Vec::new();
        let mut next = 0;
        for (x, item) in r.iter_both() {
            while open.last().is_some_and(|&(q, _, _)| x >= self.ends()[q]) {
                open.pop();
            }
            if next < self.starts().len() && self.starts()[next] == x {
                open.push((next, 0, 0));
                next += 1;
            }
            let Some((q, n, t)) = open.last_mut() else { continue };
            if item == Item::Node {
                out(x, self.low(*q, *n), item);
                *n += 1;
            } else {
                out(x, self.low(*q, self.nodes()[*q + 1] - self.nodes()[*q] + *t), item);
                *t += 1;
            }
        }
    }
}

/// Every numbered `item` of a projection in order, with its number and its id's low bits: `(number, tape node, low)`.
/// One walk over the item's set bits that carries the open granules and their counts (`Table::scan`, for one item);
/// with no table (ids in preorder), the low bits are the local id.
pub fn ids<'a>(table: Option<&'a Table>, r: &'a Ranks, item: Item, nodes: u32) -> Ids<'a> {
    Ids { table, it: r.iter(item), item, open: Vec::new(), next: 0, past: if item == Item::Node { 0 } else { nodes } }
}

/// `ids`.
pub struct Ids<'a> {
    table: Option<&'a Table>,
    it: super::ranks::Iter<'a>,
    item: Item,
    /// The open granules, innermost last, each with its `item`s so far.
    open: Vec<(usize, u32)>,
    /// The next granule to open.
    next: usize,
    /// What a local id adds to the item's number (the nodes, for a token), where ids are in preorder.
    past: u32,
}

impl Iterator for Ids<'_> {
    type Item = (u32, u32, u32);
    #[inline]
    fn next(&mut self) -> Option<(u32, u32, u32)> {
        let (k, x) = self.it.next()?;
        let Some(t) = self.table else { return Some((k, x, self.past + k)) };
        // Open each granule starting at or before `x`, closing those it is past.
        while self.next < t.starts().len() && t.starts()[self.next] <= x {
            let s = t.starts()[self.next];
            while self.open.last().is_some_and(|&(q, _)| s >= t.ends()[q]) {
                self.open.pop();
            }
            self.open.push((self.next, 0));
            self.next += 1;
        }
        while self.open.last().is_some_and(|&(q, _)| x >= t.ends()[q]) {
            self.open.pop();
        }
        let (q, n) = self.open.last_mut().expect("a numbered item is in a granule");
        let index = match self.item {
            Item::Node => *n,
            _ => t.nodes()[*q + 1] - t.nodes()[*q] + *n,
        };
        *n += 1;
        Some((k, x, t.low(*q, index)))
    }
}

