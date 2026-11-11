//! Batched semantic facts over explicit SQL demands. State belongs to one immutable document batch.
use super::{Batch, NONE, Projected};
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::{
    facts::producers::python::{
        import_screen::Imports,
        python_facts::{self, Fact, NodeFacts, Qualification},
    },
    language::GrammarDialect,
    scope::{
        Reading, ScopeTable,
        options::{DEFAULT_PYTHON_MINOR, PythonOptions},
    },
    tape::{Grammar, TapeDoc, TapeImage},
};
use ast_grep_core::{AstGrep, Node, tree_sitter::StrDoc};
use rusqlite::types::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Semantics {
    root: AstGrep<TapeDoc>,
    table: ScopeTable,
    facts: BTreeMap<u32, NodeFacts>,
}
fn at<'a>(root: &Node<'a, TapeDoc>, id: u32) -> Node<'a, TapeDoc> {
    root.root().adopt(root.get_doc().node(id))
}
impl Semantics {
    fn new(p: &Projected) -> Result<Self, String> {
        let grammar = Grammar::new(GrammarDialect::Python)?;
        let parsed = StrDoc::new(&p.facts.source, GrammarDialect::Python);
        let image = TapeImage::encode(parsed.tree.root_node(), &grammar, &p.facts.source, &|| {
            false
        })?;
        let root = AstGrep::doc(TapeDoc::restore_with_lines(
            Arc::clone(&p.facts.source),
            grammar,
            &image,
            Arc::clone(&p.facts.lines),
            &|| false,
        )?);
        if root.root().get_doc().transducer_len() != p.facts.len() {
            return Err("semantic tape does not match core node identities".into());
        }
        let table = ScopeTable::build_in(
            root.root(),
            GrammarDialect::Python,
            10_000_000,
            &|| false,
            Reading::default(),
            p.module.as_ref(),
        )
        .map_err(|e| format!("Python scope: {e:?}"))?;
        Ok(Self {
            root,
            table,
            facts: BTreeMap::new(),
        })
    }
    fn demand(&mut self, p: &Projected, ids: &BTreeSet<u32>) -> Result<usize, String> {
        let missing: Vec<_> = ids
            .iter()
            .filter(|n| !self.facts.contains_key(n))
            .copied()
            .collect();
        let count = missing.len();
        if count == 0 {
            return Ok(0);
        }
        let root = self.root.root();
        for &n in &missing {
            let tape = root.get_doc();
            let span = tape.transducer_span(n);
            if span.start != p.facts.start[n as usize] as usize
                || span.end != p.facts.end[n as usize] as usize
                || tape.transducer_kind(n)
                    != crate::code_rule_dsl::facts::core::kind_code(p.facts.kind[n as usize])
            {
                return Err("semantic tape node identity differs from demanded core node".into());
            }
        }
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let batch = python_facts::project(
            &self.table,
            python_facts::Config {
                module: p.module.as_ref(),
                python: &python,
            },
            missing.into_iter().map(|n| at(&root, n)),
        )
        .map_err(|e| format!("Python projection: {e:?}"))?;
        for fact in batch.by_range.into_values().flatten() {
            self.facts.insert(fact.node_id as u32, fact);
        }
        Ok(count)
    }
}

