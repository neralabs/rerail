//! Captured, recursive B615 transfer on an explicitly proved demand subdomain.
//! The original hf-callee matcher remains the oracle. No SQL callback delegates
//! a utility body. Only its original `$API` pattern atom crosses the boundary.
use super::super::capture_cell::Cell;

use crate::code_rule_dsl::{matcher::AstMatcher, tape::TapeDoc};
use ast_grep_core::Node;
#[path = "b615_interval_spike.rs"]
mod intervals;
#[path = "b615_suffix_spike.rs"]
mod suffix;

#[derive(Clone, Debug)]
struct Proof {
    node: u32,
    api: u32,
    incompatible: u32,
}
fn prove(node: Node<'_, TapeDoc>) -> Result<Proof, &'static str> {
    let original = node.node_id() as u32;
    let mut n = node;
    while n.kind() == "parenthesized_expression" {
        let mut expressions = n
            .children()
            .filter(|c| !matches!(c.kind().as_ref(), "(" | ")" | "comment"));
        let child = expressions.next().ok_or("empty parentheses")?;
        if expressions.next().is_some() {
            return Err("ambiguous parentheses children");
        }
        drop(expressions);
        n = child;
    }
    if n.kind() != "attribute" {
        return Err("identifier/import-binding demand outside proved structural branch");
    }
    let api = n.field("attribute").ok_or("attribute field absent")?;
    if api.kind() != "identifier" || api.text() != "from_pretrained" {
        return Err("other API family outside transfer");
    }
    let object = n.field("object").ok_or("object field absent")?;
    if object.kind() != "attribute" {
        return Err("segment may reach resolver or other branch");
    }
    let segment = object.field("attribute").ok_or("segment field absent")?;
    if segment.kind() != "identifier" || segment.text() != "transformers" {
        return Err("segment may reach resolver or other branch");
    }
    Ok(Proof {
        node: original,
        api: api.node_id() as u32,
        incompatible: segment.node_id() as u32,
    })
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Key {
    host: u32,
    scope: u32,
    node: u32,
    kind: u32,
    api: Cell,
}
// Captures are full engine cells, with an explicit slot semantic discriminant.
// The proved `$API` atom accepts only absent/node cells. Other variants are not
// coerced to SQL scalar storage classes or silently compared as node IDs.
fn key(host: u32, scope: u32, p: &Proof, kind: u32, api: Cell) -> Result<Key, &'static str> {
    if !matches!((kind, &api), (0, Cell::Null) | (1, Cell::Integer(_))) {
        return Err(
            "API demand requires absent or single-node capture; other typed cells need a separate proof",
        );
    }
    Ok(Key {
        host,
        scope,
        node: p.node,
        kind,
        api,
    })
}
fn pattern(m: &AstMatcher) -> Option<AstMatcher> {
    fn visit(
        m: &AstMatcher,
        seen: &mut std::collections::HashSet<(usize, usize)>,
    ) -> Option<AstMatcher> {
        match m {
            AstMatcher::Pattern(_) => Some(m.clone()),
            AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().find_map(|m| visit(m, seen)),
            AstMatcher::Util(u) => {
                if !seen.insert((u.table.as_ptr() as usize, u.index)) {
                    return None;
                }
                u.with_target(|m| visit(m, seen)).flatten()
            }
            AstMatcher::Has(r, _)
            | AstMatcher::Inside(r, _)
            | AstMatcher::Follows(r, _)
            | AstMatcher::Precedes(r, _) => visit(r.rule(), seen),
            _ => None,
        }
    }
    visit(m, &mut std::collections::HashSet::new())
}

struct Producer;
impl Producer {
    fn batch(
        db: &rusqlite::Connection,
        term: usize,
        language: crate::code_rule_dsl::language::GrammarDialect,
        demands: &[Key],
    ) -> Self {
        db.execute_batch("CREATE TEMP TABLE b615_demand(host INTEGER,scope INTEGER,node INTEGER,kind INTEGER,api,irrelevant TEXT);CREATE TEMP TABLE b615_summary(key INTEGER PRIMARY KEY,host INTEGER,scope INTEGER,node INTEGER,kind INTEGER,api,truth INTEGER,capture,cost INTEGER,peak INTEGER);").unwrap();
        let rows = demands
            .iter()
            .enumerate()
            .map(|(i, _)| {
                format!(
                    "(?{},?{},?{},?{},?{},'ignored')",
                    i * 5 + 1,
                    i * 5 + 2,
                    i * 5 + 3,
                    i * 5 + 4,
                    i * 5 + 5
                )
            })
            .collect::<Vec<_>>();
        let mut inputs = vec![];
        for d in demands {
            inputs.extend([
                rusqlite::types::Value::Integer(i64::from(d.host)),
                rusqlite::types::Value::Integer(i64::from(d.scope)),
                rusqlite::types::Value::Integer(i64::from(d.node)),
                rusqlite::types::Value::Integer(i64::from(d.kind)),
                match &d.api {
                    Cell::Null => rusqlite::types::Value::Null,
                    Cell::Integer(v) => rusqlite::types::Value::Integer(*v),
                    Cell::List(b) | Cell::Span(b) => rusqlite::types::Value::Blob(b.clone()),
                },
            ]);
        }
        if !demands.is_empty() {
            db.execute(
                &format!("INSERT INTO b615_demand VALUES {}", rows.join(",")),
                rusqlite::params_from_iter(inputs),
            )
            .unwrap();
        } else {
            let call = language.kinds().kind_id("call");
            let function = language.field_id("function").unwrap();
            db.execute(&format!("INSERT INTO b615_demand SELECT n.file,0,c.id,0,NULL,'ignored' FROM node n JOIN node c ON c.file=n.file AND c.parent=n.id AND c.field={function} AND c.ffirst=1 WHERE n.kind={call}"),[]).unwrap();
        }
        let kinds = language.kinds();
        let parenthesized = kinds.kind_id("parenthesized_expression");
        let attribute = kinds.kind_id("attribute");
        let identifier = kinds.kind_id("identifier");
        let comment = kinds.kind_id("comment");
        let field_attribute = language.field_id("attribute").unwrap();
        let field_object = language.field_id("object").unwrap();
        // One SQL producer statement derives all recursive/domain facts from the
        // real node vtable. No Rust-derived wrapper count or API target is input.
        let sql = format!(
            "WITH RECURSIVE keys AS MATERIALIZED(SELECT ROW_NUMBER() OVER(ORDER BY host,scope,node,kind,typeof(api),api) key,* FROM(SELECT DISTINCT host,scope,node,kind,api FROM b615_demand)), unwrap(key,host,scope,root,kind,api,node,wrappers) AS(SELECT key,host,scope,node,kind,api,node,0 FROM keys UNION ALL SELECT u.key,u.host,u.scope,u.root,u.kind,u.api,c.id,u.wrappers+1 FROM unwrap u JOIN node n ON n.file=u.host AND n.id=u.node JOIN node c ON c.file=n.file AND c.parent=n.id WHERE n.kind={parenthesized} AND c.kind<>{comment} AND NOT node_regex({},c.file,c.id) AND (SELECT COUNT(*) FROM node sibling WHERE sibling.file=n.file AND sibling.parent=n.id AND sibling.kind<>{comment} AND NOT node_regex({},sibling.file,sibling.id))=1), domain AS MATERIALIZED(SELECT u.*,a.id target FROM unwrap u JOIN node n ON n.file=u.host AND n.id=u.node AND n.kind={attribute} JOIN node a ON a.file=n.file AND a.parent=n.id AND a.field={field_attribute} AND a.ffirst=1 AND a.kind={identifier} JOIN node object ON object.file=n.file AND object.parent=n.id AND object.field={field_object} AND object.ffirst=1 AND object.kind={attribute} JOIN node segment ON segment.file=object.file AND segment.parent=object.id AND segment.field={field_attribute} AND segment.ffirst=1 AND segment.kind={identifier} WHERE node_regex({},a.file,a.id) AND node_regex({},segment.file,segment.id)) INSERT INTO b615_summary SELECT d.key,d.host,d.scope,d.root,d.kind,d.api,p.truth,p.c0,CASE WHEN p.truth=1 THEN d.wrappers+4 ELSE 4*(d.wrappers+1) END,d.wrappers+CASE WHEN p.truth=1 THEN 3 ELSE 2 END FROM domain d JOIN relation_pattern_typed p ON p.term={term} AND p.file=d.host AND p.node=d.target AND p.i0 IS d.api",
            term + 3,
            term + 3,
            term + 1,
            term + 2
        );
        db.execute(&sql, []).unwrap();
        Self
    }
    fn demand(&self, db: &rusqlite::Connection, key: Key) -> i64 {
        db.query_row("SELECT key FROM b615_summary WHERE host=?1 AND scope=?2 AND node=?3 AND kind=?4 AND api IS ?5",rusqlite::params![key.host,key.scope,key.node,key.kind,key.api],|r|r.get(0)).expect("outside SQL-proved structural demand domain")
    }
    fn key_count(&self, db: &rusqlite::Connection) -> usize {
        usize::try_from(
            db.query_row("SELECT COUNT(*) FROM b615_summary", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        )
        .unwrap()
    }
}
fn terminals(
    atom: std::sync::Arc<ast_grep_core::matcher::Pattern>,
    language: crate::code_rule_dsl::language::GrammarDialect,
) -> Vec<crate::code_rule_dsl::sql::terminals::Terminal> {
    use crate::code_rule_dsl::sql::terminals::{PatternTerm, Terminal};
    let mut pattern = PatternTerm::new(atom, language);
    pattern.reads = (0..pattern.vars.len()).collect();
    let mut terms = vec![Terminal::Pattern(pattern)];
    for regex in ["^from_pretrained$", "^transformers$", "^[()]$"] {
        terms.push(Terminal::Regex(regex::Regex::new(regex).unwrap()));
    }
    terms
}

// The fixed trace follows the actual first-recursive-arm helper and the two
// reached structural helpers. Failed API agreement tries the two later named
// utility arms; all three named arms are two-valued on this proved attribute.
const INVOKE: &str = r#"WITH RECURSIVE
input AS MATERIALIZED(SELECT ?1 logical,?2 host,?3 scope,?4 node,?5 kind,?6 api,?7 level,?8 work,?9 key),
source AS MATERIALIZED(SELECT d.*,s.truth answer,s.capture,s.cost,s.peak,CASE WHEN s.truth=1 THEN s.cost-4 ELSE s.cost/4-1 END wrappers FROM input d JOIN b615_summary s ON s.key=d.key),
lookup AS MATERIALIZED(SELECT s.*,m.truth remembered,m.cost remembered_cost,m.peak remembered_peak,m.o0 remembered_capture FROM source s LEFT JOIN relation_memo m ON m.mode IS 0 AND m.clause IS 8000000 AND m.utility IS s.logical AND m.file IS s.host AND m.node IS s.node AND m.k0 IS s.api AND m.k1 IS s.kind AND m.k2 IS s.scope),
selected AS MATERIALIZED(SELECT s.*,s.remembered IS NOT NULL AND s.level+s.remembered_peak<=256 hit FROM lookup s),
events(ordinal,relative) AS(SELECT 1,1 FROM selected UNION ALL SELECT e.ordinal+1,CASE WHEN e.ordinal+1<=s.wrappers+1 THEN e.ordinal+1 WHEN s.answer=0 THEN s.wrappers+2-((e.ordinal+1-s.wrappers-2)/3) WHEN e.ordinal+1<=s.wrappers+3 THEN s.wrappers+2 ELSE s.wrappers+3 END FROM events e JOIN selected s WHERE e.ordinal<s.cost),
fault AS MATERIALIZED(SELECT MIN(e.ordinal) ordinal FROM events e JOIN selected s WHERE s.level+e.relative>256 OR s.work<e.ordinal),
miss AS MATERIALIZED(SELECT s.*,f.ordinal fault,(SELECT CASE WHEN s.level+e.relative>256 THEN -1 ELSE -2 END FROM events e WHERE e.ordinal=f.ordinal) failure,COALESCE((SELECT MAX(s.level+e.relative) FROM events e WHERE (f.ordinal IS NULL OR e.ordinal<=f.ordinal) AND s.level+e.relative<=256),0) actual_peak FROM selected s CROSS JOIN fault f),
answer AS MATERIALIZED(SELECT s.*,CASE WHEN hit THEN CASE WHEN work>=remembered_cost THEN remembered ELSE -2 END WHEN fault IS NULL THEN answer ELSE failure END truth,CASE WHEN hit THEN MAX(0,work-remembered_cost) WHEN fault IS NULL THEN work-cost WHEN failure=-1 THEN work-(fault-1) ELSE 0 END remaining,CASE WHEN hit THEN level+remembered_peak ELSE actual_peak END reached FROM miss s)
SELECT s.truth,s.remaining,s.reached,CASE WHEN s.truth=1 THEN CASE WHEN s.hit THEN s.remembered_capture ELSE s.capture END ELSE s.api END output FROM answer s LEFT JOIN relation_memo put ON put.mode IS CASE WHEN NOT s.hit AND s.fault IS NULL THEN 1 END AND put.clause IS 8000000 AND put.utility IS s.logical AND put.file IS s.host AND put.node IS s.node AND put.k0 IS s.api AND put.k1 IS s.kind AND put.k2 IS s.scope AND put.vtruth IS s.answer AND put.vcost IS s.cost AND put.vpeak IS s.peak AND put.v0 IS s.capture"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, CompiledCodeRuleDsl, matcher,
        sql::{exec, relational::cutover_tests::execution::fixtures, tests::differential::tape},
        test_support::{CAPABILITIES, grammar_matchers},
    };
    use ast_grep_core::AstGrep;
    use serde_json::{Value, json};
    fn policy(repeat: usize, seed: Option<&str>) -> CompiledCodeRuleDsl {
        let mut p: Value = serde_json::from_str(include_str!(
            "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
        ))
        .unwrap();
        let mut all = vec![];
        if let Some(seed) = seed {
            all.push(json!({"has":{"stopBy":"end","kind":"identifier","regex":format!("^{seed}$"),"pattern":"$API"}}));
        }
        for _ in 0..repeat {
            all.push(json!({"matches":"hf-callee"}));
        }
        p["clauses"][0]["forbid"]["ast"]["rule"] = json!({"all":all});
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_captured_recursive_native_work_depth_and_repeated_calls() {
        let mut evidence = vec![];
        for seed in [None, Some("from_pretrained"), Some("transformers")] {
            for wrappers in [0, 1, 4] {
                for repeat in [1, 3] {
                    let p = policy(repeat, seed);
                    let g = &grammar_matchers(&p)[0];
                    let source = format!(
                        "{}wrapper.transformers.from_pretrained{}\n",
                        "(".repeat(wrappers),
                        ")".repeat(wrappers)
                    );
                    let doc = tape(g.language, &source);
                    let tree = AstGrep::doc(doc.clone());
                    let root = tree
                        .root()
                        .dfs()
                        .find(|n| {
                            n.kind()
                                == if wrappers == 0 {
                                    "attribute"
                                } else {
                                    "parenthesized_expression"
                                }
                        })
                        .unwrap();
                    let proof = prove(root.clone()).unwrap();
                    let atom = pattern(&g.matcher).expect("original API atom");
                    let AstMatcher::Pattern(atom) = atom else {
                        panic!("original API atom")
                    };
                    let term = 0;
                    let terminals = terminals(atom, g.language);
                    let _scope = matcher::FileScopeSlot::install(
                        Default::default(),
                        Default::default(),
                        false,
                    );
                    exec::with_terminals(
                        vec![(doc, Default::default())],
                        terminals,
                        |db, batch, _| {
                            super::super::super::leaves::register(db, batch).unwrap();
                            let keys = [
                                key(0, 0, &proof, 0, Cell::Null).unwrap(),
                                key(0, 0, &proof, 1, Cell::Integer(i64::from(proof.api))).unwrap(),
                                key(
                                    0,
                                    0,
                                    &proof,
                                    1,
                                    Cell::Integer(i64::from(proof.incompatible)),
                                )
                                .unwrap(),
                            ];
                            let producer = Producer::batch(db, term, g.language, &keys);
                            intervals::intervals(db, g.language);
                            let invocation = intervals::query();
                            let initial = match seed {
                                None => Cell::Null,
                                Some("from_pretrained") => Cell::Integer(i64::from(proof.api)),
                                _ => Cell::Integer(i64::from(proof.incompatible)),
                            };
                            let kind = u32::from(!matches!(initial, Cell::Null));
                            for incoming in [0, 250, 253, 254, 255, 256, 0] {
                                for allowance in [0, 1, 2, 3, 4, 5, 8, 16, 64] {
                                    db.query_row::<i64, _, _>(
                                        "SELECT relation_memo_reset(8000000)",
                                        [],
                                        |r| r.get(0),
                                    )
                                    .unwrap();
                                    // Native memo lifetime covers all repeated calls in the original DSL.
                                    let native = matcher::with_file_memo(&tree, || {
                                        matcher::match_at_depth(
                                            g,
                                            root.clone(),
                                            incoming,
                                            allowance,
                                        )
                                    });
                                    let mut api = initial.clone();
                                    let mut remaining = allowance as i64;
                                    let mut reached = 0;
                                    let mut truth = 1;
                                    for _ in 0..repeat {
                                        let typed = key(
                                            0,
                                            0,
                                            &proof,
                                            u32::from(!matches!(api, Cell::Null)),
                                            api.clone(),
                                        )
                                        .unwrap();
                                        let id = producer.demand(db, typed);
                                        let row: (i64, i64, u32, Cell) = db
                                            .query_row(
                                                &invocation,
                                                rusqlite::params![8000000, id, incoming, remaining],
                                                |r| {
                                                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                                                },
                                            )
                                            .unwrap();
                                        truth = row.0;
                                        remaining = row.1;
                                        reached = reached.max(row.2);
                                        api = row.3;
                                        if truth != 1 {
                                            break;
                                        }
                                    }
                                    assert_eq!(
                                        (truth, remaining, reached.saturating_sub(incoming)),
                                        (
                                            native.truth,
                                            native.work as i64,
                                            native.peak.saturating_sub(incoming)
                                        ),
                                        "seed={seed:?} wrappers={wrappers} repeat={repeat} incoming={incoming} work={allowance}"
                                    );
                                    if truth == 1 {
                                        assert_eq!(api.single_node().unwrap(), Some(proof.api));
                                    }
                                    if incoming == 0 {
                                        let mut native_work = allowance;
                                        let matched = matcher::with_file_memo(&tree, || {
                                            g.match_root(root.clone(), &mut native_work)
                                        });
                                        match matched {
                                            Ok(Some(found)) => {
                                                assert_eq!(truth, 1);
                                                assert_eq!(
                                                    api.single_node().unwrap(),
                                                    Some(
                                                        found
                                                            .get_env()
                                                            .get_match("API")
                                                            .unwrap()
                                                            .node_id()
                                                            as u32
                                                    )
                                                );
                                            }
                                            Ok(None) => assert_eq!(truth, 0),
                                            Err(_) => assert!(truth < 0),
                                        }
                                        assert_eq!(native_work as i64, remaining);
                                    }
                                    evidence.push(json!({"seed":seed,"wrappers":wrappers,"repeat":repeat,"incoming":incoming,"allowance":allowance,"truth":truth,"work":remaining,"peak":reached,"initialKind":kind,"producerKeys":producer.key_count(db)}));
                                }
                            }
                        },
                    );
                }
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_real_corpus_syntactic_demand_coverage_and_typed_keys() {
        let p = policy(1, None);
        let g = &grammar_matchers(&p)[0];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let corpus = fixtures(root);
        assert_eq!(corpus.len(), 597);
        let mut accepted = 0;
        let mut refused = 0;
        for fixture in &corpus {
            let tree = AstGrep::doc(tape(g.language, &fixture.source));
            for n in tree.root().dfs().filter(|n| n.kind() == "call") {
                if let Some(callee) = n.field("function") {
                    if prove(callee).is_ok() {
                        accepted += 1
                    } else {
                        refused += 1
                    }
                }
            }
        }
        let tree = AstGrep::doc(tape(g.language, "wrapper.transformers.from_pretrained\n"));
        let proof = prove(tree.root().dfs().find(|n| n.kind() == "attribute").unwrap()).unwrap();
        let a = key(0, 1, &proof, 0, Cell::Null).unwrap();
        assert_ne!(a, key(0, 2, &proof, 0, Cell::Null).unwrap());
        assert_ne!(a, key(1, 1, &proof, 0, Cell::Null).unwrap());
        assert_ne!(
            a,
            key(0, 1, &proof, 1, Cell::Integer(i64::from(proof.api))).unwrap()
        );
        assert!(key(0, 1, &proof, 2, Cell::Integer(i64::from(proof.api))).is_err());
        assert!(
            key(
                0,
                1,
                &proof,
                3,
                Cell::List(super::super::super::capture_cell::encode_list(0, &[]).unwrap())
            )
            .is_err()
        );
        let atom = pattern(&g.matcher).unwrap();
        let AstMatcher::Pattern(atom) = atom else {
            panic!("atom")
        };
        let terms = terminals(atom, g.language);
        let (sql_demands, sql_accepted) = exec::with_terminals(
            corpus
                .iter()
                .map(|f| (tape(g.language, &f.source), exec::FileContext::default()))
                .collect(),
            terms,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let producer = Producer::batch(db, 0, g.language, &[]);
                let demands = usize::try_from(
                    db.query_row("SELECT COUNT(*) FROM b615_demand", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap(),
                )
                .unwrap();
                (demands, producer.key_count(db))
            },
        );
        assert!(refused > 0);
        assert!(
            sql_demands > sql_accepted,
            "narrow SQL transfer must report real outside-domain refusals"
        );
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(format!("{path}.coverage.json"),serde_json::to_vec_pretty(&json!({"passed":true,"files":corpus.len(),"producerStatements":1,"acceptedCalleeDemands":sql_accepted,"refusedCalleeDemands":sql_demands-sql_accepted,"rustExpectedShapeAccepted":accepted,"rustExpectedShapeRefused":refused,"scope":"only parenthesized attribute.from_pretrained with direct attribute.transformers object; not full B615"})).unwrap()).unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_shared_typed_producer_retains_distinct_logical_memos() {
        let p = policy(1, None);
        let g = &grammar_matchers(&p)[0];
        let doc = tape(g.language, "wrapper.transformers.from_pretrained\n");
        let tree = AstGrep::doc(doc.clone());
        let root = tree.root().dfs().find(|n| n.kind() == "attribute").unwrap();
        let proof = prove(root.clone()).unwrap();
        let atom = pattern(&g.matcher).unwrap();
        let AstMatcher::Pattern(atom) = atom else {
            panic!("pattern")
        };
        let term = 0;
        let terminals = terminals(atom, g.language);
        exec::with_terminals(
            vec![(doc, Default::default())],
            terminals,
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let typed = key(0, 0, &proof, 0, Cell::Null).unwrap();
                let producer =
                    Producer::batch(db, term, g.language, &[typed.clone(), typed.clone()]);
                let id = producer.demand(db, typed.clone());
                assert_eq!(producer.demand(db, typed), id);
                assert_eq!(producer.key_count(db), 1);
                for logical in [11, 22] {
                    let row: (i64, i64, u32, Cell) = db
                        .query_row(
                            INVOKE,
                            rusqlite::params![logical, 0, 0, proof.node, 0, Cell::Null, 0, 64, id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )
                        .unwrap();
                    assert_eq!((row.0, row.1, row.2), (1, 60, 3));
                }
                for logical in [11, 22] {
                    let cost:i64=db.query_row("SELECT cost FROM relation_memo WHERE mode IS 0 AND clause IS 8000000 AND utility IS ?1 AND file IS 0 AND node IS ?2 AND k0 IS NULL AND k1 IS 0 AND k2 IS 0",rusqlite::params![logical,proof.node],|r|r.get(0)).unwrap();
                    assert_eq!(cost, 4);
                }
            },
        );
    }
    #[test]
    fn structural_spike_b615_flat_cost_trace_is_not_cached_peak_equivalent() {
        let p = policy(1, None);
        let g = &grammar_matchers(&p)[0];
        let doc = tape(g.language, "((((wrapper.transformers.from_pretrained))))\n");
        let tree = AstGrep::doc(doc.clone());
        let outer = tree
            .root()
            .dfs()
            .find(|n| n.kind() == "parenthesized_expression")
            .unwrap();
        let inner = tree.root().dfs().find(|n| n.kind() == "attribute").unwrap();
        let proofs = [prove(inner.clone()).unwrap(), prove(outer.clone()).unwrap()];
        let _scope = matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
        let native = matcher::with_file_memo(&tree, || {
            let warm = matcher::match_at_depth(g, inner, 0, 64);
            assert_eq!(warm.truth, 1);
            matcher::match_at_depth(g, outer, 0, 4)
        });
        let atom = pattern(&g.matcher).unwrap();
        let AstMatcher::Pattern(atom) = atom else {
            panic!("pattern")
        };
        let demands = proofs
            .iter()
            .map(|p| key(0, 0, p, 0, Cell::Null).unwrap())
            .collect::<Vec<_>>();
        exec::with_terminals(
            vec![(doc, Default::default())],
            terminals(atom, g.language),
            |db, batch, _| {
                super::super::super::leaves::register(db, batch).unwrap();
                let producer = Producer::batch(db, 0, g.language, &demands);
                let mut last = (0, 0, 0);
                for (i, work) in [(0, 64), (1, 4)] {
                    let id = producer.demand(db, demands[i].clone());
                    last = db
                        .query_row(
                            INVOKE,
                            rusqlite::params![1, 0, 0, proofs[i].node, 0, Cell::Null, 0, work, id],
                            |r| {
                                Ok((
                                    r.get::<_, i64>(0)?,
                                    r.get::<_, i64>(1)?,
                                    r.get::<_, u32>(2)?,
                                ))
                            },
                        )
                        .unwrap();
                }
                assert_eq!((last.0, last.1), (native.truth, native.work as i64));
                assert_eq!((last.2, native.peak), (5, 7));
            },
        );
    }
}
