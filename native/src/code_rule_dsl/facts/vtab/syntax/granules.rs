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

use super::cells::{Block, Cell, Cells};
use super::ranks::{Item, Ranks};
use super::{Dialect, Projected};
use crate::code_rule_dsl::facts::core::Facts;
use std::collections::HashMap;
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

/// The comment number (from 1) of a `Comment` span's low bits, as `extra_low(ExtraSpan::Comment, i)` made them.
pub fn comment_of(low: u32) -> Option<u32> {
    let first = EXTRA_FIRST + ExtraSpan::Comment.first();
    (low >> INDEX_BITS == first).then_some(low % INDEX)
}

/// What starts a granule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Start {
    /// A definition, anywhere (the innermost one).
    Def,
    /// A statement of a container.
    Stmt,
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
    (u8::from(is_def(kind)) * DEF) | (u8::from(is_wrapper(kind)) * WRAPPER) | (u8::from(is_body(kind)) * BODY) | (u8::from(is_container_owner(kind)) * OWNER)
}
pub const DEF: u8 = 1;
pub const WRAPPER: u8 = 2;
pub const BODY: u8 = 4;
pub const OWNER: u8 = 8;

/// Declarations named by what they bind.
fn binds(kind: &str) -> bool {
    matches!(kind, "lexical_declaration" | "variable_declaration" | "expression_statement" | "const_declaration" | "property_declaration" | "field_definition" | "public_field_definition" | "let_declaration")
}

fn is_import(kind: &str) -> bool {
    matches!(kind, "import_statement" | "import_from_statement" | "future_import_statement" | "use_declaration" | "namespace_use_declaration" | "extern_crate_declaration")
}

/// A document's granules: each node's, each granule's key and start, its first node and its kind of start.
pub(super) struct Cut {
    /// Each local node's granule, as an index into `starts`.
    pub(super) of: Vec<u32>,
    /// Each granule's first node (its start), in preorder; the root's is 0.
    pub(super) starts: Vec<u32>,
    pub(super) keys: Vec<String>,
    /// Each local node's parent (`Projected::parent`), read in the cut's walk.
    pub(super) up: Vec<u32>,
    /// Each granule's slots, once placed (`place`): its first, then its continuations.
    pub(super) slots: Vec<Vec<u32>>,
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
    pub low: Vec<u32>,
    /// Each node's order key.
    pub order: Vec<u32>,
}

