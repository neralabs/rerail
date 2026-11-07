//! T's base relations served from a batch of core facts (decision 0008; 05 Phase 7 row 3): the
//! relations Typing-support's exporter (`spikes/datalog-engines/exporter/src/main.rs`) wrote as
//! rows, here read off `facts::core` columns when a statement filters them.
//!
//! The rows are T's: over each document's named nodes other than comments, in preorder,
//!
//! ```text
//! doc(d, path, module)   pkg(d, p)             node(d, n, kind, start, end_)
//! child(p, c, idx)       field(p, f, c)        tape(n, par, size, depth)
//! text(n, s)             intval(n, v)          op(n, s)
//! ```
//!
//! with `idx` a child's index among those nodes, `par` -1 at the root and `size` the descendants
//! among them. Every value is an integer: a node is `d << 32 | its preorder index`, so a document is
//! an interval of ids and a subtree the ids `n + 1 ..= n + size`; `d` is the caller's key for the
//! document (its path atom in the push path); `kind` is the kind atom (`core::kind_atom`), `f` the
//! field atom (`tag << 16 | field code`), and paths, modules, packages, identifier text and
//! operators are name atoms (C12), interned when the batch is built.
//!
//! `node`, `child`, `field` and `tape` are views over the core columns (no node rows, design topic
//! 2); `doc`, `pkg`, `text`, `intval` and `op` are small and kept as [`Relation`]s.
//!
//! That is T's projection ([`Shape::T`]), which T's programs and the programs that declare the base relations
//! themselves read. A program that includes `core.rr` reads `core.rr`'s ([`Shape::Core`], decision 0046, `core_rr`):
//! comments are nodes, `text` is every node's, `op` every anonymous token's, and the source-text, token and literal
//! relations are made. T's `op` is `op_t` in both.

use super::{Modules, Relation, Source};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, kind_atom};
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

mod core_rr;
use core_rr::Family;

/// Which projection of the tree a batch serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Shape {
    /// T's: named nodes without comments, `text` of identifiers, `op` of operator fields.
    #[default]
    T,
    /// `core.rr`'s (decision 0046).
    Core,
}

/// The projection a run binds: its shape, and the relations its program reads (`None`: all). A relation no rule
/// reads is not made where making it costs (`text` of every node, the tokens).
#[derive(Clone, Debug, Default)]
pub(crate) struct Projection {
    pub(crate) shape: Shape,
    pub(crate) reads: Option<BTreeSet<String>>,
}

impl Projection {
    pub(crate) fn core() -> Self {
        Projection { shape: Shape::Core, reads: None }
    }
    fn wants(&self, rel: &str) -> bool {
        self.reads.as_ref().is_none_or(|r| r.contains(rel))
    }
    /// `core.rr`'s shape for a program that includes `core.rr` (declares every relation it declares as an input),
    /// T's for one that declares its base relations itself (T's programs, the converter's, O74).
    pub(crate) fn of(program: &crate::code_rule_dsl::logic::front::Program, reads: BTreeSet<String>) -> Self {
        static CORE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let core = CORE.get_or_init(|| {
            let path = crate::code_rule_dsl::logic::rr::library().join("core.rr");
            std::fs::read_to_string(path).ok().and_then(|src| crate::code_rule_dsl::logic::rr::parse(&src).ok()).map(|p| p.decls.into_keys().collect()).unwrap_or_default()
        });
        let shape = if !core.is_empty() && core.iter().all(|r| program.decls.contains_key(r)) { Shape::Core } else { Shape::T };
        Projection { shape, reads: Some(reads) }
    }
}

/// One document of a batch.
#[derive(Clone)]
pub(crate) struct Document {
    /// `d`: the document's key, its path atom in the push path.
    pub key: u32,
    /// The path relative to its root, from which `module` and `pkg` follow (T's `module`).
    pub path: String,
    /// The `#core:` name the facts were built for, and their digest (`core::digest`).
    pub core: String,
    pub digest: [u8; 32],
    pub facts: Arc<Facts>,
}

/// A path's module: `a/b.py` is `a.b`, `a/__init__.py` is `a`; a Rust file's is its path from
/// the crate root under `crate` (`src/lib.rs` and `src/main.rs` are `crate`, `src/a/mod.rs` and
/// `src/a.rs` are `crate.a`).
pub(crate) fn module(rel: &str) -> String {
    if let Some(rs) = rel.strip_suffix(".rs") {
        let rs = rs.rsplit_once("src/").map_or(rs, |(_, r)| r);
        let mut parts: Vec<&str> = rs.split('/').collect();
        if matches!(parts.last(), Some(&"mod")) || (parts.len() == 1 && matches!(parts[0], "lib" | "main")) {
            parts.pop();
        }
        return std::iter::once("crate").chain(parts).collect::<Vec<_>>().join(".");
    }
    let m = rel.trim_end_matches(".py").replace('/', ".");
    m.strip_suffix(".__init__").map(str::to_owned).unwrap_or(m)
}

/// The package relative imports resolve against (`__package__`): a package's own name, else its
/// module's parent.
pub(crate) fn package(rel: &str) -> String {
    let m = module(rel);
    if rel.ends_with("__init__.py") { m } else { m.rsplit_once('.').map(|(p, _)| p.to_owned()).unwrap_or_default() }
}

/// Which of T's rules a kind falls under, by name.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    Other,
    Comment,
    Text,
    Integer,
    Operator,
    FunctionDefinition,
    FunctionItem,
    Attribute,
    String,
}
impl Class {
    fn of(name: &str) -> Self {
        match name {
            "comment" => Self::Comment,
            "identifier" | "dotted_name" | "import_prefix" | "type_identifier" | "field_identifier" | "crate" | "self" | "super" => {
                Self::Text
            }
            "integer" => Self::Integer,
            "binary_operator" | "augmented_assignment" | "comparison_operator" | "boolean_operator" | "unary_operator" => {
                Self::Operator
            }
            "function_definition" => Self::FunctionDefinition,
            "function_item" => Self::FunctionItem,
            "attribute" => Self::Attribute,
            "string" => Self::String,
            _ => Self::Other,
        }
    }
}

