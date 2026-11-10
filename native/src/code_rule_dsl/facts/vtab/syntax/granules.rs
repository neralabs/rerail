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
//! document's own (its root); the top `SPAN_SLOTS` slots are its spans' (its text, lines and matches, which are
//! positions, in order).
//!
//! **Overflow** never falls back to preorder: up to `SPAN_SLOTS << INDEX_BITS` spans (4 M) and any number of nodes. A granule with more than `INDEX` nodes and tokens goes on in further
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
pub(super) const INDEX_BITS: u32 = 32 - SLOT_BITS;
/// Indexes in one slot.
pub(super) const INDEX: u32 = 1 << INDEX_BITS;
const SLOTS: u32 = 1 << SLOT_BITS;
/// The top slots, from `SLOTS - 1` down, hold a document's spans: up to `SPAN_SLOTS << INDEX_BITS` of them.
pub(super) const SPAN_SLOTS: u32 = 16;
/// The first slot of the spans'.
const SPAN_FIRST: u32 = SLOTS - SPAN_SLOTS;
/// Slots granules take: `1..SPAN_FIRST`.
const GRANULE_SLOTS: u32 = SPAN_FIRST - 1;

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
}

/// How far a document's granules are folded (more granules than slots).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Fold {
    None,
    Statements,
    All,
}

/// The ids of `p`'s nodes and of `tokens` (each token's node, in order): the low 32 bits.
pub(super) fn number(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> Vec<u32> {
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
        return place(&cut, &nodes, tokens);
    }
}

/// The granules of `p`, folded as `fold` says.
fn cut(p: &Projected, dialect: &Dialect, fold: Fold) -> Cut {
    let f = &*p.facts;
    let n = p.core.len();
    let kind = |l: usize| dialect.name(f, p.core[l]);
    let mut start: Vec<Option<Start>> = vec![None; n];
    // A wrapper's definition is in the wrapper's granule.
    let mut claimed = vec![false; n];
    if fold != Fold::All {
        for l in 1..n {
            let q = p.parent[l] as usize;
            let k = kind(l);
            let owner = |q: usize| {
                let o = p.parent[q] as usize;
                is_container_owner(kind(o)) && (start[o].is_some() || claimed[o])
            };
            let in_container = q == 0 || (is_body(kind(q)) && p.parent[q] != super::NONE && owner(q));
            if is_def(k) && !claimed[l] {
                start[l] = Some(Start::Def);
            } else if in_container && fold == Fold::None {
                start[l] = Some(Start::Stmt);
            }
            // A wrapper claims its first definition child, which then starts no granule of its own.
            if start[l].is_some() && is_wrapper(k) {
                if let Some(c) = p.children(l as u32).find(|&c| is_def(kind(c as usize))) {
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
            of[p.parent[l] as usize]
        };
    }
    // Keys, parents first (preorder).
    let mut keys = vec![String::new()];
    let mut seen: HashMap<String, u32> = HashMap::new();
    for g in 1..starts.len() {
        let l = starts[g] as usize;
        let parent = &keys[of[p.parent[l] as usize] as usize];
        let base = format!("{parent}/{}:{}", kind(l), name(p, dialect, &of, &starts, g));
        let ord = seen.entry(base.clone()).or_insert(0);
        keys.push(format!("{base}#{ord}"));
        *ord += 1;
    }
    Cut { of, starts, keys }
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
    let (start, end) = (f.start[id] as usize, f.end[id] as usize);
    let mut h = Sha256::new();
    let mut at = start;
    for x in l + 1..=l + p.size[l] as usize {
        // A nested granule's start whose parent is in this one: cut it out.
        if of[x] as usize != g && of[p.parent[x] as usize] as usize == g {
            let c = p.core[x] as usize;
            h.update(&f.source.as_bytes()[at..f.start[c] as usize]);
            h.update([0]);
            at = f.end[c] as usize;
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

/// The low bits of every node and token: granules placed, then their continuations.
fn place(cut: &Cut, nodes: &[u32], tokens: &[u32]) -> Vec<u32> {
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
    owners
        .map(|o| {
            let i = next[o as usize];
            next[o as usize] += 1;
            slots[o as usize][(i / INDEX) as usize] << INDEX_BITS | (i % INDEX)
        })
        .collect()
}

/// The low bits of span `i` (past the nodes and tokens): the top slots, down from the last.
pub(super) fn span_low(i: u32) -> u32 {
    let slot = (SLOTS - 1).saturating_sub((i >> INDEX_BITS).min(SPAN_SLOTS - 1));
    slot << INDEX_BITS | (i % INDEX)
}

/// The span index of low bits in the spans' slots, or `None` for a node's.
pub(super) fn span_of(low: u32) -> Option<u32> {
    let slot = low >> INDEX_BITS;
    (slot >= SPAN_FIRST).then(|| (SLOTS - 1 - slot) << INDEX_BITS | (low % INDEX))
}

/// What a document's numbering came to (for measuring the bit widths): granules, slots past the first, the most
/// nodes and tokens in one granule, and how far it folded.
#[cfg(test)]
pub(super) fn census(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> (usize, usize, u32, u8) {
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