/// The granule table of `p` (`Table`): what answers the ids of its nodes and of `tokens` (each token's node, in
/// preorder of the tokens) and its nodes' order keys in place. With `reference`, those ids and keys as arrays too,
/// to check the table against.
pub fn number(p: &Projected, dialect: &Dialect, tokens: &[u32], reference: Option<&mut Reference>) -> Table {
    let mut fold = Fold::None;
    loop {
        let cut = cut(p, dialect, fold);
        let mut nodes = vec![0u32; cut.starts.len()];
        for &g in &cut.of {
            nodes[g as usize] += 1;
        }
        let mut all = nodes.clone();
        for &t in tokens {
            all[cut.of[t as usize] as usize] += 1;
        }
        let continued: u32 = all.iter().map(|&n| n.saturating_sub(1) / INDEX).sum();
        let needed = cut.starts.len() as u32 - 1 + continued;
        if needed > GRANULE_SLOTS && fold < Fold::All {
            fold = if fold == Fold::None { Fold::Statements } else { Fold::All };
            continue;
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| "granule slots".into());
        let mut cut = cut;
        cut.slots = place(&cut, &nodes, tokens);
        if let Some(r) = reference {
            r.low = reference_low(&cut, tokens);
            r.order = order(&cut.of);
        }
        let mut table = Table::new(p, &cut, &nodes, tokens);
        table.whole = fold == Fold::All;
        return table;
    }
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
fn order(of: &[u32]) -> Vec<u32> {
    let mut index = vec![0u32; of.iter().max().map_or(1, |&m| m as usize + 1)];
    let mut rank = 0u32;
    let mut keys = Vec::with_capacity(of.len());
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
pub fn granule_of(p: &Projected, dialect: &Dialect) -> Vec<u32> {
    cut(p, dialect, Fold::None).of
}

/// The granules of `p`, folded as `fold` says.
fn cut(p: &Projected, dialect: &Dialect, fold: Fold) -> Cut {
    let f = &*p.facts;
    let n = p.len() as usize;
    let kind = |l: usize| dialect.name(f, p.node(l as u32));
    let starts_span = crate::code_rule_dsl::logic::profile::span(|| "granule starts".into());
    // The numbering's working state, one walk of the nodes: each node's parent and what its kind is to the granules.
    let (mut up, mut flags) = (vec![super::NONE; n], vec![0u8; n]);
    for (l, x, q) in p.walk() {
        up[l as usize] = q;
        flags[l as usize] = dialect.granule_kind(dialect.code(f, x));
    }
    let is = |l: usize, flag: u8| flags[l] & flag != 0;
    let mut start: Vec<Option<Start>> = vec![None; n];
    // A wrapper's definition is in the wrapper's granule.
    let mut claimed = vec![false; n];
    if fold != Fold::All {
        for l in 1..n {
            let q = up[l] as usize;
            let owner = |q: usize| {
                let o = up[q] as usize;
                is(o, OWNER) && (start[o].is_some() || claimed[o])
            };
            let in_container = q == 0 || (is(q, BODY) && up[q] != super::NONE && owner(q));
            if is(l, DEF) && !claimed[l] {
                start[l] = Some(Start::Def);
            } else if in_container && fold == Fold::None {
                start[l] = Some(Start::Stmt);
            }
            // A wrapper claims its first definition child, which then starts no granule of its own: wherever it is,
            // so a definition's decorators are its granule's in a function's body too (`units`).
            if is(l, WRAPPER)
                && let Some(c) = p.children(l as u32).find(|&c| is(c as usize, DEF)) {
                    claimed[c as usize] = true;
                    start[l] = Some(Start::Def);
                }
            if claimed[l] {
                start[l] = None;
            }
        }
    }
    let mut of = vec![0u32; n];
    let mut starts = vec![0u32];
    for l in 1..n {
        of[l] = if start[l].is_some() {
            starts.push(l as u32);
            starts.len() as u32 - 1
        } else {
            of[up[l] as usize]
        };
    }
    drop(starts_span);
    let _keys_span = crate::code_rule_dsl::logic::profile::span(|| "granule keys".into());
    // Keys, parents first (preorder).
    let mut keys = vec![String::new()];
    let mut seen: HashMap<String, u32> = HashMap::new();
    for g in 1..starts.len() {
        let l = starts[g] as usize;
        let parent = &keys[of[up[l] as usize] as usize];
        let base = format!("{parent}/{}:{}", kind(l), name(p, dialect, &of, &up, &starts, g));
        let ord = seen.entry(base.clone()).or_insert(0);
        keys.push(format!("{base}#{ord}"));
        *ord += 1;
    }
    Cut { of, starts, keys, up, slots: Vec::new() }
}

/// Granule `g`'s name: what it defines or binds, an import's text, or its own text's digest.
fn name(p: &Projected, dialect: &Dialect, of: &[u32], up: &[u32], starts: &[u32], g: usize) -> String {
    let f = &*p.facts;
    let l = starts[g];
    let id = p.node(l);
    let kind = dialect.name(f, id);
    // A wrapper is named by its definition, or by the declaration it exports.
    let inner = |want: fn(&str) -> bool| f.children(id).iter().copied().find(|&c| f.named(c) && want(dialect.name(f, c)));
    let def = if is_wrapper(kind) { inner(is_def).or_else(|| inner(binds)).unwrap_or(id) } else { id };
    let short = |s: &str| -> Option<String> {
        let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
        (!s.is_empty() && s.len() <= 160).then_some(s)
    };
    let text_of = |field: &str| dialect.field_child(f, def, field).map(|c| f.text(c));
    let named = match dialect.name(f, def) {
        "impl_item" => short(&format!("{} for {}", text_of("trait").unwrap_or(""), text_of("type").unwrap_or(""))),
        k if k.starts_with("create_") => f.children(def).iter().copied().find(|&c| dialect.name(f, c) == "object_reference").and_then(|c| short(f.text(c))),
        k if is_def(k) => text_of("name").and_then(short),
        k if is_import(k) => short(f.text(def)),
        k if binds(k) => bound(f, dialect, def).and_then(|s| short(&s)),
        _ => None,
    };
    if let Some(n) = named {
        return n;
    }
    if dialect.name(f, def).ends_with("_element") {
        // A Vue file's template, script and style: by kind and ordinal.
        return String::new();
    }
    own_digest(p, of, up, starts, g)
}

/// The names a declaration binds: its declarators' names, or an assignment's targets, joined.
fn bound(f: &Facts, dialect: &Dialect, id: u32) -> Option<String> {
    let mut names = Vec::new();
    let mut stack = vec![id];
    // A declaration's declarators, an expression statement's assignment, a field's property name: a short walk.
    while let Some(x) = stack.pop() {
        for &c in f.children(x).iter().rev() {
            if !f.named(c) {
                continue;
            }
            match dialect.name(f, c) {
                "variable_declarator" | "const_element" | "property_element" => {
                    if let Some(n) = dialect.field_child(f, c, "name").or_else(|| f.children(c).iter().copied().find(|&k| f.named(k))) {
                        names.push(f.text(n).to_owned());
                    }
                }
                "assignment" | "augmented_assignment" => {
                    names.push(f.text(dialect.field_child(f, c, "left")?).to_owned());
                }
                "property_identifier" | "private_property_identifier" | "identifier" if dialect.name(f, x).contains("field") => {
                    names.push(f.text(c).to_owned());
                    break;
                }
                "variable_name" if dialect.name(f, x) == "property_declaration" => names.push(f.text(c).to_owned()),
                "pattern" | "identifier" if dialect.name(f, x) == "let_declaration" => {
                    names.push(f.text(c).to_owned());
                    break;
                }
                _ => {}
            }
        }
    }
    (!names.is_empty()).then(|| names.join(","))
}

/// A digest of granule `g`'s own text: its source with each nested granule cut out (standing for its kind).
fn own_digest(p: &Projected, of: &[u32], up: &[u32], starts: &[u32], g: usize) -> String {
    use sha2::{Digest, Sha256};
    let f = &*p.facts;
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
    let d = h.finalize();
    d[..8].iter().map(|b| format!("{b:02x}")).collect()
}

fn home(key: &str) -> u32 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(key.as_bytes());
    (u64::from_le_bytes(d[..8].try_into().unwrap()) % u64::from(GRANULE_SLOTS)) as u32 + 1
}

/// Slots for `keys`, each from its home in (home, key) order; `taken` marks those already used.
fn settle(keys: &[(usize, String)], taken: &mut [bool], out: &mut HashMap<usize, u32>) {
    let mut order: Vec<(u32, &str, usize)> = keys.iter().map(|(i, k)| (home(k), k.as_str(), *i)).collect();
    order.sort_unstable();
    for (h, _, i) in order {
        let mut s = h;
        while taken[s as usize] {
            s = if s == GRANULE_SLOTS { 1 } else { s + 1 };
        }
        taken[s as usize] = true;
        out.insert(i, s);
    }
}

/// The low bits of every node and token: granules placed, then their continuations. Returns each granule's slots,
/// its first then its continuations.
fn place(cut: &Cut, nodes: &[u32], tokens: &[u32]) -> Vec<Vec<u32>> {
    let g = cut.starts.len();
    let mut ntok = vec![0u32; g];
    for &t in tokens {
        ntok[cut.of[t as usize] as usize] += 1;
    }
    let mut taken = vec![false; SPAN_FIRST as usize];
    taken[0] = true;
    // Each granule's slots: its first, then its continuations (indexes past `INDEX`).
    let mut slots: Vec<Vec<u32>> = vec![Vec::new(); g];
    slots[0].push(0);
    let mut first = HashMap::new();
    settle(&(1..g).map(|i| (i, cut.keys[i].clone())).collect::<Vec<_>>(), &mut taken, &mut first);
    for (i, s) in first {
        slots[i].push(s);
    }
    // Continuations: the nodes' first, then the tokens', so a token never moves a node.
    for (pass, counts) in [(0, nodes.to_vec()), (1, (0..g).map(|i| nodes[i] + ntok[i]).collect::<Vec<_>>())] {
        let mut more: Vec<(usize, String)> = Vec::new();
        let mut who: Vec<usize> = Vec::new();
        for i in 0..g {
            let need = (counts[i].max(1) - 1) / INDEX + 1;
            for c in slots[i].len() as u32..need {
                more.push((more.len(), format!("{}\0+{pass}.{c}", cut.keys[i])));
                who.push(i);
            }
        }
        let mut got = HashMap::new();
        settle(&more, &mut taken, &mut got);
        let mut placed: Vec<(usize, u32)> = got.into_iter().collect();
        placed.sort_unstable();
        for (m, s) in placed {
            slots[who[m]].push(s);
        }
    }
    slots
}

/// Each node's, then each token's, id's low bits, from the placed slots: the reference the table is checked against.
fn reference_low(cut: &Cut, tokens: &[u32]) -> Vec<u32> {
    let mut next = vec![0u32; cut.starts.len()];
    let owners = cut.of.iter().copied().chain(tokens.iter().map(|&t| cut.of[t as usize]));
    owners
        .map(|o| {
            let i = next[o as usize];
            next[o as usize] += 1;
            cut.slots[o as usize][(i / INDEX) as usize] << INDEX_BITS | (i % INDEX)
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
        let cut = cut(p, dialect, fold);
        let mut size = vec![0u32; cut.starts.len()];
        for &o in cut.of.iter().chain(tokens.iter().map(|&t| &cut.of[t as usize])) {
            size[o as usize] += 1;
        }
        let continued: u32 = size.iter().map(|&n| n.saturating_sub(1) / INDEX).sum();
        if cut.starts.len() as u32 - 1 + continued <= GRANULE_SLOTS || fold == Fold::All {
            return (cut.starts.len(), continued as usize, size.iter().copied().max().unwrap_or(0), i as u8);
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
    /// Each granule's start and its subtree's end (tape ids), and its parent granule (`u32::MAX` for the root's), in
    /// preorder of the starts.
    starts: Cells<u32>,
    ends: Cells<u32>,
    parents: Cells<u32>,
    /// The numbered nodes and tokens of granules `..g`: prefix sums, one longer than the granules.
    nodes: Cells<u32>,
    tokens: Cells<u32>,
    /// Granule `g`'s slots are `slots[at[g]..at[g + 1]]`: its first, then its continuations.
    at: Cells<u32>,
    slots: Cells<u32>,
    /// `(slot, granule, continuation)`, by slot.
    by_slot: Cells<[u32; 3]>,
    /// Granule `g`'s child granules are `kids[first_kid[g]..first_kid[g + 1]]`, in order, each with the numbered
    /// nodes and tokens of `g` before it: the stretches of `g` between them, found by bisection.
    first_kid: Cells<u32>,
    kids: Cells<Kid>,
    /// The run rank of each stretch of each granule (`order`): granule `g`'s stretch `s` (after `s` of its child
    /// granules) is `runs[first_kid[g] + g + s]`, `u32::MAX` where the stretch holds no node.
    runs: Cells<u32>,
    /// Each run's granule and stretch, by rank.
    run_at: Cells<[u32; 2]>,
    /// The order keys are preorder indexes: the runs or a granule's nodes outgrow the key's bits (`order`).
    preorder_keys: bool,
    /// Every granule folded into the root (`Fold::All`): no definition starts one, so a node's definition unit is
    /// climbed on the tree (`units`).
    whole: bool,
    /// Each granule's start and end as ranks, by item (`[nodes, tokens]`): the numbered items before its start and
    /// before its subtree's end. With them a numbered item's id is read from its number alone, no select.
    rank_starts: [Cells<u32>; 2],
    rank_ends: [Cells<u32>; 2],
    /// By item, the last granule starting at or before item number `JUMP * b`, for each `b`: a number's granule is
    /// searched between two of them (`num_granule`), not among all the granules. 4 bytes a `JUMP` items.
    jumps: [Cells<u32>; 2],
}

/// The items one of `Table::jumps` stands for.
const JUMP: u32 = rerail_clif::ids::JUMP;

/// `v`, a table column being computed, written into `block`.
fn placed<T: Cell>(block: &Arc<Block>, v: &[T]) -> Cells<T> {
    Cells::build(block, v.len(), |out| {
        // copy: derived (a table column, computed here and written once into the block)
        out.copy_from_slice(v);
        v.len()
    })
    .expect("a batch's block holds its projection")
}

/// A child granule, and its parent's own numbered nodes and tokens before its start.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct Kid {
    granule: u32,
    before: [u32; 2],
}

// SAFETY: `#[repr(C)]`, three `u32`s: every byte a named field, no address.
unsafe impl rerail_blocks::Pod for Kid {}

impl Table {
    fn new(p: &Projected, cut: &Cut, nodes: &[u32], tokens: &[u32]) -> Self {
        let (f, r) = (&*p.facts, p.ranks());
        // A projection with no node (a withheld document) numbers nothing: no granule.
        let g = if p.len() == 0 { 0 } else { cut.starts.len() };
        let starts: Vec<u32> = cut.starts[..g].iter().map(|&l| p.node(l)).collect();
        let ends: Vec<u32> = starts.iter().map(|&s| f.subtree_end(s)).collect();
        let parents: Vec<u32> = (0..g).map(|i| if i == 0 { u32::MAX } else { cut.of[cut.up[cut.starts[i] as usize] as usize] }).collect();
        let mut ntok = vec![0u32; g];
        for &t in tokens {
            ntok[cut.of[t as usize] as usize] += 1;
        }
        let prefix = |c: &[u32]| std::iter::once(0).chain(c.iter().scan(0u32, |a, &n| { *a += n; Some(*a) })).collect::<Vec<u32>>();
        let at = prefix(&cut.slots[..g].iter().map(|s| s.len() as u32).collect::<Vec<_>>());
        let mut by_slot: Vec<[u32; 3]> = cut.slots[..g].iter().enumerate().flat_map(|(i, s)| s.iter().enumerate().map(move |(c, &slot)| [slot, i as u32, c as u32])).collect();
        by_slot.sort_unstable();
        // Each granule's children (parents come first in preorder, so a counting pass places them), and its own
        // items before each: the stretch before it, and the stretches before that.
        let mut count = vec![0u32; g + 1];
        for &q in parents.iter().skip(1) {
            count[q as usize + 1] += 1;
        }
        let first_kid: Vec<u32> = count.iter().scan(0u32, |a, &n| { *a += n; Some(*a) }).collect();
        let mut kids = vec![Kid { granule: 0, before: [0; 2] }; g.saturating_sub(1)];
        let mut fill = first_kid.clone();
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
        let mut runs = vec![u32::MAX; g + kids.len()];
        let mut run_at: Vec<[u32; 2]> = Vec::new();
        let mut passed = vec![0u32; g];
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
        let ranked = |at: &[u32], item: Item| at.iter().map(|&x| r.rank(item, x)).collect::<Vec<u32>>();
        let rank_starts = [ranked(&starts, Item::Node), ranked(&starts, Item::Token)];
        let rank_ends = [ranked(&ends, Item::Node), ranked(&ends, Item::Token)];
        // Written once into the projection's block, beside its rank index (decision 0353 step 2), where the device's
        // reads will find them too.
        let b = r.block();
        let jumped = |at: &[u32], item: Item| {
            (0..=r.count(item) / JUMP).map(|q| at.partition_point(|&s| s <= q * JUMP).saturating_sub(1) as u32).collect::<Vec<u32>>()
        };
        let jumps = [jumped(&rank_starts[0], Item::Node), jumped(&rank_starts[1], Item::Token)];
        let [rs0, rs1] = rank_starts;
        let [re0, re1] = rank_ends;
        Table {
            starts: placed(b, &starts),
            ends: placed(b, &ends),
            parents: placed(b, &parents),
            nodes: placed(b, &prefix(&nodes[..g])),
            tokens: placed(b, &prefix(&ntok)),
            at: placed(b, &at),
            slots: placed(b, &cut.slots[..g].concat()),
            by_slot: placed(b, &by_slot),
            first_kid: placed(b, &first_kid),
            kids: placed(b, &kids),
            runs: placed(b, &runs),
            run_at: placed(b, &run_at),
            preorder_keys,
            whole: false,
            rank_starts: [placed(b, &rs0), placed(b, &rs1)],
            rank_ends: [placed(b, &re0), placed(b, &re1)],
            jumps: [placed(b, &jumps[0]), placed(b, &jumps[1])],
        }
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
            (*d).granules = self.starts.len() as u32;
            (*d).preorder_keys = u32::from(self.preorder_keys);
            set(&raw mut (*d).starts, &self.starts);
            set(&raw mut (*d).ends, &self.ends);
            set(&raw mut (*d).parents, &self.parents);
            set(&raw mut (*d).nodes, &self.nodes);
            set(&raw mut (*d).tokens, &self.tokens);
            set(&raw mut (*d).at, &self.at);
            set(&raw mut (*d).slots, &self.slots);
            Run::set(&raw mut (*d).by_slot, self.by_slot.as_ptr(), self.by_slot.len());
            set(&raw mut (*d).first_kid, &self.first_kid);
            Run::set(&raw mut (*d).kids, self.kids.as_ptr(), self.kids.len());
            set(&raw mut (*d).runs, &self.runs);
            Run::set(&raw mut (*d).run_at, self.run_at.as_ptr(), self.run_at.len());
            for i in 0..2 {
                set(&raw mut (*d).rank_starts[i], &self.rank_starts[i]);
                set(&raw mut (*d).rank_ends[i], &self.rank_ends[i]);
                set(&raw mut (*d).jumps[i], &self.jumps[i]);
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

    /// The start of the innermost granule whose subtree holds tape node `x` below its start (an ancestor of `x`),
    /// 0 when none does: where a parent search may start (`Facts::parent_within`).
    pub fn holding(&self, x: u32) -> u32 {
        let mut g = self.starts.partition_point(|&s| s < x);
        while g > 0 {
            g -= 1;
            if self.starts[g] < x && self.ends[g] > x {
                return self.starts[g];
            }
            // An earlier granule's subtree ended before `x`, or it starts at `x`: its parent's may hold it.
            let up = self.parents[g];
            if up == u32::MAX {
                return 0;
            }
            g = up as usize + 1;
        }
        0
    }

    /// The innermost granule whose subtree holds tape node `x`, its start included (`None` outside the document's).
    pub fn granule_of(&self, x: u32) -> Option<usize> {
        let mut g = self.starts.partition_point(|&s| s <= x).checked_sub(1)?;
        while self.ends[g] <= x {
            let up = self.parents[g];
            if up == u32::MAX {
                return None;
            }
            g = up as usize;
        }
        Some(g)
    }
    /// Granule `g`'s start (a tape node) and its parent granule (`None` for the root's).
    pub fn start_up(&self, g: usize) -> (u32, Option<usize>) {
        (self.starts[g], (self.parents[g] != u32::MAX).then_some(self.parents[g] as usize))
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
        4 * (self.starts.len() * 3 + self.nodes.len() * 2 + self.at.len() + self.slots.len() + 3 * self.by_slot.len() + self.first_kid.len() + self.runs.len() + 2 * self.run_at.len() + 4 * self.starts.len() + self.jumps[0].len() + self.jumps[1].len())
            + std::mem::size_of::<Kid>() * self.kids.len()
    }

    /// The most nodes and tokens of one granule.
    pub fn largest(&self) -> u32 {
        (0..self.starts.len()).map(|g| self.nodes[g + 1] - self.nodes[g] + self.tokens[g + 1] - self.tokens[g]).max().unwrap_or(0)
    }

    /// The granules.
    pub fn len(&self) -> usize {
        self.starts.len()
    }
    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
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
        self.slots[(self.at[g] + index / INDEX) as usize] << INDEX_BITS | (index % INDEX)
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
            while open.last().is_some_and(|&(q, _, _)| x >= self.ends[q]) {
                open.pop();
            }
            if next < self.starts.len() && self.starts[next] == x {
                open.push((next, 0, 0));
                next += 1;
            }
            let Some((q, n, t)) = open.last_mut() else { continue };
            if item == Item::Node {
                out(x, self.low(*q, *n), item);
                *n += 1;
            } else {
                out(x, self.low(*q, self.nodes[*q + 1] - self.nodes[*q] + *t), item);
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
        while self.next < t.starts.len() && t.starts[self.next] <= x {
            let s = t.starts[self.next];
            while self.open.last().is_some_and(|&(q, _)| s >= t.ends[q]) {
                self.open.pop();
            }
            self.open.push((self.next, 0));
            self.next += 1;
        }
        while self.open.last().is_some_and(|&(q, _)| x >= t.ends[q]) {
            self.open.pop();
        }
        let (q, n) = self.open.last_mut().expect("a numbered item is in a granule");
        let index = match self.item {
            Item::Node => *n,
            _ => t.nodes[*q + 1] - t.nodes[*q] + *n,
        };
        *n += 1;
        Some((k, x, t.low(*q, index)))
    }
}

