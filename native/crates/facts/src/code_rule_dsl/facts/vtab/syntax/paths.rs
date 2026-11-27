//! Node paths (decision 0722): a node named by its granule and the steps down to it, coded by its language's tables
//! (`path_tables`, each language's source).
//!
//! This part holds what both directions read: a parent's **path children** (its named children, extras such as
//! comments left out, ERROR and MISSING nodes kept), each one's slot, and each grammar's tables made into codes by kind
//! and field once (`Codes`, held by its `Dialect`).

use crate::code_rule_dsl::facts::core::FactsIn;
use rerail_lang::code_rule_dsl::path_tables::PathTables;
use std::collections::HashMap;

/// The flags (`FactsIn::flags`) of a named node and of an extra.
const NAMED: u8 = 1;
const EXTRA: u8 = 2;

/// Whether node `c` is a path child of its parent: named, and no extra.
pub fn is_path_child(f: FactsIn<'_>, c: u32) -> bool {
    f.flags(c) & (NAMED | EXTRA) == NAMED
}

/// Node `id`'s path children in order, each with its field (0 for none).
pub fn path_children(f: FactsIn<'_>, id: u32, mut each: impl FnMut(u32, u16)) {
    f.each_child_field(id, |c, field| {
        if is_path_child(f, c) {
            each(c, field);
        }
    });
}

/// A slot of a parent kind as its grammar's codes: its field id (0 for none), whether it is kind-grouped, and its kinds
/// by name, most frequent first.
#[derive(Debug)]
pub struct SlotCode {
    pub field: u16,
    pub grouped: bool,
    pub kinds: Vec<&'static str>,
    /// Each kind's rank in `kinds`.
    pub rank: HashMap<&'static str, u32>,
}

/// A parent kind's slots, in its table's order.
#[derive(Debug, Default)]
pub struct ParentCode {
    pub slots: Vec<SlotCode>,
}

impl ParentCode {
    /// The index of the slot of `field`, if the table lists it.
    pub fn slot(&self, field: u16) -> Option<usize> {
        self.slots.iter().position(|s| s.field == field)
    }
}

/// A grammar's node path tables as its codes (`Dialect::paths`): each parent kind's, by kind code; a kind the tables
/// do not list has none, and all its children take the escape.
#[derive(Debug, Default)]
pub struct Codes {
    parents: Vec<Option<std::sync::Arc<ParentCode>>>,
    /// The grammar's field count: an escaped slot's field id is coded in `field_bits`.
    pub field_bits: u32,
    /// An escaped kind's code is coded in `kind_bits`: the grammar's codes, then one for ERROR.
    pub kind_bits: u32,
}

/// The bits a fixed-width code over `n` values takes.
pub fn width(n: usize) -> u32 {
    if n <= 1 { 0 } else { usize::BITS - (n - 1).leading_zeros() }
}