impl Batch {
    /// Operations return facts, never rule findings. Node IDs include document and world identity.
    pub(crate) fn python_project(
        &self,
        kind: &str,
        requests: &[i64],
        targets: &BTreeSet<String>,
    ) -> Result<Vec<Vec<Value>>, String> {
        if !["qualified", "origins", "assigned", "arguments"].contains(&kind) {
            return Err(format!("unknown Python producer {kind}"));
        }
        let mut by_doc: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for &n in requests {
            if n >= 0 {
                by_doc.entry((n >> 32) as u32).or_default().insert(n as u32);
            }
        }
        let mut rows = Vec::new();
        let (mut requested, mut admitted, mut scopes, mut semantic, mut arguments) =
            (0usize, 0usize, 0usize, 0usize, 0usize);
        for (doc, low) in by_doc {
            let Ok(index) = self.docs.binary_search_by_key(&doc, |p| p.key) else {
                return Err(format!("Python demand references absent document {doc}"));
            };
            let p = &self.docs[index];
            if crate::code_rule_dsl::language::from_tape_tag(p.tag) != Some(GrammarDialect::Python)
            {
                return Err("Python producer received another grammar".into());
            }
            let grammar = &self.bound::<super::py_families::Py>().ok_or("Python facts unavailable")?.grammar;
            let mut ids = BTreeSet::new();
            for n in low {
                let local = p
                    .local_of(n)
                    .ok_or("Python demand references an invalid node")?;
                let core = *p
                    .core
                    .get(local as usize)
                    .ok_or("Python demand references a span instead of a node")?;
                ids.insert(core);
            }
            requested += ids.len();
            let global = |n: u32| -> Result<i64, String> {
                let local = *p
                    .local
                    .get(n as usize)
                    .ok_or("semantic result outside core")?;
                if local == NONE {
                    return Err("semantic result outside projected world".into());
                }
                Ok(p.global(local))
            };
            if kind == "arguments" {
                let input: Vec<_> = ids.iter().copied().collect();
                let mut positions: BTreeMap<u32, Option<i64>> = BTreeMap::new();
                for [call, _, _, keyword, spread, value] in py::arguments(grammar, &p.facts, &input)
                {
                    let pos = positions.entry(call).or_insert(Some(0));
                    let ordinal = if keyword == NONE && spread == 0 {
                        let v = pos.unwrap_or(-1);
                        if let Some(i) = pos {
                            *i += 1;
                        }
                        v
                    } else {
                        -1
                    };
                    if spread == 1 {
                        *pos = None;
                    }
                    let keyword = if keyword == NONE {
                        String::new()
                    } else {
                        crate::code_rule_dsl::identifier::python_name(p.facts.text(keyword).into())
                            .into_owned()
                    };
                    rows.push(vec![
                        Value::Integer(global(call)?),
                        Value::Integer(ordinal),
                        Value::Text(keyword),
                        Value::Text(py::SPREADS[spread as usize].into()),
                        Value::Integer(global(value)?),
                    ]);
                    arguments += 1;
                }
                continue;
            }
            let wanted_imports = targets
                .iter()
                .filter(|s| !s.starts_with("builtins."))
                .cloned()
                .collect();
            let wanted_globals = targets
                .iter()
                .filter_map(|s| s.strip_prefix("builtins.").map(str::to_owned))
                .collect();
            let imports = p.python_imports.get_or_init(|| {
                Imports::from_core(&p.facts, &self.dialects[&p.tag].catalog, p.module.as_ref())
            });
            if kind == "qualified" {
                ids.retain(|&n| {
                    py::qualification_chain(grammar, &p.facts, n).is_none_or(|(head, tail)| {
                        imports.may_qualify_parts(
                            head,
                            tail,
                            &wanted_imports,
                            &wanted_globals,
                            p.module.as_ref(),
                        )
                    })
                });
            }
            admitted += ids.len();
            if ids.is_empty() {
                continue;
            }
            // Origins of calls/literals include the expression itself; only references need a scope lookup.
            let references: BTreeSet<_> = ids
                .iter()
                .copied()
                .filter(|&n| py::qualification_chain(grammar, &p.facts, n).is_some())
                .collect();
            let mut state = p
                .python_semantics
                .lock()
                .map_err(|_| "Python semantic cache poisoned")?;
            if !references.is_empty() {
                if state.is_none() {
                    *state = Some(Semantics::new(p)?);
                    scopes += 1;
                }
                semantic += state.as_mut().unwrap().demand(p, &references)?;
            }
            for n in ids {
                let fact = state.as_ref().and_then(|s| s.facts.get(&n));
                if kind == "qualified" {
                    let q = match fact.map(|f| &f.qualification) {
                        Some(Fact::Known(Qualification::Qualified { name, .. })) => {
                            Some(name.clone())
                        }
                        Some(Fact::Known(Qualification::Unshadowed {
                            name,
                            builtin: true,
                            ..
                        })) if fact.unwrap().attributes.is_empty() => {
                            Some(format!("builtins.{name}"))
                        }
                        _ => None,
                    };
                    if let Some(q) = q.filter(|q| targets.contains(q)) {
                        rows.push(vec![Value::Integer(global(n)?), Value::Text(q)]);
                    }
                } else {
                    let owner = global(n)?;
                    rows.push(vec![
                        Value::Integer(owner),
                        Value::Integer(global(py::unparen(grammar, &p.facts, n))?),
                    ]);
                    let Some(s) = state.as_ref() else {
                        continue;
                    };
                    let binding = fact.filter(|f| f.binding_eligible).and_then(|f| {
                        match if kind == "assigned" {
                            &f.head_bindings.stop_at_global
                        } else {
                            &f.head_bindings.default
                        } {
                            Fact::Known(Some(b)) => Some(b),
                            _ => None,
                        }
                    });
                    for value in python_facts::value_origins(
                        at(&s.root.root(), n),
                        binding,
                        &s.root.root(),
                        kind == "origins",
                    ) {
                        rows.push(vec![
                            Value::Integer(owner),
                            Value::Integer(global(value.node_id() as u32)?),
                        ]);
                    }
                }
            }
        }
        crate::code_rule_dsl::logic::profile::decision("python demand", || {
            format!(
                "{kind}: requested={requested} admitted={admitted} semantic={semantic} scopes={scopes} arguments={arguments} rows={}",
                rows.len()
            )
        });
        crate::code_rule_dsl::prof::count("python.semantic_requests", semantic);
        crate::code_rule_dsl::prof::count("python.argument_rows", arguments);
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Document, Projection};
    use super::*;
use crate::code_rule_dsl::facts::producers::python::families as py;
    use crate::{
        code_rule_dsl::{facts::core, model::FileInput},
        content_store::ContentStore,
    };
    #[test]
    fn selective_batches_cache_only_admitted_nodes_and_preserve_document_identity() {
        let store_path = crate::code_rule_dsl::prefilter::tests::temp();
        let store = ContentStore::open(&store_path).unwrap();
        let sources = [
            "import requests as r\nasync def f():\n r.get('url')\n irrelevant(1,2,3)\n",
            "import requests as r\nasync def f(r):\n r.get('url')\n",
            "async def f():\n irrelevant(1,2,3)\n",
        ];
        let docs = sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let path = format!("d{i}.py");
                let input = FileInput::new(
                    path.clone(),
                    "python".into(),
                    Arc::new(s.to_string()),
                    None,
                    None,
                    "core",
                )
                .unwrap();
                let core::Built::Facts(f) = core::build(&input).unwrap() else {
                    panic!("parse")
                };
                let facts = Arc::new(f);
                Document {
                    key: i as u32,
                    core: format!("/w#core:{path}"),
                    digest: core::digest(&facts),
                    path,
                    facts,
                    guest: None,
                    module: None,
                    tape: None,
                }
            })
            .collect();
        let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
        let grammar = &batch.bound::<super::super::py_families::Py>().unwrap().grammar;
        let mut requests = Vec::new();
        let mut calls = Vec::new();
        for p in &batch.docs {
            for [call, callee] in py::structure(grammar, &p.facts).call {
                requests.push(p.global(p.local[callee as usize]));
                if p.key == 0 && p.facts.text(callee) == "r.get" {
                    calls.push(p.global(p.local[call as usize]));
                }
            }
        }
        let target = BTreeSet::from(["requests.get".into()]);
        let rows = batch
            .python_project("qualified", &requests, &target)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "only the unshadowed first document qualifies"
        );
        assert!(matches!(rows[0][0],Value::Integer(n) if n>>32==0));
        for (i, p) in batch.docs.iter().enumerate() {
            let state = p.python_semantics.lock().unwrap();
            if i == 2 {
                assert!(state.is_none(), "screened document built scopes");
            } else {
                assert_eq!(
                    state.as_ref().unwrap().facts.len(),
                    1,
                    "unrelated call resolved"
                );
            }
        }
        assert_eq!(
            batch
                .python_project("qualified", &requests, &target)
                .unwrap(),
            rows
        );
        assert!(batch.bound::<super::super::py_families::Py>().unwrap().argument_rows.get().is_none());
        let args = batch
            .python_project("arguments", &calls, &BTreeSet::new())
            .unwrap();
        assert_eq!(args.len(), 1, "arguments from undemanded calls leaked");
        assert!(
            batch.bound::<super::super::py_families::Py>().unwrap().argument_rows.get().is_none(),
            "demand filled the global argument relation"
        );
        assert!(
            batch
                .python_project("arguments", &[], &target)
                .unwrap()
                .is_empty()
        );
    }
}
