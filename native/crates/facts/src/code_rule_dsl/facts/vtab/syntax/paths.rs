//! Node paths (decision 0722): a node named by its granule and the steps down to it, coded by its language's tables
//! (`path_tables`, each language's source).
//!
//! This part holds what both directions read: a parent's **path children** (its named children, extras such as
//! comments left out, ERROR and MISSING nodes kept), each one's slot, and each grammar's tables made into codes by kind
//! and field once (`Codes`, held by its `Dialect`).

use crate::code_rule_dsl::facts::core::FactsIn;
use rerail_lang::code_rule_dsl::path_tables::PathTables;
use rustc_hash::FxHashMap;
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

/// A path's steps as bits, the first step's first bit first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bits(pub Vec<bool>);

impl Bits {
    /// `v`'s low `n` bits, its highest first.
    fn put(&mut self, v: u64, n: u32) {
        self.0.extend((0..n).rev().map(|i| v >> i & 1 != 0));
    }
    /// `v` in Elias gamma: `v + 1`'s length less one in zeros, then `v + 1`.
    fn gamma(&mut self, v: u32) {
        let x = u64::from(v) + 1;
        let n = u64::BITS - x.leading_zeros();
        self.put(0, n - 1);
        self.put(x, n);
    }
    pub fn len(&self) -> u32 {
        self.0.len() as u32
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// As hex digits, padded with zero bits to a whole digit (an anchor's text: its length says where they end).
    pub fn hex(&self) -> String {
        self.0.chunks(4).map(|c| char::from_digit(c.iter().enumerate().fold(0, |a, (i, &b)| a | u32::from(b) << (3 - i)), 16).unwrap()).collect()
    }
    /// `hex`'s inverse: the first `n` bits of `digits`.
    pub fn of_hex(digits: &str, n: u32) -> Option<Bits> {
        let mut b = Bits::default();
        for d in digits.chars() {
            b.put(u64::from(d.to_digit(16)?), 4);
        }
        (b.len() >= n && b.len() < n + 4).then(|| {
            b.0.truncate(n as usize);
            b
        })
    }
}

/// Bits read from the first: `None` past the end.
struct Reader<'a> {
    bits: &'a [bool],
    at: usize,
}

impl Reader<'_> {
    fn get(&mut self, n: u32) -> Option<u64> {
        let end = self.at.checked_add(n as usize).filter(|&e| e <= self.bits.len())?;
        let v = self.bits[self.at..end].iter().fold(0u64, |a, &b| a << 1 | u64::from(b));
        self.at = end;
        Some(v)
    }
    fn gamma(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !*self.bits.get(self.at)? {
            zeros += 1;
            self.at += 1;
            if zeros > 32 {
                return None;
            }
        }
        u32::try_from(self.get(zeros + 1)? - 1).ok()
    }
    fn done(&self) -> bool {
        self.at == self.bits.len()
    }
}

/// A parent's path children as its steps read them: each child, its slot's index in the parent's codes (`None`: a
/// slot the table lacks), its field and its kind code (ERROR's is the grammar's kind count).
struct Children {
    kids: Vec<(u32, Option<usize>, u16, u16)>,
    escaped: bool,
}

fn children_of(f: FactsIn<'_>, dialect: &super::Dialect, q: u32) -> Children {
    let parent = dialect.paths.parent(dialect.code(f, q));
    let mut kids = Vec::new();
    path_children(f, q, |c, field| {
        let code = dialect.code(f, c);
        let code = if code == u16::MAX { dialect.names.len() as u16 } else { code };
        kids.push((c, parent.and_then(|p| p.slot(field)), field, code));
    });
    let escaped = kids.iter().any(|k| k.1.is_none());
    Children { kids, escaped }
}

