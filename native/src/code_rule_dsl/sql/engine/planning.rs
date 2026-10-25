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
        Self {
            owner: grammar.clone(),
            names: names.into_iter().collect(),
            counts,
            predicate,
            native,
        }
    }
}
pub(super) fn manifest(clauses: &[Clause]) -> serde_json::Value {
    let mut registry = Registry::<String>::default();
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
pub(super) fn decisions(clauses: &[Clause], stats: &statistics::PlannerStats) -> Vec<bool> {
    clauses.iter().map(|c|{
        let Some(leaf)=c.template.predicate.as_ref().and_then(|p|p.rare_descendant())else{return false};
        let Some(roots)=c.grammar.anchors.root_kinds()else{return false};
        matches!((stats.rows(leaf),stats.rows(&roots.iter().copied().collect::<Vec<_>>())),(Some(l),Some(r)) if l<r)
    }).collect()
}
pub(super) fn queries(clauses: &[Clause], stats: &statistics::PlannerStats) -> Vec<String> {
    let choices = decisions(clauses, stats);
    clauses.chunks(ARMS).enumerate().map(|(chunk,clauses)|{
        let width=clauses.iter().map(|c|c.template.names.len()).max().unwrap_or(0);
        clauses.iter().enumerate().map(|(index,c)|{
            let id=chunk*ARMS+index;
            let columns=(0..width).map(|i|if i<c.template.names.len(){format!(",c{i}")}else{format!(",NULL AS c{i}")}).collect::<String>();
            if let Some(predicate)=&c.template.predicate{
                let demand=if choices[id]{
                    let kinds=predicate.rare_descendant().unwrap().iter().map(u16::to_string).collect::<Vec<_>>().join(",");
                    let roots=c.grammar.anchors.root_kinds().unwrap().iter().map(u16::to_string).collect::<Vec<_>>().join(",");
                    format!("WITH candidates AS MATERIALIZED (SELECT * FROM candidates_{id}) SELECT DISTINCT c.request,c.file,c.node FROM node s CROSS JOIN node n ON n.file=s.file AND n.id<s.id AND n.last>=s.id AND n.kind IN ({roots}) JOIN candidates c ON c.file=n.file AND c.node=n.id WHERE s.kind IN ({kinds}) AND s.file IN (SELECT file FROM candidates)")
                }else{
                    format!("SELECT c.request,c.file,c.node FROM candidates_{id} c CROSS JOIN node n ON n.file=c.file AND n.id=c.node WHERE {}",predicate.sql("n",0))
                };
                if c.template.native.is_none(){format!("SELECT {id} AS clause,file,node,1 AS truth,0 AS ordinal{columns} FROM ({demand})")}
                else{format!("SELECT {id} AS clause,file,node,truth,ordinal{columns} FROM matcher_{id} WHERE demand=(SELECT demand_{id}(request,file,node) FROM ({demand})) AND (truth=1 OR truth<0)")}
            }else{format!("SELECT {id} AS clause,file,node,truth,ordinal{columns} FROM matcher_{id} WHERE truth=1 OR truth<0")}
        }).collect::<Vec<_>>().join(" UNION ALL ")
    }).collect()
}