impl Codes {
    /// `tables` as the codes of a grammar of `kinds` kind names by code (`Dialect::names`) and `fields` field names by
    /// id (from 1); a name the grammar lacks is left out, so one family's tables serve each of its grammars.
    pub fn new(tables: &PathTables, kinds: &[&'static str], fields: &dyn Fn(&str) -> Option<u16>, field_count: usize) -> Codes {
        let mut by_name: HashMap<&str, std::sync::Arc<ParentCode>> = HashMap::new();
        let known: std::collections::HashSet<&str> = kinds.iter().copied().filter(|k| !k.is_empty()).collect();
        for p in tables.parents {
            if !known.contains(p.kind) {
                continue;
            }
            let slots = p
                .slots
                .iter()
                .filter_map(|s| {
                    let field = if s.field.is_empty() { 0 } else { fields(s.field)? };
                    let kinds: Vec<&'static str> = s.kinds.iter().copied().filter(|k| known.contains(k)).collect();
                    let rank = kinds.iter().enumerate().map(|(r, &k)| (k, r as u32)).collect();
                    Some(SlotCode { field, grouped: s.grouped, kinds, rank })
                })
                .collect();
            by_name.insert(p.kind, std::sync::Arc::new(ParentCode { slots }));
        }
        let parents = kinds.iter().map(|k| by_name.get(k).cloned()).collect();
        Codes { parents, field_bits: width(field_count + 1), kind_bits: width(kinds.len() + 1) }
    }

    /// The slots of parent kind `code`, where the tables list it.
    pub fn parent(&self, code: u16) -> Option<&ParentCode> {
        self.parents.get(usize::from(code)).and_then(|p| p.as_deref())
    }
}

/// What a sample's trees hold, slot by slot (`node-path-table`): each slot's children, the most one node's slot held,
/// and its children's kinds, counted.
#[derive(Default)]
pub struct Census {
    pub slots: std::collections::BTreeMap<(&'static str, &'static str), SlotCount>,
}

#[derive(Default)]
pub struct SlotCount {
    pub children: u64,
    pub most: u32,
    pub kinds: std::collections::BTreeMap<&'static str, u64>,
}

impl Census {
    /// Counts the slots of `f`'s tree: from its root, each path child's (`path_children`).
    pub fn add(&mut self, f: FactsIn<'_>, dialect: &super::Dialect) {
        if f.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        let mut here: Vec<(u16, u32)> = Vec::new();
        while let Some(p) = stack.pop() {
            let parent = dialect.code_name(dialect.code(f, p));
            here.clear();
            path_children(f, p, |c, field| {
                let name = if field == 0 { "" } else { dialect.catalog.field_name(field).unwrap_or("") };
                let s = self.slots.entry((parent, name)).or_default();
                s.children += 1;
                let kind = dialect.code_name(dialect.code(f, c));
                if kind != "ERROR" {
                    *s.kinds.entry(kind).or_default() += 1;
                }
                match here.iter_mut().find(|(g, _)| *g == field) {
                    Some((_, n)) => *n += 1,
                    None => here.push((field, 1)),
                }
                stack.push(c);
            });
            for &(field, n) in &here {
                let name = if field == 0 { "" } else { dialect.catalog.field_name(field).unwrap_or("") };
                let s = self.slots.get_mut(&(parent, name)).expect("counted");
                s.most = s.most.max(n);
            }
        }
    }
}

/// A grammar's node types (tree-sitter's `node-types.json`), as slots: each named kind's fields and children in no
/// field, each with the named kinds it may hold (supertypes expanded) and whether it holds several.
pub fn node_type_slots(json: &serde_json::Value) -> std::collections::BTreeMap<(String, String), (bool, std::collections::BTreeSet<String>)> {
    use std::collections::{BTreeMap, BTreeSet};
    let types = json.as_array().map_or(&[][..], Vec::as_slice);
    let mut sup: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for t in types {
        if let (Some(name), Some(subs)) = (t["type"].as_str(), t["subtypes"].as_array()) {
            sup.insert(name, subs.iter().filter(|s| s["named"] == true).filter_map(|s| s["type"].as_str()).collect());
        }
    }
    fn expand<'a>(t: &'a str, sup: &BTreeMap<&'a str, Vec<&'a str>>, seen: &mut Vec<&'a str>, out: &mut BTreeSet<String>) {
        match sup.get(t) {
            Some(subs) if !seen.contains(&t) => {
                seen.push(t);
                for s in subs {
                    expand(s, sup, seen, out);
                }
                seen.pop();
            }
            Some(_) => {}
            None => {
                out.insert(t.to_owned());
            }
        }
    }
    let mut slots = BTreeMap::new();
    for t in types {
        let Some(name) = t["type"].as_str() else { continue };
        if t["named"] != true || t.get("subtypes").is_some() {
            continue;
        }
        let mut add = |field: &str, spec: &serde_json::Value| {
            let mut kinds = BTreeSet::new();
            for k in spec["types"].as_array().map_or(&[][..], Vec::as_slice).iter().filter(|k| k["named"] == true) {
                if let Some(k) = k["type"].as_str() {
                    expand(k, &sup, &mut Vec::new(), &mut kinds);
                }
            }
            slots.insert((name.to_owned(), field.to_owned()), (spec["multiple"] == true, kinds));
        };
        if let Some(fields) = t["fields"].as_object() {
            for (f, spec) in fields {
                add(f, spec);
            }
        }
        if let Some(children) = t.get("children") {
            add("", children);
        }
    }
    slots
}

