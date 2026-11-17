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
//! positions, in order), and the `EXTRA_SLOTS` below them `Extra`'s spans' (`ExtraSpan`: core.rr's source, lines,
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

use super::{Dialect, Projected};
use crate::code_rule_dsl::facts::core::Facts;
use std::collections::HashMap;

pub(super) const SLOT_BITS: u32 = 14;
pub const INDEX_BITS: u32 = 32 - SLOT_BITS;
/// Indexes in one slot.
pub const INDEX: u32 = 1 << INDEX_BITS;
const SLOTS: u32 = 1 << SLOT_BITS;
/// The top slots, from `SLOTS - 1` down, hold a document's spans: up to `SPAN_SLOTS << INDEX_BITS` of them.
pub(super) const SPAN_SLOTS: u32 = 16;
/// The positions (`span_low`) a document may have: past them a run fails (`super::Batch::build`), never wraps.
pub const POSITIONS: u64 = SPAN_SLOTS as u64 * INDEX as u64;
/// The first slot of the spans'.
const SPAN_FIRST: u32 = SLOTS - SPAN_SLOTS;
/// Slots of `Extra`'s spans (`ExtraSpan`): one for each kind of line and for the comments, `MATCH_SLOTS` for the
/// matches, below the positions'.
const MATCH_SLOTS: u32 = 12;
pub const EXTRA_SLOTS: u32 = 4 + MATCH_SLOTS;
const EXTRA_FIRST: u32 = SPAN_FIRST - EXTRA_SLOTS;
/// Slots granules take: `1..EXTRA_FIRST`.
const GRANULE_SLOTS: u32 = EXTRA_FIRST - 1;

/// The kinds of `Extra`'s spans (core.rr's `span`s made beside the projection: `super::Extra::document`), each in slots
/// of its own between the granules' and the positions', so no node, position or other kind has its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtraSpan {
    /// The whole source (index 0) and its physical lines (`src_line`, from 1).
    Line,
    /// The `\n`-lines (`src_line_lf`, from 1).
    Lf,
    /// The tape's lines (`tape_line`, from 1).
    Tape,
    /// The comments no node is, from 1.
    Comment,
    /// Each `match_all`/`match_group`/`rx_span` match or group, from 0.
    Match,
}

