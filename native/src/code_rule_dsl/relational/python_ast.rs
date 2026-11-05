//! Canonical Python facts: one broad preparation pass, then indexed typed SQL reads.
use super::{Result, invalid, source, syntax};
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
    pub source: bool,
    pub fields: Vec<String>,
    pub expand: Option<String>,
}
#[derive(Clone)]
pub(super) struct DemandSpec {
    pub fields: Vec<String>,
    pub expand: Option<String>,
    pub sources: Option<Vec<String>>,
}
pub(in crate::code_rule_dsl) const FIELDS: &[&str] = &[
    "syntax_error",
    "dump",
    "body_sha256",
    "signature_sha256",
    "identifier",
    "unparse",
    "literal_json",
    "dump_sha256",
    "column",
    "repr_unparse",
    "ast_kind",
    "ast_name",
    "ast_alias",
    "ast_module",
    "ast_level",
    "ast_value",
    "ast_value_kind",
    "ast_preorder",
    "ast_breadth_order",
    "ast_depth",
    "ast_parent",
    "ast_parent_kind",
    "ast_field",
    "ast_index",
    "ast_end",
    "ast_function",
    "ast_class",
    "ast_op",
    "ast_op_count",
    "ast_start",
    "ast_stop",
    "ast_line",
    "ast_context",
    "source_lines",
    "source_line",
    "previous_line",
    "annotation_names",
    "ast_name_repr",
    "ast_value_repr",
    "ast_annotation",
    "ast_annotation_repr",
];
pub(in crate::code_rule_dsl) fn shape(spec: &Spec) -> String {
    let fields = match spec.expand.as_deref() {
        Some("annotations") => vec!["annotation".into(), "name".into()],
        Some("annotation_names") => vec!["name".into()],
        _ => spec.fields.clone(),
    };
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
    if spec.source {
        let mut paths = Vec::new();
        let mut sources = Vec::new();
        let mut bytes = 0usize;
        let mut statement = db.prepare_cached(&spec.roots).map_err(super::sql_error)?;
        let mut rows = statement.query([]).map_err(super::sql_error)?;
        while let Some(row) = rows.next().map_err(super::sql_error)? {
            let path: String = row.get(0).map_err(super::sql_error)?;
            let text: String = row.get(1).map_err(super::sql_error)?;
            bytes = bytes.saturating_add(text.len());
            if bytes > 8 * 1024 * 1024 {
                return Err(invalid("Python source batch capacity"));
            }
            source::charge(&work, 1 + text.len().div_ceil(256))?;
            paths.push(path);
            sources.push(text);
        }
        let roots = (0..paths.len())
            .map(|file| super::producer::Root {
                file: file as u32,
                node: 0,
                expand: 0,
            })
            .collect();
        return Ok(super::producer::Demand {
            roots,
            paths,
            trees: vec![],
            work,
            python_ast: Some(DemandSpec {
                fields: spec.fields.clone(),
                expand: spec.expand.clone(),
                sources: Some(sources),
            }),
        });
    }
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
    demand.python_ast = Some(DemandSpec {
        fields: spec.fields.clone(),
        expand: spec.expand.clone(),
        sources: None,
    });
    Ok(demand)
}
/// Column storage, not a JSON property bag or a vector of complete syntax rows.
pub(in crate::code_rule_dsl) struct Output {
    fields: Vec<String>,
    values: Vec<Vec<String>>,
    files: Vec<u32>,
    nodes: Vec<u32>,
    spans: BTreeMap<String, syntax::CanonicalSpans>,
}
impl Output {
    fn new(fields: Vec<String>) -> Self {
        Self {
            values: fields.iter().map(|_| Vec::new()).collect(),
            fields,
            files: Vec::new(),
            nodes: Vec::new(),
            spans: BTreeMap::new(),
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
        let fields = &demand.python_ast.as_ref().expect("Python demand").fields;
        let mut mapping = Vec::new();
        let prepared = contexts[*index].run(|| -> Result<()> {
            if let Some(sources) = &demand.python_ast.as_ref().unwrap().sources {
                for source in sources {
                    let expansion = demand.python_ast.as_ref().unwrap().expand.as_deref();
                    let requested = match expansion {
                        Some("nodes") => vec!["ast_nodes".into()],
                        Some("annotations") => vec!["ast_annotations".into()],
                        _ => fields.clone(),
                    };
                    let document = documents.len();
                    documents.push(canonical::Document {
                        source: source.clone(),
                        requests: vec![canonical::Request {
                            start: 0,
                            end: source.len(),
                            kind: "module".into(),
                            fields: requested,
                            columns: (expansion == Some("nodes")).then(|| fields.clone()),
                        }],
                    });
                    mapping.push((document, 0));
                }
                return Ok(());
            }
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
                let requested = match demand.python_ast.as_ref().unwrap().expand.as_deref() {
                    Some("nodes") => vec!["ast_nodes".into()],
                    Some("annotations") => vec!["ast_annotations".into()],
                    _ => fields.clone(),
                };
                let mut request = facts.python_ast_request(root.node, requested)?;
                if demand.python_ast.as_ref().unwrap().expand.as_deref() == Some("nodes") {
                    request.columns = Some(fields.clone());
                }
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
                let request = demand.python_ast.clone().expect("Python demand");
                let fields = request.fields;
                let mut output = Output::new(match request.expand.as_deref() {
                    Some("annotations") => vec!["annotation".into(), "name".into()],
                    Some("annotation_names") => vec!["name".into()],
                    _ => fields.clone(),
                });
                let mut bytes = 0usize;
                for (root, (document, row)) in demand.roots.iter().zip(mapping) {
                    let mut row = rows[document][row]
                        .take()
                        .expect("one consumer per requested row");
                    if matches!(request.expand.as_deref(), Some("nodes" | "annotations")) {
                        let encoded = row
                            .remove(if request.expand.as_deref() == Some("nodes") {
                                "ast_nodes"
                            } else {
                                "ast_annotations"
                            })
                            .expect("expanded canonical relation");
                        let mut expanded: BTreeMap<String, Vec<String>> =
                            serde_json::from_str(&encoded).map_err(|e| invalid(e.to_string()))?;
                        let len = expanded.values().next().map_or(0, Vec::len);
                        if expanded.values().any(|column| column.len() != len) {
                            return Err(invalid("canonical column lengths differ"));
                        }
                        if request.expand.as_deref() == Some("nodes") {
                            let starts = expanded
                                .get("ast_start")
                                .ok_or_else(|| invalid("missing canonical starts"))?;
                            let stops = expanded
                                .get("ast_stop")
                                .ok_or_else(|| invalid("missing canonical ends"))?;
                            let spans = starts
                                .iter()
                                .zip(stops)
                                .map(|(start, end)| {
                                    Ok((
                                        start
                                            .parse::<u32>()
                                            .map_err(|_| invalid("invalid canonical start"))?,
                                        end.parse::<u32>()
                                            .map_err(|_| invalid("invalid canonical end"))?,
                                    ))
                                })
                                .collect::<Result<Vec<_>>>()?;
                            if let Some(sources) = &request.sources {
                                use sha2::{Digest, Sha256};
                                let text = &sources[root.file as usize];
                                if spans.iter().any(|&(start, end)| {
                                    start > end || end as usize > text.len()
                                        || !text.is_char_boundary(start as usize)
                                        || !text.is_char_boundary(end as usize)
                                }) {
                                    return Err(invalid("canonical Python span exceeds its source"));
                                }
                                output.spans.insert(
                                    demand.paths[root.file as usize].clone(),
                                    syntax::CanonicalSpans {
                                        digest: Sha256::digest(text.as_bytes()).into(),
                                        spans,
                                    },
                                );
                            } else {
                                demand.trees[root.file as usize].facts.register_python_spans(spans)?;
                            }
                        }
                        source::charge(&demand.work, len)?;
                        output.files.extend(std::iter::repeat_n(root.file, len));
                        output.nodes.extend(std::iter::repeat_n(root.node, len));
                        for (column, field) in output.fields.iter().enumerate() {
                            let values = expanded
                                .remove(field)
                                .ok_or_else(|| invalid("missing canonical column"))?;
                            let before = bytes;
                            for value in &values {
                                bytes = bytes.saturating_add(value.len());
                                if bytes > 64 * 1024 * 1024 {
                                    return Err(invalid("Python AST output capacity"));
                                }
                            }
                            // Column transfer is bulk work. Count row setup separately,
                            // then charge actual text and String headers in 256-byte units.
                            let transferred = (bytes - before).saturating_add(
                                values.len().saturating_mul(std::mem::size_of::<String>()),
                            );
                            source::charge(&demand.work, transferred.div_ceil(256))?;
                            output.values[column].extend(values);
                        }
                        continue;
                    }
                    if request.expand.is_some() {
                        let encoded = row
                            .remove("annotation_names")
                            .expect("validated annotation field");
                        let names: Vec<String> =
                            serde_json::from_str(&encoded).map_err(|e| invalid(e.to_string()))?;
                        for name in names {
                            bytes = bytes.saturating_add(name.len());
                            if bytes > 64 * 1024 * 1024 {
                                return Err(invalid("Python AST output capacity"));
                            }
                            source::charge(&demand.work, 1)?;
                            output.files.push(root.file);
                            output.nodes.push(root.node);
                            output.values[0].push(name);
                        }
                        continue;
                    }
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
    fn canonical_spans(&self) -> Option<&BTreeMap<String, syntax::CanonicalSpans>> {
        Some(&self.output.spans)
    }
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
pub(super) fn into_source(
    output: Output,
    paths: Vec<String>,
    work: Arc<AtomicUsize>,
) -> Result<Arc<dyn syntax::Source>> {
    let mut by_path = HashMap::<String, Vec<usize>>::new();
    let mut by_node = BTreeMap::<u32, Vec<usize>>::new();
    for (row, (&file, &node)) in output.files.iter().zip(&output.nodes).enumerate() {
        by_path
            .entry(paths[file as usize].clone())
            .or_default()
            .push(row);
        by_node.entry(node).or_default().push(row);
    }
    Ok(Arc::new(Published {
        output,
        paths,
        by_path,
        by_node,
        work,
    }))
}
