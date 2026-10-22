//! Exported cross-file facts (Spike B's part of fact set v0): extraction from one Python content
//! and the `.xfacts` file format shared with Spike A's `.facts` layout.
//!
//! Every relation is a pure function of the file bytes; nothing depends on the path. Rows are
//! sorted and distinct, except `import`, whose rows keep source order (later binding wins).
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser, Tree};

pub const SCHEMA_VERSION: u32 = 0;
pub const MAGIC: &[u8; 8] = b"RRFACTS\0";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Def {
    pub name: String,
    /// `function` (not inside a class), `method` (nearest enclosing class), or `class`.
    pub kind: String,
    /// Jeremy's id without the module: `Class.name` for methods, `name` otherwise.
    pub qualname: String,
    pub line: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Import {
    /// Dotted module as written, leading dots kept for relative imports (`..pkg`, `.`).
    pub module: String,
    /// The imported member for `from M import name`; None for `import M`.
    pub name: Option<String>,
    /// The name the statement binds in its scope.
    pub local: String,
    /// The statement is a direct child of `module`.
    pub top: bool,
    /// The statement binds at module scope: not inside any function or class.
    pub module_scope: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Exported {
    pub defs: Vec<Def>,
    pub refs: Vec<String>,
    pub imports: Vec<Import>,
    /// (caller qualname, callee dotted text); only calls whose callee head is not bound locally.
    pub calls: Vec<(String, String)>,
    pub decorators: Vec<(String, String)>,
    /// First module-level `__all__ = ...`: Some(true) when a list/tuple literal.
    pub dunder_all: Option<bool>,
    pub dunder_all_names: Vec<String>,
    pub seeds: Vec<(String, String)>,
    pub has_error: bool,
}

pub fn language() -> tree_sitter::Language {
    unsafe extern "C" {
        fn tree_sitter_python() -> *const ();
    }
    let f = unsafe { tree_sitter_language::LanguageFn::from_raw(tree_sitter_python) };
    tree_sitter::Language::new(f)
}

pub fn parser() -> Parser {
    let mut p = Parser::new();
    p.set_language(&language()).expect("python grammar");
    p
}

pub fn parse(parser: &mut Parser, src: &[u8]) -> Tree {
    parser.parse(src, None).expect("parse")
}

fn text<'a>(n: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

/// Dotted text of an identifier or an attribute chain of identifiers; None for anything else.
pub fn dotted(n: Node, src: &[u8]) -> Option<String> {
    match n.kind() {
        "identifier" => Some(text(n, src).to_string()),
        "attribute" => {
            let obj = n.child_by_field_name("object")?;
            let attr = n.child_by_field_name("attribute")?;
            let head = dotted(obj, src)?;
            Some(format!("{head}.{}", text(attr, src)))
        }
        _ => None,
    }
}

fn is_mp_string(s: &str) -> bool {
    let s = s.strip_prefix(['r', 'R', 'u', 'U']).unwrap_or(s);
    s == "\"materialized_pipeline\"" || s == "'materialized_pipeline'"
}

fn is_nested_exec(s: &str) -> bool {
    s == "execute_runtime_nested_steps_async" || s == "execute_runtime_nested_steps_sync"
}

/// Identifiers a binding target binds: names, and the names inside tuple/list patterns.
fn collect_targets(n: Node, src: &[u8], out: &mut BTreeSet<String>) {
    match n.kind() {
        "identifier" => {
            out.insert(text(n, src).to_string());
        }
        "pattern_list" | "tuple_pattern" | "list_pattern" | "tuple" | "list" | "parenthesized_expression"
        | "list_splat_pattern" | "list_splat" | "as_pattern_target" | "dictionary_splat_pattern" => {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                collect_targets(ch, src, out);
            }
        }
        _ => {}
    }
}

struct Owner {
    qualname: String,
    locals: BTreeSet<String>,
    globals: BTreeSet<String>,
    calls: Vec<(String, String)>, // (callee, head)
    seeds: BTreeSet<&'static str>,
}

pub fn extract(src: &[u8], tree: &Tree) -> Exported {
    let root = tree.root_node();
    let mut defs: Vec<Def> = Vec::new();
    let mut refs: BTreeSet<String> = BTreeSet::new();
    let mut imports: Vec<Import> = Vec::new();
    let mut decorators: BTreeSet<(String, String)> = BTreeSet::new();
    let mut calls: BTreeSet<(String, String)> = BTreeSet::new();
    let mut seeds: BTreeSet<(String, String)> = BTreeSet::new();
    let mut dunder_all: Option<bool> = None;
    let mut dunder_all_names: BTreeSet<String> = BTreeSet::new();

    // Explicit DFS. Frame: node, field name in parent, class ctx, owner index, guarded (inside
    // function/class/if/try), entering flag.
    struct Frame<'t> {
        node: Node<'t>,
        class: Option<String>,
        owner: Option<usize>,
        guarded: bool,
        exit_owner: bool,
    }
    let mut stack: Vec<Frame> = vec![Frame { node: root, class: None, owner: None, guarded: false, exit_owner: false }];
    let finish = |o: Owner, calls: &mut BTreeSet<(String, String)>, seeds: &mut BTreeSet<(String, String)>| {
        for (callee, head) in o.calls {
            // `self`/`cls` are parameters, kept: the receiver a method call resolves through.
            if !o.locals.contains(&head) || o.globals.contains(&head) || head == "self" || head == "cls" {
                calls.insert((o.qualname.clone(), callee));
            }
        }
        for s in o.seeds {
            seeds.insert((o.qualname.clone(), s.to_string()));
        }
    };
    let mut open: Vec<Option<Owner>> = Vec::new();
    while let Some(f) = stack.pop() {
        if f.exit_owner {
            let o = open[f.owner.unwrap()].take().unwrap();
            finish(o, &mut calls, &mut seeds);
            continue;
        }
        let n = f.node;
        let kind = n.kind();
        let mut class = f.class.clone();
        let mut owner = f.owner;
        let mut guarded = f.guarded;
        match kind {
            "function_definition" | "class_definition" => {
                let name = n.child_by_field_name("name").map(|x| text(x, src).to_string()).unwrap_or_default();
                if let Some(o) = owner {
                    // Nested definition: binds its name in the owner's scope.
                    open[o].as_mut().unwrap().locals.insert(name);
                } else {
                    let (k, q) = if kind == "class_definition" {
                        ("class", name.clone())
                    } else if let Some(c) = &class {
                        ("method", format!("{c}.{name}"))
                    } else {
                        ("function", name.clone())
                    };
                    defs.push(Def { name: name.clone(), kind: k.into(), qualname: q.clone(), line: n.start_position().row as u32 + 1 });
                    if let Some(p) = n.parent().filter(|p| p.kind() == "decorated_definition") {
                        let mut c = p.walk();
                        for d in p.named_children(&mut c).filter(|d| d.kind() == "decorator") {
                            let t = text(d, src);
                            let t = t.trim_start_matches('@').trim_start();
                            let end = t.find(|ch: char| ch.is_whitespace() || ch == '(').unwrap_or(t.len());
                            if end > 0 {
                                decorators.insert((q.clone(), t[..end].to_string()));
                            }
                        }
                    }
                    if kind == "class_definition" {
                        class = Some(name);
                    } else {
                        open.push(Some(Owner { qualname: q, locals: BTreeSet::new(), globals: BTreeSet::new(), calls: Vec::new(), seeds: BTreeSet::new() }));
                        owner = Some(open.len() - 1);
                        stack.push(Frame { node: n, class: None, owner, guarded, exit_owner: true });
                    }
                }
                guarded = true;
            }
            "if_statement" | "try_statement" => guarded = true,
            "identifier" => {
                let is_def_name = n.parent().is_some_and(|p| {
                    matches!(p.kind(), "function_definition" | "class_definition")
                        && p.child_by_field_name("name").is_some_and(|x| x.id() == n.id())
                });
                if !is_def_name {
                    refs.insert(text(n, src).to_string());
                }
            }
            "import_statement" | "import_from_statement" => {
                let top = n.parent().is_some_and(|p| p.kind() == "module");
                let module_scope = owner.is_none() && class.is_none();
                let module_node = n.child_by_field_name("module_name");
                let mut c = n.walk();
                let items: Vec<Node> = n
                    .children_by_field_name("name", &mut c)
                    .collect();
                for it in items {
                    let (name_node, alias) = if it.kind() == "aliased_import" {
                        (it.child_by_field_name("name").unwrap(), it.child_by_field_name("alias").map(|a| text(a, src).to_string()))
                    } else {
                        (it, None)
                    };
                    let nm = text(name_node, src).to_string();
                    let imp = if let Some(m) = module_node {
                        let local = alias.clone().unwrap_or_else(|| nm.clone());
                        Import { module: text(m, src).split_whitespace().collect(), name: Some(nm), local, top, module_scope }
                    } else {
                        let local = alias.clone().unwrap_or_else(|| nm.split('.').next().unwrap().to_string());
                        Import { module: nm, name: None, local, top, module_scope }
                    };
                    if let Some(o) = owner {
                        open[o].as_mut().unwrap().locals.insert(imp.local.clone());
                    }
                    imports.push(imp);
                }
            }
            "call" => {
                if let (Some(o), Some(func)) = (owner, n.child_by_field_name("function")) {
                    let ow = open[o].as_mut().unwrap();
                    if let Some(d) = dotted(func, src) {
                        let head = d.split('.').next().unwrap().to_string();
                        ow.calls.push((d, head));
                    }
                    // seed: nested execution helper call
                    let fname = match func.kind() {
                        "identifier" => Some(text(func, src)),
                        "attribute" => func.child_by_field_name("attribute").map(|a| text(a, src)),
                        _ => None,
                    };
                    if fname.is_some_and(is_nested_exec) {
                        ow.seeds.insert("nested");
                    }
                    // seed: getattr(ctx, "materialized_pipeline")
                    if func.kind() == "identifier" && text(func, src) == "getattr" {
                        if let Some(args) = n.child_by_field_name("arguments").filter(|a| a.kind() == "argument_list") {
                            let a0 = args.named_child(0);
                            let a1 = args.named_child(1);
                            if a0.is_some_and(|a| a.kind() == "identifier" && text(a, src) == "ctx")
                                && a1.is_some_and(|a| a.kind() == "string" && is_mp_string(text(a, src)))
                            {
                                ow.seeds.insert("mp");
                            }
                        }
                    }
                }
            }
            "attribute" => {
                if let Some(o) = owner {
                    if n.child_by_field_name("attribute").is_some_and(|a| text(a, src) == "materialized_pipeline") {
                        open[o].as_mut().unwrap().seeds.insert("mp");
                    }
                }
            }
            "global_statement" => {
                if let Some(o) = owner {
                    let mut c = n.walk();
                    for id in n.named_children(&mut c) {
                        open[o].as_mut().unwrap().globals.insert(text(id, src).to_string());
                    }
                }
            }
            _ => {}
        }
        // Local bindings inside an owner function.
        if let Some(o) = owner {
            let ow = open[o].as_mut().unwrap();
            match kind {
                "parameters" | "lambda_parameters" => {
                    let mut c = n.walk();
                    for p in n.named_children(&mut c) {
                        match p.kind() {
                            "identifier" | "list_splat_pattern" | "dictionary_splat_pattern" | "tuple_pattern" => collect_targets(p, src, &mut ow.locals),
                            "typed_parameter" => {
                                if let Some(first) = p.named_child(0) {
                                    collect_targets(first, src, &mut ow.locals)
                                }
                            }
                            "default_parameter" | "typed_default_parameter" => {
                                if let Some(nm) = p.child_by_field_name("name") {
                                    collect_targets(nm, src, &mut ow.locals)
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                    if let Some(l) = n.child_by_field_name("left") {
                        collect_targets(l, src, &mut ow.locals)
                    }
                }
                "as_pattern" => {
                    if let Some(a) = n.child_by_field_name("alias") {
                        collect_targets(a, src, &mut ow.locals)
                    }
                }
                "named_expression" => {
                    if let Some(a) = n.child_by_field_name("name") {
                        collect_targets(a, src, &mut ow.locals)
                    }
                }
                _ => {}
            }
        }
        // __all__ (first module-level assignment outside function/class/if/try).
        if kind == "assignment" && !f.guarded && dunder_all.is_none() {
            let left_is_all = n.child_by_field_name("left").is_some_and(|l| l.kind() == "identifier" && text(l, src) == "__all__");
            let in_stmt = n.parent().is_some_and(|p| p.kind() == "expression_statement");
            if left_is_all && in_stmt {
                let right = n.child_by_field_name("right");
                let literal = right.is_some_and(|r| r.kind() == "list" || r.kind() == "tuple");
                dunder_all = Some(literal);
                if literal {
                    let r = right.unwrap();
                    let mut c = r.walk();
                    for el in r.named_children(&mut c) {
                        if el.kind() == "string" || el.kind() == "concatenated_string" {
                            let mut s = String::new();
                            let mut stack2 = vec![el];
                            let mut contents = Vec::new();
                            while let Some(x) = stack2.pop() {
                                if x.kind() == "string_content" {
                                    contents.push(x);
                                } else {
                                    let mut c2 = x.walk();
                                    let ch: Vec<Node> = x.named_children(&mut c2).collect();
                                    stack2.extend(ch.into_iter().rev());
                                }
                            }
                            for x in contents {
                                s.push_str(text(x, src));
                            }
                            dunder_all_names.insert(s);
                        }
                    }
                }
            }
        }
        let mut c = n.walk();
        let children: Vec<Node> = n.children(&mut c).collect();
        for ch in children.into_iter().rev() {
            stack.push(Frame { node: ch, class: class.clone(), owner, guarded, exit_owner: false });
        }
    }
    Exported {
        defs,
        refs: refs.into_iter().collect(),
        imports,
        calls: calls.into_iter().collect(),
        decorators: decorators.into_iter().collect(),
        dunder_all,
        dunder_all_names: dunder_all_names.into_iter().collect(),
        seeds: seeds.into_iter().collect(),
        has_error: root.has_error(),
    }
}

// ---------------------------------------------------------------------------------------------
// File format (`<hash>.xfacts`), Spike A's `.facts` layout:
// magic b"RRFACTS\0", u32 schema_version, [u8;32] content hash, u32 n_strings, strings as
// (u32 len, utf8), u32 n_relations, per relation: u32 name idx, u32 n_rows, u32 n_cols, per column
// (u32 name idx, u8 type: 0=u32 1=u16 2=u8 3=sym 4=opt_u32 5=opt_sym), then each column at native
// width, zero-padded to 4 bytes. All little-endian.

enum Col {
    U32(Vec<u32>),
    U8(Vec<u8>),
    Sym(Vec<u32>),
    OptSym(Vec<u32>),
}

struct Writer {
    strings: Vec<String>,
    index: BTreeMap<String, u32>,
    rels: Vec<(u32, u32, Vec<(u32, Col)>)>,
}

impl Writer {
    fn sym(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), i);
        i
    }
    fn rel(&mut self, name: &str, rows: usize, cols: Vec<(&str, Col)>) {
        let n = self.sym(name);
        let cols = cols.into_iter().map(|(c, v)| (self.sym(c), v)).collect();
        self.rels.push((n, rows as u32, cols));
    }
}

pub fn encode(hash: &[u8; 32], e: &Exported) -> Vec<u8> {
    let mut w = Writer { strings: Vec::new(), index: BTreeMap::new(), rels: Vec::new() };
    let syms = |w: &mut Writer, it: &mut dyn Iterator<Item = &str>| -> Vec<u32> { it.map(|s| w.sym(s)).collect() };
    let c_name = syms(&mut w, &mut e.defs.iter().map(|d| d.name.as_str()));
    let c_kind = syms(&mut w, &mut e.defs.iter().map(|d| d.kind.as_str()));
    let c_q = syms(&mut w, &mut e.defs.iter().map(|d| d.qualname.as_str()));
    let c_line = e.defs.iter().map(|d| d.line).collect();
    w.rel("def", e.defs.len(), vec![("name", Col::Sym(c_name)), ("kind", Col::Sym(c_kind)), ("qualname", Col::Sym(c_q)), ("line", Col::U32(c_line))]);
    let r = syms(&mut w, &mut e.refs.iter().map(|s| s.as_str()));
    w.rel("ref", e.refs.len(), vec![("name", Col::Sym(r))]);
    let m = syms(&mut w, &mut e.imports.iter().map(|i| i.module.as_str()));
    let nm: Vec<u32> = e.imports.iter().map(|i| i.name.as_deref().map(|s| w.sym(s)).unwrap_or(u32::MAX)).collect();
    let l = syms(&mut w, &mut e.imports.iter().map(|i| i.local.as_str()));
    let t = e.imports.iter().map(|i| i.top as u8).collect();
    let ms = e.imports.iter().map(|i| i.module_scope as u8).collect();
    w.rel(
        "import",
        e.imports.len(),
        vec![("module", Col::Sym(m)), ("name", Col::OptSym(nm)), ("local", Col::Sym(l)), ("top", Col::U8(t)), ("module_scope", Col::U8(ms))],
    );
    let a = syms(&mut w, &mut e.calls.iter().map(|c| c.0.as_str()));
    let b = syms(&mut w, &mut e.calls.iter().map(|c| c.1.as_str()));
    w.rel("call_edge", e.calls.len(), vec![("caller", Col::Sym(a)), ("callee", Col::Sym(b))]);
    let a = syms(&mut w, &mut e.decorators.iter().map(|c| c.0.as_str()));
    let b = syms(&mut w, &mut e.decorators.iter().map(|c| c.1.as_str()));
    w.rel("def_decorator", e.decorators.len(), vec![("qualname", Col::Sym(a)), ("name", Col::Sym(b))]);
    let da: Vec<u8> = e.dunder_all.iter().map(|&b| b as u8).collect();
    w.rel("dunder_all", da.len(), vec![("literal", Col::U8(da))]);
    let a = syms(&mut w, &mut e.dunder_all_names.iter().map(|s| s.as_str()));
    w.rel("dunder_all_name", e.dunder_all_names.len(), vec![("name", Col::Sym(a))]);
    let a = syms(&mut w, &mut e.seeds.iter().map(|c| c.0.as_str()));
    let b = syms(&mut w, &mut e.seeds.iter().map(|c| c.1.as_str()));
    w.rel("seed", e.seeds.len(), vec![("qualname", Col::Sym(a)), ("family", Col::Sym(b))]);
    w.rel("syntax_error", e.has_error as usize, vec![]);

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(hash);
    out.extend_from_slice(&(w.strings.len() as u32).to_le_bytes());
    for s in &w.strings {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    out.extend_from_slice(&(w.rels.len() as u32).to_le_bytes());
    for (name, rows, cols) in &w.rels {
        out.extend_from_slice(&name.to_le_bytes());
        out.extend_from_slice(&rows.to_le_bytes());
        out.extend_from_slice(&(cols.len() as u32).to_le_bytes());
        for (cn, col) in cols {
            out.extend_from_slice(&cn.to_le_bytes());
            out.push(match col {
                Col::U32(_) => 0,
                Col::U8(_) => 2,
                Col::Sym(_) => 3,
                Col::OptSym(_) => 5,
            });
        }
        for (_, col) in cols {
            match col {
                Col::U32(v) | Col::Sym(v) | Col::OptSym(v) => v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes())),
                Col::U8(v) => {
                    out.extend_from_slice(v);
                    while out.len() % 4 != 0 {
                        out.push(0);
                    }
                }
            }
        }
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl Reader<'_> {
    fn u32(&mut self) -> u32 {
        let v = u32::from_le_bytes(self.b[self.at..self.at + 4].try_into().unwrap());
        self.at += 4;
        v
    }
    fn bytes(&mut self, n: usize) -> &[u8] {
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        s
    }
}

pub fn decode(bytes: &[u8]) -> ([u8; 32], Exported) {
    let mut r = Reader { b: bytes, at: 0 };
    assert_eq!(r.bytes(8), MAGIC, "bad magic");
    assert_eq!(r.u32(), SCHEMA_VERSION, "schema version");
    let hash: [u8; 32] = r.bytes(32).try_into().unwrap();
    let ns = r.u32() as usize;
    let mut strings = Vec::with_capacity(ns);
    for _ in 0..ns {
        let l = r.u32() as usize;
        strings.push(String::from_utf8(r.bytes(l).to_vec()).unwrap());
    }
    let nr = r.u32();
    let mut rels: BTreeMap<String, (u32, Vec<(String, u8, Vec<u32>)>)> = BTreeMap::new();
    for _ in 0..nr {
        let name = strings[r.u32() as usize].clone();
        let rows = r.u32();
        let nc = r.u32();
        let heads: Vec<(String, u8)> = (0..nc)
            .map(|_| {
                let n = strings[r.u32() as usize].clone();
                let t = r.bytes(1)[0];
                (n, t)
            })
            .collect();
        let mut cols = Vec::new();
        for (n, t) in heads {
            let vals: Vec<u32> = match t {
                0 | 3 | 4 | 5 => (0..rows).map(|_| r.u32()).collect(),
                1 => {
                    let v = (0..rows).map(|i| u16::from_le_bytes(r.b[r.at + 2 * i as usize..r.at + 2 * i as usize + 2].try_into().unwrap()) as u32).collect();
                    r.at += 2 * rows as usize;
                    r.at = r.at.div_ceil(4) * 4;
                    v
                }
                2 => {
                    let v = r.bytes(rows as usize).iter().map(|&x| x as u32).collect();
                    r.at = r.at.div_ceil(4) * 4;
                    v
                }
                _ => panic!("unknown column type {t}"),
            };
            cols.push((n, t, vals));
        }
        rels.insert(name, (rows, cols));
    }
    let s = |i: u32| strings[i as usize].clone();
    let col = |rel: &str, c: usize| -> Vec<u32> { rels.get(rel).map(|r| r.1[c].2.clone()).unwrap_or_default() };
    let rows = |rel: &str| rels.get(rel).map(|r| r.0).unwrap_or(0) as usize;
    let mut e = Exported::default();
    let (a, b, c, d) = (col("def", 0), col("def", 1), col("def", 2), col("def", 3));
    for i in 0..rows("def") {
        e.defs.push(Def { name: s(a[i]), kind: s(b[i]), qualname: s(c[i]), line: d[i] });
    }
    e.refs = col("ref", 0).into_iter().map(s).collect();
    let (a, b, c, d, f) = (col("import", 0), col("import", 1), col("import", 2), col("import", 3), col("import", 4));
    for i in 0..rows("import") {
        e.imports.push(Import { module: s(a[i]), name: (b[i] != u32::MAX).then(|| s(b[i])), local: s(c[i]), top: d[i] != 0, module_scope: f[i] != 0 });
    }
    let pairs = |rel: &str| -> Vec<(String, String)> { col(rel, 0).into_iter().zip(col(rel, 1)).map(|(x, y)| (s(x), s(y))).collect() };
    e.calls = pairs("call_edge");
    e.decorators = pairs("def_decorator");
    e.dunder_all = col("dunder_all", 0).first().map(|&x| x != 0);
    e.dunder_all_names = col("dunder_all_name", 0).into_iter().map(s).collect();
    e.seeds = pairs("seed");
    e.has_error = rows("syntax_error") > 0;
    (hash, e)
}

pub fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn store_path(store: &std::path::Path, hex: &str) -> std::path::PathBuf {
    store.join(&hex[..2]).join(format!("{hex}.xfacts"))
}