impl ExtraSpan {
    const fn first(self) -> u32 {
        match self {
            ExtraSpan::Line => 0,
            ExtraSpan::Lf => 1,
            ExtraSpan::Tape => 2,
            ExtraSpan::Comment => 3,
            ExtraSpan::Match => 4,
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
    u8::from(is_def(kind)) * DEF | u8::from(is_wrapper(kind)) * WRAPPER | u8::from(is_body(kind)) * BODY
        | u8::from(is_container_owner(kind)) * OWNER
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

/// The ids of `p`'s nodes and of `tokens` (each token's node, in preorder of the tokens): the low 32 bits, written to
/// `low` (as many as nodes and tokens); and each node's order key (`order`), returned. With `in_place`, also the
/// granule table that answers the same ids with no array (`Table`).
pub fn number(p: &Projected, dialect: &Dialect, tokens: &[u32], low: &mut [u32], in_place: bool) -> (Vec<u32>, Option<Table>) {
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
        cut.slots = place(&cut, &nodes, tokens, low);
        let table = in_place.then(|| Table::new(p, &cut, &nodes, tokens));
        return (order(&cut.of), table);
    }
}

/// Bits of an order key's index inside its granule; the rest are its run's rank (`order`).
const ORDER_INDEX_BITS: u32 = 18;

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
    let n = p.core.len();
    let kind = |l: usize| dialect.name(f, p.core[l]);
    let is = |l: usize, flag: u8| dialect.granule_kind(dialect.code(f, p.core[l])) & flag != 0;
    let starts_span = crate::code_rule_dsl::logic::profile::span(|| "granule starts".into());
    let mut start: Vec<Option<Start>> = vec![None; n];
    // A wrapper's definition is in the wrapper's granule.
    let mut claimed = vec![false; n];
    if fold != Fold::All {
        for l in 1..n {
            let q = p.parent(l) as usize;
            let owner = |q: usize| {
                let o = p.parent(q) as usize;
                is(o, OWNER) && (start[o].is_some() || claimed[o])
            };
            let in_container = q == 0 || (is(q, BODY) && p.parent(q) != super::NONE && owner(q));
            if is(l, DEF) && !claimed[l] {
                start[l] = Some(Start::Def);
            } else if in_container && fold == Fold::None {
                start[l] = Some(Start::Stmt);
            }
            // A wrapper claims its first definition child, which then starts no granule of its own.
            if start[l].is_some() && is(l, WRAPPER) {
                if let Some(c) = p.children(l as u32).find(|&c| is(c as usize, DEF)) {
                    claimed[c as usize] = true;
                    start[l] = Some(Start::Def);
                }
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
            of[p.parent(l) as usize]
        };
    }
    drop(starts_span);
    let _keys_span = crate::code_rule_dsl::logic::profile::span(|| "granule keys".into());
    // Keys, parents first (preorder).
    let mut keys = vec![String::new()];
    let mut seen: HashMap<String, u32> = HashMap::new();
    for g in 1..starts.len() {
        let l = starts[g] as usize;
        let parent = &keys[of[p.parent(l) as usize] as usize];
        let base = format!("{parent}/{}:{}", kind(l), name(p, dialect, &of, &starts, g));
        let ord = seen.entry(base.clone()).or_insert(0);
        keys.push(format!("{base}#{ord}"));
        *ord += 1;
    }
    Cut { of, starts, keys, slots: Vec::new() }
}

/// Granule `g`'s name: what it defines or binds, an import's text, or its own text's digest.
fn name(p: &Projected, dialect: &Dialect, of: &[u32], starts: &[u32], g: usize) -> String {
    let f = &*p.facts;
    let l = starts[g];
    let id = p.core[l as usize];
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
    own_digest(p, of, starts, g)
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
fn own_digest(p: &Projected, of: &[u32], starts: &[u32], g: usize) -> String {
    use sha2::{Digest, Sha256};
    let f = &*p.facts;
    let l = starts[g] as usize;
    let id = p.core[l] as usize;
    let (start, end) = (f.start(id as u32) as usize, f.end(id as u32) as usize);
    let mut h = Sha256::new();
    let mut at = start;
    // Core IDs retain preorder. Find this subtree without materializing projection sizes.
    let until = p.core.partition_point(|&node| node < f.subtree_end(id as u32));
    for x in l + 1..until {
        // A nested granule's start whose parent is in this one: cut it out.
        if of[x] as usize != g && of[p.parent(x) as usize] as usize == g {
            let c = p.core[x] as usize;
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
fn place(cut: &Cut, nodes: &[u32], tokens: &[u32], low: &mut [u32]) -> Vec<Vec<u32>> {
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
    let mut next = vec![0u32; g];
    let owners = cut.of.iter().copied().chain(tokens.iter().map(|&t| cut.of[t as usize]));
    assert_eq!(low.len(), cut.of.len() + tokens.len(), "a low id for each node and token");
    for (slot, o) in low.iter_mut().zip(owners) {
        let i = next[o as usize];
        next[o as usize] += 1;
        *slot = slots[o as usize][(i / INDEX) as usize] << INDEX_BITS | (i % INDEX);
    }
    slots
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

/// A document's granule table (decision 0353 step 2): what answers a tape node's id, and the tape node of an id,
/// with no per-node array. Written once where the granules are numbered, a derived fact of the tape.
///
/// A granule is its start's subtree less its nested granules' subtrees. So a node's granule is the innermost one
/// whose subtree holds it, and its index is its rank among the granule's numbered nodes: those from the granule's
/// start to it, less those of the granules nested between (their counts' prefix sums). A token's is the granule's
/// node count and its rank among its tokens.
///
/// The ranks are read off a rank index (`Group`): for each 64 tape nodes, which are numbered nodes and which tokens,
/// and how many of each come before. 24 bytes for 64 nodes, 3 bits a node, where the array held 32 (and its way
/// back 64 more). A code alone cannot say which nodes are numbered (an anonymous node under an anonymous one, Python's
/// `is not`'s `is`, is no token), and testing codes 64 at a time cost about 1,200 instructions a group on hogql
/// (decision 0353, "Node identity"), so the bits are kept.
pub struct Table {
    /// Each granule's start and its subtree's end (tape ids), and its parent granule (`u32::MAX` for the root's), in
    /// preorder of the starts.
    starts: Vec<u32>,
    ends: Vec<u32>,
    parents: Vec<u32>,
    /// The numbered nodes and tokens of granules `..g`: prefix sums, one longer than the granules.
    nodes: Vec<u32>,
    tokens: Vec<u32>,
    /// Granule `g`'s slots are `slots[at[g]..at[g + 1]]`: its first, then its continuations.
    at: Vec<u32>,
    slots: Vec<u32>,
    /// `(slot, granule, continuation)`, by slot.
    by_slot: Vec<(u32, u32, u32)>,
    /// Granule `g`'s child granules are `kids[first_kid[g]..first_kid[g + 1]]`, in order, each with the numbered
    /// nodes and tokens of `g` before it: the stretches of `g` between them, found by bisection.
    first_kid: Vec<u32>,
    kids: Vec<Kid>,
    /// The rank index from tape node `64 * first` on, one past the document's last group.
    first: u32,
    groups: Vec<Group>,
}

/// 64 tape nodes of the rank index: bit j of `bits[item]` set when node `64 * group + j` is a numbered `item`, and
/// how many numbered of each item the document has before the group.
#[derive(Clone, Copy, Default)]
struct Group {
    bits: [u64; 2],
    before: [u32; 2],
}

/// A child granule, and its parent's own numbered nodes and tokens before its start.
#[derive(Clone, Copy)]
struct Kid {
    granule: u32,
    before: [u32; 2],
}

/// What a numbered tape node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Item {
    Node = 0,
    Token = 1,
}

impl Table {
    fn new(p: &Projected, cut: &Cut, nodes: &[u32], tokens: &[u32]) -> Self {
        let f = &*p.facts;
        // A projection with no node (a withheld document) numbers nothing: no granule, no group.
        let g = if p.core.is_empty() { 0 } else { cut.starts.len() };
        let starts: Vec<u32> = cut.starts[..g].iter().map(|&l| p.core[l as usize]).collect();
        let ends: Vec<u32> = starts.iter().map(|&s| f.subtree_end(s)).collect();
        let parents = (0..g).map(|i| if i == 0 { u32::MAX } else { cut.of[p.parent(cut.starts[i] as usize) as usize] }).collect();
        let mut ntok = vec![0u32; g];
        for &t in tokens {
            ntok[cut.of[t as usize] as usize] += 1;
        }
        let prefix = |c: &[u32]| std::iter::once(0).chain(c.iter().scan(0u32, |a, &n| { *a += n; Some(*a) })).collect::<Vec<u32>>();
        let at = prefix(&slots_len(&cut.slots[..g]));
        let mut by_slot: Vec<(u32, u32, u32)> = cut.slots[..g].iter().enumerate().flat_map(|(i, s)| s.iter().enumerate().map(move |(c, &slot)| (slot, i as u32, c as u32))).collect();
        by_slot.sort_unstable();
        // The rank index: the numbered nodes' bits, then the tokens', then the counts before each group.
        let first = starts.first().map_or(0, |&s| s / 64);
        let last = ends.first().map_or(0, |&e| e.div_ceil(64));
        let mut groups = vec![Group::default(); if g == 0 { 0 } else { (last - first) as usize + 1 }];
        let mut mark = |x: u32, item: Item| groups[(x / 64 - first) as usize].bits[item as usize] |= 1 << (x % 64);
        for &x in p.core.iter() {
            mark(x, Item::Node);
        }
        if !tokens.is_empty() {
            for (x, _) in p.token_order() {
                mark(x, Item::Token);
            }
        }
        let mut before = [0u32; 2];
        for group in &mut groups {
            group.before = before;
            before[0] += group.bits[0].count_ones();
            before[1] += group.bits[1].count_ones();
        }
        let mut t = Table { starts, ends, parents, nodes: prefix(&nodes[..g]), tokens: prefix(&ntok), at, slots: cut.slots[..g].concat(), by_slot, first_kid: Vec::new(), kids: Vec::new(), first, groups };
        // Each granule's children (parents come first in preorder, so a counting pass places them), and its own
        // items before each: the stretch before it, and the stretches before that.
        let mut count = vec![0u32; g + 1];
        for h in 1..g {
            count[t.parents[h] as usize + 1] += 1;
        }
        let first_kid: Vec<u32> = count.iter().scan(0u32, |a, &n| { *a += n; Some(*a) }).collect();
        let mut kids = vec![Kid { granule: 0, before: [0; 2] }; g.saturating_sub(1)];
        let mut fill = first_kid.clone();
        for h in 1..g {
            let q = t.parents[h] as usize;
            kids[fill[q] as usize].granule = h as u32;
            fill[q] += 1;
        }
        for q in 0..g {
            let (mut pos, mut own) = (t.starts[q], [0u32; 2]);
            for kid in &mut kids[first_kid[q] as usize..first_kid[q + 1] as usize] {
                let h = kid.granule as usize;
                for item in [Item::Node, Item::Token] {
                    own[item as usize] += t.rank(item, t.starts[h]) - t.rank(item, pos);
                }
                kid.before = own;
                pos = t.ends[h];
            }
        }
        (t.first_kid, t.kids) = (first_kid, kids);
        t
    }

    /// Bytes the table holds.
    pub fn bytes(&self) -> usize {
        4 * (self.starts.len() * 3 + self.nodes.len() * 2 + self.at.len() + self.slots.len() + 3 * self.by_slot.len() + self.first_kid.len())
            + std::mem::size_of::<Kid>() * self.kids.len()
            + std::mem::size_of::<Group>() * self.groups.len()
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

    /// The innermost granule whose subtree holds tape node `x`, and the last granule starting at or before it.
    #[inline]
    fn granule_of(&self, x: u32) -> Option<(usize, usize)> {
        let j = self.starts.partition_point(|&s| s <= x).checked_sub(1)?;
        let mut g = j;
        while x >= self.ends[g] {
            g = self.parents[g] as usize;
            if g == u32::MAX as usize {
                return None;
            }
        }
        Some((g, j))
    }

    /// Numbered `item`s before tape node `x` (within the document's groups).
    #[inline]
    fn rank(&self, item: Item, x: u32) -> u32 {
        let group = &self.groups[(x / 64 - self.first) as usize];
        group.before[item as usize] + (group.bits[item as usize] & ((1u64 << (x % 64)) - 1)).count_ones()
    }

    /// The tape node of numbered `item` number `r` (the document's, from 0).
    fn select(&self, item: Item, r: u32) -> Option<u32> {
        let i = self.groups.partition_point(|g| g.before[item as usize] <= r).checked_sub(1)?;
        let group = &self.groups[i];
        let m = group.bits[item as usize];
        let k = r - group.before[item as usize];
        if k >= m.count_ones() {
            return None;
        }
        Some((self.first + i as u32) * 64 + select64(m, k))
    }

    /// Tape node `x`'s id's low 32 bits, and what it is; `None` for a node the projection does not number.
    pub fn id(&self, x: u32) -> Option<(u32, Item)> {
        let group = self.groups.get((x / 64).checked_sub(self.first)? as usize)?;
        let bit = 1u64 << (x % 64);
        let item = if group.bits[0] & bit != 0 {
            Item::Node
        } else if group.bits[1] & bit != 0 {
            Item::Token
        } else {
            return None;
        };
        let (g, j) = self.granule_of(x)?;
        let own = self.rank(item, x) - self.rank(item, self.starts[g]);
        let index = match item {
            Item::Node => own - (self.nodes[j + 1] - self.nodes[g + 1]),
            Item::Token => self.nodes[g + 1] - self.nodes[g] + own - (self.tokens[j + 1] - self.tokens[g + 1]),
        };
        Some((self.low(g, index), item))
    }

    /// Granule `g`'s item `index`'s low bits.
    #[inline]
    fn low(&self, g: usize, index: u32) -> u32 {
        self.slots[(self.at[g] + index / INDEX) as usize] << INDEX_BITS | (index % INDEX)
    }

    /// The tape node whose id's low 32 bits are `low` (`id`'s inverse), and what it is.
    pub fn node(&self, low: u32) -> Option<(u32, Item)> {
        let slot = low >> INDEX_BITS;
        let at = self.by_slot.partition_point(|e| e.0 < slot);
        let &(s, g, c) = self.by_slot.get(at)?;
        if s != slot {
            return None;
        }
        let g = g as usize;
        let index = c * INDEX + low % INDEX;
        let own = self.nodes[g + 1] - self.nodes[g];
        let (item, k) = if index < own { (Item::Node, index) } else { (Item::Token, index - own) };
        let all = match item {
            Item::Node => own,
            Item::Token => self.tokens[g + 1] - self.tokens[g],
        };
        if k >= all {
            return None;
        }
        Some((self.own(g, item, k)?, item))
    }

    /// Granule `g`'s `k`-th numbered `item`: the stretch it lies in, the last whose child granule has at most `k` of
    /// `g`'s before it (by bisection), then a select from the stretch's start.
    fn own(&self, g: usize, item: Item, k: u32) -> Option<u32> {
        let kids = &self.kids[self.first_kid[g] as usize..self.first_kid[g + 1] as usize];
        let i = kids.partition_point(|kid| kid.before[item as usize] <= k);
        let (pos, before) = match i.checked_sub(1) {
            None => (self.starts[g], 0),
            Some(i) => (self.ends[kids[i].granule as usize], kids[i].before[item as usize]),
        };
        self.select_from(item, pos, k - before)
    }

    /// The `k`-th numbered `item` from tape node `pos` on: forward over the groups from `pos`'s, where a stretch's
    /// items mostly lie; past 8 groups, by bisection of the counts.
    fn select_from(&self, item: Item, pos: u32, mut k: u32) -> Option<u32> {
        let at = (pos / 64 - self.first) as usize;
        let mut m = self.groups.get(at)?.bits[item as usize] & (!0u64 << (pos % 64));
        for i in at..(at + 8).min(self.groups.len()) {
            if i > at {
                m = self.groups[i].bits[item as usize];
            }
            let c = m.count_ones();
            if k < c {
                return Some((self.first + i as u32) * 64 + select64(m, k));
            }
            k -= c;
        }
        self.select(item, self.rank(item, pos) + self.count_from(item, pos, k))
    }

    /// `k` plus the `item`s from `pos` to the end of the 8 groups `select_from` walked: the rank from `pos` it had
    /// left to go.
    fn count_from(&self, item: Item, pos: u32, k: u32) -> u32 {
        let at = (pos / 64 - self.first) as usize;
        let end = ((at + 8).min(self.groups.len() - 1) as u32 + self.first) * 64;
        k + self.rank(item, end) - self.rank(item, pos)
    }

    /// Every numbered node and token of the document, in preorder, with its id's low bits: one walk over the rank
    /// index's set bits that carries the innermost granule and its running counts.
    pub fn scan(&self, mut out: impl FnMut(u32, u32, Item)) {
        // The open granules, innermost last: (granule, its nodes so far, its tokens so far).
        let mut open: Vec<(usize, u32, u32)> = Vec::new();
        let mut next = 0;
        for (i, group) in self.groups.iter().enumerate() {
            let mut m = group.bits[0] | group.bits[1];
            while m != 0 {
                let j = m.trailing_zeros();
                m &= m - 1;
                let x = (self.first + i as u32) * 64 + j;
                while open.last().is_some_and(|&(q, _, _)| x >= self.ends[q]) {
                    open.pop();
                }
                if next < self.starts.len() && self.starts[next] == x {
                    open.push((next, 0, 0));
                    next += 1;
                }
                let Some((q, n, t)) = open.last_mut() else { continue };
                if group.bits[0] >> j & 1 != 0 {
                    out(x, self.low(*q, *n), Item::Node);
                    *n += 1;
                } else {
                    out(x, self.low(*q, self.nodes[*q + 1] - self.nodes[*q] + *t), Item::Token);
                    *t += 1;
                }
            }
        }
    }
}

/// The position of `m`'s set bit `k` (from 0; `k` below its popcount): the byte by the bytes' running popcounts,
/// then the bit within it.
#[inline]
fn select64(m: u64, k: u32) -> u32 {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let mut x = m - ((m >> 1) & 0x5555_5555_5555_5555);
    x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
    x = (x + (x >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    // Byte b of `sums`: the set bits of bytes 0..=b.
    let sums = x.wrapping_mul(ONES);
    // The bytes whose running count is at most `k` come before the bit's byte: one per high bit set below.
    let k8 = u64::from(k) * ONES;
    let le = ((k8 | 0x8080_8080_8080_8080) - sums) & 0x8080_8080_8080_8080;
    let byte = le.count_ones();
    let before = if byte == 0 { 0 } else { (sums >> (8 * (byte - 1))) & 0xff } as u32;
    let mut b = (m >> (8 * byte)) & 0xff;
    for _ in 0..k - before {
        b &= b - 1;
    }
    8 * byte + b.trailing_zeros()
}

fn slots_len(slots: &[Vec<u32>]) -> Vec<u32> {
    slots.iter().map(|s| s.len() as u32).collect()
}