/// A parent's path children and each one's step (decision 0722 §2, as built), made in one pass: a slot's ordinals
/// counted as they go, so a parent of many children costs one read of them, not one each. Its only child, where that
/// step costs no bits (one child, in its kind's one slot, not kind-grouped): a path down a run of them codes how far it
/// goes as one number (`DocPaths::down`), or the decoder could not tell a node from its only child. Steps are prefix
/// free among siblings, so a step is read by its bits (`by_bits`).
pub struct Steps {
    pub kids: Vec<u32>,
    pub bits: Vec<Bits>,
    pub only: Option<u32>,
    by_bits: FxHashMap<Vec<bool>, u32>,
    index: FxHashMap<u32, u32>,
    longest: usize,
}

impl Steps {
    fn of(f: FactsIn<'_>, dialect: &super::Dialect, q: u32) -> Steps {
        let codes = &dialect.paths;
        let parent = codes.parent(dialect.code(f, q));
        let n = parent.map_or(0, |p| p.slots.len());
        let ch = children_of(f, dialect, q);
        let w = width(if ch.escaped { n + 1 } else { n });
        let mut held: HashMap<u16, u32> = HashMap::new();
        for k in &ch.kids {
            *held.entry(k.2).or_default() += 1;
        }
        let (mut in_slot, mut of_kind): (HashMap<u16, u32>, HashMap<(u16, u16), u32>) = Default::default();
        let mut bits = Vec::with_capacity(ch.kids.len());
        for &(_, slot, field, kind) in &ch.kids {
            let mut b = Bits::default();
            match slot {
                Some(s) => b.put(s as u64, w),
                None => {
                    b.put(n as u64, w);
                    b.put(u64::from(field), codes.field_bits);
                }
            }
            let before = in_slot.entry(field).or_default();
            let ordinal = *before;
            *before += 1;
            let before_kind = of_kind.entry((field, kind)).or_default();
            let ordinal_kind = *before_kind;
            *before_kind += 1;
            match slot.and_then(|s| parent.map(|p| &p.slots[s])).filter(|s| s.grouped) {
                Some(sc) => {
                    let name = dialect.code_name(if usize::from(kind) == dialect.names.len() { u16::MAX } else { kind });
                    match sc.rank.get(name) {
                        Some(&r) => b.gamma(r),
                        None => {
                            b.gamma(sc.kinds.len() as u32);
                            b.put(u64::from(kind), codes.kind_bits);
                        }
                    }
                    b.gamma(ordinal_kind);
                }
                None if held[&field] > 1 => b.gamma(ordinal),
                None => {}
            }
            bits.push(b);
        }
        let kids: Vec<u32> = ch.kids.iter().map(|k| k.0).collect();
        let only = (kids.len() == 1 && bits[0].is_empty()).then(|| kids[0]);
        let by_bits = kids.iter().zip(&bits).map(|(&c, b)| (b.0.clone(), c)).collect();
        let longest = bits.iter().map(|b| b.0.len()).max().unwrap_or(0);
        let index = kids.iter().enumerate().map(|(i, &c)| (c, i as u32)).collect();
        Steps { kids, bits, only, by_bits, index, longest }
    }
    /// The step to child `c`.
    pub fn to(&self, c: u32) -> Option<&Bits> {
        self.index.get(&c).map(|&i| &self.bits[i as usize])
    }
    /// The child whose step `r`'s next bits are, the bits read.
    fn read(&self, r: &mut Reader<'_>) -> Option<u32> {
        for n in 1..=self.longest.min(r.bits.len() - r.at) {
            if let Some(&c) = self.by_bits.get(&r.bits[r.at..r.at + n]) {
                r.at += n;
                return Some(c);
            }
        }
        None
    }
}

/// The steps a word holds past its stop bit: 31.
pub const WORD_STEPS: u32 = 31;
/// An anchor is placed at most this many bits below its base.
pub const ANCHOR_REACH: u32 = 16;

/// The atoms a path's bases are: the run's interning, or a test's.
pub trait Atoms {
    fn atom(&self, text: &str) -> u32;
    fn text(&self, atom: u32) -> Option<String>;
}

/// A base's text less its anchors' (`\t@<hex>.<bits>` each, after its granule's): its granule's base text.
pub fn unit_of(text: &str) -> Option<&str> {
    Some(&text[..text.find("\t@").unwrap_or(text.len())])
}

