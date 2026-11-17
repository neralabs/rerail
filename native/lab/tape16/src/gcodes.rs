//! E25 (t39 step 17): a language's code table, every code the grammar allows, from its `node-types.json`.
//!
//! A code is a node's (kind, field, flags, supertype memberships) (0156 §1.2). The table must hold every code
//! a file can have, so it comes from the grammar, not from a corpus; a corpus only orders it (the most frequent
//! first, the rest after). This lists the codes the grammar allows and checks every node of a capture against
//! the list: a node whose code the list lacks is a hole in the derivation.
//!
//! Where a code can come from:
//! - **field**: a type's field lists a type; a supertype there stands for every kind under it, each with the
//!   supertypes on its path as memberships (a node's memberships are the hidden supertype nodes between it and
//!   its parent, `ts_tree_cursor_current_status`), or any inner part of that path (node-types.json names only
//!   the outermost supertype a place allows);
//! - **child**: the same for a type's unfielded children;
//! - **root**: the grammar's first rule, no field;
//! - **extra**: the grammar's extras, under any node, no field, flagged extra;
//! - **error**: an ERROR node under any field (or none), flagged error, as `Code::error` writes it, or flagged
//!   error and extra (an ERROR the parser skipped), with no field;
//! - **in error**: any kind under an ERROR node, no field, with any membership path it has anywhere;
//! - **missing**: a leaf the parser inserted, at any place a leaf of its kind can be, flagged missing.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub const EXTRA: u8 = 1;
pub const ERROR: u8 = 2;
pub const MISSING: u8 = 4;

/// A kind: its name and whether it is named.
pub type Kind = (String, bool);

/// A code by names: kind, field ("" for none), flags, memberships sorted by name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Tuple {
    pub kind: Kind,
    pub field: String,
    pub flags: u8,
    pub supers: Vec<String>,
}

pub const CTX: [&str; 7] = ["field", "child", "root", "extra", "error", "in error", "missing"];

/// Every code the grammar allows, each with the contexts (bits over `CTX`) that allow it.
pub struct Allowed {
    pub tuples: BTreeMap<Tuple, u8>,
    pub kinds: usize,
    pub fields: usize,
    pub supertypes: usize,
}

/// Where the language's grammar is: its `grammar.json`, and its `node-types.json` if shipped (else generated
/// from the grammar with the generator the build uses, tree-sitter-generate 0.27.1).
fn sources(tag: u8) -> Option<(PathBuf, Option<PathBuf>)> {
    let native = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let registry = || -> Option<PathBuf> {
        let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))?;
        std::fs::read_dir(home.join("registry/src")).ok()?.flatten().map(|e| e.path()).find(|p| p.join("tree-sitter-rust-0.24.2").exists())
    };
    let vendored = |dir: &str| Some((native.join("grammars").join(dir).join("src/grammar.json"), None));
    let shipped = |dir: &str| registry().map(|r| (r.join(dir).join("grammar.json"), Some(r.join(dir).join("node-types.json"))));
    match tag {
        1 => vendored("tree-sitter-python"),
        2 => vendored("tree-sitter-typescript/typescript"),
        3 => vendored("tree-sitter-typescript/tsx"),
        6 => vendored("tree-sitter-javascript"),
        10 => vendored("tree-sitter-sql"),
        12 => vendored("tree-sitter-vue"),
        7 => shipped("tree-sitter-php-0.24.2/php/src"),
        9 => shipped("tree-sitter-rust-0.24.2/src"),
        14 => shipped("tree-sitter-json-0.24.8/src"),
        _ => None,
    }
}

/// The language's grammar and node types, as JSON.
pub fn load(tag: u8, scratch: &Path) -> Result<(Value, Value), String> {
    let (grammar, types) = sources(tag).ok_or("no tree-sitter grammar for this language")?;
    let read = |p: &Path| -> Result<Value, String> {
        serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())
    };
    let types = match types {
        Some(t) => t,
        None => {
            let out = scratch.join(format!("node-types-{tag}"));
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let mut diags = Vec::new();
            tree_sitter_generate::generate_parser_in_directory(
                out.clone(),
                Some(out.clone()),
                Some(grammar.clone()),
                tree_sitter_generate::ABI_VERSION_MAX,
                None,
                None,
                false,
                tree_sitter_generate::OptLevel::default(),
                &mut diags,
            )
            .map_err(|e| format!("generating node types: {e}"))?;
            out.join("node-types.json")
        }
    };
    Ok((read(&grammar)?, read(&types)?))
}

