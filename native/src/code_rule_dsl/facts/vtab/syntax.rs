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
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, TEXT, kind_atom, kind_code};
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

mod within;

/// The language's `within(n, a)`, `n` strictly below `a` (`within::Within`, tree mode).
pub(crate) const WITHIN: &str = "within";
/// `span_within(inner, outer)`, `outer`'s span holding `inner`'s (`within::Within`, span mode).
pub(crate) const SPAN_WITHIN: &str = "span_within";

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
    /// The tape the facts were read off and the file it parsed, which W's values and f-string trees read (`TAPED`);
    /// kept only for a program that reads one of them, else `None`.
    pub(in crate::code_rule_dsl) tape: Option<(crate::code_rule_dsl::tape::TapeDoc, crate::code_rule_dsl::model::FileInput)>,
}

/// The relations read off a document's tape, for which a batch's documents keep their tapes (`Document::tape`): W's
/// values (`Extra`), its f-string trees (`fstring`) and the string annotations' (`annotation`).
pub(crate) const TAPED: [&str; 8] =
    ["value", "static_value", "value_type_of", "value_type", fstring::RELATIONS[0], fstring::RELATIONS[1], annotation::RELATIONS[0], annotation::RELATIONS[1]];

/// A comment's text without its marker (core.rr's `comment`): `#`, `--`, `//` (and a doc comment's `/` or `!`),
/// `/*` to `*/` (and a doc comment's `*` or `!`) or `<!--` to `-->` taken off, then the whitespace around the rest.
pub(crate) fn comment_body(text: &str) -> &str {
    let body = if let Some(rest) = text.strip_prefix("/*") {
        let rest = rest.strip_suffix("*/").unwrap_or(rest);
        rest.strip_prefix(['*', '!']).unwrap_or(rest)
    } else if let Some(rest) = text.strip_prefix("//") {
        rest.strip_prefix(['/', '!']).unwrap_or(rest)
    } else if let Some(rest) = text.strip_prefix("<!--") {
        rest.strip_suffix("-->").unwrap_or(rest)
    } else {
        text.strip_prefix('#').or_else(|| text.strip_prefix("--")).unwrap_or(text)
    };
    body.trim()
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
    /// Node `id`'s kind code (ERROR's is `u16::MAX`).
    fn code(&self, f: &Facts, id: u32) -> u16 {
        kind_code(self.tag, f.kind[id as usize]).unwrap_or(0)
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(self.code(f, id))).copied().unwrap_or(Class::Other)
    }
    fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(self.code(f, id)).unwrap_or("")
    }
    /// Node `id`'s kind atom as the `kind` column holds it: its name's canonical code's.
    fn kind(&self, f: &Facts, id: u32) -> i64 {
        let code = self.code(f, id);
        i64::from(kind_atom(self.tag, self.canonical.get(usize::from(code)).copied().unwrap_or(code)))
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
    extra: Option<Box<Locs>>,
}

/// A document's lines, and its spans past its nodes (`Shape::Core`'s): from local id `len()`, its anonymous tokens
/// (`tokens` of them, in preorder), then its whole text, then each line. Not `Extra`'s spans (`SPAN`), which the
/// relations core.rr's programs read beyond T's are made over.
struct Locs {
    lines: core_rr::Lines,
    spans: Vec<[u32; 2]>,
    tokens: u32,
}