/// A tape value's root text (decision 0722 §7): its identity, its grammar and text (0431: one per grammar and text,
/// whatever holds it), after a `\u{1}`; the text escaped so the root holds no tab. A tape value is no file's.
pub fn tape_root(grammar: &str, text: &str) -> String {
    format!("\u{1}{grammar}:{}", text.replace('\\', "\\\\").replace('\t', "\\t"))
}

/// A tape value's grammar and text, from its root text (`tape_root`); `None` for a file's root.
pub fn tape_of_root(root: &str) -> Option<(&str, String)> {
    let (grammar, text) = root.strip_prefix('\u{1}')?.split_once(':')?;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            },
            (c, false) => out.push(c),
        }
    }
    Some((grammar, out))
}

/// A base's root text: its document's path (and a guest's key), before its granule's key.
pub fn root_of(text: &str) -> Option<&str> {
    unit_of(text)?.rsplit_once('\t').map(|(root, _)| root)
}

/// One document's node paths (decision 0722): its granules' keys, found by key, and each granule's anchors, placed
/// the first time a path of the granule is made. Made over a whole projection of the document (`Projected::new`), so
/// its granules are the numbering's whatever a batch narrowed.
pub struct DocPaths {
    path: String,
    keys: Vec<String>,
    by_key: HashMap<String, u32>,
    anchors: std::sync::Mutex<FxHashMap<u32, std::sync::Arc<std::collections::BTreeSet<u32>>>>,
    /// Each parent's steps, made the first time a path passes it.
    steps: std::sync::Mutex<FxHashMap<u32, std::sync::Arc<Steps>>>,
}

impl DocPaths {
    /// `p`'s paths, the document at `path`; `None` where it has no granules (`RERAIL_NODE_IDS=preorder`).
    pub fn new(p: &super::Projected, dialect: &super::Dialect, path: &str) -> Option<DocPaths> {
        let keys = p.path_table(dialect)?.keys(p, dialect);
        let by_key = keys.iter().enumerate().map(|(g, k)| (k.clone(), g as u32)).collect();
        Some(DocPaths { path: path.to_owned(), keys, by_key, anchors: Default::default(), steps: Default::default() })
    }

    /// Parent `q`'s steps.
    pub fn steps(&self, f: FactsIn<'_>, dialect: &super::Dialect, q: u32) -> std::sync::Arc<Steps> {
        let mut cache = self.steps.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        std::sync::Arc::clone(cache.entry(q).or_insert_with(|| std::sync::Arc::new(Steps::of(f, dialect, q))))
    }

    /// The steps from `from` down through `nodes` (each a path child of the one before): each node's step, but where a
    /// node has an only child (`Steps::only`), how many nodes of that run the path takes, less one, in gamma.
    pub fn down(&self, f: FactsIn<'_>, dialect: &super::Dialect, from: u32, nodes: &[u32]) -> Option<Bits> {
        let mut b = Bits::default();
        let (mut q, mut i) = (from, 0);
        while i < nodes.len() {
            let mut st = self.steps(f, dialect, q);
            if st.only.is_some() {
                let mut k = 0;
                while i < nodes.len() && st.only == Some(nodes[i]) {
                    q = nodes[i];
                    i += 1;
                    k += 1;
                    if i < nodes.len() {
                        st = self.steps(f, dialect, q);
                    }
                }
                if k == 0 {
                    return None;
                }
                b.gamma(k - 1);
            } else {
                b.0.extend_from_slice(&st.to(nodes[i])?.0);
                q = nodes[i];
                i += 1;
            }
        }
        Some(b)
    }

    /// The node `bits` name below `from` (`down`'s inverse): `None` where they name none.
    fn read_down(&self, f: FactsIn<'_>, dialect: &super::Dialect, from: u32, bits: &Bits) -> Option<u32> {
        let mut r = Reader { bits: &bits.0, at: 0 };
        let mut x = from;
        while !r.done() {
            let st = self.steps(f, dialect, x);
            if st.only.is_some() {
                for _ in 0..r.gamma()? + 1 {
                    x = self.steps(f, dialect, x).only?;
                }
            } else {
                x = st.read(&mut r)?;
            }
        }
        Some(x)
    }