/// A dialect's kinds as T's exporter reads them. T's kinds were names, and a grammar gives some names several codes
/// (aliases): the `kind` column holds each name's canonical code (`KindCatalog::kind_id`, what a program's constant
/// lowers to), and a lookup reads the postings of every code of that name.
struct Dialect {
    tag: u8,
    family: Family,
    catalog: KindCatalog,
    classes: Vec<Class>,
    operator: Option<u16>,
    /// The `operators` field: a comparison's operator tokens (`a < b <= c`).
    operators: Option<u16>,
    canonical: Vec<u16>,
    codes: HashMap<u16, Vec<u16>>,
}
impl Dialect {
    fn new(tag: u8) -> StoreResult<Self> {
        let catalog = from_tape_tag(tag).ok_or_else(|| StoreError::Invalid(format!("no dialect has tape tag {tag}")))?.kinds();
        let classes = (0..catalog.kind_count()).map(|k| catalog.kind_name(k as u16).map_or(Class::Other, Class::of)).collect();
        let canonical: Vec<u16> = (0..catalog.kind_count() as u16)
            .map(|k| match catalog.kind_name(k) {
                Some(name) if catalog.kind_is_named(k) => Some(catalog.kind_id(name)).filter(|&c| c != 0).unwrap_or(k),
                _ => k,
            })
            .collect();
        let mut codes: HashMap<u16, Vec<u16>> = HashMap::new();
        for (k, &c) in canonical.iter().enumerate() {
            codes.entry(c).or_default().push(k as u16);
        }
        let family = Family::of(from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label));
        Ok(Self { tag, family, catalog, classes, operator: catalog.field_id("operator"), operators: catalog.field_id("operators"), canonical, codes })
    }
    fn code(f: &Facts, id: u32) -> u16 {
        (f.kind[id as usize] % KIND_STRIDE) as u16
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(Self::code(f, id))).copied().unwrap_or(Class::Other)
    }
    fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(Self::code(f, id)).unwrap_or("")
    }
    /// The child of `id` in field `field`.
    fn field_child(&self, f: &Facts, id: u32, field: &str) -> Option<u32> {
        let field = self.catalog.field_id(field)?;
        f.children(id).iter().copied().find(|&c| f.field[c as usize] == field)
    }
    /// A name's dotted text: an identifier's, or an attribute's object's and attribute's joined by a dot (`a.b.c`),
    /// whatever spaces the source has between them. `None` where the object has no name (a call, a subscript).
    fn dotted(&self, f: &Facts, id: u32) -> Option<String> {
        match self.class(f, id) {
            Class::Text => Some(f.text(id).to_owned()).filter(|t| !t.contains(['\t', '\n', '\r', '\\'])),
            Class::Attribute => {
                let object = self.dotted(f, self.field_child(f, id, "object")?)?;
                let attribute = self.field_child(f, id, "attribute")?;
                Some(format!("{object}.{}", f.text(attribute)))
            }
            _ => None,
        }
    }
    /// A string literal's prefix (lowercased, `""` for none) and, where it has no interpolation, its content
    /// between the quotes, as written (escapes kept).
    fn string<'f>(&self, f: &'f Facts, id: u32) -> Option<(String, Option<&'f str>)> {
        let children = f.children(id);
        let start = *children.iter().find(|&&c| self.name(f, c) == "string_start")?;
        let end = *children.iter().rev().find(|&&c| self.name(f, c) == "string_end")?;
        let opening = f.text(start);
        let prefix = opening.trim_end_matches(['"', '\'']).to_lowercase();
        let interpolated = children.iter().any(|&c| self.name(f, c) == "interpolation");
        let whole = f.text(id);
        let content = (!interpolated && whole.len() >= opening.len() + f.text(end).len()).then(|| &whole[opening.len()..whole.len() - f.text(end).len()]);
        Some((prefix, content))
    }
}

/// A document's nodes, numbered in preorder: its named nodes other than comments (T's), or every named node
/// (`core.rr`'s).
struct Projected {
    key: u32,
    tag: u8,
    facts: Arc<Facts>,
    /// The core id of each node.
    core: Vec<u32>,
    /// The node of each core id, `NONE` for one left out.
    local: Vec<u32>,
    parent: Vec<u32>,
    idx: Vec<u32>,
    size: Vec<u32>,
    depth: Vec<u32>,
    /// The nodes hanging from their parent under a field, ascending.
    fielded: Vec<u32>,
    /// `core.rr`'s lines and spans; `None` in T's shape.
    extra: Option<Box<Extra>>,
}

/// A document's lines, and its spans past its nodes: from local id `len()`, its anonymous tokens (`tokens` of them,
/// in preorder), then its whole text, then each line.
struct Extra {
    lines: core_rr::Lines,
    spans: Vec<[u32; 2]>,
    tokens: u32,
}

