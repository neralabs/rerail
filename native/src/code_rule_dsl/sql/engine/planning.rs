//! A compiled relation contract contains output columns, never matcher instructions.
use super::*;
use crate::code_rule_dsl::producers::{
    Binding, Column, Contract, Definition, OptionValue, Order, Registry,
};
pub(super) struct Template {
    pub owner: GrammarMatcher,
    pub names: Vec<String>,
    pub counts: BTreeSet<String>,
    pub predicate: Option<super::structural::Predicate>,
    pub native: Option<matcher::AstMatcher>,
    pub prefix: Arc<matcher::factor::Prefix>,
}
impl Template {
    pub fn compile(grammar: &GrammarMatcher, each: Option<&str>) -> Self {
        let mut analysis = matcher::CaptureAnalysis::default();
        let counts = analysis.counts(&grammar.matcher);
        let mut names = analysis.binds(&grammar.matcher);
        names.extend(counts.clone());
        use super::structural::Predicate;
        let (predicate, native) = if each.is_none() {
            if let Some(p) = Predicate::lower(&grammar.matcher) {
                (Some(p), None)
            } else if let matcher::AstMatcher::All(terms) = &grammar.matcher {
                let mut predicates = vec![];
                let mut remainder = vec![];
                for term in terms {
                    if let Some(p) = Predicate::lower(term) {
                        predicates.push(p);
                    } else {
                        remainder.push(term.clone());
                    }
                }
                // Root kinds are already candidate gates. A second copy does not filter any work.
                predicates.retain(|p| !matches!(p, Predicate::Kinds(_)));
                if predicates.is_empty() {
                    (None, Some(grammar.matcher.clone()))
                } else {
                    (
                        Some(Predicate::All(predicates)),
                        Some(matcher::AstMatcher::All(remainder)),
                    )
                }
            } else {
                (None, Some(grammar.matcher.clone()))
            }
        } else {
            (None, Some(grammar.matcher.clone()))
        };
        let predicate = predicate.map(|p| {
            grammar.anchors.root_kinds().map_or(p.clone(), |k| {
                p.simplify(&k.iter().copied().collect::<Vec<_>>())
            })
        });
        let predicate = predicate.filter(|p| native.is_none() || !matches!(p, Predicate::True));
        let prefix = Arc::new(matcher::factor::Prefix::of(
            native.as_ref().unwrap_or(&grammar.matcher),
            each.is_some(),
            grammar
                .anchors
                .root_kinds()
                .map(|k| k.iter().copied().collect::<Vec<_>>())
                .as_deref(),
        ));
        Self {
            owner: grammar.clone(),
            names: names.into_iter().collect(),
            counts,
            predicate,
            native,
            prefix,
        }
    }
}
pub(super) fn manifest(clauses: &[Clause]) -> serde_json::Value {
    let mut registry = Registry::<String>::default();
    registry
        .register(matcher::factor::contract())
        .expect("closed predicate contract");
    for (id, clause) in clauses.iter().enumerate() {
        let name = format!(
            "matcher.{}",
            clause
                .grammar
                .source_digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let columns = [
            ("request", "request-id"),
            ("file", "document"),
            ("node", "node"),
            ("truth", "outcome"),
            ("ordinal", "integer"),
        ]
        .into_iter()
        .map(|(name, kind)| Column {
            name: name.into(),
            kind: kind.into(),
            nullable: false,
        })
        .chain(clause.template.names.iter().map(|name| {
            Column {
                name: format!("capture:{name}"),
                kind: if clause.template.counts.contains(name) {
                    "integer"
                } else {
                    "node-or-list-or-span"
                }
                .into(),
                nullable: true,
            }
        }))
        .collect();
        let contract = registry
            .register(Contract {
                name,
                origin: "DSL compiler".into(),
                definition: Definition {
                    inputs: BTreeMap::from([("demand".into(), "typed-request-relation".into())]),
                    options: BTreeMap::from([("each".into(), "text".into())]),
                    columns,
                    key: vec!["file".into(), "request".into(), "ordinal".into()],
                    completion: "domain".into(),
                    multiplicity: "bag".into(),
                    ordering: vec![
                        Order {
                            column: "file".into(),
                            direction: "asc".into(),
                            nulls: "first".into(),
                        },
                        Order {
                            column: "node".into(),
                            direction: "asc".into(),
                            nulls: "first".into(),
                        },
                        Order {
                            column: "ordinal".into(),
                            direction: "asc".into(),
                            nulls: "first".into(),
                        },
                    ],
                    implementation: BTreeMap::from([(
                        "native".into(),
                        "batched AstMatcher".into(),
                    )]),
                },
            })
            .expect("compiler contract");
        registry
            .bind(
                Binding {
                    contract,
                    inputs: BTreeMap::from([(
                        "demand".into(),
                        format!("clause:{id}:candidate-scope"),
                    )]),
                    options: BTreeMap::from([(
                        "each".into(),
                        OptionValue::Text(clause.each.clone().unwrap_or_default()),
                    )]),
                    domain: Some(format!("clause:{id}:files")),
                },
                format!("clause:{id}"),
            )
            .expect("compiler binding");
    }
    #[cfg(any(test, feature = "code-rule-profile"))]
    {
        registry.manifest("runtime batch", |id| {
            crate::code_rule_dsl::producers::External {
                name: id.clone(),
                generation: "active batch".into(),
            }
        })
    }
    #[cfg(not(any(test, feature = "code-rule-profile")))]
    {
        serde_json::json!({"contracts":clauses.len()})
    }
}
// 0: candidate probes, 1: rare descendant, 2: selective parent relation.
pub(super) fn decisions(clauses: &[Clause], stats: &statistics::PlannerStats) -> Vec<u8> {
    clauses.iter().map(|c| {
        let Some(p)=&c.template.predicate else {return 0};
        let roots=c.grammar.anchors.root_kinds().map(|k|k.iter().copied().collect::<Vec<_>>());
        if let (Some(leaf),Some(roots))=(p.rare_descendant(),roots.as_ref()) {
            if matches!((stats.rows(leaf),stats.rows(roots)),(Some(l),Some(r)) if l<r) {return 1;}
        }
        if let Some(parent)=p.parent_seed() {
            if roots.is_none() || matches!((stats.rows(parent.seed_kinds().unwrap()),roots.as_ref().and_then(|k|stats.rows(k))),(Some(l),Some(r)) if l<r) {return 2;}
        }
        0
    }).collect()
}
pub(super) fn demand(c: &Clause, id: usize, choice: u8) -> String {
    let p = c.template.predicate.as_ref().expect("structural input");
    match choice {
        1 => {
            let kinds = p
                .rare_descendant()
                .unwrap()
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let roots = c
                .grammar
                .anchors
                .root_kinds()
                .unwrap()
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "SELECT DISTINCT c.request,c.file,c.node FROM candidate_files_{id} f CROSS JOIN node s ON s.file=f.file CROSS JOIN node n ON n.file=s.file AND n.id<s.id AND n.last>=s.id AND n.kind IN ({roots}) CROSS JOIN candidates_{id} c ON c.file=n.file AND c.node=n.id WHERE s.kind IN ({kinds})"
            )
        }
        2 => {
            let parent = p.parent_seed().unwrap();
            if matches!(parent, structural::Predicate::Kinds(_)) {
                format!(
                    "SELECT c.request,c.file,c.node FROM candidate_files_{id} f CROSS JOIN node p ON p.file=f.file CROSS JOIN node n ON n.file=p.file AND n.parent=p.id CROSS JOIN candidates_{id} c ON c.file=n.file AND c.node=n.id WHERE {}",
                    parent.sql("p", 0)
                )
            } else {
                // SQLite can turn parent EXISTS predicates into loops after the child join.
                // Finish those predicates once per parent before expanding its children.
                format!(
                    "WITH qualifying_parent AS MATERIALIZED (SELECT p.file,p.id FROM candidate_files_{id} f CROSS JOIN node p ON p.file=f.file WHERE {}) SELECT c.request,c.file,c.node FROM qualifying_parent p CROSS JOIN node n ON n.file=p.file AND n.parent=p.id CROSS JOIN candidates_{id} c ON c.file=n.file AND c.node=n.id",
                    parent.sql("p", 0)
                )
            }
        }
        _ if matches!(p, structural::Predicate::True) => {
            format!("SELECT request,file,node FROM candidates_{id}")
        }
        _ => format!(
            "SELECT c.request,c.file,c.node FROM candidates_{id} c CROSS JOIN node n ON n.file=c.file AND n.id=c.node WHERE {}",
            p.sql("n", 0)
        ),
    }
}
pub(super) fn queries(clauses: &[Clause], stats: &statistics::PlannerStats) -> Vec<String> {
    let choices = decisions(clauses, stats);
    clauses.chunks(ARMS).enumerate().map(|(chunk,clauses)| {
        let width=clauses.iter().map(|c|c.template.names.len()).max().unwrap_or(0);
        let mut arms=vec![];
        if clauses.iter().any(|c|c.template.native.is_some()) {
            let mut inputs=clauses.iter().enumerate().filter(|(_,c)|c.template.native.is_some() && c.template.predicate.is_some()).map(|(i,c)|{
                let id=chunk*ARMS+i;
                format!("SELECT {id} AS clause,(SELECT demand_{id}(request,file,node) FROM ({})) AS handle",demand(c,id,choices[id]))
            }).collect::<Vec<_>>();
            if inputs.is_empty(){inputs.push("SELECT 0 AS clause,0 AS handle WHERE 0".into());}
            let columns=(0..width).map(|i|format!(",c{i}")).collect::<String>();
            arms.push(format!("SELECT clause,file,node,truth,ordinal{columns} FROM matcher_batch_{chunk} WHERE demand=(SELECT batch_demands(clause,handle) FROM ({})) AND (truth=1 OR truth<0)",inputs.join(" UNION ALL ")));
        }
        for (i,c) in clauses.iter().enumerate().filter(|(_,c)|c.template.native.is_none()) {
            let id=chunk*ARMS+i;
            let columns=(0..width).map(|i|format!(",NULL AS c{i}")).collect::<String>();
            arms.push(format!("SELECT {id} AS clause,file,node,1 AS truth,0 AS ordinal{columns} FROM ({})",demand(c,id,choices[id])));
        }
        arms.join(" UNION ALL ")
    }).collect()
}