    /// The text of granule `g`'s base atom: the document's path, a tab, its key.
    fn base_text(&self, g: usize) -> String {
        format!("{}\t{}", self.path, self.keys[g])
    }

    /// Node `x`'s path: its base's atom, then its stop bit and steps. `None` for a node no path reaches (an extra, an
    /// anonymous node, one outside the document's granules).
    pub fn encode(&self, p: &super::Projected, dialect: &super::Dialect, atoms: &dyn Atoms, x: u32) -> Option<u64> {
        let (f, table) = (p.f(), p.path_table(dialect)?);
        let g = table.granule_of(x)?;
        let (start, _) = table.start_up(g);
        // The chain from the granule's start down to x.
        let mut chain = Vec::new();
        let mut c = x;
        while c != start {
            if !is_path_child(f, c) {
                return None;
            }
            chain.push(c);
            c = f.parent(c)?;
        }
        chain.reverse();
        let anchors = self.anchors(p, dialect, g);
        let mut base = self.base_text(g);
        let (mut from, mut at) = (start, 0);
        for (i, &c) in chain.iter().enumerate() {
            if anchors.contains(&c) {
                let steps = self.down(f, dialect, from, &chain[at..=i])?;
                base = format!("{base}\t@{}.{}", steps.hex(), steps.len());
                (from, at) = (c, i + 1);
            }
        }
        let steps = self.down(f, dialect, from, &chain[at..])?;
        debug_assert!(steps.len() <= WORD_STEPS, "anchors keep a path's steps in its word");
        let low = steps.0.iter().fold(1u64, |a, &b| a << 1 | u64::from(b));
        Some(u64::from(atoms.atom(&base)) << 32 | low)
    }

    /// The node path `word` names in this document: `None` for another document's path, or one whose granule or
    /// steps its tape no longer has.
    pub fn decode(&self, p: &super::Projected, dialect: &super::Dialect, atoms: &dyn Atoms, word: u64) -> Option<u32> {
        self.decode_text(p, dialect, atoms.text((word >> 32) as u32)?, word as u32)
    }

    /// The node a path of base text `text` and low 32 bits `low` names (`decode`, its atom's text read).
    pub fn decode_text(&self, p: &super::Projected, dialect: &super::Dialect, mut text: String, low: u32) -> Option<u32> {
        let (f, table) = (p.f(), p.path_table(dialect)?);
        // Its anchors' steps, the outermost first.
        let mut anchored = Vec::new();
        while let Some(t) = text.rfind("\t@") {
            let (digits, n) = text[t + 2..].split_once('.')?;
            anchored.push(Bits::of_hex(digits, n.parse().ok()?)?);
            text.truncate(t);
        }
        anchored.reverse();
        let (path, key) = text.rsplit_once('\t')?;
        if path != self.path {
            return None;
        }
        let g = *self.by_key.get(key)? as usize;
        let (mut x, _) = table.start_up(g);
        let n = (u32::BITS - 1).checked_sub(low.leading_zeros())?;
        let mut own = Bits::default();
        own.put(u64::from(low), n);
        anchored.push(own);
        for bits in &anchored {
            x = self.read_down(f, dialect, x, bits)?;
        }
        Some(x)
    }

