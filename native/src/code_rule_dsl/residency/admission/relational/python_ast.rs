//! Canonical Python facts: one broad preparation pass, then indexed typed SQL reads.
use super::{Result, invalid, source, sql_error, syntax, table};
use crate::code_rule_dsl::{budget, value::python_ast as canonical};
use rusqlite::{Connection, types::Value, vtab::Context};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, atomic::AtomicUsize},
};

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Spec {
    pub roots: String,
    pub syntax: String,
    pub fields: Vec<String>,
}
pub(in crate::code_rule_dsl) const FIELDS: &[&str] = &[
    "dump",
    "body_sha256",
    "signature_sha256",
    "identifier",
    "unparse",
    "literal_json",
    "dump_sha256",
    "column",
    "repr_unparse",
];
pub(super) fn shape(fields: &[String]) -> String {
    let mut columns = vec![
        "CAST(NULL AS TEXT) AS __path".to_owned(),
        "CAST(NULL AS INTEGER) AS id".to_owned(),
    ];
    columns.extend(fields.iter().map(|f| format!("CAST(NULL AS TEXT) AS {f}")));
    format!("SELECT {} WHERE 0", columns.join(","))
}
pub(super) fn demand(
    db: &Connection,
    spec: &Spec,
    trees: &[super::producer::Tree],
    files: &syntax::Files,
    work: Arc<AtomicUsize>,
) -> Result<super::producer::Demand> {
    let request = super::Reachability {
        ordered_first: None,
        roots: spec.roots.clone(),
        edges: String::new(),
        batch_size: 256,
        forest: false,
        syntax: Some(spec.syntax.clone()),
        expand: false,
    };
    let mut demand = syntax::demand(db, &request, trees, files, work)?;
    demand.python_ast = Some(spec.fields.clone());
    Ok(demand)
}
/// Column storage, not a JSON property bag or a vector of complete syntax rows.
pub(in crate::code_rule_dsl) struct Output {
    fields: Vec<String>,
    values: Vec<Vec<String>>,
    files: Vec<u32>,
    nodes: Vec<u32>,
}
impl Output {
    fn new(fields: Vec<String>) -> Self {
        Self {
            values: fields.iter().map(|_| Vec::new()).collect(),
            fields,
            files: Vec::new(),
            nodes: Vec::new(),
        }
    }
}
pub(super) fn execute(
    pending: Vec<(usize, super::producer::Demand)>,
    contexts: &mut [budget::Context],
    allowance: &mut super::producer::Allowance,
) -> Vec<Result<super::producer::Completed>> {
    #[cfg(feature = "code-rule-profile")]
    let mut trace =
        crate::code_rule_dsl::profile::trace::Span::start("producer-wave", "python-ast");
    let fail = |message: &str, n: usize| (0..n).map(|_| Err(invalid(message))).collect();
    if let Err(e) = allowance.reserve(1) {
        return fail(&e.reason, pending.len());
    }
    let mut documents: Vec<canonical::Document> = Vec::new();
    let mut by_document = HashMap::new();
    let mut mappings = Vec::new();
    for (index, demand) in &pending {
        let fields = demand.python_ast.as_ref().expect("Python demand");
        let mut mapping = Vec::new();
        let prepared = contexts[*index].run(|| -> Result<()> {
            for root in &demand.roots {
                source::charge(&demand.work, 1)?;
                let facts = &demand.trees[root.file as usize].facts;
                let key = Arc::as_ptr(facts) as usize;
                let document = *by_document.entry(key).or_insert_with(|| {
                    let i = documents.len();
                    documents.push(canonical::Document {
                        source: facts.python_ast_source(),
                        requests: Vec::new(),
                    });
                    i
                });
                let request = facts.python_ast_request(root.node, fields.clone())?;
                let row = documents[document].requests.len();
                documents[document].requests.push(request);
                mapping.push((document, row));
            }
            Ok(())
        });
        if let Err(e) = prepared {
            return fail(&e.reason, pending.len());
        }
        mappings.push(mapping);
    }
    let mut rows = match canonical::batch_with_metrics(&documents) {
        Ok(response) => {
            allowance.python_ast_parses += response.parses;
            allowance.python_ast_module_walks += response.module_walks;
            response
                .documents
                .into_iter()
                .map(|r| r.into_iter().map(Some).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        }
        Err(e) => return fail(&e.0, pending.len()),
    };
    #[cfg(feature="code-rule-profile")]
    trace.detail(||serde_json::json!({"relations":pending.len(),"roots":mappings.iter().map(Vec::len).sum::<usize>(),"documents":documents.len(),"sourceBytes":documents.iter().map(|d|d.source.len()).sum::<usize>(),"stages":1,"backend":"cpython-3.13"}));
    pending
        .into_iter()
        .zip(mappings)
        .map(|((index, demand), mapping)| {
            contexts[index].run(|| {
                let fields = demand.python_ast.clone().expect("Python demand");
                let mut output = Output::new(fields.clone());
                let mut bytes = 0usize;
                for (root, (document, row)) in demand.roots.iter().zip(mapping) {
                    let mut row = rows[document][row]
                        .take()
                        .expect("one consumer per requested row");
                    output.files.push(root.file);
                    output.nodes.push(root.node);
                    for (column, field) in fields.iter().enumerate() {
                        let value = row.remove(field).expect("validated canonical field");
                        bytes = bytes.saturating_add(value.len());
                        if bytes > 64 * 1024 * 1024 {
                            return Err(invalid("Python AST output capacity"));
                        }
                        source::charge(&demand.work, value.len().div_ceil(256))?;
                        output.values[column].push(value);
                    }
                }
                Ok(super::producer::Completed::python(output, demand.paths))
            })
        })
        .collect()
}
struct Published {
    output: Output,
    paths: Vec<String>,
    by_path: HashMap<String, Vec<usize>>,
    by_node: BTreeMap<u32, Vec<usize>>,
    work: Arc<AtomicUsize>,
}
impl syntax::Source for Published {
    fn columns(&self) -> Vec<String> {
        let mut c = vec!["__path".into(), "id".into()];
        c.extend(self.output.fields.clone());
        c
    }
    fn column_types(&self) -> Vec<String> {
        (0..self.output.fields.len() + 2)
            .map(|i| if i == 1 { "INTEGER" } else { "TEXT" }.into())
            .collect()
    }
    fn len(&self) -> usize {
        self.output.nodes.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let distinct = if column == 0 {
            self.by_path.len()
        } else {
            self.by_node.len()
        }
        .max(1);
        let n = self.len().div_ceil(distinct);
        if batch {
            n.saturating_mul(256).min(self.len())
        } else {
            n.max(1)
        }
    }
    fn indexed(&self, column: usize) -> bool {
        column < 2
    }
    fn lookup(&self, column: usize, values: &[Value]) -> rusqlite::Result<Option<Vec<usize>>> {
        if column > 1 {
            return Ok(None);
        }
        let mut rows = Vec::new();
        for value in values {
            self.charge(1)?;
            match (column, value) {
                (0, Value::Text(path)) => {
                    if let Some(postings) = self.by_path.get(path) {
                        rows.extend_from_slice(postings);
                    }
                }
                (1, Value::Integer(id)) => {
                    if let Ok(id) = u32::try_from(*id)
                        && let Some(postings) = self.by_node.get(&id)
                    {
                        rows.extend_from_slice(postings);
                    }
                }
                _ => {}
            }
        }
        rows.sort_unstable();
        rows.dedup();
        self.charge(rows.len())?;
        Ok(Some(rows))
    }
    fn column(&self, row: usize, column: usize, ctx: &mut Context) -> rusqlite::Result<()> {
        self.charge(1)?;
        match column {
            0 => ctx.set_result(&self.paths[self.output.files[row] as usize]),
            1 => ctx.set_result(&i64::from(self.output.nodes[row])),
            c => ctx.set_result(&self.output.values[c - 2][row]),
        }
    }
    fn charge(&self, n: usize) -> rusqlite::Result<()> {
        source::charge(&self.work, n.div_ceil(64))
            .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))
    }
}
pub(super) fn publish(
    output: Output,
    paths: Vec<String>,
    db: &Connection,
    name: &str,
    work: Arc<AtomicUsize>,
) -> Result<()> {
    let mut by_path = HashMap::<String, Vec<usize>>::new();
    let mut by_node = BTreeMap::<u32, Vec<usize>>::new();
    for (row, (&file, &node)) in output.files.iter().zip(&output.nodes).enumerate() {
        by_path
            .entry(paths[file as usize].clone())
            .or_default()
            .push(row);
        by_node.entry(node).or_default().push(row);
    }
    table::register(
        db,
        name,
        Arc::new(Published {
            output,
            paths,
            by_path,
            by_node,
            work,
        }),
    )
    .map(|_| ())
    .map_err(sql_error)
}