fn kind_of(v: &Value) -> Kind {
    (v["type"].as_str().unwrap_or("").to_string(), v["named"].as_bool().unwrap_or(false))
}

pub fn allowed(grammar: &Value, types: &Value) -> Allowed {
    let list = types.as_array().cloned().unwrap_or_default();
    let mut subtypes: HashMap<Kind, Vec<Kind>> = HashMap::new();
    for t in &list {
        if let Some(s) = t["subtypes"].as_array() {
            subtypes.insert(kind_of(t), s.iter().map(kind_of).collect());
        }
    }
    // Every (kind, memberships) a listed type stands for: itself, or through a supertype each kind under it
    // with the supertypes on its path.
    fn paths(t: &Kind, subtypes: &HashMap<Kind, Vec<Kind>>, seen: &mut Vec<Kind>, out: &mut BTreeSet<(Kind, Vec<String>)>, above: &mut Vec<String>) {
        match subtypes.get(t) {
            // node-types.json lists the outermost supertype a place allows (`expression`), not the inner ones
            // the grammar also names there (`primary_expression`, or the kind itself): every suffix of the
            // path, the empty one included.
            None => {
                for i in 0..=above.len() {
                    let mut m = above[i..].to_vec();
                    m.sort();
                    m.dedup();
                    out.insert((t.clone(), m));
                }
            }
            Some(subs) => {
                if seen.contains(t) {
                    return;
                }
                seen.push(t.clone());
                above.push(t.0.clone());
                for s in subs {
                    paths(s, subtypes, seen, out, above);
                }
                above.pop();
                seen.pop();
            }
        }
    }
    let expand = |t: &Kind| {
        let mut out = BTreeSet::new();
        paths(t, &subtypes, &mut Vec::new(), &mut out, &mut Vec::new());
        out
    };
    let mut tuples: BTreeMap<Tuple, u8> = BTreeMap::new();
    let add = |tuples: &mut BTreeMap<Tuple, u8>, kind: Kind, field: &str, flags: u8, supers: Vec<String>, ctx: usize| {
        *tuples.entry(Tuple { kind, field: field.to_string(), flags, supers }).or_default() |= 1 << ctx;
    };
    let mut fields: BTreeSet<String> = BTreeSet::new();
    // Every kind that is ever a node, and every membership path each has anywhere.
    let mut kinds: BTreeSet<Kind> = BTreeSet::new();
    let mut leaves: BTreeSet<Kind> = BTreeSet::new();
    for t in &list {
        let k = kind_of(t);
        if subtypes.contains_key(&k) {
            continue;
        }
        if t["fields"].as_object().is_none_or(|f| f.is_empty()) && t["children"].is_null() {
            leaves.insert(k.clone());
        }
        kinds.insert(k);
    }
    for t in &list {
        if subtypes.contains_key(&kind_of(t)) {
            continue;
        }
        if let Some(fs) = t["fields"].as_object() {
            for (f, info) in fs {
                fields.insert(f.clone());
                for ty in info["types"].as_array().into_iter().flatten() {
                    for (k, m) in expand(&kind_of(ty)) {
                        kinds.insert(k.clone());
                        add(&mut tuples, k, f, 0, m, 0);
                    }
                }
            }
        }
        for ty in t["children"]["types"].as_array().into_iter().flatten() {
            for (k, m) in expand(&kind_of(ty)) {
                kinds.insert(k.clone());
                add(&mut tuples, k, "", 0, m, 1);
            }
        }
    }
    // Anonymous tokens are leaves.
    for k in &kinds {
        if !k.1 {
            leaves.insert(k.clone());
        }
    }
    let root = grammar["rules"].as_object().and_then(|r| r.keys().next().cloned()).unwrap_or_default();
    add(&mut tuples, (root, true), "", 0, vec![], 2);
    let extras: Vec<Kind> = grammar["extras"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["type"] == "SYMBOL")
        .filter_map(|e| e["name"].as_str())
        .filter(|n| !n.starts_with('_'))
        .map(|n| (n.to_string(), true))
        .collect();
    for e in &extras {
        kinds.insert(e.clone());
        add(&mut tuples, e.clone(), "", EXTRA, vec![], 3);
    }
    let error: Kind = ("ERROR".to_string(), true);
    for f in std::iter::once(String::new()).chain(fields.iter().cloned()) {
        add(&mut tuples, error.clone(), &f, ERROR, vec![], 4);
    }
    // An ERROR the parser skipped as an extra.
    add(&mut tuples, error.clone(), "", ERROR | EXTRA, vec![], 4);
    // Under an ERROR node: every kind, with no field, with each membership path it has anywhere (and none).
    let mut paths_of: BTreeMap<Kind, BTreeSet<Vec<String>>> = BTreeMap::new();
    for t in tuples.keys() {
        paths_of.entry(t.kind.clone()).or_default().insert(t.supers.clone());
    }
    for k in &kinds {
        let mut ps = paths_of.get(k).cloned().unwrap_or_default();
        ps.insert(vec![]);
        let flags = if extras.contains(k) { EXTRA } else { 0 };
        for p in ps {
            add(&mut tuples, k.clone(), "", flags, p, 5);
        }
    }
    // Missing: a leaf wherever a leaf of its kind can be.
    let leafy: Vec<Tuple> = tuples.keys().filter(|t| t.flags == 0 && leaves.contains(&t.kind)).cloned().collect();
    for t in leafy {
        add(&mut tuples, t.kind, &t.field, MISSING, t.supers, 6);
    }
    Allowed { tuples, kinds: kinds.len(), fields: fields.len(), supertypes: subtypes.len() }
}