/// The Rust of a language's slots (`PathTables::parents`, `node-path-table`): every parent kind the sample or the node
/// types have, by name; its slots, the sample's most frequent first, then those it never had; a field is kind-grouped
/// where it holds several children (in its node types, or once in the sample), children in no field where the parent
/// is one of `lists`; a grouped slot's kinds the same way. A kind or field the family's grammars lack (`kind`, `field`:
/// a node type of another version of a grammar) is left out.
pub fn table_source(census: &Census, node_types: &[serde_json::Value], lists: &[&str], kind: &dyn Fn(&str) -> bool, field: &dyn Fn(&str) -> bool, header: &str) -> String {
    use std::collections::{BTreeMap, BTreeSet};
    let typed: Vec<_> = node_types.iter().map(node_type_slots).collect();
    // (parent, field) → (children seen, held several, kinds seen by count, kinds the node types allow)
    let mut slots: BTreeMap<(String, String), (u64, bool, BTreeMap<String, u64>, BTreeSet<String>)> = BTreeMap::new();
    for ((p, f), s) in &census.slots {
        let e = slots.entry(((*p).to_owned(), (*f).to_owned())).or_default();
        e.0 += s.children;
        e.1 |= s.most > 1;
        for (k, n) in &s.kinds {
            *e.2.entry((*k).to_owned()).or_default() += n;
        }
    }
    for t in &typed {
        for (key, (multiple, kinds)) in t {
            if !kind(&key.0) || !(key.1.is_empty() || field(&key.1)) {
                continue;
            }
            let e = slots.entry(key.clone()).or_default();
            e.1 |= *multiple;
            e.3.extend(kinds.iter().filter(|k| kind(k)).cloned());
        }
    }
    let mut parents: BTreeMap<&str, Vec<(&str, &(u64, bool, BTreeMap<String, u64>, BTreeSet<String>))>> = BTreeMap::new();
    for ((p, f), e) in &slots {
        parents.entry(p.as_str()).or_default().push((f.as_str(), e));
    }
    let quote = |s: &str| format!("{s:?}");
    let mut out = String::from(header);
    out.push_str("{\n    use ::rerail_lang::code_rule_dsl::path_tables::{PathParent as P, PathSlot as S};\n    &[\n");
    for (p, mut fs) in parents {
        fs.sort_by(|(a, x), (b, y)| y.0.cmp(&x.0).then(a.cmp(b)));
        out.push_str(&format!("        P {{ kind: {}, slots: &[\n", quote(p)));
        for (f, (_, several, seen, allowed)) in fs {
            let grouped = if f.is_empty() { lists.contains(&p) } else { *several };
            let kinds = if grouped {
                let mut ks: Vec<(&String, u64)> = seen.iter().map(|(k, n)| (k, *n)).collect();
                ks.sort_by(|(a, x), (b, y)| y.cmp(x).then(a.cmp(b)));
                let mut names: Vec<&str> = ks.iter().map(|(k, _)| k.as_str()).collect();
                names.extend(allowed.iter().filter(|k| !seen.contains_key(*k)).map(String::as_str));
                format!("&[{}]", names.iter().map(|k| quote(k)).collect::<Vec<_>>().join(", "))
            } else {
                "&[]".to_owned()
            };
            out.push_str(&format!("            S {{ field: {}, grouped: {grouped}, kinds: {kinds} }},\n", quote(f)));
        }
        out.push_str("        ] },\n");
    }
    out.push_str("    ]\n}\n");
    out
}