    /// Node `x`'s path spelled (decision 0722 §4): `file "::" unit ("." unit)* ["/" step ("." step)*]`. A unit is a
    /// granule's name, else its kind's label; a step its field's label, or its kind's where it has no field or its slot
    /// is kind-grouped. `#k` is the 0-based ordinal among siblings spelled the same, written only where there are
    /// several. A name holding `. / # :` or a space is backquoted. `None` where `x` has no path (`encode`'s).
    pub fn spell(&self, p: &super::Projected, dialect: &super::Dialect, x: u32) -> Option<String> {
        let (f, table) = (p.f(), p.path_table(dialect)?);
        let tables = dialect.path_tables();
        let quote = |s: &str| if s.is_empty() || s.contains(['.', '/', '#', ':', ' ', '`']) { format!("`{}`", s.replace('`', "``")) } else { s.to_owned() };
        let g = table.granule_of(x)?;
        // The units, the outermost first: each granule's own part of its key, `/kind:name#ord`.
        let mut units = Vec::new();
        let mut at = g;
        while let (_, Some(up)) = table.start_up(at) {
            let (key, parent) = (&self.keys[at], &self.keys[up]);
            let own = key.strip_prefix(parent.as_str())?.strip_prefix('/')?;
            let (base, ord) = own.rsplit_once('#')?;
            let (kind, name) = base.split_once(':')?;
            let kind = dialect.names.iter().copied().find(|k| *k == kind).unwrap_or("");
            let mut unit = if name.is_empty() { quote(rerail_lang::code_rule_dsl::path_tables::kind_label(tables, kind)) } else { quote(name) };
            if self.by_key.contains_key(&format!("{parent}/{base}#1")) {
                unit = format!("{unit}#{ord}");
            }
            units.push(unit);
            at = up;
        }
        units.reverse();
        // A guest's root, `path\t#key`, is spelled `path#key`; a tape value's, its grammar and its text quoted.
        let file = match tape_of_root(&self.path) {
            Some((grammar, text)) => format!("{grammar}:{}", quote(&text)),
            None => self.path.replace('\t', ""),
        };
        let mut out = format!("{file}::{}", units.join("."));
        // The steps from the granule's start down to x.
        let (start, _) = table.start_up(g);
        let mut chain = Vec::new();
        let mut c = x;
        while c != start {
            if !is_path_child(f, c) {
                return None;
            }
            chain.push(c);
            c = f.parent(c)?;
        }
        chain.reverse();
        let mut steps = Vec::with_capacity(chain.len());
        let mut q = start;
        for &c in &chain {
            let ch = children_of(f, dialect, q);
            let parent = dialect.paths.parent(dialect.code(f, q));
            let spelled = |&(k, slot, field, _): &(u32, Option<usize>, u16, u16)| -> String {
                let grouped = slot.and_then(|s| parent.map(|p| p.slots[s].grouped)).unwrap_or(false);
                match dialect.field_name(field).filter(|_| field != 0 && !grouped) {
                    Some(name) => rerail_lang::code_rule_dsl::path_tables::field_label(tables, name).to_owned(),
                    None => rerail_lang::code_rule_dsl::path_tables::kind_label(tables, dialect.name(f, k)).to_owned(),
                }
            };
            let names: Vec<String> = ch.kids.iter().map(spelled).collect();
            let i = ch.kids.iter().position(|k| k.0 == c)?;
            let same = names.iter().filter(|n| **n == names[i]).count();
            let ord = names[..i].iter().filter(|n| **n == names[i]).count();
            steps.push(if same > 1 { format!("{}#{ord}", quote(&names[i])) } else { quote(&names[i]) });
            q = c;
        }
        if !steps.is_empty() {
            out.push('/');
            out.push_str(&steps.join("."));
        }
        Some(out)
    }