/// `kinds.tsv` by id: (tag, id) to (name, named), (tag, field id) to name.
pub struct Ids {
    pub kinds: HashMap<(u8, u16), Kind>,
    pub fields: HashMap<(u8, u8), String>,
}

impl Ids {
    pub fn load() -> Ids {
        let (mut kinds, mut fields) = (HashMap::new(), HashMap::new());
        for line in include_str!("../kinds.tsv").lines() {
            let c: Vec<&str> = line.split('\t').collect();
            let Ok(tag) = c[0].parse::<u8>() else { continue };
            if let Some(id) = c[1].strip_prefix("field ") {
                fields.insert((tag, id.parse().unwrap()), c[2].to_string());
            } else if let Ok(id) = c[1].parse::<u16>() {
                kinds.insert((tag, id), (c[2].to_string(), c.get(3) == Some(&"1")));
            }
        }
        Ids { kinds, fields }
    }
}

#[derive(Default)]
pub struct Report {
    /// No node types declared (an own parser): `allowed` is the product bound.
    pub bound: bool,
    pub tag: u8,
    pub files: usize,
    pub nodes: u64,
    pub allowed: usize,
    pub by_ctx: [usize; 7],
    pub kinds: usize,
    pub fields: usize,
    pub supertypes: usize,
    /// Distinct codes seen, and those the grammar's list lacks (each with its node count).
    pub seen: usize,
    pub holes: Vec<(Tuple, u64)>,
    pub hole_nodes: u64,
    /// Codes seen by frequency: the share of nodes the first 255 cover, numbered from the whole capture.
    pub top255: f64,
    /// The table's size and what it takes: codes past the cutoff need a low byte (≤ 511) or two.
    pub low_bytes: u32,
}

/// A language Rerail parses itself (`.rr`, Blade) declares no node types: the bound is every kind under every
/// field (or none), plain, extra or missing, and ERROR under every field, with no supertypes.
pub fn own_bound(tag: u8, ids: &Ids) -> Allowed {
    let kinds: BTreeSet<Kind> = ids.kinds.iter().filter(|((t, id), _)| *t == tag && *id != 0).map(|(_, k)| k.clone()).collect();
    let fields: Vec<String> = std::iter::once(String::new()).chain(ids.fields.iter().filter(|((t, _), _)| *t == tag).map(|(_, f)| f.clone())).collect();
    let mut tuples = BTreeMap::new();
    for f in &fields {
        for k in &kinds {
            for flags in [0, EXTRA, MISSING] {
                tuples.insert(Tuple { kind: k.clone(), field: f.clone(), flags, supers: vec![] }, 1 << 0);
            }
        }
        tuples.insert(Tuple { kind: ("ERROR".into(), true), field: f.clone(), flags: ERROR, supers: vec![] }, 1 << 4);
    }
    tuples.insert(Tuple { kind: ("ERROR".into(), true), field: String::new(), flags: ERROR | EXTRA, supers: vec![] }, 1 << 4);
    Allowed { tuples, kinds: kinds.len(), fields: fields.len() - 1, supertypes: 0 }
}

