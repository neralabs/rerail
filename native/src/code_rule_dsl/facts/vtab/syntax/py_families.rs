//! The `py.*` families (`facts::producers::python::families`) as a batch's lazy families (`syntax::Families`):
//! made on the first read of any of them, so a program that reads none does no work.

use super::*;
use crate::code_rule_dsl::facts::producers::python::families as py;

/// The `py.*` families' tables and columns (all integers; `-1` is none), as `PY_COLUMNS[i]` is `PY[i]`'s.
pub(crate) const PY: [&str; 5] = ["py_call", "py_call_site", "py_exec_context", "py_call_arg", "py_attribute"];
pub(crate) const PY_COLUMNS: [&[&str]; 5] = [
    &["d", "call", "callee"],
    &["d", "report", "call", "ref"],
    &["d", "call", "owner", "kind"],
    &["d", "call", "arg", "ordinal", "keyword", "spread", "value"],
    &["d", "node", "object", "name"],
];

/// The atoms the families' rows hold: the call contexts' names, then the argument spreads'.
static STRINGS: std::sync::LazyLock<Vec<&'static str>> =
    std::sync::LazyLock::new(|| py::CONTEXTS.iter().chain(py::SPREADS.iter()).copied().collect());

/// The `py.*` families of a batch's Python documents.
pub struct Families;

impl super::Families for Families {
    fn dialect(&self) -> crate::code_rule_dsl::language::GrammarDialect {
        crate::code_rule_dsl::language::GrammarDialect::Python
    }
    fn names(&self) -> &'static [&'static str] {
        &PY
    }
    fn columns(&self) -> &'static [&'static [&'static str]] {
        &PY_COLUMNS
    }
    fn strings(&self) -> &'static [&'static str] {
        &STRINGS
    }
    fn derived(&self) -> &'static str {
        "#py:"
    }
    fn producer(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"rerail.py/v1\0");
        hash.update(py::VERSION.to_le_bytes());
        hash.finalize().into()
    }
    fn rows_digest(&self, facts: &Facts) -> [u8; 32] {
        use crate::code_rule_dsl::language::GrammarDialect;
        match crate::code_rule_dsl::language::tape_tag(GrammarDialect::Python) {
            Some(tag) => py::digest(&py::rows(&py::Grammar::new(tag, &GrammarDialect::Python.kinds()), facts)),
            None => crate::content_store::ABSENT,
        }
    }
    fn bind(&self, tag: u8, catalog: &KindCatalog, atoms: &[i64], documents: usize) -> Box<dyn BoundFamilies> {
        let (contexts, spreads) = atoms.split_at(py::CONTEXTS.len());
        Box::new(Py {
            grammar: py::Grammar::new(tag, catalog),
            contexts: contexts.try_into().expect("a context's atom each"),
            spreads: spreads.try_into().expect("a spread's atom each"),
            rows: Default::default(),
            argument_rows: Default::default(),
            imports: (0..documents).map(|_| Default::default()).collect(),
            semantics: (0..documents).map(|_| Default::default()).collect(),
        })
    }
}

/// The batch's `py.*` families: the producer's grammar, the atoms of its names, and the rows, made on the first read.
pub(crate) struct Py {
    pub grammar: py::Grammar,
    pub(crate) contexts: [i64; 5],
    pub(crate) spreads: [i64; 3],
    rows: std::sync::OnceLock<Vec<Relation>>,
    pub argument_rows: std::sync::OnceLock<Relation>,
    /// Per batch document (in the batch's order), its imports' screen, made on its first read.
    imports: Vec<std::sync::OnceLock<crate::code_rule_dsl::facts::producers::python::import_screen::Imports>>,
    /// Per batch document, its scopes for semantic demands (`python_demand`), made on the first that needs them.
    pub semantics: Vec<std::sync::Mutex<Option<super::python_demand::Semantics>>>,
}

impl Py {
    /// `p`'s place in `batch` (documents are in key order).
    pub(crate) fn at(batch: &Batch, p: &Projected) -> usize {
        batch.docs.binary_search_by_key(&p.key, |q| q.key).expect("a document of the batch")
    }

    /// Document `p`'s imports' screen, made on its first read.
    pub(crate) fn imports(&self, batch: &Batch, p: &Projected) -> &crate::code_rule_dsl::facts::producers::python::import_screen::Imports {
        self.imports[Self::at(batch, p)].get_or_init(|| {
            crate::code_rule_dsl::facts::producers::python::import_screen::Imports::from_core(&p.facts, &batch.dialects[&p.tag].catalog, p.module.as_ref())
        })
    }
}

impl BoundFamilies for Py {
    fn rows<'b>(&'b self, batch: &'b Batch, family: usize) -> &'b Relation {
        if family == 3 {
            self.arguments(batch)
        } else {
            &self.all(batch)[family]
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Py {
    /// Every family's rows, made on the first call.
    fn all(&self, batch: &Batch) -> &[Relation] {
        let py = self;
        py.rows.get_or_init(|| {
            let started = std::time::Instant::now();
            let mut out: Vec<Relation> = PY_COLUMNS.iter().map(|columns| Relation::new(columns)).collect();
            let mut skipped = 0;
            for p in batch.docs.iter().filter(|p| p.tag == py.grammar.tag()) {
                let rows = py::structure(&py.grammar, &p.facts);
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

    /// `py_call_arg`'s rows alone (the arguments' structure, without the calls').
    fn arguments(&self, batch: &Batch) -> &Relation {
        let py = self;
        py.argument_rows.get_or_init(|| {
            let mut out=Relation::new(PY_COLUMNS[3]);
            for p in batch.docs.iter().filter(|p|p.tag==py.grammar.tag()) {
                let rows=py::rows(&py.grammar,&p.facts);
                let id=|n:u32| if n==NONE {Some(-1)}else{p.local.get(n as usize).copied().filter(|&l|l!=NONE).map(|l|p.global(l))};
                for [c,a,i,k,s,v] in rows.arg {
                    if let (Some(c),Some(a),Some(k),Some(v))=(id(c),id(a),id(k),id(v)) {out.push(&[p.key as i64,c,a,i as i64,k,py.spreads[s as usize],v]);}
                }
            }
            out
        })
    }
}