impl Projected {
    /// Document `key`'s projection in `shape`; `dialect` is its facts' (`None` for a text-only document, which has no
    /// node).
    fn new(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape) -> Self {
        let f = &*facts;
        let mut p = Projected {
            key,
            tag: f.tag,
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
            if !f.named(id) || dialect.is_none_or(|dialect| shape == Shape::T && dialect.class(f, id) == Class::Comment) {
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
            let lines = core_rr::Lines::new(&f.source, dialect.is_some_and(|d| d.family == Family::Python));
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
            p.extra = Some(Box::new(Locs { lines, spans, tokens }));
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
    /// The relations made as rows for what the program reads (`Extra` and those beside it), bound in place of a view
    /// of the same name (`line`, `end_line`, `col`, `docof`: their spans have places too).
    made: BTreeSet<&'static str>,
}

/// A span's id (core.rr's `span`: a piece of a document's text): `d << 32 | SPAN | k`, past every node of its
/// document, so no node view has a row of it (`Batch::node`). `k` is 0 for the whole source (`source`), `l` for
/// physical line `l` (`src_line`), `LF | l` for `\n`-line `l` (`src_line_lf`), `TAPE | l` for the tape's line `l`
/// (`tape_line`), `COMMENT | k` for the document's `k`-th comment from 1, in source order, of those no node is
/// (`comment`: T's nodes leave out a grammar's `comment` kind, as Python's, but keep Rust's `line_comment`, which is
/// then the comment's id) and `MATCH | i` for the document's `i`th match or group of a `match_all` or
/// `match_group` regex.
pub(crate) const SPAN: u32 = 1 << 31;
pub(crate) const LF: u32 = 1 << 30;
pub(crate) const TAPE: u32 = 1 << 29;
pub(crate) const COMMENT: u32 = 1 << 28;
pub(crate) const MATCH: u32 = 1 << 27;

/// The physical lines of `source`, as byte ranges without their terminators: as Python's `str.splitlines()`
/// splits (`splitlines`: `\r\n`, `\n`, `\r`, `\v`, `\f`, `\x1c`-`\x1e`, `\x85`, U+2028, U+2029; W's `lines`
/// regexes), else at `\n` only. A terminator at the end ends the last line: no empty line follows it, and an empty
/// source has none.
pub(crate) fn physical_lines(source: &str, splitlines: bool) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = source.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let ends = ch == '\n'
            || splitlines && matches!(ch, '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !ends {
            continue;
        }
        out.push(start..at);
        start = at + ch.len_utf8();
        if ch == '\r' && chars.peek().is_some_and(|&(_, c)| c == '\n') {
            chars.next();
            start += 1;
        }
    }
    if start < source.len() {
        out.push(start..source.len());
    }
    out
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
    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
    /// `core.rr`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`core_rr::Lines`), rows laid out over `Projected::locs`. A batch that makes
    /// one of them as rows (`Extra`, whose spans have places too) binds those rows instead (`Batch::made`).
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

/// The relations of core.rr a batch makes only for a program that reads them (its `reads`), T's exporter having
/// had none of them, so a program that reads none does no work and interns no text:
///
/// ```text
/// source(d, sp)          src_line(d, l, sp)     src_line_lf(d, l, sp)  tape_line(d, l, sp)
/// line_count(d, k)       node_text(n, s)        end_line(n, l)         col(n, c)
/// docof(n, d)
/// ```
///
/// with `sp` a span (`SPAN`); `src_line` split as `physical_lines` does (Python's `str.splitlines()`; other
/// languages at `\n`), `src_line_lf` at `\n` only (W's `terminators: "lf"`), and `tape_line` the lines `line(n, l)`
/// numbers (the tape's `LineTable`: `\r\n`, `\r` and `\n` for Python; W's `layout`), the empty line after a final
/// terminator included; `line_count` counts `src_line`'s lines. Spans have rows in `text` (their text, terminators
/// left out), `line`, `end_line` and `col` (0): a line span's line is its number; the source's first line is 1 and
/// its end line the line holding its end offset (`src_line`'s numbering). A node's `end_line` is the line holding its
/// end offset, as W's `layout` counts (its last byte's, but where the node ends with a line terminator, as a module
/// may), and its `col` its start's byte column, both on the tape's lines; `node_text` is its source text, exactly.
/// `value(n, v)` is W's value of expression node `n` as a string (`matcher::file_values`: a clause's `value` in its
/// per-file context; Python's `str(v)` of a str, int, float, bool, None or Ellipsis), read off the document's tape
/// (`Document::tape`), no row where W does not know it or it is no scalar; `static_value(n, v)` the same with names
/// followed through their file-local bindings (W's `value.follow`). `value_type_of(n, t)` names the type of W's value
/// of `n` as W's `value.type` does (containers and bytes included, a JavaScript RegExp also `object`), and
/// `value_type(v, t)` pairs `value`'s strings with those types: one row per string and type some node of the batch has
/// (the str `"12"` and the int `12` both give `12`). `comment(c, s)` is each comment's text without its marker
/// (`comment_body`); a comment no node is (`COMMENT`) has rows in `text` and `node_text` (its text, marker included),
/// `line`, `end_line` and `col` as a node's. `docof` has every node and every span (comments and matches included),
/// each with its document (a span's `sp >> 32`, as a node's). Rows hold string indexes, as `Batch::build`'s do.
///
/// In `core.rr`'s shape (`Shape::Core`) a relation made here is the batch's, in place of `core_rr`'s rows or the `Loc`
/// view of that name (`Batch::build`), and its node rows cover that shape's anonymous tokens too (`Locs`); `comment`
/// is `core_rr`'s there (comments are nodes), so it is not made here.
///
///
/// And each `matchall_<k>(sp, m)` read (`logic::strings::as_relations`, core.rr's `match_all`): `m` each match of
/// regex `k` in span `sp`'s text, leftmost-first and non-overlapping (Rust's `find_iter`, as W's `regex` reads a
/// file), over every span above. A match is a span (`MATCH`) with rows in `text` (what it matched), `line`,
/// `end_line` and `col`, as W's finding of that regex is placed: in a line span, that line and the byte column in it
/// (W's `lines` regexes); in the source, the lines and column of the tape's line table (W's content regex).
#[derive(Default)]
struct Extra {
    source: bool,
    src_line: bool,
    src_line_lf: bool,
    tape_line: bool,
    line_count: bool,
    text: bool,
    line: bool,
    end_line: bool,
    col: bool,
    node_text: bool,
    docof: bool,
    value: bool,
    static_value: bool,
    value_type_of: bool,
    value_type: bool,
    comment: bool,
    sources: Vec<[i64; 2]>,
    src_lines: Vec<[i64; 3]>,
    src_lines_lf: Vec<[i64; 3]>,
    tape_lines: Vec<[i64; 3]>,
    line_counts: Vec<[i64; 2]>,
    lines: Vec<[i64; 2]>,
    end_lines: Vec<[i64; 2]>,
    cols: Vec<[i64; 2]>,
    node_texts: Vec<[i64; 2]>,
    docofs: Vec<[i64; 2]>,
    values: Vec<[i64; 2]>,
    static_values: Vec<[i64; 2]>,
    value_types_of: Vec<[i64; 2]>,
    value_types: BTreeSet<[i64; 2]>,
    comments: Vec<[i64; 2]>,
    /// The regexes of the `match_all` and `match_group` relations read (`Found`).
    matches: Vec<Found>,
}

/// Regex `k`'s rows: its matches (`matchall_<k>(sp, m)`, when read) and their named groups' spans
/// (`matchgroup_<k>(sp, m, g, gsp)`, when read; `g` a string index), with one numbering of `m` for both.
struct Found {
    k: usize,
    regex: Arc<regex::Regex>,
    all: bool,
    groups: bool,
    rows: Vec<[i64; 2]>,
    group_rows: Vec<[i64; 4]>,
}

/// A span made for a document: its id, its bytes, and its line where it is one line (else the source).
type Made = (i64, std::ops::Range<usize>, Option<usize>);

impl Extra {
    fn new(reads: &BTreeSet<String>, matches: &[Arc<regex::Regex>], shape: Shape) -> Self {
        let read = |rel: &str| reads.contains(rel);
        Self {
            source: read("source"),
            src_line: read("src_line"),
            src_line_lf: read("src_line_lf"),
            tape_line: read("tape_line"),
            line_count: read("line_count"),
            text: read("text"),
            line: read("line"),
            end_line: read("end_line"),
            col: read("col"),
            node_text: read("node_text"),
            docof: read("docof"),
            value: read("value"),
            static_value: read("static_value"),
            value_type_of: read("value_type_of"),
            value_type: read("value_type"),
            comment: shape == Shape::T && read("comment"),
            matches: matches
                .iter()
                .enumerate()
                .map(|(k, regex)| Found {
                    k,
                    regex: Arc::clone(regex),
                    all: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::MATCHES)),
                    groups: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::GROUPS)),
                    rows: Vec::new(),
                    group_rows: Vec::new(),
                })
                .filter(|found| found.all || found.groups)
                .collect(),
            ..Self::default()
        }
    }

    fn spans(&self) -> bool {
        self.source || self.src_line || self.src_line_lf || self.tape_line || self.comment
    }

    /// Whether `line` is made as rows rather than read off the node view: spans have lines too.
    fn line_rows(&self) -> bool {
        self.line && self.spans()
    }

    /// A span's rows in `text` (into `text`), `line`, `end_line` and `col` (its start's byte column).
    #[allow(clippy::too_many_arguments)]
    fn span(&mut self, sp: i64, first: usize, last: usize, col: usize, body: &str, text: &mut Vec<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) {
        if self.docof {
            self.docofs.push([sp, sp >> 32]);
        }
        if self.text {
            text.push([sp, string(body) as i64]);
        }
        if self.line_rows() {
            self.lines.push([sp, first as i64]);
        }
        if self.end_line {
            self.end_lines.push([sp, last as i64]);
        }
        if self.col {
            self.cols.push([sp, col as i64]);
        }
    }

    /// Document `p`'s node rows: `line`, `end_line`, `col`, `node_text` and `docof`; and `line`, `end_line`, `col` and
    /// `docof` of `core.rr`'s anonymous tokens (`Locs`), which follow the nodes.
    fn nodes(&mut self, p: &Projected, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
        let tokens = p.extra.as_ref().map_or(0, |x| x.tokens);
        if self.line_rows() || self.end_line || self.col || self.node_text {
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                let (start, end) = (f.start[id as usize] as usize, f.end[id as usize] as usize);
                if self.line_rows() {
                    self.lines.push([n, i64::from(f.start_line[id as usize])]);
                }
                if self.end_line {
                    self.end_lines.push([n, f.lines.line(end) as i64 + 1]);
                }
                if self.col {
                    self.cols.push([n, (start - f.lines.start(f.lines.line(start))) as i64]);
                }
                if self.node_text {
                    self.node_texts.push([n, string(f.text(id)) as i64]);
                }
            }
            for local in p.len()..p.len() + tokens {
                let n = p.global(local);
                let [start, end] = p.span(local).map(|at| at as usize);
                if self.line_rows() {
                    self.lines.push([n, f.lines.line(start) as i64 + 1]);
                }
                if self.end_line {
                    self.end_lines.push([n, f.lines.line(end) as i64 + 1]);
                }
                if self.col {
                    self.cols.push([n, (start - f.lines.start(f.lines.line(start))) as i64]);
                }
            }
        }
        if self.docof {
            let key = i64::from(p.key);
            self.docofs.extend((0..p.len() + tokens).map(|local| [p.global(local), key]));
        }
    }

    /// Document `p`'s rows (its `Document` `d`, its grammar's kinds `dialect`, none for a text-only document);
    /// `splitlines`: its lines split as Python's `str.splitlines()` does.
    #[allow(clippy::too_many_arguments)]
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, splitlines: bool, text: &mut Vec<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let f = &*p.facts;
        let plain = self.value || self.value_type_of || self.value_type;
        // A text-only document has no node, so no value.
        if (plain || self.static_value) && dialect.is_some() {
            let Some((tape, file)) = &d.tape else {
                return Err(StoreError::Invalid(format!("{}: W's values need the document's tape", d.path)));
            };
            if tape.transducer_len() != f.len() {
                return Err(StoreError::Invalid(format!("{}: the tape is not the one the facts were read off", d.path)));
            }
            let (value, value_type_of, value_type) = (self.value, self.value_type_of, self.value_type);
            let (values, static_values, types_of, types) = (&mut self.values, &mut self.static_values, &mut self.value_types_of, &mut self.value_types);
            crate::code_rule_dsl::matcher::file_values(tape, file, plain, self.static_value, &mut |followed, ordinal, v, names| {
                let local = p.local[ordinal as usize];
                if local == NONE {
                    return;
                }
                // A value's text is interned only for a relation that holds it.
                let v = v.filter(|_| followed || value || value_type).map(|v| string(&v) as i64);
                let n = p.global(local);
                if followed {
                    static_values.extend(v.map(|v| [n, v]));
                    return;
                }
                if value && let Some(v) = v {
                    values.push([n, v]);
                }
                for name in names {
                    let t = string(name) as i64;
                    if value_type_of {
                        types_of.push([n, t]);
                    }
                    if value_type && let Some(v) = v {
                        types.insert([v, t]);
                    }
                }
            });
        }
        self.nodes(p, string);
        let key = i64::from(p.key);
        let span = |k: u32| key << 32 | i64::from(SPAN | k);
        let source = f.source.as_str();
        if self.comment && let Some(dialect) = dialect {
            use crate::code_rule_dsl::matcher::is_comment_kind;
            let mut k = 0;
            for id in 0..f.len() as u32 {
                // A comment kind's node, not inside another (Rust's doc comment parts are not comments).
                if !f.named(id) || !is_comment_kind(dialect.name(f, id)) || f.parent(id).is_some_and(|q| is_comment_kind(dialect.name(f, q))) {
                    continue;
                }
                let whole = f.text(id);
                let c = match p.local[id as usize] {
                    NONE => {
                        k += 1;
                        let c = span(COMMENT | k);
                        let (start, end) = (f.start[id as usize] as usize, f.end[id as usize] as usize);
                        let col = start - f.lines.start(f.lines.line(start));
                        self.span(c, f.start_line[id as usize] as usize, f.lines.line(end) + 1, col, whole, text, string);
                        if self.node_text {
                            self.node_texts.push([c, string(whole) as i64]);
                        }
                        c
                    }
                    local => p.global(local),
                };
                self.comments.push([c, string(comment_body(whole)) as i64]);
            }
        }
        if !(self.spans() || self.line_count) {
            return Ok(());
        }
        let physical = physical_lines(source, splitlines);
        if self.line_count {
            self.line_counts.push([key, physical.len() as i64]);
        }
        let mut made: Vec<Made> = Vec::new();
        if self.source {
            let sp = span(0);
            self.sources.push([key, sp]);
            let terminated = physical.last().map_or(!source.is_empty(), |r| r.end < source.len());
            self.span(sp, 1, (physical.len() + usize::from(terminated)).max(1), 0, source, text, string);
            made.push((sp, 0..source.len(), None));
        }
        if self.src_line {
            for (i, r) in physical.iter().enumerate() {
                let (l, sp) = (i + 1, span(i as u32 + 1));
                self.src_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], text, string);
                made.push((sp, r.clone(), Some(l)));
            }
        }
        if self.src_line_lf {
            for (i, r) in physical_lines(source, false).into_iter().enumerate() {
                let (l, sp) = (i + 1, span(LF | (i as u32 + 1)));
                self.src_lines_lf.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], text, string);
                made.push((sp, r, Some(l)));
            }
        }
        if self.tape_line {
            for i in 0..f.lines.starts().len() {
                let (l, sp) = (i + 1, span(TAPE | (i as u32 + 1)));
                let r = f.lines.start(i)..f.lines.end(i);
                self.tape_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], text, string);
                made.push((sp, r, Some(l)));
            }
        }
        // Each regex's matches in each span, and their named groups, numbered through the document.
        let mut next: u32 = 0;
        let mut id = |p: &Projected| -> StoreResult<i64> {
            if next >= MATCH {
                return Err(StoreError::Invalid(format!("document {} has more than {MATCH} match_all matches and groups", p.key)));
            }
            next += 1;
            Ok(span(MATCH | (next - 1)))
        };
        // A piece of span `r` (`at` within it) placed as W places a regex's finding there.
        let place = |r: &std::ops::Range<usize>, line: &Option<usize>, at: std::ops::Range<usize>| match line {
            Some(l) => (*l, *l, at.start),
            None => {
                let s = f.lines.span(r.start + at.start, r.start + at.end);
                (s.start_line + 1, s.end_line + 1, s.start_column)
            }
        };
        for k in 0..self.matches.len() {
            let (regex, groups) = (Arc::clone(&self.matches[k].regex), self.matches[k].groups);
            let names: Vec<&str> = regex.capture_names().flatten().collect();
            for (sp, r, line) in &made {
                let body = &source[r.clone()];
                for captures in regex.captures_iter(body) {
                    let found = captures.get(0).expect("group 0 always matches");
                    let m = id(p)?;
                    self.matches[k].rows.push([*sp, m]);
                    let (first, last, col) = place(r, line, found.range());
                    self.span(m, first, last, col, found.as_str(), text, string);
                    if !groups {
                        continue;
                    }
                    // A group that takes no part in the match has no row.
                    for name in &names {
                        let Some(group) = captures.name(name) else { continue };
                        let g = id(p)?;
                        self.matches[k].group_rows.push([*sp, m, string(name) as i64, g]);
                        let (first, last, col) = place(r, line, group.range());
                        self.span(g, first, last, col, group.as_str(), text, string);
                    }
                }
            }
        }
        Ok(())
    }

    /// The relations read, their strings as atoms (`atom` of a string index), and the `match_all` relations read.
    #[allow(clippy::type_complexity)]
    fn relations(self, atom: &dyn Fn(i64) -> i64) -> (Vec<(&'static str, Relation)>, Vec<(String, Relation)>) {
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let mut out = Vec::new();
        if self.source {
            out.push(("source", relation(&["d", "sp"], &mut self.sources.iter().map(|r| r.to_vec()))));
        }
        if self.src_line {
            out.push(("src_line", relation(&["d", "l", "sp"], &mut self.src_lines.iter().map(|r| r.to_vec()))));
        }
        if self.src_line_lf {
            out.push(("src_line_lf", relation(&["d", "l", "sp"], &mut self.src_lines_lf.iter().map(|r| r.to_vec()))));
        }
        if self.tape_line {
            out.push(("tape_line", relation(&["d", "l", "sp"], &mut self.tape_lines.iter().map(|r| r.to_vec()))));
        }
        if self.line_count {
            out.push(("line_count", relation(&["d", "k"], &mut self.line_counts.iter().map(|r| r.to_vec()))));
        }
        if self.line_rows() {
            out.push(("line", relation(&["n", "l"], &mut self.lines.iter().map(|r| r.to_vec()))));
        }
        if self.end_line {
            out.push(("end_line", relation(&["n", "l"], &mut self.end_lines.iter().map(|r| r.to_vec()))));
        }
        if self.col {
            out.push(("col", relation(&["n", "c"], &mut self.cols.iter().map(|r| r.to_vec()))));
        }
        if self.node_text {
            out.push(("node_text", relation(&["n", "s"], &mut self.node_texts.iter().map(|&[n, s]| vec![n, atom(s)]))));
        }
        if self.docof {
            out.push(("docof", relation(&["n", "d"], &mut self.docofs.iter().map(|r| r.to_vec()))));
        }
        if self.value {
            out.push(("value", relation(&["n", "v"], &mut self.values.iter().map(|&[n, v]| vec![n, atom(v)]))));
        }
        if self.static_value {
            out.push(("static_value", relation(&["n", "v"], &mut self.static_values.iter().map(|&[n, v]| vec![n, atom(v)]))));
        }
        if self.value_type_of {
            out.push(("value_type_of", relation(&["n", "t"], &mut self.value_types_of.iter().map(|&[n, t]| vec![n, atom(t)]))));
        }
        if self.value_type {
            out.push(("value_type", relation(&["v", "t"], &mut self.value_types.iter().map(|&[v, t]| vec![atom(v), atom(t)]))));
        }
        if self.comment {
            out.push(("comment", relation(&["n", "s"], &mut self.comments.iter().map(|&[n, s]| vec![n, atom(s)]))));
        }
        let mut matched = Vec::new();
        for found in self.matches {
            if found.all {
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::MATCHES, found.k), relation(&["sp", "m"], &mut found.rows.iter().map(|r| r.to_vec()))));
            }
            if found.groups {
                let rows = &mut found.group_rows.iter().map(|&[sp, m, g, gsp]| vec![sp, m, atom(g), gsp]);
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::GROUPS, found.k), relation(&["sp", "m", "g", "gsp"], rows)));
            }
        }
        (out, matched)
    }
}