impl Projected {
    fn new(key: u32, facts: Arc<Facts>, dialect: &Dialect, shape: Shape) -> Self {
        let f = &*facts;
        let mut p = Projected {
            key,
            tag: dialect.tag,
            core: Vec::new(),
            local: vec![NONE; f.len()],
            parent: Vec::new(),
            idx: Vec::new(),
            size: Vec::new(),
            depth: Vec::new(),
            fielded: Vec::new(),
            extra: None,
            facts: Arc::clone(&facts),
        };
        let mut next_idx: Vec<u32> = Vec::new();
        for id in 0..f.len() as u32 {
            if !f.named(id) || (shape == Shape::T && dialect.class(f, id) == Class::Comment) {
                continue;
            }
            // T walks named children from the root: a node under one left out is not reached.
            let parent = match f.parent(id) {
                None => NONE,
                Some(q) if p.local[q as usize] != NONE => p.local[q as usize],
                Some(_) => continue,
            };
            if parent == NONE && !p.core.is_empty() {
                continue;
            }
            let local = p.core.len() as u32;
            p.local[id as usize] = local;
            p.core.push(id);
            p.parent.push(parent);
            next_idx.push(0);
            if parent == NONE {
                p.idx.push(0);
                p.depth.push(0);
            } else {
                p.idx.push(next_idx[parent as usize]);
                next_idx[parent as usize] += 1;
                p.depth.push(p.depth[parent as usize] + 1);
                if f.field[id as usize] != 0 {
                    p.fielded.push(local);
                }
            }
        }
        p.size = vec![0; p.core.len()];
        for i in (1..p.core.len()).rev() {
            let q = p.parent[i] as usize;
            p.size[q] += p.size[i] + 1;
        }
        if shape == Shape::Core {
            let lines = core_rr::Lines::new(&f.source, dialect.family == Family::Python);
            let mut spans: Vec<[u32; 2]> = Vec::new();
            for &id in &p.core {
                for &c in f.children(id) {
                    if !f.named(c) && f.end[c as usize] > f.start[c as usize] {
                        spans.push([f.start[c as usize], f.end[c as usize]]);
                    }
                }
            }
            let tokens = spans.len() as u32;
            spans.push([0, f.source.len() as u32]);
            spans.extend((1..=lines.count()).map(|l| lines.span(l)));
            p.extra = Some(Box::new(Extra { lines, spans, tokens }));
        }
        p
    }
    /// Nodes and spans: the ids `text`, `line` and `docof` take.
    fn locs(&self) -> u32 {
        self.len() + self.extra.as_ref().map_or(0, |x| x.spans.len() as u32)
    }
    /// The byte span of node or span `local`.
    fn span(&self, local: u32) -> [u32; 2] {
        match local.checked_sub(self.len()) {
            None => {
                let id = self.core[local as usize] as usize;
                [self.facts.start[id], self.facts.end[id]]
            }
            Some(i) => self.extra.as_ref().map_or([0, 0], |x| x.spans[i as usize]),
        }
    }
    fn len(&self) -> u32 {
        self.core.len() as u32
    }
    fn global(&self, local: u32) -> i64 {
        i64::from(self.key) << 32 | i64::from(local)
    }
    /// The included children of `local`, in order.
    fn children(&self, local: u32) -> impl Iterator<Item = u32> + '_ {
        self.facts.children(self.core[local as usize]).iter().map(|&c| self.local[c as usize]).filter(|&c| c != NONE)
    }
}

/// One wave's documents, sorted by key, and the row offsets of the views.
pub(crate) struct Batch {
    docs: Vec<Projected>,
    dialects: HashMap<u8, Dialect>,
    nodes: Vec<usize>,
    children: Vec<usize>,
    fields: Vec<usize>,
    /// Nodes and spans (`Projected::locs`).
    locs: Vec<usize>,
    shape: Shape,
    /// The `py.*` families of the batch's Python documents, made on their first read (list step 25).
    py: Option<Py>,
}

/// The `py.*` families (`facts::producers::python::families`): the producer's grammar, the atoms of its
/// names, and the rows, made on the first read of any of them: a program that reads none does no work.
struct Py {
    grammar: py::Grammar,
    contexts: [i64; 5],
    spreads: [i64; 3],
    rows: std::sync::OnceLock<Vec<Relation>>,
}

/// The `py.*` families' tables and columns (all integers; `-1` is none), as `PY_COLUMNS[i]` is `PY[i]`'s.
pub(crate) const PY: [&str; 5] = ["py_call", "py_call_site", "py_exec_context", "py_call_arg", "py_attribute"];
pub(crate) const PY_COLUMNS: [&[&str]; 5] = [
    &["d", "call", "callee"],
    &["d", "report", "call", "ref"],
    &["d", "call", "owner", "kind"],
    &["d", "call", "arg", "ordinal", "keyword", "spread", "value"],
    &["d", "node", "object", "name"],
];