    /// Granule `g`'s anchors (decision 0722 §5): from its start, in preorder, a node whose steps from its base pass a
    /// word's makes its deepest ancestor within `ANCHOR_REACH` bits of that base an anchor (the shallowest below it
    /// where none is, itself at worst), again until its steps fit. A node's base is its nearest anchor at or above
    /// it, else the granule's start. Nested granules' nodes are theirs.
    fn anchors(&self, p: &super::Projected, dialect: &super::Dialect, g: usize) -> std::sync::Arc<std::collections::BTreeSet<u32>> {
        if let Some(a) = self.anchors.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&(g as u32)) {
            return std::sync::Arc::clone(a);
        }
        let (f, table) = (p.f(), p.path_table(dialect).expect("a document with paths has its granules"));
        let (start, _) = table.start_up(g);
        let mut anchors = std::collections::BTreeSet::new();
        // The chain below the start to the node being placed, and which of it are anchors.
        let (mut chain, mut marks): (Vec<u32>, Vec<bool>) = (Vec::new(), Vec::new());
        let mut stack: Vec<(u32, usize)> = vec![(start, 0)];
        // The bits from the base at chain index `base` (the start's where none) down to chain index `to`.
        let length = |chain: &[u32], base: usize, to: usize| {
            let from = if base == 0 { start } else { chain[base - 1] };
            self.down(f, dialect, from, &chain[base..=to]).map_or(u32::MAX, |b| b.len())
        };
        while let Some((x, depth)) = stack.pop() {
            // The chain holds x's ancestors below the start, then x (at `depth - 1`): an earlier sibling's branch goes.
            chain.truncate(depth.saturating_sub(1));
            marks.truncate(depth.saturating_sub(1));
            if depth > 0 {
                chain.push(x);
                marks.push(false);
                loop {
                    let base = marks.iter().rposition(|&m| m).map_or(0, |b| b + 1);
                    if base == chain.len() || length(&chain, base, chain.len() - 1) <= WORD_STEPS {
                        break;
                    }
                    // Within reach of the base: the deepest strictly above x; else the shallowest below the base.
                    let at = (base..chain.len() - 1).take_while(|&i| length(&chain, base, i) <= ANCHOR_REACH).last();
                    let a = at.unwrap_or(base);
                    marks[a] = true;
                    anchors.insert(chain[a]);
                }
            }
            path_children(f, x, |c, _| {
                if table.granule_of(c) == Some(g) {
                    stack.push((c, depth + 1));
                }
            });
            // Children in order: the stack pops the first first.
            let n = stack.iter().rev().take_while(|e| e.1 == depth + 1).count();
            let len = stack.len();
            stack[len - n..].reverse();
        }
        let anchors = std::sync::Arc::new(anchors);
        self.anchors.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(g as u32, std::sync::Arc::clone(&anchors));
        anchors
    }
}

/// A node path shown as text (`path_tables::path_text`: its base's text, a tab, `/`, its low bits in hex) read in
/// `facts`, the tape of the document whose root is `root` (a file's path; a guest's, its path, a tab and `#` its key,
/// `guest` its place and its file's lines): the path spelled (`DocPaths::spell`) and its node's first byte in its
/// file's text. What the writer puts in a message's `{p}` and `{p.line}` (decision 0722 §7). `None` for a text no
/// path's, or one the tape no longer has.
pub fn spell_text(facts: &std::sync::Arc<crate::code_rule_dsl::facts::core::Facts>, guest: Option<(&super::Guest, std::sync::Arc<crate::code_rule_dsl::lines::LineTable>)>, root: &str, shown: &str) -> Option<(String, u32)> {
    let (base, low) = shown.rsplit_once("\t/")?;
    let low = u32::from_str_radix(low, 16).ok()?;
    let dialect = super::Dialect::new(crate::code_rule_dsl::facts::core::facts_tag(facts)).ok()?;
    let place = guest.as_ref().map(|(g, _)| g.in_file.clone());
    let p = super::Projected::new(0, std::sync::Arc::clone(facts), Some(&dialect), super::Shape::Core, false, guest.as_ref().map(|(g, l)| (*g, std::sync::Arc::clone(l))));
    let paths = DocPaths::new(&p, &dialect, root)?;
    let x = paths.decode_text(&p, &dialect, base.to_owned(), low)?;
    let start = p.f().start(x);
    let at = place.map_or(start, |pl| pl.host([start, start])[0]);
    Some((paths.spell(&p, &dialect, x)?, at))
}

/// The file a node path's shown text names (`{p.file}`): its root's path, a guest's key left out.
pub fn file_of_text(shown: &str) -> Option<&str> {
    let (base, _) = shown.rsplit_once("\t/")?;
    let root = root_of(base)?;
    // A tape value is no file's.
    if root.starts_with('\u{1}') {
        return Some("");
    }
    Some(root.split_once("\t#").map_or(root, |(path, _)| path))
}
