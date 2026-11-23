//! `string_type_definition(host)` (SCHEMA.md § 1.3): the string literals Ruff 0.16.9 reads as
//! type definitions, with their parsed annotation trees, taken from the native engine's scope
//! table (`scope/python/typedefs.rs`, `ScopeTable::annotations`) by calling its builder, not by
//! re-implementing it. Per content: the table reads one file (default reading, no module path:
//! see SCHEMA § 1.3 for the module-path condition).
use super::facts::{self, NONE};
use ast_grep_core::AstGrep;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::SubDoc;
use crate::code_rule_dsl::origin::Origin;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::code_rule_dsl::tape::TapeDoc;

/// One node of a parsed annotation: its kind, the source range it stands for (the primary host
/// range), its text in the annotation, its parent (index in the tree), and whether it is named.
#[derive(Clone, Debug)]
pub struct Rec {
    pub kind: u32,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub parent: u32,
    pub named: bool,
}

/// One string type definition: the literal's range (in the source, or for a nested one in its
/// outer annotation's range space, as native keeps it) and its tree in preorder (root first).
#[derive(Clone, Debug)]
pub struct Annotation {
    pub start: usize,
    pub end: usize,
    pub nodes: Vec<Rec>,
}

/// The file's string type definitions, sorted by start (native's order).
pub fn build(doc: &TapeDoc) -> Result<Vec<Annotation>, String> {
    let grep = AstGrep::doc(doc.clone());
    let table = ScopeTable::build_in(grep.root(), GrammarDialect::Python, usize::MAX, &|| false, Reading::default(), None).map_err(|e| format!("scope table: {e:?}"))?;
    let mut out = Vec::with_capacity(table.annotations.len());
    for a in &table.annotations {
        let tree = a.tree.clone();
        let sub = AstGrep::doc(SubDoc { tree: tree.clone() });
        let root = sub.root();
        let mut index: rustc_hash::FxHashMap<usize, u32> = Default::default();
        let mut nodes = Vec::new();
        for n in root.dfs() {
            let range = n.range();
            let host = tree.of_node(n.node_id(), range.clone()).map(|r| r.primary()).ok_or("annotation node without a host range")?;
            let parent = n.parent().and_then(|p| index.get(&p.node_id()).copied()).unwrap_or(NONE);
            index.insert(n.node_id(), nodes.len() as u32);
            nodes.push(Rec { kind: facts::intern(&n.kind()), start: host.start as u32, end: host.end as u32, text: n.text().into_owned(), parent, named: n.is_named() });
        }
        out.push(Annotation { start: a.start, end: a.end, nodes });
    }
    Ok(out)
}

/// Native's `annotation_at`: the definition whose literal spans exactly `[start, end)`.
pub fn at(defs: &[Annotation], start: usize, end: usize) -> Option<&Annotation> {
    let i = defs.binary_search_by_key(&start, |a| a.start).ok()?;
    (defs[i].end == end).then_some(&defs[i])
}