/// Which view a source serves.
#[derive(Clone, Copy)]
enum View {
    Node,
    /// `line(n, l)`: a node's first line, 1-based (rows laid out as `node`'s).
    Line,
    Child,
    Field,
    Tape,
    /// `core_node(d, t, n)`: document `d`'s tape node `t` (the matcher's preorder id over every
    /// node) is node `n`. The wave's matches are tape nodes; this joins them to the rows above.
    Core,
    /// `visit_index(n, i)`: the node's preorder index in its document, from 0 (`core.rr`).
    Visit,
    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
    /// `core.rr`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`core_rr::Lines`), rows laid out over `Projected::locs`.
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    #[allow(dead_code, reason = "bound once the lowering reads `docof` as an input, not as `node`'s projection")]
    Docof,
}

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`).
    /// Also returns the batch's distinct strings and their symbols: its vocabulary, which text tests read
    /// (`logic::strings::Vocabulary`, decision 0037).
    #[allow(clippy::type_complexity)]
    fn build(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, (Vec<String>, Vec<u32>))> {
        let shape = projection.shape;
        let core = shape == Shape::Core;
        let mut documents = documents;
        documents.sort_by_key(|d| d.key);
        if documents.windows(2).any(|w| w[0].key == w[1].key) {
            return Err(StoreError::Invalid("a batch holds a document key twice".into()));
        }
        let mut dialects: HashMap<u8, Dialect> = HashMap::new();
        let mut strings: Vec<String> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut string = |s: &str| -> usize {
            if let Some(&i) = at.get(s) {
                return i;
            }
            strings.push(s.to_owned());
            at.insert(s.to_owned(), strings.len() - 1);
            strings.len() - 1
        };
        // Rows with string indexes; the indexes become atoms once the batch's strings are interned.
        let (mut doc, mut pkg, mut text, mut intval, mut op_t) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
        // Text relations (decision 0037): `name(n, s)` (an identifier's text, an attribute's dotted text),
        // `str(n, s)` (a string literal's content, between its quotes) and `strprefix(n, p)` (its prefix, lowercased).
        let (mut name, mut strs, mut strprefix) = (Vec::new(), Vec::new(), Vec::new());
        // `value(n, v)`: a literal's static value (`core.rr`): a Python string decoded (`value::python::str_value`),
        // an integer in decimal.
        let mut value: Vec<[i64; 2]> = Vec::new();
        // `op_at(n, i, s)`: n's operator tokens in order, from 0 (`core.rr`): in `a < b <= c`, (c, 0, "<") and
        // (c, 1, "<="). T's: the tokens of the `operator` and `operators` fields; `core.rr`'s: every anonymous child
        // but punctuation.
        let mut op_at: Vec<[i64; 3]> = Vec::new();
        let python_tag = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `syntax_error(d)`: the documents whose parse has an error (`Facts::syntax_errors`); `core.rr`'s
        // `syntax_error(d, l, msg)`: the first error's line and message.
        let mut errors = Vec::new();
        // `core.rr`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        let wants_text = core && projection.wants("text");
        let wants_tokens = core && ["first_token", "last_token", "token_before", "token_after"].iter().any(|r| projection.wants(r));
        let mut docs = Vec::with_capacity(documents.len());
        for d in &documents {
            let tag = (d.facts.kind.first().copied().unwrap_or(0) / KIND_STRIDE) as u8;
            if !dialects.contains_key(&tag) {
                dialects.insert(tag, Dialect::new(tag)?);
            }
            let dialect = &dialects[&tag];
            let p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape);
            let key = i64::from(d.key);
            // core.rr: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            let python = d.path.ends_with(".py");
            let dotted = python || d.path.ends_with(".rs");
            let module = if shape == Shape::Core && !dotted { String::new() } else { module(&d.path) };
            doc.push([key, string(&d.path) as i64, string(&module) as i64]);
            if shape != Shape::Core || python {
                pkg.push([key, string(&package(&d.path)) as i64]);
            }
            let parts: Vec<&str> = d.path.split('/').filter(|s| !s.is_empty()).collect();
            if let Some(last) = parts.last() {
                file_name.push([key, string(last) as i64]);
            }
            let mut seen = std::collections::HashSet::new();
            for part in parts {
                if seen.insert(part) {
                    path_part.push([key, string(part) as i64]);
                }
            }
            if d.facts.syntax_errors {
                errors.push(key);
            }
            let f = &*d.facts;
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                if !core {
                    let is_operator = |c: &&u32| {
                        let field = f.field[**c as usize];
                        field != 0 && (dialect.operator == Some(field) || dialect.operators == Some(field))
                    };
                    for (i, &c) in f.children(id).iter().filter(is_operator).enumerate() {
                        op_at.push([n, i as i64, string(dialect.name(f, c)) as i64]);
                    }
                }
                match dialect.class(f, id) {
                    Class::Text if !core => {
                        let t = f.text(id);
                        if !t.contains(['\t', '\n', '\r', '\\']) {
                            text.push([n, string(t) as i64]);
                            name.push([n, string(t) as i64]);
                        }
                    }
                    Class::Attribute if !core => {
                        if let Some(dotted) = dialect.dotted(f, id) {
                            name.push([n, string(&dotted) as i64]);
                        }
                    }
                    Class::String if !core => {
                        if let Some((prefix, content)) = dialect.string(f, id) {
                            strprefix.push([n, string(&prefix) as i64]);
                            if let Some(content) = content {
                                strs.push([n, string(content) as i64]);
                            }
                            if Some(tag) == python_tag
                                && let Some(v) = crate::code_rule_dsl::value::python::str_value(f.text(id))
                            {
                                value.push([n, string(&v) as i64]);
                            }
                        }
                    }
                    Class::Integer => {
                        if let Ok(v) = f.text(id).replace('_', "").parse::<i64>() {
                            intval.push([n, v]);
                            if !core {
                                value.push([n, string(&v.to_string()) as i64]);
                            }
                        }
                    }
                    Class::Operator => {
                        let operator = dialect.operator.filter(|&o| o != 0);
                        if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field[c as usize])) {
                            op_t.push([n, string(dialect.name(f, o)) as i64]);
                        }
                    }
                    // `async def`: the keyword is an anonymous child, recorded as the definition's modifier.
                    Class::FunctionDefinition if f.children(id).first().is_some_and(|&k| dialect.name(f, k) == "async") => {
                        op_t.push([n, string("async") as i64]);
                    }
                    Class::FunctionItem
                        if f.children(id).iter().any(|&m| {
                            dialect.name(f, m) == "function_modifiers" && f.children(m).iter().any(|&k| dialect.name(f, k) == "async")
                        }) =>
                    {
                        op_t.push([n, string("async") as i64]);
                    }
                    _ => {}
                }
                if core {
                    c.node(&p, dialect, local as u32, &mut string, wants_text);
                }
            }
            if core {
                c.document(&p, dialect, &mut string, wants_text, wants_tokens);
            }
            docs.push(p);
        }
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        let vocabulary = python.filter(|tag| dialects.contains_key(tag)).map(|_| {
            (py::CONTEXTS.map(|name| string(name) as i64), py::SPREADS.map(|name| string(name) as i64))
        });
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let py = vocabulary.zip(python).map(|((contexts, spreads), tag)| Py {
            grammar: py::Grammar::new(tag, &dialects[&tag].catalog),
            contexts: contexts.map(atom),
            spreads: spreads.map(atom),
            rows: Default::default(),
        });
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let pairs = |columns: &[&str], rows: &[[i64; 2]], sym: [bool; 2]| {
            relation(columns, &mut rows.iter().map(|&[a, b]| vec![if sym[0] { atom(a) } else { a }, if sym[1] { atom(b) } else { b }]))
        };
        let mut relations = vec![
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| vec![d, atom(p), atom(m)]))),
            ("pkg", pairs(&["d", "p"], &pkg, [false, true])),
            ("path_part", pairs(&["d", "s"], &path_part, [false, true])),
            ("file_name", pairs(&["d", "s"], &file_name, [false, true])),
            ("intval", pairs(&["n", "v"], &intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &op_t, [false, true])),
        ];
        if core {
            relations.extend(c.relations(&atom));
        } else {
            relations.extend([
                ("value", pairs(&["n", "s"], &value, [false, true])),
                ("text", pairs(&["n", "s"], &text, [false, true])),
                ("op", pairs(&["n", "s"], &op_t, [false, true])),
                ("op_at", relation(&["n", "i", "s"], &mut op_at.iter().map(|&[n, i, s]| vec![n, i, atom(s)]))),
                ("name", pairs(&["n", "s"], &name, [false, true])),
                ("str", pairs(&["n", "s"], &strs, [false, true])),
                ("strprefix", pairs(&["n", "s"], &strprefix, [false, true])),
                ("syntax_error", relation(&["d"], &mut errors.iter().map(|&d| vec![d]))),
            ]);
        }
        let offsets = |count: &dyn Fn(&Projected) -> usize| {
            let mut v = vec![0];
            docs.iter().for_each(|p| v.push(v.last().unwrap() + count(p)));
            v
        };
        let nodes = offsets(&|p| p.core.len());
        let children = offsets(&|p| p.core.len().saturating_sub(1));
        let fields = offsets(&|p| p.fielded.len());
        let locs = offsets(&|p| p.locs() as usize);
        Ok((Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, py }), relations, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth => &self.nodes,
            View::Child => &self.children,
            View::Field => &self.fields,
            View::Loc(_) => &self.locs,
        }
    }
    /// The document and its row index of `row`.
    fn locate(&self, view: View, row: usize) -> (&Projected, usize) {
        let offsets = self.offsets(view);
        let doc = offsets.partition_point(|&o| o <= row) - 1;
        (&self.docs[doc], row - offsets[doc])
    }
    /// The document of key `d`, by index.
    fn doc(&self, d: i64) -> Option<usize> {
        let d = u32::try_from(d).ok()?;
        self.docs.binary_search_by_key(&d, |p| p.key).ok()
    }
    /// The document index and node of id `n`.
    fn node(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = (n & 0xffff_ffff) as u32;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The document index and node or span of id `n`.
    fn loc(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = (n & 0xffff_ffff) as u32;
        (n >= 0 && local < self.docs[doc].locs()).then_some((doc, local))
    }
    /// The view's row of node `local` of document `doc`, if it has one.
    fn row(&self, view: View, doc: usize, local: u32) -> Option<usize> {
        let p = &self.docs[doc];
        let within = match view {
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Loc(_) => Some(local as usize),
            View::Child => (local > 0).then(|| local as usize - 1),
            View::Field => p.fielded.binary_search(&local).ok(),
        }?;
        Some(self.offsets(view)[doc] + within)
    }
    /// The rows of nodes with ids in `lo..=hi`.
    fn between(&self, view: View, lo: i64, hi: i64) -> Vec<usize> {
        let mut rows = Vec::new();
        let first = self.docs.partition_point(|p| i64::from(p.key) < lo >> 32);
        for (doc, p) in self.docs.iter().enumerate().skip(first) {
            let base = p.global(0);
            if base > hi {
                break;
            }
            let start = (lo.max(base) - base) as u32;
            let count = if matches!(view, View::Loc(_)) { p.locs() } else { p.len() };
            let end = ((hi - base).min(i64::from(count) - 1) + 1).max(0) as u32;
            rows.extend((start..end.max(start)).filter_map(|local| self.row(view, doc, local)));
        }
        rows
    }
}

struct Viewed {
    batch: Arc<Batch>,
    view: View,
}
impl Viewed {
    fn columns(view: View) -> &'static [&'static str] {
        match view {
            View::Node => &["d", "n", "kind", "start", "end_"],
            View::Line => &["n", "l"],
            View::Child => &["p", "c", "idx"],
            View::Field => &["p", "f", "c"],
            View::Tape => &["n", "par", "size", "depth"],
            View::Core => &["d", "t", "n"],
            View::Visit => &["n", "i"],
            View::Depth => &["n", "k"],
            View::Loc(Place::Line | Place::EndLine) => &["n", "l"],
            View::Loc(Place::Col) => &["n", "c"],
            View::Loc(Place::Docof) => &["n", "d"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child => 1,
            View::Line | View::Visit | View::Depth | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Visit | View::Depth | View::Loc(_) => None,
        }
    }
}
impl Source for Viewed {
    fn columns(&self) -> Vec<String> {
        Self::columns(self.view).iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        *self.batch.offsets(self.view).last().unwrap()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let per = if column == self.own() {
            1
        } else if Some(column) == self.parent() {
            4
        } else if matches!(self.view, View::Node | View::Core) && column == 0 {
            self.len() / self.batch.docs.len().max(1)
        } else if matches!(self.view, View::Core) && column == 1 {
            // One probe per document of the batch.
            self.batch.docs.len()
        } else {
            (self.len() / 64).max(1)
        };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column == self.own()
            || Some(column) == self.parent()
            || matches!(self.view, View::Node) && matches!(column, 0 | 2)
            || matches!(self.view, View::Core) && matches!(column, 0 | 1)
    }
    fn range_indexed(&self, column: usize) -> bool {
        column == self.own()
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        if column != self.own() {
            return None;
        }
        let lo = match lower {
            None => 0,
            Some((v, inclusive)) => v.saturating_add(i64::from(!inclusive)),
        };
        let hi = match upper {
            None => i64::MAX,
            Some((v, inclusive)) => v.saturating_sub(i64::from(!inclusive)),
        };
        Some(if lo > hi { Vec::new() } else { self.batch.between(self.view, lo, hi) })
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let b = &*self.batch;
        let mut rows = Vec::new();
        for &v in values {
            if column == self.own() {
                let found = if matches!(self.view, View::Loc(_)) { b.loc(v) } else { b.node(v) };
                rows.extend(found.and_then(|(doc, local)| b.row(self.view, doc, local)));
            } else if Some(column) == self.parent() {
                if v == -1 && matches!(self.view, View::Tape) {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).map(|doc| b.nodes[doc]));
                } else if let Some((doc, local)) = b.node(v) {
                    rows.extend(b.docs[doc].children(local).filter_map(|c| b.row(self.view, doc, c)));
                }
            } else if matches!(self.view, View::Node | View::Core) && column == 0 {
                rows.extend(b.doc(v).map_or(0..0, |doc| b.nodes[doc]..b.nodes[doc + 1]));
            } else if matches!(self.view, View::Core) && column == 1 {
                // A tape node of each document that has it as a node; the statement filters `d`.
                let Ok(t) = usize::try_from(v) else { continue };
                for (doc, p) in b.docs.iter().enumerate() {
                    if let Some(&local) = p.local.get(t).filter(|&&local| local != NONE) {
                        rows.push(b.nodes[doc] + local as usize);
                    }
                }
            } else if matches!(self.view, View::Node) && column == 2 {
                let (Ok(tag), Ok(code)) = (u8::try_from(v / i64::from(KIND_STRIDE)), u16::try_from(v % i64::from(KIND_STRIDE))) else {
                    continue;
                };
                let Some(dialect) = b.dialects.get(&tag).filter(|_| v >= 0) else { continue };
                let atoms: Vec<u32> = dialect.codes.get(&code).map_or(&[][..], Vec::as_slice).iter().map(|&c| kind_atom(tag, c)).collect();
                for (doc, p) in b.docs.iter().enumerate().filter(|(_, p)| p.tag == tag) {
                    for &kind in &atoms {
                        rows.extend(p.facts.of_kind(kind).iter().map(|&id| p.local[id as usize]).filter(|&l| l != NONE).map(|l| b.nodes[doc] + l as usize));
                    }
                }
            } else {
                return None;
            }
        }
        Some(rows)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let (p, i) = self.batch.locate(self.view, row);
        if let View::Loc(place) = self.view {
            let local = i as u32;
            if column == 0 {
                return p.global(local);
            }
            let lines = &p.extra.as_ref().expect("core.rr's lines").lines;
            let (line, last, col) = lines.place(p.span(local));
            return match place {
                Place::Line => i64::from(line),
                Place::EndLine => i64::from(last),
                Place::Col => i64::from(col),
                Place::Docof => i64::from(p.key),
            };
        }
        let local = match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent[l] == NONE { -1 } else { p.global(p.parent[l]) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => {
                let code = (p.facts.kind[id] % KIND_STRIDE) as usize;
                i64::from(kind_atom(p.tag, self.batch.dialects[&p.tag].canonical.get(code).copied().unwrap_or(code as u16)))
            }
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line[id]),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx[l]),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size[l]),
            (View::Tape, 3) => i64::from(p.depth[l]),
            (View::Visit, 1) => i64::from(local),
            (View::Depth, 1) => i64::from(p.depth[l]),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub(crate) fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// T's nine base relations over `documents`, and the `py.*` families, by table name.
pub(crate) fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub(crate) fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations, _) = Batch::build(store, documents, projection)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> =
        relations.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)).collect();
    out.extend(views(&batch));
    Ok(out)
}

/// The batch's views: `node`, `child`, `field`, `tape` and the `py.*` families.
fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = Vec::new();
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core)];
    match batch.shape {
        Shape::T => named.push(("line", View::Line)),
        Shape::Core => named.extend([
            ("line", View::Loc(Place::Line)),
            ("end_line", View::Loc(Place::EndLine)),
            ("col", View::Loc(Place::Col)),
            // `core.rr`'s `docof(x: loc, d)` is an input (`rr::lower`): nodes' and spans' documents.
            ("docof", View::Loc(Place::Docof)),
            ("visit_index", View::Visit),
            ("ast_depth", View::Depth),
        ]),
    }
    for (name, view) in named {
        out.push((name, Arc::new(Viewed { batch: Arc::clone(batch), view })));
    }
    for (family, name) in PY.into_iter().enumerate() {
        out.push((name, Arc::new(PyFamily { batch: Arc::clone(batch), family })));
    }
    out
}

impl Batch {
    /// The `py.*` families' relations (empty without Python documents), made on the first call.
    fn py(&self) -> &[Relation] {
        static EMPTY: std::sync::OnceLock<Vec<Relation>> = std::sync::OnceLock::new();
        let Some(py) = &self.py else {
            return EMPTY.get_or_init(|| PY_COLUMNS.iter().map(|columns| Relation::new(columns)).collect());
        };
        py.rows.get_or_init(|| {
            let started = std::time::Instant::now();
            let mut out: Vec<Relation> = PY_COLUMNS.iter().map(|columns| Relation::new(columns)).collect();
            let mut skipped = 0;
            for p in self.docs.iter().filter(|p| p.tag == py.grammar.tag()) {
                let rows = py::rows(&py.grammar, &p.facts);
                let d = i64::from(p.key);
                // A node id, `-1` for none; `None` for a node the projection leaves out.
                let id = |core: u32| -> Option<i64> {
                    if core == NONE {
                        return Some(-1);
                    }
                    let local = p.local[core as usize];
                    (local != NONE).then(|| p.global(local))
                };
                let mut push = |family: usize, row: Option<Vec<i64>>| match row {
                    Some(row) => out[family].push(&row),
                    None => skipped += 1,
                };
                for &[call, callee] in &rows.call {
                    push(0, (|| Some(vec![d, id(call)?, id(callee)?]))());
                }
                for &[report, call, reference] in &rows.site {
                    push(1, (|| Some(vec![d, id(report)?, id(call)?, id(reference)?]))());
                }
                for &[call, owner, kind] in &rows.context {
                    push(2, (|| Some(vec![d, id(call)?, id(owner)?, py.contexts[kind as usize]]))());
                }
                for &[call, arg, ordinal, keyword, spread, value] in &rows.arg {
                    push(3, (|| Some(vec![d, id(call)?, id(arg)?, i64::from(ordinal), id(keyword)?, py.spreads[spread as usize], id(value)?]))());
                }
                for &[node, object, name] in &rows.attribute {
                    push(4, (|| Some(vec![d, id(node)?, id(object)?, id(name)?]))());
                }
            }
            crate::code_rule_dsl::prof::count("py.rows", out.iter().map(|r| r.len()).sum());
            crate::code_rule_dsl::prof::count("py.skipped", skipped);
            crate::code_rule_dsl::prof::count("py.us", started.elapsed().as_micros() as usize);
            out
        })
    }
}

/// One `py.*` family of a batch (`PY[family]`), served from the rows its first read makes.
struct PyFamily {
    batch: Arc<Batch>,
    family: usize,
}
impl PyFamily {
    fn rows(&self) -> &Relation {
        &self.batch.py()[self.family]
    }
}
impl Source for PyFamily {
    fn columns(&self) -> Vec<String> {
        PY_COLUMNS[self.family].iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        self.rows().len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.rows().estimated_matches(column, batch)
    }
    fn indexed(&self, column: usize) -> bool {
        self.rows().indexed(column)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.rows().range_indexed(column)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.rows().range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.rows().lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.rows().value(row, column)
    }
}

impl Batch {
    /// The batch's trees for the file-local FIX: per node row, its parent's row (or `file_local::NONE`)
    /// and its kind atom, as `node.kind` reads it.
    fn trees(&self) -> (Vec<u32>, Vec<i64>) {
        let n = *self.nodes.last().unwrap_or(&0);
        let (mut parent, mut kind) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for (doc, p) in self.docs.iter().enumerate() {
            for local in 0..p.len() as usize {
                let up = p.parent[local];
                parent.push(if up == NONE { crate::code_rule_dsl::typing::file_local::NONE } else { (self.nodes[doc] + up as usize) as u32 });
                let code = (p.facts.kind[p.core[local] as usize] % KIND_STRIDE) as usize;
                kind.push(i64::from(kind_atom(p.tag, self.dialects[&p.tag].canonical.get(code).copied().unwrap_or(code as u16))));
            }
        }
        (parent, kind)
    }
    /// The node id of row `row` of the node view.
    fn id(&self, row: usize) -> i64 {
        let (p, local) = self.locate(View::Node, row);
        p.global(local as u32)
    }
}

/// `nearest_<set>(n, a)`: `a` is `n`'s nearest strict ancestor of a kind in the set, a row per node that has
/// one, by `n` and by `a`; or `inside_<set>(n, a)`, a row per such ancestor (decision 0036). Made by the transducer's file-local FIX (`typing::file_local::nearest`). Rows are node rows
/// of the batch (`u32`), ids made on read.
struct Nearest {
    batch: Arc<Batch>,
    /// Per row, its node and its ancestor (node rows), ascending by node.
    node: Vec<u32>,
    ancestor: Vec<u32>,
    /// The rows ascending by ancestor, made on the first lookup by ancestor: a program that only climbs (`n` bound,
    /// as T's do) never sorts them.
    by_ancestor: std::sync::OnceLock<Vec<u32>>,
}

impl Nearest {
    /// The node row of id `v`.
    fn row_of(&self, v: i64) -> Option<u32> {
        let (doc, local) = self.batch.node(v)?;
        Some((self.batch.nodes[doc] + local as usize) as u32)
    }
}

impl Source for Nearest {
    fn columns(&self) -> Vec<String> {
        vec!["n".to_owned(), "a".to_owned()]
    }
    fn len(&self) -> usize {
        self.node.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let per = if column == 0 { 2 } else { 16 };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let mut out = Vec::new();
        for &v in values {
            let Some(at) = self.row_of(v) else { continue };
            if column == 0 {
                // One row per node for `nearest_<set>`, a run of them for `inside_<set>`.
                let start = self.node.partition_point(|&r| r < at);
                out.extend((start..self.node.len()).take_while(|&r| self.node[r] == at));
            } else {
                let by_ancestor = self.by_ancestor.get_or_init(|| {
                    let mut rows: Vec<u32> = (0..self.node.len() as u32).collect();
                    rows.sort_unstable_by_key(|&r| (self.ancestor[r as usize], self.node[r as usize]));
                    rows
                });
                let start = by_ancestor.partition_point(|&r| self.ancestor[r as usize] < at);
                out.extend(by_ancestor[start..].iter().take_while(|&&r| self.ancestor[r as usize] == at).map(|&r| r as usize));
            }
        }
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.batch.id(if column == 0 { self.node[row] } else { self.ancestor[row] } as usize)
    }
}

/// T's base relations over `documents` (`sources`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms),
/// made by the transducer's file-local FIX bound as `binding`: `rr_nearest_<set>(n, a)`, the nearest ancestor in the
/// set, `rr_inside_<set>(n, a)`, every one (decision 0036), or `rr_has_<set>(n, a)`, n in the set and a each of its
/// strict ancestors.
pub(crate) fn with_nearest(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    use crate::code_rule_dsl::typing::file_local;
    let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
    let (batch, relations, (strings, symbols)) = Batch::build(store, documents, projection)?;
    drop(built);
    let mut out: Vec<(String, Arc<dyn Source>)> = relations.into_iter().map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
    // The text tests' relations (decision 0037): the batch's strings that pass each, each string tested once a run.
    let text = crate::code_rule_dsl::logic::profile::span(|| "text tests".into());
    for ((name, _), rows) in tests.tests.iter().zip(tests.rows(&strings, &symbols)) {
        let mut r = Relation::new(&["s"]);
        rows.into_iter().for_each(|s| r.push(&[s]));
        out.push((name.clone(), Arc::new(r)));
    }
    drop(text);
    let viewing = crate::code_rule_dsl::logic::profile::span(|| "views".into());
    out.extend(views(&batch).into_iter().map(|(name, source)| (name.to_owned(), source)));
    drop(viewing);
    let _nearest = crate::code_rule_dsl::logic::profile::span(|| "nearest".into());
    let (parent, kind) = batch.trees();
    use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS};
    // `rr_has_<set>` needs no nearest ancestors: it is read off the parents.
    let kinds: Vec<Vec<i64>> = sets.iter().filter(|(name, _)| !name.starts_with(DESCENDANTS)).map(|(_, kinds)| kinds.clone()).collect();
    let mut found = file_local::nearest(&file_local::Trees { parent: &parent, kind: &kind }, &kinds, binding)?.into_iter();
    for (name, members) in sets {
        let (mut node, mut ancestor) = (Vec::new(), Vec::new());
        if name.starts_with(DESCENDANTS) {
            // `rr_has_<set>` (`core.rr`'s `has`): every node of a kind in the set, with each of its strict ancestors.
            let members: std::collections::HashSet<i64> = members.iter().copied().collect();
            for (row, k) in kind.iter().enumerate() {
                if !members.contains(k) {
                    continue;
                }
                let mut a = parent[row];
                while a != file_local::NONE {
                    node.push(row as u32);
                    ancestor.push(a);
                    a = parent[a as usize];
                }
            }
            out.push((name.clone(), Arc::new(Nearest { batch: Arc::clone(&batch), node, ancestor, by_ancestor: Default::default() })));
            continue;
        }
        let ancestors = found.next().expect("one nearest ancestor list per set");
        // `rr_inside_<set>` (decision 0036): every ancestor in the set, the chain of nearest ones.
        let all = name.starts_with(AXIS);
        for (row, &first) in ancestors.iter().enumerate() {
            let mut a = first;
            while a != file_local::NONE {
                node.push(row as u32);
                ancestor.push(a);
                if !all {
                    break;
                }
                a = ancestors[a as usize];
            }
        }
        out.push((name.clone(), Arc::new(Nearest { batch: Arc::clone(&batch), node, ancestor, by_ancestor: Default::default() })));
    }
    Ok(out)
}

/// The planner's statistics of `source` (`stat_col`'s rows, ndv, sq and sqd per column, `sqd` by
/// document `d = id >> 32`), for a relation made at bind time rather than at ingestion.
pub(crate) fn statistics(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    let rows = source.len();
    let docs: Vec<i64> = (0..rows).map(|row| source.value(row, 0) >> 32).collect();
    // One sort per column by (value, document): a run of one value is its postings (ndv, and the sum of squares of
    // their lengths), and within it a run of one document is that document's (the sum of squares per document).
    source
        .columns()
        .into_iter()
        .enumerate()
        .map(|(c, name)| {
            let mut values: Vec<(i64, i64)> = (0..rows).map(|row| (source.value(row, c), docs[row])).collect();
            values.sort_unstable();
            let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
            let mut at = 0;
            while at < values.len() {
                let v = values[at].0;
                let end = at + values[at..].partition_point(|x| x.0 == v);
                ndv += 1;
                sq += ((end - at) * (end - at)) as i64;
                let mut d = at;
                while d < end {
                    let doc = values[d].1;
                    let de = d + values[d..end].partition_point(|x| x.1 == doc);
                    sqd += ((de - d) * (de - d)) as i64;
                    d = de;
                }
                at = end;
            }
            (name, rows as i64, ndv, sq, sqd)
        })
        .collect()
}

/// Fails if a document's facts are not those its `#core:` name binds now (§4).
pub(crate) fn fresh(graph: &(impl Graph + ?Sized), documents: &[Document]) -> StoreResult<()> {
    for d in documents {
        if graph.binding(&d.core)? != Some(d.digest) {
            return Err(StoreError::Invalid(format!("stale core facts for {}", d.core)));
        }
    }
    Ok(())
}

/// Binds T's base relations over `documents` on `db`, after checking that each document's facts are
/// those its `#core:` name binds now (§4): a stale document fails the binding.
pub(crate) fn bind(modules: &Modules, db: &Connection, graph: &(impl Graph + ?Sized), documents: Vec<Document>) -> StoreResult<()> {
    fresh(graph, &documents)?;
    for (name, source) in sources(graph.store(), documents)? {
        modules.bind(db, name, source).map_err(StoreError::Database)?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::code_rule_dsl) mod tests;