pub fn run(tag: u8, files: &[&crate::capture::File], scratch: &Path) -> Result<Report, String> {
    let ids = Ids::load();
    let (a, bound) = match load(tag, scratch) {
        Ok((grammar, types)) => (allowed(&grammar, &types), false),
        Err(_) if ids.kinds.keys().any(|(t, _)| *t == tag) => (own_bound(tag, &ids), true),
        Err(e) => return Err(e),
    };
    let mut r = Report { bound, tag, files: files.len(), allowed: a.tuples.len(), kinds: a.kinds, fields: a.fields, supertypes: a.supertypes, ..Report::default() };
    for c in a.tuples.values() {
        for (k, slot) in r.by_ctx.iter_mut().enumerate() {
            *slot += usize::from(c & (1 << k) != 0);
        }
    }
    let mut seen: HashMap<Tuple, u64> = HashMap::new();
    for f in files {
        let tree = crate::v15::read(&f.image)?;
        let aliases = crate::v15::aliases(&f.image);
        for n in &tree.nodes {
            r.nodes += 1;
            let kind = if n.kind & 0x8000 != 0 {
                aliases.get(usize::from(n.kind & 0x7fff)).cloned().unwrap_or(("?alias".into(), true))
            } else if n.kind == 0 {
                ("ERROR".to_string(), true)
            } else {
                ids.kinds.get(&(tag, n.kind)).cloned().unwrap_or((format!("?{}", n.kind), true))
            };
            let field = if n.field == 0 { String::new() } else { ids.fields.get(&(tag, n.field)).cloned().unwrap_or(format!("?{}", n.field)) };
            let mut supers: Vec<String> = n.classes.iter().map(|&c| ids.kinds.get(&(tag, c)).map_or(format!("?{c}"), |k| k.0.clone())).collect();
            supers.sort();
            *seen.entry(Tuple { kind, field, flags: n.flags, supers }).or_default() += 1;
        }
    }
    r.seen = seen.len();
    let mut holes: Vec<(Tuple, u64)> = seen.iter().filter(|(t, _)| !a.tuples.contains_key(t)).map(|(t, c)| (t.clone(), *c)).collect();
    holes.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    r.hole_nodes = holes.iter().map(|h| h.1).sum();
    r.holes = holes;
    let mut counts: Vec<u64> = seen.values().copied().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    r.top255 = counts.iter().take(255).sum::<u64>() as f64 / r.nodes.max(1) as f64;
    let total = a.tuples.len() + r.holes.len();
    r.low_bytes = if total <= 256 { 0 } else if total <= 255 + 256 { 1 } else { 2 };
    Ok(r)
}

pub fn print(title: &str, r: &Report) {
    println!("### {title}\n");
    println!("{} files, {} nodes; the grammar: {} kinds, {} fields, {} supertypes\n", r.files, r.nodes, r.kinds, r.fields, r.supertypes);
    println!("| codes the grammar allows{} | {} |", if r.bound { " (bound: kinds × fields × flags; the parser declares no node types)" } else { "" }, r.allowed);
    println!("|---|---|");
    for (k, c) in CTX.iter().zip(r.by_ctx) {
        println!("| from {k} | {c} |");
    }
    println!("| codes seen in the capture | {} |", r.seen);
    println!("| seen, not allowed (holes) | {} codes, {} nodes |", r.holes.len(), r.hole_nodes);
    println!("| nodes the 255 most frequent seen codes cover | {:.2}% |", 100.0 * r.top255);
    println!("| low bytes a code past the cutoff takes | {} |", r.low_bytes);
    println!("| the table format 16 uses (`f16::grammar_codes`) | {} |\n", crate::f16::grammar_codes(r.tag));
    if !r.holes.is_empty() {
        println!("| hole: kind | field | flags | memberships | nodes |\n|---|---|---|---|---|");
        for (t, c) in r.holes.iter().take(25) {
            let k = if t.kind.1 { t.kind.0.clone() } else { format!("\"{}\"", t.kind.0) };
            println!("| {k} | {} | {} | {} | {c} |", t.field, t.flags, t.supers.join(", "));
        }
        println!();
    }
}