/// The small relations of a document's nodes, with string indexes: `text(n, s)`, `intval(n, v)`, `op(n, s)` (T's,
/// also bound as `op_t`), and the text relations (decision 0037) `name(n, s)` (an identifier's text, an attribute's
/// dotted text), `str(n, s)` (a string literal's content, between its quotes) and `strprefix(n, p)` (its prefix,
/// lowercased). In `core.rr`'s shape only `intval` and T's `op`: `core_rr` makes the others as `core.rr` states them.
#[derive(Default)]
struct NodeRows {
    text: Vec<[i64; 2]>,
    intval: Vec<[i64; 2]>,
    op: Vec<[i64; 2]>,
    name: Vec<[i64; 2]>,
    strs: Vec<[i64; 2]>,
    strprefix: Vec<[i64; 2]>,
}

impl NodeRows {
    /// Document `p`'s rows (`dialect` its grammar's kinds; `core`: in `core.rr`'s shape).
    fn document(&mut self, p: &Projected, dialect: &Dialect, core: bool, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
        for (local, &id) in p.core.iter().enumerate() {
            let n = p.global(local as u32);
            match dialect.class(f, id) {
                Class::Text | Class::Attribute | Class::String if core => {}
                Class::Text => {
                    let t = f.text(id);
                    if !t.contains(['\t', '\n', '\r', '\\']) {
                        self.text.push([n, string(t) as i64]);
                        self.name.push([n, string(t) as i64]);
                    }
                }
                Class::Attribute => {
                    if let Some(dotted) = dialect.dotted(f, id) {
                        self.name.push([n, string(&dotted) as i64]);
                    }
                }
                Class::String => {
                    if let Some((prefix, content)) = dialect.string(f, id) {
                        self.strprefix.push([n, string(&prefix) as i64]);
                        if let Some(content) = content {
                            self.strs.push([n, string(content) as i64]);
                        }
                    }
                }
                Class::Integer => {
                    if let Ok(v) = f.text(id).replace('_', "").parse::<i64>() {
                        self.intval.push([n, v]);
                    }
                }
                Class::Operator => {
                    let operator = dialect.operator.filter(|&o| o != 0);
                    if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field[c as usize])) {
                        self.op.push([n, string(dialect.name(f, o)) as i64]);
                    }
                }
                // `async def`: the keyword is an anonymous child, recorded as the definition's modifier.
                Class::FunctionDefinition if f.children(id).first().is_some_and(|&k| dialect.name(f, k) == "async") => {
                    self.op.push([n, string("async") as i64]);
                }
                Class::FunctionItem
                    if f.children(id).iter().any(|&m| {
                        dialect.name(f, m) == "function_modifiers" && f.children(m).iter().any(|&k| dialect.name(f, k) == "async")
                    }) =>
                {
                    self.op.push([n, string("async") as i64]);
                }
                _ => {}
            }
        }
    }
}

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`), and those
    /// of core.rr's the program `reads` that T had not (`Extra`), the `match_all` relations among them (of `matches`)
    /// apart. A text-only document (`core::TEXT`) has the document's relations and its text's, and no node.
    /// Also returns the batch's distinct strings and their symbols: its vocabulary, which text tests read
    /// (`logic::strings::Vocabulary`, decision 0037).
    #[allow(clippy::type_complexity)]
    fn build(
        store: &ContentStore,
        documents: Vec<Document>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
    ) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, Vec<(String, Relation)>, (Vec<String>, Vec<u32>))> {
        let shape = projection.shape;
        let core = shape == Shape::Core;
        // What the program reads, for the relations made only for a program that reads them (`Extra` and those beside
        // it); a projection that names none (T's, `Projection::core`'s) reads none of them.
        let none = BTreeSet::new();
        let reads = projection.reads.as_ref().unwrap_or(&none);
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
        let (mut doc, mut pkg) = (Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
        // `text`, `intval`, T's `op` and the text relations (`NodeRows`).
        let mut rows = NodeRows::default();
        // `syntax_error(d)`: the documents whose parse has an error (`Facts::syntax_errors`); `core.rr`'s
        // `syntax_error(d, l, msg)` (`core_rr`): the first error's line and message.
        let mut errors = Vec::new();
        let mut extra = Extra::new(reads, matches, shape);
        let mut tree = tree::Tree::new(reads);
        let mut fstrings = fstring::FStrings::new(reads, markers);
        let mut cpython = cpython::CPython::new(reads);
        let mut annotations = annotation::Annotations::new(reads);
        // The documents of the string annotations' trees, placed after the real documents (keys `annotation::PARSED`).
        let mut parsed = Vec::new();
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `core.rr`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        let wants_text = core && projection.wants("text");
        let wants_tokens = core && ["first_token", "last_token", "token_before", "token_after"].iter().any(|r| projection.wants(r));
        // Only a program that reads it: every node's hash is a symbol.
        let wants_digests = core && projection.reads.as_ref().is_some_and(|r| r.contains("tree_digest"));
        let mut docs = Vec::with_capacity(documents.len());
        for d in &documents {
            // The document's own dialect: its first node's kind may be ERROR's, past the stride (`kind_atom`).
            let tag = d.facts.tag;
            let dialect = if tag == TEXT {
                None
            } else {
                if !dialects.contains_key(&tag) {
                    dialects.insert(tag, Dialect::new(tag)?);
                }
                Some(&dialects[&tag])
            };
            let p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape);
            let key = i64::from(d.key);
            // core.rr: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            let py_file = d.path.ends_with(".py");
            let dotted = py_file || d.path.ends_with(".rs");
            let module = if core && !dotted { String::new() } else { module(&d.path) };
            doc.push([key, string(&d.path) as i64, string(&module) as i64]);
            if !core || py_file {
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
            if let Some(dialect) = dialect {
                rows.document(&p, dialect, core, &mut string);
                if core {
                    for local in 0..p.len() {
                        c.node(&p, dialect, local, &mut string, wants_text);
                    }
                    c.document(&p, dialect, &mut string, wants_text, wants_tokens);
                    if wants_digests {
                        c.digests(&p, dialect, &mut string);
                    }
                }
            }
            // A text document's lines split as W's `lines` regexes split any file's.
            extra.document(&p, d, dialect, python == Some(tag) || tag == TEXT, &mut rows.text, &mut string)?;
            tree.document(&p, dialect, python == Some(tag), &mut string);
            fstrings.document(&p, d, dialect, python == Some(tag), &mut string)?;
            cpython.document(&p, dialect, python == Some(tag), &mut string);
            parsed.extend(annotations.document(&p, d, dialect, python == Some(tag), &mut string)?);
            docs.push(p);
        }
        if let Some(p) = parsed.first() {
            if docs.last().is_some_and(|q| q.key >= annotation::PARSED) {
                return Err(StoreError::Invalid(format!("a document key reaches {}, where string annotations' trees are numbered", annotation::PARSED)));
            }
            let dialect = &dialects[&p.tag];
            // An annotation's tree is T's projection of its nodes (no comment, no token of its own); in `core.rr`'s
            // shape its nodes have `core_rr`'s node rows.
            for p in parsed {
                rows.document(&p, dialect, core, &mut string);
                if core {
                    for local in 0..p.len() {
                        c.node(&p, dialect, local, &mut string, wants_text);
                    }
                }
                extra.nodes(&p, &mut string);
                docs.push(p);
            }
        }
        let vocabulary = python.filter(|tag| dialects.contains_key(tag)).map(|_| {
            (py::CONTEXTS.map(|name| string(name) as i64), py::SPREADS.map(|name| string(name) as i64))
        });
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        drop(intern);
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
            ("intval", pairs(&["n", "v"], &rows.intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &rows.op, [false, true])),
        ];
        let (mut read, matched) = extra.relations(&atom);
        read.extend(tree.relations(&atom));
        read.extend(fstrings.relations(&atom));
        read.extend(cpython.relations(&atom));
        read.extend(annotations.relations(&atom));
        // The relations made for what the program reads are the batch's: in `core.rr`'s shape they stand in for
        // `core_rr`'s rows and the views of the same name (W's values, the spans and places `match_all` reads,
        // `op_at`'s operators, `visit_index`'s CPython order).
        let made: BTreeSet<&'static str> = read.iter().map(|(name, _)| *name).collect();
        if core {
            // `core_rr`'s `text` and the text of `Extra`'s spans.
            c.extend_text(&rows.text);
            relations.extend(c.relations(&atom).into_iter().filter(|(name, _)| !made.contains(name)));
        } else {
            relations.extend([
                ("text", pairs(&["n", "s"], &rows.text, [false, true])),
                ("op", pairs(&["n", "s"], &rows.op, [false, true])),
                ("name", pairs(&["n", "s"], &rows.name, [false, true])),
                ("str", pairs(&["n", "s"], &rows.strs, [false, true])),
                ("strprefix", pairs(&["n", "s"], &rows.strprefix, [false, true])),
                ("syntax_error", relation(&["d"], &mut errors.iter().map(|&d| vec![d]))),
            ]);
        }
        relations.extend(read);
        let offsets = |count: &dyn Fn(&Projected) -> usize| {
            let mut v = vec![0];
            docs.iter().for_each(|p| v.push(v.last().unwrap() + count(p)));
            v
        };
        let nodes = offsets(&|p| p.core.len());
        let children = offsets(&|p| p.core.len().saturating_sub(1));
        let fields = offsets(&|p| p.fielded.len());
        let locs = offsets(&|p| p.locs() as usize);
        Ok((Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, py, made }), relations, matched, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth => &self.nodes,
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
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Loc(_) => Some(local as usize),
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
            View::Line | View::Depth | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Depth | View::Loc(_) => None,
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
        self.lookup_docs(column, values, None)
    }
    fn doc_column(&self) -> Option<usize> {
        matches!(self.view, View::Node | View::Core).then_some(0)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        matches!((self.view, column), (View::Node, 2) | (View::Core, 1))
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        if !self.kept_to_doc(column) {
            return None;
        }
        // A document not in the batch has no rows.
        Some(match self.batch.doc(doc) {
            Some(doc) => self.lookup_docs(column, values, Some(doc))?,
            None => Vec::new(),
        })
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.value_at(row, column)
    }
}
impl Viewed {
    /// The rows whose `column` is one of `values`, in document `only` (by index) where the column spans the
    /// documents (a kind, a tape node), else in every document.
    fn lookup_docs(&self, column: usize, values: &[i64], only: Option<usize>) -> Option<Vec<usize>> {
        let b = &*self.batch;
        let kept = |doc: &usize| only.is_none_or(|o| o == *doc);
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
                for (doc, p) in b.docs.iter().enumerate().filter(|(doc, _)| kept(doc)) {
                    if let Some(&local) = p.local.get(t).filter(|&&local| local != NONE) {
                        rows.push(b.nodes[doc] + local as usize);
                    }
                }
            } else if matches!(self.view, View::Node) && column == 2 {
                // The constant's dialect is the batch's whose code it is (ERROR's reaches past the stride).
                let Ok(v) = u32::try_from(v) else { continue };
                for (&tag, dialect) in &b.dialects {
                    let Some(code) = kind_code(tag, v).filter(|&c| u32::from(c) < KIND_STRIDE || c == u16::MAX) else { continue };
                    // ERROR has no canonical entry: its nodes hold its atom itself.
                    let atoms: Vec<u32> = match dialect.codes.get(&code) {
                        Some(codes) => codes.iter().map(|&c| kind_atom(tag, c)).collect(),
                        None if code == u16::MAX => vec![v],
                        None => Vec::new(),
                    };
                    for (doc, p) in b.docs.iter().enumerate().filter(|(doc, p)| p.tag == tag && kept(doc)) {
                        for &kind in &atoms {
                            rows.extend(p.facts.of_kind(kind).iter().map(|&id| p.local[id as usize]).filter(|&l| l != NONE).map(|l| b.nodes[doc] + l as usize));
                        }
                    }
                }
            } else {
                return None;
            }
        }
        Some(rows)
    }
    fn value_at(&self, row: usize, column: usize) -> i64 {
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
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent[l] == NONE { -1 } else { p.global(p.parent[l]) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => self.batch.dialects[&p.tag].kind(&p.facts, id as u32),
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line[id]),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx[l]),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size[l]),
            (View::Tape, 3) => i64::from(p.depth[l]),
            (View::Depth, 1) => i64::from(p.depth[l]),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub(crate) fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// Document `key`'s `function_definition` and `class_definition` nodes by byte range (start, end), as ids of
/// `node`: what W's call summaries name a definition by (`calls::summary::Def`).
pub(crate) fn definitions(key: u32, facts: &Arc<Facts>) -> StoreResult<HashMap<(u32, u32), i64>> {
    let dialect = Dialect::new((facts.kind.first().copied().unwrap_or(0) / KIND_STRIDE) as u8)?;
    let p = Projected::new(key, Arc::clone(facts), Some(&dialect), Shape::T);
    Ok(p.core
        .iter()
        .enumerate()
        .filter(|(_, id)| matches!(dialect.name(facts, **id), "function_definition" | "class_definition"))
        .map(|(local, &id)| ((facts.start[id as usize], facts.end[id as usize]), p.global(local as u32)))
        .collect())
}

/// T's nine base relations over `documents`, and the `py.*` families, by table name.
pub(crate) fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub(crate) fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations, _, _) = Batch::build(store, documents, projection, &[], &[])?;
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
            ("ast_depth", View::Depth),
        ]),
    }
    // A relation the batch made as rows (`Batch::made`: its spans have places too) is bound in place of the view.
    for (name, view) in named.into_iter().filter(|(name, _)| !batch.made.contains(name)) {
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
            // An annotation's tree is no file: it has no call sites of its own.
            for p in self.docs.iter().filter(|p| p.tag == py.grammar.tag() && p.key < annotation::PARSED) {
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
                kind.push(self.dialects[&p.tag].kind(&p.facts, p.core[local]));
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

/// `rr_nearest_<set>(n, a)`: `a` is `n`'s nearest strict ancestor of a kind in the set, a row per node that has
/// one, by `n` and by `a`; `rr_inside_<set>(n, a)`, a row per such ancestor; or `rr_has_<set>(n, a)`, n of a kind
/// in the set and a row per strict ancestor a of it (decision 0036). Made by the transducer's file-local FIX
/// (`typing::file_local::nearest`), or for `has` by a climb from each node of the set. Rows are node rows
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

/// T's base relations over `documents` (`sources`) in `projection`'s shape and those of core.rr's the program reads
/// that T had not (`Extra`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms),
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
    let (batch, relations, matched, (strings, symbols)) = Batch::build(store, documents, projection, &tests.matches, &tests.markers)?;
    drop(built);
    let mut out: Vec<(String, Arc<dyn Source>)> = relations.into_iter().map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
    // The `match_all` relations (`logic::strings::as_relations`): each regex's matches in the batch's spans.
    out.extend(matched.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)));
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
    // Containment (decision 0050): made per looked-up node, so a program that never reads it pays nothing.
    out.push((WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(&batch), within::Mode::Tree))));
    out.push((SPAN_WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(&batch), within::Mode::Span))));
    Ok(out)
}

/// The planner's statistics of `source` (`stat_col`'s rows, ndv, sq and sqd per column, `sqd` by
/// document `d = id >> 32`), for a relation made at bind time rather than at ingestion.
pub(crate) fn statistics(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    if let Some(known) = source.statistics() {
        return known;
    }
    let rows = source.len();
    // Postings by sorting (no map per value): a run of equal values is one value's postings.
    let moments = |mut values: Vec<(i64, i64)>| -> (i64, i64) {
        values.sort_unstable();
        let (mut ndv, mut sq, mut at) = (0, 0, 0);
        while at < values.len() {
            let run = values[at..].iter().take_while(|v| **v == values[at]).count();
            ndv += 1;
            sq += (run * run) as i64;
            at += run;
        }
        (ndv, sq)
    };
    // Each row's document (the first column's high half), read once for every column.
    let docs: Vec<i64> = (0..rows).map(|row| source.value(row, 0) >> 32).collect();
    source
        .columns()
        .into_iter()
        .enumerate()
        .map(|(c, name)| {
            let values: Vec<i64> = (0..rows).map(|row| source.value(row, c)).collect();
            // A key column (strictly increasing, as a node column of a relation in node order): every value once,
            // and so every (document, value) once. No sort.
            if values.windows(2).all(|pair| pair[0] < pair[1]) {
                return (name, rows as i64, rows as i64, rows as i64, rows as i64);
            }
            let (ndv, sq) = moments(values.iter().map(|&v| (0, v)).collect());
            let (_, sqd) = moments(docs.iter().zip(&values).map(|(&d, &v)| (d, v)).collect());
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
pub(crate) mod annotation;
pub(crate) mod cpython;
mod fstring;
mod tree;
