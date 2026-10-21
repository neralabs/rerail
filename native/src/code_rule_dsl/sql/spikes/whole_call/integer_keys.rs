//! Integer fact demand IDs separate memo identity from returned capture occurrences.
use super::{
    graph::{Boundary, Column, Graph},
    run,
};
use crate::code_rule_dsl::sql::tests::differential::tape;
use crate::code_rule_dsl::{
    language::GrammarDialect,
    matcher::node_at,
    sql::{
        exec::FileContext,
        plan::SlotKind,
        terminals::{FactTerm, Terminal, Verdict, Verdicts},
    },
    tape::TapeDoc,
};
use ast_grep_core::{Node, meta_var::MetaVarEnv};
use rusqlite::types::Value;

struct ReadCapture;
impl Verdicts for ReadCapture {
    fn rows<'t>(
        &self,
        _: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> Result<Vec<Verdict>, String> {
        Ok(match env.get_match("X") {
            Some(node) if node.text() == "a" => {
                vec![Verdict::holds(vec![Value::Integer(node.node_id() as i64)])]
            }
            Some(_) => vec![Verdict::unknown()],
            None => Vec::new(),
        })
    }
}
#[test]
fn typed_fact_ids_deduplicate_exact_inputs_and_preserve_capture_occurrences() {
    let docs = vec![(
        tape(GrammarDialect::TypeScript, "a; b; a;"),
        FileContext::default(),
    )];
    let term = Terminal::Fact(FactTerm {
        verdicts: Box::new(ReadCapture),
        reads: vec![("X".into(), SlotKind::Node)],
    });
    crate::code_rule_dsl::sql::exec::with_terminals(
        docs,
        vec![term],
        |connection, batch, frames| {
            let file = &batch.files[0];
            let ids: Vec<_> = (0..file.facts.len())
                .filter(|&id| node_at(&file.root, id).kind() == "identifier")
                .collect();
            let values = format!(
                "VALUES(1,0,0,{}),(2,0,0,{}),(3,0,0,{}),(4,0,0,{}),(5,0,0,NULL)",
                ids[0], ids[0], ids[1], ids[2]
            );
            let key = "f.file||','||f.node||','||COALESCE(f.s0,'')";
            let old = format!(
                "WITH inputs(k,file,node,s0) AS ({values}), answers AS MATERIALIZED(SELECT ek,unk,v0 FROM fact WHERE term=0 AND ek IN(SELECT {key} FROM inputs f)) SELECT 0 AS clause,f.file,f.k AS node,json_array(f.s0,t.unk,t.v0) AS captures FROM inputs f JOIN answers t ON t.ek={key} ORDER BY node"
            );
            let before = run::execute(connection, frames, &Graph::default(), &old).unwrap();
            let mut graph = Graph::default();
            let names = ["k", "file", "node", "s0"]
                .iter()
                .map(|s| Column {
                    name: (*s).into(),
                    integer: true,
                })
                .collect();
            let domain_body = format!(
                "WITH inputs(k,file,node,s0) AS ({values}) SELECT ROW_NUMBER() OVER (),file,node,s0 FROM(SELECT DISTINCT file,node,s0 FROM inputs)"
            );
            let domain = graph.add(names, domain_body.clone(), Boundary::Identity);
            graph.handles.push(("integer_fact".into(), domain));
            let query = format!(
                "WITH inputs(k,file,node,s0) AS ({values}), demand(k,file,node,s0) AS MATERIALIZED({domain_body}), published AS MATERIALIZED(SELECT publish_integer_fact(k,file,node,s0) AS handle FROM demand), answers AS MATERIALIZED(SELECT ek,unk,v0 FROM fact WHERE term=0 AND frame=(SELECT handle FROM published)||',s0' AND ek IN(SELECT k FROM demand)) SELECT 0 AS clause,f.file,f.k AS node,json_array(f.s0,t.unk,t.v0) AS captures FROM inputs f JOIN demand d ON d.file=f.file AND d.node=f.node AND d.s0 IS f.s0 JOIN answers t ON t.ek=d.k ORDER BY node"
            );
            let after = run::execute(connection, frames, &graph, &query).unwrap();
            assert_eq!(before.hits, after.hits);
            assert_eq!(after.hits.len(), 4);
            assert_eq!(after.metrics["publishedRows"], 4);
            assert_ne!(
                after.hits[0].3[0], after.hits[3].3[0],
                "equal text cannot replace original capture occurrences"
            );
            eprintln!(
                "whole-call fact integer before {} after {}",
                before.metrics, after.metrics
            );
        },
    );
}
