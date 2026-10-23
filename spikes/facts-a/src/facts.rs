//! Per-content syntactic facts (schema v0), built from rerail's own tape so node ids, kinds,
//! fields and spans equal the native engine's (preorder transducer ids over every tree-sitter
//! node, named and anonymous).
use ast_grep_core::source::SgNode;
use rerail_engine::code_rule_dsl::{eval, lines::LineTable, model::FileInput, tape::TapeDoc};
use parking_lot::Mutex;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

pub const NONE: u32 = u32::MAX;
pub const SCHEMA_VERSION: u32 = 1;

/// Process-wide interner for kind and field names (the contract's `sym`).
pub struct Symbols {
    names: Vec<Arc<str>>,
    ids: rustc_hash::FxHashMap<Arc<str>, u32>,
}
static SYMBOLS: LazyLock<Mutex<Symbols>> = LazyLock::new(|| {
    Mutex::new(Symbols {
        names: Vec::new(),
        ids: Default::default(),
    })
});
pub fn intern(name: &str) -> u32 {
    let mut s = SYMBOLS.lock();
    if let Some(&id) = s.ids.get(name) {
        return id;
    }
    let id = s.names.len() as u32;
    let shared: Arc<str> = Arc::from(name);
    s.names.push(shared.clone());
    s.ids.insert(shared, id);
    id
}
pub fn lookup(name: &str) -> Option<u32> {
    SYMBOLS.lock().ids.get(name).copied()
}
/// A symbol's name: a reference-count bump on a per-thread copy, no allocation and no lock
/// after the first use of a symbol on a thread.
pub fn name(sym: u32) -> Rc<str> {
    NAME_CACHE.with(|c| {
        if let Some(Some(n)) = c.borrow().get(sym as usize) {
            return n.clone();
        }
        let n: Rc<str> = Rc::from(&*SYMBOLS.lock().names[sym as usize]);
        let mut c = c.borrow_mut();
        if c.len() <= sym as usize {
            c.resize(sym as usize + 1, None);
        }
        c[sym as usize] = Some(n.clone());
        n
    })
}
thread_local! {
    static NAME_CACHE: std::cell::RefCell<Vec<Option<Rc<str>>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The patched Python grammar the native engine parses with (field codes and names).
pub static GRAMMAR: LazyLock<Arc<rerail_engine::code_rule_dsl::tape::Grammar>> = LazyLock::new(|| {
    let file = FileInput::new("g.py".into(), "python".into(), Arc::new(String::new()), None, None, "grammar")
        .unwrap_or_else(|e| panic!("{}", e.reason));
    let language = eval::file_language(&file).expect("python grammar");
    rerail_engine::code_rule_dsl::tape::Grammar::new(language).expect("grammar")
});
pub fn field_name(code: u16) -> String {
    GRAMMAR.kinds.field_name(code).unwrap_or("").to_owned()
}

/// One file's facts. Columns are indexed by node id.
pub struct Facts {
    pub source: Arc<String>,
    pub lines: Arc<LineTable>,
    /// node(id, kind, parent, start, end, start_line, end_line) + subtree_end, flags, field.
    pub kind: Vec<u32>,
    pub parent: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub start_line: Vec<u32>,
    pub end_line: Vec<u32>,
    pub subtree_end: Vec<u32>,
    /// named | extra<<1 | missing<<2 | error<<3
    pub flags: Vec<u8>,
    /// Field (grammar field code, 0 = none) under which the node hangs from its parent.
    pub field: Vec<u16>,
    /// Children in order: CSR over parent ids.
    pub child_offsets: Vec<u32>,
    pub children: Vec<u32>,
    /// Kind postings (CSR over kind symbols present in this file, sorted by kind sym).
    pub kind_keys: Vec<u32>,
    pub kind_offsets: Vec<u32>,
    pub kind_nodes: Vec<u32>,
    pub syntax_errors: bool,
    /// The native tape the facts came from (absent when read from a fact file): the input of the
    /// parser facts computed on demand (`typedefs`).
    pub tape: Option<TapeDoc>,
    /// `string_type_definition` (SCHEMA § 1.3), computed on first use.
    pub typedefs: std::cell::OnceCell<Result<Vec<crate::typedefs::Annotation>, String>>,
    /// `py_binding` (SCHEMA § 1.4), computed on first use.
    pub bindings: std::cell::OnceCell<Result<Vec<crate::bindings::Binding>, String>>,
}

pub enum Built {
    Facts(Facts),
    /// The parser stopped at a limit (`file_tape` returned `None`).
    Unparsed,
}

thread_local! {
    static KIND_CACHE: std::cell::RefCell<rustc_hash::FxHashMap<String, u32>> = Default::default();
}

pub fn build(path: &str, source: Arc<String>) -> Result<Built, String> {
    build_timed(path, source, &mut BuildNs::default())
}

/// Nanoseconds of one build: tree-sitter parse plus the engine's tape (`parse`), then the fact
/// columns copied out of the tape (`extract`).
#[derive(Default, Clone, Copy)]
pub struct BuildNs {
    pub parse: u64,
    pub extract: u64,
}

pub fn build_timed(path: &str, source: Arc<String>, ns: &mut BuildNs) -> Result<Built, String> {
    let t0 = std::time::Instant::now();
    let file = FileInput::new(
        path.to_owned(),
        "python".to_owned(),
        Arc::clone(&source),
        None,
        None,
        "facts",
    )
    .map_err(|e| e.reason.to_string())?;
    let language = eval::file_language(&file).ok_or("python has no grammar")?;
    let tape = eval::file_tape(language, &file).map_err(|e| e.reason.to_string());
    ns.parse = t0.elapsed().as_nanos() as u64;
    let Some((doc, lines)) = tape? else {
        return Ok(Built::Unparsed);
    };
    let t1 = std::time::Instant::now();
    let facts = from_tape(&doc, lines, source);
    ns.extract = t1.elapsed().as_nanos() as u64;
    Ok(Built::Facts(facts))
}

fn kind_sym(name: &str) -> u32 {
    KIND_CACHE.with(|cache| {
        if let Some(&s) = cache.borrow().get(name) {
            return s;
        }
        let s = intern(name);
        cache.borrow_mut().insert(name.to_owned(), s);
        s
    })
}

pub fn from_tape(doc: &TapeDoc, lines: Arc<LineTable>, source: Arc<String>) -> Facts {
    let n = doc.transducer_len();
    let mut f = Facts {
        source,
        lines,
        kind: Vec::with_capacity(n),
        parent: Vec::with_capacity(n),
        start: Vec::with_capacity(n),
        end: Vec::with_capacity(n),
        start_line: Vec::with_capacity(n),
        end_line: Vec::with_capacity(n),
        subtree_end: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        field: Vec::with_capacity(n),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        syntax_errors: !doc.syntax_error_ranges().is_empty(),
        tape: Some(doc.clone()),
        typedefs: Default::default(),
        bindings: Default::default(),
    };
    for id in 0..n as u32 {
        let node = doc.node(id);
        f.kind.push(kind_sym(&node.kind()));
        f.parent.push(doc.transducer_parent(id).unwrap_or(NONE));
        let span = doc.transducer_span(id);
        f.start.push(span.start as u32);
        f.end.push(span.end as u32);
        f.start_line.push(f.lines.line(span.start) as u32 + 1);
        f.end_line.push(f.lines.line(span.end) as u32 + 1);
        f.subtree_end.push(doc.transducer_descendants(id).end);
        f.flags.push(doc.transducer_flags(id));
        f.field.push(doc.transducer_field(id));
    }
    index(&mut f);
    f
}

/// The derived columns: the children CSR (preorder lists a parent's children in order) and the
/// kind postings. Rebuilt from `parent` and `kind` when facts are read from a fact file.
pub fn index(f: &mut Facts) {
    let n = f.len();
    let mut counts = vec![0u32; n + 1];
    for &p in &f.parent {
        if p != NONE {
            counts[p as usize + 1] += 1;
        }
    }
    for i in 1..=n {
        counts[i] += counts[i - 1];
    }
    let mut cursor = counts.clone();
    let mut children = vec![0u32; counts[n] as usize];
    for (id, &p) in f.parent.iter().enumerate() {
        if p != NONE {
            children[cursor[p as usize] as usize] = id as u32;
            cursor[p as usize] += 1;
        }
    }
    f.child_offsets = counts;
    f.children = children;
    f.kind_keys.clear();
    f.kind_offsets.clear();
    f.kind_nodes.clear();
    let mut pairs: Vec<(u32, u32)> = f.kind.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect();
    pairs.sort_unstable();
    for (k, id) in pairs {
        if f.kind_keys.last() != Some(&k) {
            f.kind_keys.push(k);
            f.kind_offsets.push(f.kind_nodes.len() as u32);
        }
        f.kind_nodes.push(id);
    }
    f.kind_offsets.push(f.kind_nodes.len() as u32);
}

impl Facts {
    pub fn len(&self) -> usize {
        self.kind.len()
    }
    pub fn children(&self, id: u32) -> &[u32] {
        let a = self.child_offsets[id as usize] as usize;
        let b = self.child_offsets[id as usize + 1] as usize;
        &self.children[a..b]
    }
    pub fn of_kind(&self, kind: u32) -> &[u32] {
        match self.kind_keys.binary_search(&kind) {
            Ok(i) => &self.kind_nodes[self.kind_offsets[i] as usize..self.kind_offsets[i + 1] as usize],
            Err(_) => &[],
        }
    }
    pub fn text(&self, id: u32) -> &str {
        &self.source[self.start[id as usize] as usize..self.end[id as usize] as usize]
    }
    pub fn named(&self, id: u32) -> bool {
        self.flags[id as usize] & 1 != 0
    }
    pub fn parent(&self, id: u32) -> Option<u32> {
        let p = self.parent[id as usize];
        (p != NONE).then_some(p)
    }
    /// `string_type_definition` (SCHEMA § 1.3): computed from the tape on first use.
    pub fn typedefs(&self) -> Result<&[crate::typedefs::Annotation], String> {
        let r = self.typedefs.get_or_init(|| match &self.tape {
            Some(t) => crate::typedefs::build(t),
            None => Err("string_type_definition needs the parse (facts read from a fact file)".into()),
        });
        r.as_deref().map_err(Clone::clone)
    }
    /// `py_binding` (SCHEMA § 1.4): computed from the tape on first use, sorted by node.
    pub fn bindings(&self) -> Result<&[crate::bindings::Binding], String> {
        let r = self.bindings.get_or_init(|| match &self.tape {
            Some(t) => crate::bindings::build(t),
            None => Err("py_binding needs the parse (facts read from a fact file)".into()),
        });
        r.as_deref().map_err(Clone::clone)
    }
    /// Approximate resident bytes of the columns (excluding source).
    pub fn bytes(&self) -> usize {
        let n = self.len();
        n * (4 * 7 + 1 + 2) + self.children.len() * 4 + self.child_offsets.len() * 4 + self.kind_nodes.len() * 4
    }
}
