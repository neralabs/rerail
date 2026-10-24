//! Shared utility-node/edge control for the admitted structural B615 domain.
//! Each (AST node, typed semantic context) is produced once. Roots share suffix
//! summaries and utility children; no complete per-root execution trace exists.
use super::*;
use crate::code_rule_dsl::{language::GrammarDialect, sql::relational::outcomes::emit::bridge};
#[derive(Clone, Copy, Debug)]
enum EdgePlan {
    Baseline,
    Unique,
    Ordered,
    Analyzed,
    ParentLinks,
}
impl EdgePlan {
    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Unique => "unique",
            Self::Ordered => "unique-ordered",
            Self::Analyzed => "unique-analyzed",
            Self::ParentLinks => "indexed-parent-links",
        }
    }
    fn unique(self) -> bool {
        matches!(self, Self::Unique | Self::Ordered | Self::Analyzed)
    }
}
struct Suffix {
    edge_select: String,
    hf_select: String,
}
impl Suffix {
    fn batch(db: &rusqlite::Connection, language: GrammarDialect, demands: &[Key]) -> Self {
        Self::batch_mode(db, language, demands, false)
    }
    fn batch_mode(
        db: &rusqlite::Connection,
        language: GrammarDialect,
        demands: &[Key],
        three: bool,
    ) -> Self {
        Self::batch_plan(db, language, demands, three, EdgePlan::ParentLinks)
    }
    fn batch_plan(
        db: &rusqlite::Connection,
        language: GrammarDialect,
        demands: &[Key],
        three: bool,
        plan: EdgePlan,
    ) -> Self {
        let native_mode = usize::from(three);
        db.execute_batch("CREATE TEMP TABLE suffix_demand(host INTEGER,scope INTEGER,node INTEGER,kind INTEGER,api);").unwrap();
        let rows = demands
            .iter()
            .enumerate()
            .map(|(i, _)| {
                format!(
                    "(?{},?{},?{},?{},?{})",
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
                &format!("INSERT INTO suffix_demand VALUES {}", rows.join(",")),
                rusqlite::params_from_iter(inputs),
            )
            .unwrap();
        }
        let kinds = language.kinds();
        let parenthesized = kinds.kind_id("parenthesized_expression");
        let attribute = kinds.kind_id("attribute");
        let identifier = kinds.kind_id("identifier");
        let comment = kinds.kind_id("comment");
        let attr = language.field_id("attribute").unwrap();
        let object = language.field_id("object").unwrap();
        let parent_links = if matches!(plan, EdgePlan::ParentLinks) {
            format!(
                r#"CREATE TEMP TABLE suffix_parent_links AS SELECT expression.host,expression.scope,expression.parent,expression.kind,expression.api,expression.node child,COALESCE(SUM(CASE WHEN punctuation.ordinal<expression.ordinal THEN 4 ELSE 0 END),0) leading_cost,COALESCE(SUM(CASE WHEN punctuation.ordinal>expression.ordinal THEN 4 ELSE 0 END),0) trailing_cost FROM suffix_children expression LEFT JOIN suffix_children punctuation ON {native_mode}=1 AND punctuation.host=expression.host AND punctuation.scope=expression.scope AND punctuation.parent=expression.parent AND punctuation.kind=expression.kind AND punctuation.api IS expression.api AND NOT punctuation.comment AND punctuation.punctuation WHERE NOT expression.comment AND NOT expression.punctuation GROUP BY expression.host,expression.scope,expression.parent,expression.kind,expression.api,expression.node;CREATE UNIQUE INDEX suffix_parent_link_child ON suffix_parent_links(host,scope,child,kind,COALESCE(api,0));"#
            )
        } else {
            String::new()
        };
        let recursive_answer = if matches!(plan, EdgePlan::ParentLinks) {
            "SELECT a.host,a.scope,link.parent,a.kind,a.api,a.truth,a.capture,a.cost+1+link.leading_cost+CASE WHEN a.truth=0 THEN link.trailing_cost+3 ELSE 0 END,a.peak+1 FROM answer a JOIN suffix_parent_links link ON link.host=a.host AND link.scope=a.scope AND link.child=a.node AND link.kind=a.kind AND COALESCE(link.api,0)=COALESCE(a.api,0)".to_owned()
        } else {
            format!(
                r#"SELECT a.host,a.scope,expression.parent,a.kind,a.api,a.truth,a.capture,a.cost+1+CASE WHEN a.truth=0 THEN 3 ELSE 0 END+CASE WHEN {native_mode}=1 THEN(SELECT COALESCE(SUM(4),0) FROM suffix_children punctuation WHERE punctuation.host=a.host AND punctuation.scope=a.scope AND punctuation.parent=expression.parent AND punctuation.kind=a.kind AND punctuation.api IS a.api AND NOT punctuation.comment AND punctuation.punctuation AND(a.truth=0 OR punctuation.ordinal<expression.ordinal)) ELSE 0 END,a.peak+1 FROM answer a JOIN suffix_children expression ON expression.host=a.host AND expression.scope=a.scope AND expression.node=a.node AND expression.kind=a.kind AND expression.api IS a.api AND NOT expression.comment AND NOT expression.punctuation"#
            )
        };
        let hf_select = format!(
            r#"WITH RECURSIVE answer(host,scope,node,kind,api,truth,capture,cost,peak) AS(SELECT host,scope,node,kind,api,truth,CASE WHEN truth=1 THEN capture ELSE api END,4,CASE WHEN truth=1 THEN 3 ELSE 2 END FROM suffix_base UNION ALL SELECT r.host,r.scope,r.node,r.kind,r.api,0,r.api,4,2 FROM suffix_reach r WHERE {native_mode}=1 AND node_regex(3,r.host,r.node) UNION ALL {recursive_answer}) SELECT * FROM answer"#
        );
        db.execute_batch(&format!(r#"CREATE TEMP TABLE suffix_reach AS WITH RECURSIVE reach(host,scope,node,kind,api) AS(SELECT DISTINCT host,scope,node,kind,api FROM suffix_demand UNION SELECT r.host,r.scope,c.id,r.kind,r.api FROM reach r JOIN node p ON p.file=r.host AND p.id=r.node JOIN node c ON c.file=p.file AND c.parent=p.id WHERE p.kind={parenthesized} AND c.kind<>{comment} AND ({native_mode}=1 OR NOT node_regex(3,c.file,c.id)) AND(SELECT COUNT(*) FROM node sibling WHERE sibling.file=p.file AND sibling.parent=p.id AND sibling.kind<>{comment} AND NOT node_regex(3,sibling.file,sibling.id))=1) SELECT * FROM reach;
CREATE INDEX suffix_reach_key ON suffix_reach(host,scope,node,kind,api);
CREATE TEMP TABLE suffix_children AS SELECT r.host,r.scope,r.node parent,r.kind,r.api,c.id node,ROW_NUMBER() OVER(PARTITION BY r.host,r.scope,r.node,r.kind,r.api ORDER BY c.id)-1 ordinal,c.kind={comment} comment,node_regex(3,c.file,c.id) punctuation FROM suffix_reach r JOIN node p ON p.file=r.host AND p.id=r.node AND p.kind={parenthesized} JOIN node c ON c.file=p.file AND c.parent=p.id;
CREATE INDEX suffix_child_key ON suffix_children(host,scope,parent,kind,api);
CREATE TEMP TABLE suffix_base AS SELECT r.*,object.id object,p.truth,p.c0 capture FROM suffix_reach r JOIN node n ON n.file=r.host AND n.id=r.node AND n.kind={attribute} JOIN node api ON api.file=n.file AND api.parent=n.id AND api.field={attr} AND api.ffirst=1 AND api.kind={identifier} JOIN node object ON object.file=n.file AND object.parent=n.id AND object.field={object} AND object.ffirst=1 AND object.kind={attribute} JOIN node segment ON segment.file=object.file AND segment.parent=object.id AND segment.field={attr} AND segment.ffirst=1 AND segment.kind={identifier} JOIN relation_pattern_typed p ON p.term=0 AND p.file=r.host AND p.node=api.id AND p.i0 IS r.api WHERE node_regex(1,api.file,api.id) AND node_regex(2,segment.file,segment.id);
{parent_links}
CREATE TEMP TABLE suffix_hf AS {hf_select};
CREATE UNIQUE INDEX suffix_hf_key ON suffix_hf(host,scope,node,kind,api);
CREATE TEMP TABLE suffix_nodes AS WITH descriptions(utility,host,scope,node,kind,api,truth,capture,cost,peak) AS(
 SELECT 10,host,scope,node,kind,api,truth,capture,cost,peak FROM suffix_hf
 UNION SELECT 20,h.host,h.scope,h.node,h.kind,h.api,CASE WHEN n.kind={attribute} THEN h.truth ELSE 0 END,CASE WHEN n.kind={attribute} AND h.truth=1 THEN h.capture ELSE h.api END,1,1 FROM suffix_hf h JOIN node n ON n.file=h.host AND n.id=h.node WHERE n.kind<>{parenthesized} OR h.truth=0
 UNION SELECT 20+j.column1,h.host,h.scope,h.node,h.kind,h.api,0,h.api,1,1 FROM suffix_hf h CROSS JOIN(VALUES(1),(2))j WHERE h.truth=0
 UNION SELECT 30,b.host,b.scope,b.node,1,b.capture,1,b.capture,2,2 FROM suffix_base b WHERE b.truth=1
 UNION SELECT 30,b.host,b.scope,b.object,1,b.capture,1,b.capture,1,1 FROM suffix_base b WHERE b.truth=1)
 SELECT ROW_NUMBER() OVER(ORDER BY utility,host,scope,node,kind,typeof(api),api) id,{native_mode} native_mode,* FROM descriptions;
CREATE UNIQUE INDEX suffix_node_id ON suffix_nodes(id);
CREATE INDEX suffix_node_key ON suffix_nodes(utility,host,scope,node,kind,api);
"#)).unwrap();
        if plan.unique() {
            let invalid:i64=db.query_row("SELECT COUNT(*) FROM suffix_nodes WHERE utility IS NULL OR host IS NULL OR scope IS NULL OR node IS NULL OR kind IS NULL OR native_mode IS NULL OR NOT((kind=0 AND api IS NULL) OR(kind=1 AND typeof(api)='integer' AND api>=0 AND api<=4294967295))",[],|r|r.get(0)).unwrap();
            assert_eq!(
                invalid, 0,
                "normalized key requires the declared absent/node protocol"
            );
            // Tag0/null and tag1/Node0 remain distinct. All joins below use
            // equality on this exact non-null payload plus the semantic tag.
            db.execute_batch("CREATE UNIQUE INDEX suffix_semantic_key ON suffix_nodes(utility,host,scope,node,kind,COALESCE(api,0),native_mode);CREATE INDEX suffix_base_key ON suffix_base(host,scope,node,capture,truth);").unwrap();
        }
        if matches!(plan, EdgePlan::Analyzed) {
            db.execute_batch("ANALYZE suffix_nodes;ANALYZE suffix_children;ANALYZE suffix_base;")
                .unwrap();
        }
        let edge_select = Self::edge_select(language, three, plan);
        db.execute_batch(&format!("CREATE TEMP TABLE suffix_edges AS {edge_select};CREATE UNIQUE INDEX suffix_edge_key ON suffix_edges(parent,ordinal);")).unwrap();
        Self::failure_followers(db, language);
        Self {
            edge_select,
            hf_select,
        }
    }
    fn failure_followers(db: &rusqlite::Connection, language: GrammarDialect) {
        let parenthesized = language.kinds().kind_id("parenthesized_expression");
        db.execute_batch(&format!(r#"CREATE TEMP TABLE suffix_failure_sites AS
SELECT p.id parent,ROW_NUMBER() OVER(PARTITION BY p.id ORDER BY raw.ordinal)-1 position,10 utility,p.host,p.scope,raw.node,p.kind,p.api,p.native_mode FROM suffix_nodes p JOIN node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind={parenthesized} JOIN suffix_children raw ON raw.host=p.host AND raw.scope=p.scope AND raw.parent=p.node AND raw.kind=p.kind AND raw.api IS p.api WHERE p.utility=10 AND NOT raw.comment AND(p.native_mode=1 OR NOT raw.punctuation)
UNION ALL SELECT p.id,(SELECT COUNT(*) FROM suffix_children raw WHERE raw.host=p.host AND raw.scope=p.scope AND raw.parent=p.node AND raw.kind=p.kind AND raw.api IS p.api AND NOT raw.comment AND(p.native_mode=1 OR NOT raw.punctuation))+j.column1,20+j.column1,p.host,p.scope,p.node,p.kind,p.api,p.native_mode FROM suffix_nodes p JOIN node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind={parenthesized} CROSS JOIN(VALUES(0),(1),(2))j WHERE p.utility=10
UNION ALL SELECT p.id,j.column1,20+j.column1,p.host,p.scope,p.node,p.kind,p.api,p.native_mode FROM suffix_nodes p JOIN node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind<>{parenthesized} CROSS JOIN(VALUES(0),(1),(2))j WHERE p.utility=10;
CREATE UNIQUE INDEX suffix_failure_site_order ON suffix_failure_sites(parent,position);
CREATE INDEX suffix_failure_site_key ON suffix_failure_sites(parent,utility,node,kind,api);
CREATE TEMP TABLE suffix_failure_followers AS SELECT edge.parent,edge.ordinal after_ordinal,ROW_NUMBER() OVER(PARTITION BY edge.parent,edge.ordinal ORDER BY following.position)-1 ordinal,following.utility,following.host,following.scope,following.node,following.kind,following.api,following.native_mode FROM suffix_edges edge JOIN suffix_nodes parent ON parent.id=edge.parent JOIN suffix_nodes current ON current.id=edge.child LEFT JOIN suffix_failure_sites own ON own.parent=edge.parent AND own.utility=current.utility AND own.node=current.node AND own.kind=current.kind AND own.api IS current.api JOIN suffix_failure_sites following ON following.parent=edge.parent AND following.position>COALESCE(own.position,CASE WHEN parent.utility=10 AND current.utility=30 THEN 0 END);
CREATE UNIQUE INDEX suffix_failure_follower_key ON suffix_failure_followers(parent,after_ordinal,ordinal);"#)).unwrap();
    }
    fn edge_select(language: GrammarDialect, three: bool, plan: EdgePlan) -> String {
        let parenthesized = language.kinds().kind_id("parenthesized_expression");
        let native_mode = usize::from(three);
        let link = if matches!(plan, EdgePlan::Ordered) {
            "CROSS JOIN"
        } else {
            "JOIN"
        };
        let same_api = if plan.unique() {
            "COALESCE(c.api,0)=COALESCE(p.api,0) AND c.native_mode=p.native_mode"
        } else {
            "c.api IS p.api"
        };
        let same_capture = if plan.unique() {
            "COALESCE(c.api,0)=COALESCE(p.capture,0) AND c.native_mode=p.native_mode"
        } else {
            "c.api IS p.capture"
        };
        format!(
            r#" SELECT p.id parent,ROW_NUMBER() OVER(PARTITION BY p.id ORDER BY raw.ordinal)-1 ordinal,c.id child FROM suffix_nodes p {link} node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind={parenthesized} {link} suffix_children raw ON raw.host=p.host AND raw.scope=p.scope AND raw.parent=p.node AND raw.kind=p.kind AND raw.api IS p.api {link} suffix_children expression ON expression.host=p.host AND expression.scope=p.scope AND expression.parent=p.node AND expression.kind=p.kind AND expression.api IS p.api AND NOT expression.comment AND NOT expression.punctuation {link} suffix_nodes c ON c.utility=10 AND c.host=p.host AND c.scope=p.scope AND c.node=raw.node AND c.kind=p.kind AND {same_api} WHERE p.utility=10 AND NOT raw.comment AND(({native_mode}=1 AND(p.truth=0 OR raw.ordinal<=expression.ordinal)) OR({native_mode}=0 AND NOT raw.punctuation))
 UNION ALL SELECT p.id,(SELECT COUNT(*) FROM suffix_children raw WHERE raw.host=p.host AND raw.scope=p.scope AND raw.parent=p.node AND raw.kind=p.kind AND raw.api IS p.api AND NOT raw.comment AND({native_mode}=1 OR NOT raw.punctuation))+j.column1,c.id FROM suffix_nodes p {link} node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind={parenthesized} CROSS JOIN(VALUES(0),(1),(2))j {link} suffix_nodes c ON c.utility=20+j.column1 AND c.host=p.host AND c.scope=p.scope AND c.node=p.node AND c.kind=p.kind AND {same_api} WHERE p.utility=10 AND p.truth=0
 UNION ALL SELECT p.id,0,c.id FROM suffix_nodes p {link} node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind<>{parenthesized} {link} suffix_nodes c ON c.utility=20 AND c.host=p.host AND c.scope=p.scope AND c.node=p.node AND c.kind=p.kind AND {same_api} WHERE p.utility=10
 UNION ALL SELECT p.id,j.column1,c.id FROM suffix_nodes p {link} node ast ON ast.file=p.host AND ast.id=p.node AND ast.kind<>{parenthesized} CROSS JOIN(VALUES(1),(2))j {link} suffix_nodes c ON c.utility=20+j.column1 AND c.host=p.host AND c.scope=p.scope AND c.node=p.node AND c.kind=p.kind AND {same_api} WHERE p.utility=10 AND p.truth=0
 UNION ALL SELECT p.id,1,c.id FROM suffix_nodes p {link} suffix_nodes c ON c.utility=30 AND c.host=p.host AND c.scope=p.scope AND c.node=p.node AND c.kind=1 AND {same_capture} WHERE p.utility=10 AND p.truth=1 AND p.cost=4
 UNION ALL SELECT DISTINCT p.id,0,c.id FROM suffix_nodes p {link} suffix_base b ON b.host=p.host AND b.scope=p.scope AND b.node=p.node AND b.capture IS p.api AND b.truth=1 {link} suffix_nodes c ON c.utility=30 AND c.host=p.host AND c.scope=p.scope AND c.node=b.object AND c.kind=1 AND {same_api} WHERE p.utility=30 AND p.cost=2"#
        )
    }
    fn root(&self, db: &rusqlite::Connection, k: &Key) -> i64 {
        db.query_row("SELECT id FROM suffix_nodes WHERE utility=10 AND host=?1 AND scope=?2 AND node=?3 AND kind=?4 AND api IS ?5",rusqlite::params![k.host,k.scope,k.node,k.kind,k.api],|r|r.get(0)).unwrap()
    }
}
fn query() -> String {
    query_cleanup(true)
}
fn query_cleanup(cleanup: bool) -> String {
    let identity = "m.clause IS ?1 AND m.utility IS e.utility AND m.file IS e.host AND m.node IS e.node AND m.k0 IS e.api AND m.k1 IS e.kind AND m.k2 IS e.scope AND m.k3 IS e.native_mode";
    let hit = "m.truth IS NOT NULL AND w.level+m.peak<=256";
    let fail = format!("CASE WHEN ({hit}) THEN w.work<m.cost ELSE w.level>=256 OR w.work=0 END");
    let entered = format!("w.phase=0 AND NOT ({hit}) AND w.level<256 AND w.work>0");
    let entry = format!(
        "SELECT CASE WHEN ({entered}) AND child.child IS NOT NULL THEN child.child ELSE w.id END,CASE WHEN ({entered}) AND child.child IS NOT NULL THEN w.level+1 ELSE w.level END,CASE WHEN ({hit}) THEN MAX(0,w.work-m.cost) WHEN w.level>=256 THEN w.work ELSE MAX(0,w.work-1) END,COALESCE(frame.handle,w.handle),CASE WHEN ({fail}) THEN CASE WHEN NOT ({hit}) AND w.level>=256 THEN -1 ELSE -2 END WHEN ({hit}) THEN e.truth ELSE 0 END,CASE WHEN ({fail}) THEN 3 WHEN ({hit}) THEN 2 WHEN child.child IS NULL THEN 1 ELSE 0 END,CASE WHEN ({hit}) THEN MAX(w.peak,w.level+m.peak) WHEN w.level<256 THEN MAX(w.peak,w.level+1) ELSE w.peak END,0 FROM suffix_nodes e LEFT JOIN suffix_edges child ON child.parent=e.id AND child.ordinal=0 LEFT JOIN relation_memo m ON m.mode IS CASE WHEN w.phase=0 THEN 0 END AND {identity} LEFT JOIN relation_continuation frame ON frame.mode IS CASE WHEN ({entered}) AND child.child IS NOT NULL THEN 1 END AND frame.parent_input IS w.handle AND frame.v0 IS e.id AND frame.v1 IS w.level AND frame.v2 IS 1 WHERE e.id=w.id AND w.phase=0"
    );
    let complete = format!(
        "SELECT w.id,w.level,w.work,w.handle,e.truth,2,w.peak,0 FROM suffix_nodes e LEFT JOIN relation_memo m ON m.mode IS CASE WHEN w.phase=1 THEN 1 END AND {identity} AND m.vtruth IS e.truth AND m.vcost IS e.cost AND m.vpeak IS e.peak AND m.v0 IS e.capture WHERE e.id=w.id AND w.phase=1"
    );
    let returned = "SELECT CASE WHEN child.child IS NOT NULL THEN child.child ELSE p.c0 END,CASE WHEN child.child IS NOT NULL THEN p.c1+1 ELSE p.c1 END,w.work,COALESCE(updated.handle,popped.parent),w.truth,CASE WHEN w.handle IS NULL THEN 4 WHEN child.child IS NULL THEN 1 ELSE 0 END,w.peak,0 FROM(SELECT 1)guard LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.phase=2 AND w.handle IS NOT NULL THEN 0 END AND p.h IS w.handle LEFT JOIN suffix_edges child ON child.parent=p.c0 AND child.ordinal=p.c2 LEFT JOIN relation_continuation popped ON popped.mode IS CASE WHEN w.phase=2 AND p.handle IS NOT NULL THEN 2 END AND popped.h IS p.handle LEFT JOIN relation_continuation updated ON updated.mode IS CASE WHEN w.phase=2 AND p.handle IS NOT NULL AND child.child IS NOT NULL THEN 1 END AND updated.parent_input IS popped.parent AND updated.v0 IS p.c0 AND updated.v1 IS p.c1 AND updated.v2 IS p.c2+1 WHERE w.phase=2";
    let abort = if cleanup {
        "SELECT CASE WHEN f.ordinal IS NULL THEN p.c0 ELSE w.id END,CASE WHEN f.ordinal IS NULL THEN p.c1 ELSE w.level END,w.work,CASE WHEN f.ordinal IS NULL THEN popped.parent ELSE w.handle END,w.truth,CASE WHEN f.ordinal IS NULL AND popped.parent IS NULL THEN 4 ELSE 3 END,CASE WHEN m.truth IS NOT NULL AND p.c1+1+m.peak<=256 THEN MAX(w.peak,p.c1+1+m.peak) ELSE w.peak END,CASE WHEN f.ordinal IS NULL THEN 0 ELSE w.follower+1 END FROM(SELECT 1)guard LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.phase=3 AND w.handle IS NOT NULL THEN 0 END AND p.h IS w.handle LEFT JOIN suffix_failure_followers f ON f.parent=p.c0 AND f.after_ordinal=p.c2-1 AND f.ordinal=w.follower LEFT JOIN relation_memo m ON m.mode IS CASE WHEN w.phase=3 AND f.ordinal IS NOT NULL THEN 0 END AND m.clause IS ?1 AND m.utility IS f.utility AND m.file IS f.host AND m.node IS f.node AND m.k0 IS f.api AND m.k1 IS f.kind AND m.k2 IS f.scope AND m.k3 IS f.native_mode LEFT JOIN relation_continuation popped ON popped.mode IS CASE WHEN w.phase=3 AND p.handle IS NOT NULL AND f.ordinal IS NULL THEN 2 END AND popped.h IS p.handle WHERE w.phase=3"
    } else {
        "SELECT w.id,w.level,w.work,p.parent,w.truth,CASE WHEN p.parent IS NULL THEN 4 ELSE 3 END,w.peak,0 FROM(SELECT 1)guard LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN w.phase=3 AND w.handle IS NOT NULL THEN 2 END AND p.h IS w.handle WHERE w.phase=3"
    };
    let dispatch = format!("{entry} UNION ALL {complete} UNION ALL {returned} UNION ALL {abort}");
    format!(
        "WITH RECURSIVE machine(id,level,work,handle,truth,phase,peak,follower) AS(SELECT ?2,?3,?4,NULL,0,0,0,0 UNION ALL SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7 FROM machine w {} WHERE w.phase<>4) SELECT w.truth,w.work,w.peak,CASE WHEN w.truth=1 THEN root.capture ELSE root.api END FROM machine w JOIN suffix_nodes root ON root.id=?2 WHERE w.phase=4",
        bridge(&dispatch, 8)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        CodeRuleDslRuntime, CompiledCodeRuleDsl, matcher,
        sql::{exec, tests::differential::tape},
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
        if let Some(name) = seed {
            all.push(json!({"has":{"stopBy":"end","kind":"identifier","regex":format!("^{name}$"),"pattern":"$API"}}));
        }
        for _ in 0..repeat {
            all.push(json!({"matches":"hf-callee"}));
        }
        p["clauses"][0]["forbid"]["ast"]["rule"] = json!({"all":all});
        CodeRuleDslRuntime::compile(p.to_string(), "json".into(), CAPABILITIES.into()).unwrap()
    }
    #[test]
    fn structural_spike_b615_suffix_graph_native_cold_warm_false_and_depth() {
        for seed in [None, Some("from_pretrained"), Some("transformers")] {
            for repeat in [1, 3] {
                let p = policy(repeat, seed);
                let g = &grammar_matchers(&p)[0];
                let doc = tape(g.language, "((((wrapper.transformers.from_pretrained))))\n");
                let tree = AstGrep::doc(doc.clone());
                let roots = tree
                    .root()
                    .dfs()
                    .filter(|n| {
                        matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                    })
                    .filter(|n| prove(n.clone()).is_ok())
                    .collect::<Vec<_>>();
                let proofs = roots
                    .iter()
                    .cloned()
                    .map(|n| prove(n).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(proofs.len(), 5);
                let demands = proofs
                    .iter()
                    .flat_map(|p| {
                        [
                            key(0, 0, p, 0, Cell::Null).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.api))).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.incompatible))).unwrap(),
                        ]
                    })
                    .collect::<Vec<_>>();
                let cases = [
                    (4, 0, 64),
                    (0, 0, 4),
                    (0, 0, 64),
                    (2, 0, 64),
                    (0, 0, 0),
                    (0, 0, 8),
                    (0, 0, 10),
                    (1, 253, 64),
                    (0, 255, 64),
                    (4, 256, 64),
                    (0, 0, 64),
                    (4, 0, 0),
                    (0, 250, 8),
                ];
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let expected = matcher::with_file_memo(&tree, || {
                    cases
                        .iter()
                        .map(|&(root, level, work)| {
                            matcher::match_at_depth(g, roots[root].clone(), level, work)
                        })
                        .collect::<Vec<_>>()
                });
                let atom = pattern(&g.matcher).unwrap();
                let AstMatcher::Pattern(atom) = atom else {
                    panic!("pattern")
                };
                exec::with_terminals(
                    vec![(doc, Default::default())],
                    terminals(atom, g.language),
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        let producer = Suffix::batch(db, g.language, &demands);
                        let sql = query();
                        for ((root, level, work), native) in cases.into_iter().zip(expected) {
                            let p = &proofs[root];
                            let mut capture = match seed {
                                None => Cell::Null,
                                Some("from_pretrained") => Cell::Integer(i64::from(p.api)),
                                _ => Cell::Integer(i64::from(p.incompatible)),
                            };
                            let mut remaining = work as i64;
                            let mut peak = 0;
                            let mut truth = 1;
                            for _ in 0..repeat {
                                let input = key(
                                    0,
                                    0,
                                    p,
                                    u32::from(!matches!(capture, Cell::Null)),
                                    capture.clone(),
                                )
                                .unwrap();
                                let id = producer.root(db, &input);
                                let row: (i64, i64, u32, Cell) = db
                                    .query_row(
                                        &sql,
                                        rusqlite::params![9300000, id, level, remaining],
                                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                                    )
                                    .unwrap();
                                truth = row.0;
                                remaining = row.1;
                                peak = peak.max(row.2);
                                capture = row.3;
                                if truth != 1 {
                                    break;
                                }
                            }
                            assert_eq!(
                                (truth, remaining, peak.saturating_sub(level)),
                                (
                                    native.truth,
                                    native.work as i64,
                                    native.peak.saturating_sub(level)
                                ),
                                "seed={seed:?} repeat={repeat} root={root} level={level} work={work}"
                            );
                            if truth == 1 {
                                assert_eq!(capture.single_node().unwrap(), Some(p.api));
                            }
                            assert_eq!(
                                db.query_row("SELECT relation_continuation_rows()", [], |r| r
                                    .get::<_, i64>(0))
                                    .unwrap(),
                                0
                            );
                        }
                    },
                );
            }
        }
    }
    #[derive(Default)]
    struct ProfileRows(Vec<Value>);
    struct ProfileTrace<'a> {
        db: &'a rusqlite::Connection,
        rows: Box<ProfileRows>,
    }
    impl Drop for ProfileTrace<'_> {
        fn drop(&mut self) {
            // SAFETY: remove the callback before freeing its stable boxed rows.
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(self.db.handle(), 0, None, std::ptr::null_mut());
            }
        }
    }
    fn measured<T>(db: &rusqlite::Connection, body: impl FnOnce() -> T) -> (T, Value) {
        unsafe extern "C" fn profile(
            mask: u32,
            context: *mut std::ffi::c_void,
            statement: *mut std::ffi::c_void,
            duration: *mut std::ffi::c_void,
        ) -> i32 {
            if mask == rusqlite::ffi::SQLITE_TRACE_PROFILE {
                // SAFETY: SQLite supplies a live statement and nanosecond integer
                // during PROFILE; the stable box lives until callback removal.
                let (rows, sql, vm, fullscan, autoindex, sorts, ns) = unsafe {
                    let statement = statement.cast::<rusqlite::ffi::sqlite3_stmt>();
                    let text = rusqlite::ffi::sqlite3_sql(statement);
                    let sql = if text.is_null() {
                        std::borrow::Cow::Borrowed("<internal>")
                    } else {
                        std::ffi::CStr::from_ptr(text).to_string_lossy()
                    };
                    (
                        &mut (*context.cast::<ProfileRows>()).0,
                        sql,
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_VM_STEP,
                            1,
                        ),
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,
                            1,
                        ),
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_AUTOINDEX,
                            1,
                        ),
                        rusqlite::ffi::sqlite3_stmt_status(
                            statement,
                            rusqlite::ffi::SQLITE_STMTSTATUS_SORT,
                            1,
                        ),
                        *duration.cast::<i64>(),
                    )
                };
                rows.push(json!({"sql":sql.to_string(),"fullscanSteps":fullscan,"automaticIndexRows":autoindex,"sorts":sorts,"sqlPrefix":sql.split_whitespace().take(3).collect::<Vec<_>>().join(" "),"sqlBytes":sql.len(),"vmSteps":vm,"executionWallNs":ns}));
            }
            0
        }
        let mut trace = ProfileTrace {
            db,
            rows: Box::default(),
        };
        // SAFETY: the connection and stable boxed vector outlive the callback.
        let status = unsafe {
            rusqlite::ffi::sqlite3_trace_v2(
                db.handle(),
                rusqlite::ffi::SQLITE_TRACE_PROFILE,
                Some(profile),
                (&mut *trace.rows as *mut ProfileRows).cast(),
            )
        };
        assert_eq!(status, rusqlite::ffi::SQLITE_OK);
        let start = std::time::Instant::now();
        let result = body();
        let wall = start.elapsed().as_nanos() as u64;
        // SAFETY: stop callbacks before reading or moving the profile vector.
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(db.handle(), 0, None, std::ptr::null_mut());
        }
        let statements = std::mem::take(&mut trace.rows.0);
        let vm = statements
            .iter()
            .map(|v| v["vmSteps"].as_u64().unwrap())
            .sum::<u64>();
        let execution_wall = statements
            .iter()
            .map(|v| v["executionWallNs"].as_u64().unwrap())
            .sum::<u64>();
        (
            result,
            json!({"vmSteps":vm,"envelopeWallNs":wall,"executionWallNs":execution_wall,"executionClock":"SQLite PROFILE duration, coarse clock resolution","statements":statements}),
        )
    }

    #[test]
    fn structural_spike_b615_shared_suffix_build_and_consumption_scaling() {
        let mut evidence = vec![];
        for depth in [1, 8, 32] {
            for mode in ["inner", "outer", "all"] {
                for diversity in ["absent", "bound", "both"] {
                    let absent = policy(1, None);
                    let bound = policy(1, Some("from_pretrained"));
                    let g = &grammar_matchers(&absent)[0];
                    let source = format!(
                        "{}wrapper.transformers.from_pretrained{}\n",
                        "(".repeat(depth),
                        ")".repeat(depth)
                    );
                    let doc = tape(g.language, &source);
                    let tree = AstGrep::doc(doc.clone());
                    let all = tree
                        .root()
                        .dfs()
                        .filter(|n| {
                            matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                        })
                        .filter(|n| prove(n.clone()).is_ok())
                        .collect::<Vec<_>>();
                    assert_eq!(all.len(), depth + 1);
                    let selected = match mode {
                        "inner" => vec![all.len() - 1],
                        "outer" => vec![0],
                        _ => (0..all.len()).rev().collect::<Vec<_>>(),
                    };
                    let variants = match diversity {
                        "absent" => vec![false],
                        "bound" => vec![true],
                        _ => vec![false, true],
                    };
                    let proofs = selected
                        .iter()
                        .map(|&root| prove(all[root].clone()).unwrap())
                        .collect::<Vec<_>>();
                    let demands = proofs
                        .iter()
                        .flat_map(|p| {
                            variants.iter().map(|present| {
                                key(
                                    0,
                                    0,
                                    p,
                                    u32::from(*present),
                                    if *present {
                                        Cell::Integer(i64::from(p.api))
                                    } else {
                                        Cell::Null
                                    },
                                )
                                .unwrap()
                            })
                        })
                        .collect::<Vec<_>>();
                    let _scope = matcher::FileScopeSlot::install(
                        Default::default(),
                        Default::default(),
                        false,
                    );
                    let native = variants
                        .iter()
                        .map(|present| {
                            let native_grammar = if *present {
                                &grammar_matchers(&bound)[0]
                            } else {
                                g
                            };
                            matcher::with_file_memo(&tree, || {
                                selected
                                    .iter()
                                    .map(|&root| {
                                        matcher::match_at_depth(
                                            native_grammar,
                                            all[root].clone(),
                                            0,
                                            10000,
                                        )
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .collect::<Vec<_>>();
                    let atom = pattern(&g.matcher).unwrap();
                    let AstMatcher::Pattern(atom) = atom else {
                        panic!("atom")
                    };
                    exec::with_terminals(
                        vec![(doc, Default::default())],
                        terminals(atom, g.language),
                        |db, batch, _| {
                            crate::code_rule_dsl::sql::relational::leaves::register(db, batch)
                                .unwrap();
                            let (_producer, build) =
                                measured(db, || Suffix::batch(db, g.language, &demands));
                            let count = |table: &str| {
                                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                                    r.get::<_, i64>(0)
                                })
                                .unwrap()
                            };
                            let reachable = count("suffix_reach");
                            let bases = count("suffix_base");
                            let nodes = count("suffix_nodes");
                            let edges = count("suffix_edges");
                            let sql = query();
                            let prepared = std::time::Instant::now();
                            let mut invocation = db.prepare(&sql).unwrap();
                            let mut routing=db.prepare("SELECT id FROM suffix_nodes WHERE utility=10 AND host=?1 AND scope=?2 AND node=?3 AND kind=?4 AND api IS ?5").unwrap();
                            let prepare = prepared.elapsed().as_nanos() as u64;
                            let before =
                                crate::code_rule_dsl::sql::relational::continuation_store::traffic(
                                );
                            let (_, consume) = measured(db, || {
                                for (variant_index, present) in variants.iter().enumerate() {
                                    for (root_index, p) in proofs.iter().enumerate() {
                                        let input = key(
                                            0,
                                            0,
                                            p,
                                            u32::from(*present),
                                            if *present {
                                                Cell::Integer(i64::from(p.api))
                                            } else {
                                                Cell::Null
                                            },
                                        )
                                        .unwrap();
                                        let id: i64 = routing
                                            .query_row(
                                                rusqlite::params![
                                                    input.host,
                                                    input.scope,
                                                    input.node,
                                                    input.kind,
                                                    input.api
                                                ],
                                                |r| r.get(0),
                                            )
                                            .unwrap();
                                        let row: (i64, i64, u32, Cell) = invocation
                                            .query_row(
                                                rusqlite::params![
                                                    9400000 + variant_index as i64,
                                                    id,
                                                    0,
                                                    10000
                                                ],
                                                |r| {
                                                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                                                },
                                            )
                                            .unwrap();
                                        let expected = &native[variant_index][root_index];
                                        assert_eq!(
                                            (row.0, row.1, row.2),
                                            (expected.truth, expected.work as i64, expected.peak)
                                        );
                                        assert_eq!(row.3.single_node().unwrap(), Some(p.api));
                                    }
                                }
                            });
                            let after =
                                crate::code_rule_dsl::sql::relational::continuation_store::traffic(
                                );
                            let traffic = after
                                .into_iter()
                                .zip(before)
                                .map(|(a, b)| a - b)
                                .collect::<Vec<_>>();
                            let(live,peak):(i64,i64)=db.query_row("SELECT relation_continuation_rows(),relation_continuation_peak()",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                            assert_eq!(live, 0);
                            evidence.push(json!({"depth":depth,"rootMode":mode,"captureDiversity":diversity,"distinctDemands":demands.len(),"reachableTypedNodes":reachable,"basePatternEvaluations":bases,"sharedUtilityNodes":nodes,"sharedUtilityEdges":edges,"buildCTASAndIndexes":build,"frontierConsumption":consume,"frontierPrepareWallNs":prepare,"totalVmSteps":build["vmSteps"].as_u64().unwrap()+consume["vmSteps"].as_u64().unwrap(),"totalEnvelopeWallNs":build["envelopeWallNs"].as_u64().unwrap()+consume["envelopeWallNs"].as_u64().unwrap(),"totalExecutionWallNs":build["executionWallNs"].as_u64().unwrap()+consume["executionWallNs"].as_u64().unwrap(),"continuationTraffic":traffic,"continuationLive":live,"continuationPeak":peak,"representation":"shared suffix node/edge graph; no per-root interval rows or lazy per-root trace construction","accounting":"all SQL schema/population/CTAS/index work included; prepared consumption; per-demand helper calls are diagnostic, not a job budget claim"}));
                        },
                    );
                }
            }
        }
        for depth in [1, 8, 32] {
            for diversity in ["absent", "bound", "both"] {
                let outer = evidence
                    .iter()
                    .find(|r| {
                        r["depth"] == depth
                            && r["captureDiversity"] == diversity
                            && r["rootMode"] == "outer"
                    })
                    .unwrap();
                let all = evidence
                    .iter()
                    .find(|r| {
                        r["depth"] == depth
                            && r["captureDiversity"] == diversity
                            && r["rootMode"] == "all"
                    })
                    .unwrap();
                assert_eq!(outer["sharedUtilityNodes"], all["sharedUtilityNodes"]);
                assert_eq!(outer["sharedUtilityEdges"], all["sharedUtilityEdges"]);
                assert_eq!(
                    outer["basePatternEvaluations"],
                    all["basePatternEvaluations"]
                );
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.suffix-scaling.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_suffix_empty_demand_is_empty() {
        let p = policy(1, None);
        let g = &grammar_matchers(&p)[0];
        let doc = tape(g.language, "wrapper.transformers.from_pretrained\n");
        let atom = pattern(&g.matcher).unwrap();
        let AstMatcher::Pattern(atom) = atom else {
            panic!("pattern")
        };
        exec::with_terminals(
            vec![(doc, Default::default())],
            terminals(atom, g.language),
            |db, batch, _| {
                crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                Suffix::batch(db, g.language, &[]);
                assert_eq!(
                    db.query_row("SELECT COUNT(*) FROM suffix_nodes", [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                assert_eq!(
                    db.query_row("SELECT COUNT(*) FROM suffix_edges", [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            },
        );
    }
    fn original_context(
        repeat: usize,
        seed: Option<&str>,
    ) -> crate::code_rule_dsl::matcher::GrammarMatcher {
        let original = CodeRuleDslRuntime::compile(
            include_str!(
                "../../../../../../packs/python/rules/bandit-b615-huggingface-unsafe-download.json"
            )
            .into(),
            "json".into(),
            CAPABILITIES.into(),
        )
        .unwrap();
        let mut g = grammar_matchers(&original)[0].clone();
        assert!(g.three_valued, "preserve ORIGINAL full-clause mode");
        let index = g
            .utils
            .names
            .iter()
            .position(|name| name == "hf-callee")
            .unwrap();
        let hf = AstMatcher::Util(matcher::UtilRef {
            table: std::sync::Arc::downgrade(&g.utils),
            index,
        });
        let mut all = vec![];
        if let Some(seed) = seed {
            let p = policy(1, Some(seed));
            let AstMatcher::All(prefix) = &grammar_matchers(&p)[0].matcher else {
                panic!("seed prefix")
            };
            all.push(prefix[0].clone());
        }
        for _ in 0..repeat {
            all.push(hf.clone());
        }
        g.matcher = AstMatcher::All(all);
        assert!(g.three_valued);
        g
    }
    #[test]
    fn structural_spike_b615_original_three_mode_source_order_and_native_prefixes() {
        let mut observations = vec![];
        for depth in [0, 1, 3] {
            for comments in [false, true] {
                for seed in [None, Some("from_pretrained"), Some("transformers")] {
                    let g = original_context(1, seed);
                    let mut source = "wrapper.transformers.from_pretrained".to_owned();
                    for _ in 0..depth {
                        source = if comments {
                            format!("( # leading\n{source} # trailing\n)")
                        } else {
                            format!("({source})")
                        };
                    }
                    source.push('\n');
                    let doc = tape(g.language, &source);
                    let tree = AstGrep::doc(doc.clone());
                    let root = tree
                        .root()
                        .dfs()
                        .find(|n| {
                            n.kind()
                                == if depth == 0 {
                                    "attribute"
                                } else {
                                    "parenthesized_expression"
                                }
                        })
                        .unwrap();
                    let p = prove(root.clone()).unwrap();
                    let initial = match seed {
                        None => Cell::Null,
                        Some("from_pretrained") => Cell::Integer(i64::from(p.api)),
                        _ => Cell::Integer(i64::from(p.incompatible)),
                    };
                    let input =
                        key(0, 0, &p, u32::from(!matches!(initial, Cell::Null)), initial).unwrap();
                    let _scope = matcher::FileScopeSlot::install(
                        Default::default(),
                        Default::default(),
                        false,
                    );
                    let (native, calls) =
                        crate::code_rule_dsl::matcher::value_probe::run_reached(None, || {
                            matcher::with_file_memo(&tree, || {
                                matcher::match_at_depth(&g, root.clone(), 0, 10000)
                            })
                        });
                    assert!(calls.is_empty());
                    let atom = pattern(&g.matcher).unwrap();
                    let AstMatcher::Pattern(atom) = atom else {
                        panic!("pattern")
                    };
                    exec::with_terminals(
                        vec![(doc, Default::default())],
                        terminals(atom, g.language),
                        |db, batch, _| {
                            crate::code_rule_dsl::sql::relational::leaves::register(db, batch)
                                .unwrap();
                            let graph = Suffix::batch_mode(
                                db,
                                g.language,
                                std::slice::from_ref(&input),
                                true,
                            );
                            let id = graph.root(db, &input);
                            let (cost, peak): (i64, u32) = db
                                .query_row(
                                    "SELECT cost,peak FROM suffix_nodes WHERE id=?1",
                                    [id],
                                    |r| Ok((r.get(0)?, r.get(1)?)),
                                )
                                .unwrap();
                            assert_eq!(
                                (cost, peak),
                                (10000 - native.work as i64, native.peak),
                                "depth={depth} seed={seed:?} comments={comments}"
                            );
                            let comment_facts: i64 = db
                                .query_row(
                                    "SELECT COUNT(*) FROM suffix_children WHERE comment",
                                    [],
                                    |r| r.get(0),
                                )
                                .unwrap();
                            if comments && depth > 0 {
                                assert!(comment_facts > 0);
                            }
                            let comment_calls:i64=db.query_row("SELECT COUNT(*) FROM suffix_nodes s JOIN node n ON n.file=s.host AND n.id=s.node WHERE s.utility=10 AND n.kind=?1",[g.language.kinds().kind_id("comment")],|r|r.get(0)).unwrap();
                            assert_eq!(
                                comment_calls, 0,
                                "source not-comment guard precedes hf invocation"
                            );
                            let punctuation_calls:i64=db.query_row("SELECT COUNT(*) FROM suffix_edges edge JOIN suffix_nodes n ON n.id=edge.child WHERE n.utility=10 AND node_regex(3,n.host,n.node)",[],|r|r.get(0)).unwrap();
                            if depth > 0 {
                                assert!(punctuation_calls > 0);
                            }
                            let row: (i64, i64, u32, Cell) = db
                                .query_row(
                                    &query(),
                                    rusqlite::params![9500000, id, 0, 10000],
                                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                                )
                                .unwrap();
                            assert_eq!(
                                (row.0, row.1, row.2),
                                (native.truth, native.work as i64, native.peak)
                            );
                            observations.push(json!({"ambientThreeValued":true,"originalGrammarContextPreserved":true,"depth":depth,"comments":comments,"APISeed":seed,"nativeTruth":native.truth,"nativeExecutionCost":10000-native.work,"nativePeak":native.peak,"graphReplayCost":cost,"sourceCommentRows":comment_facts,"commentUtilityCalls":comment_calls,"punctuationUtilityEdges":punctuation_calls,"valueProbes":calls.len()}));
                        },
                    );
                }
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.three-mode.json"),
                serde_json::to_vec_pretty(&observations).unwrap(),
            )
            .unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_original_three_mode_warm_depth_and_work() {
        for seed in [None, Some("from_pretrained"), Some("transformers")] {
            for repeat in [1, 3] {
                let g = original_context(repeat, seed);
                let doc = tape(
                    g.language,
                    "( # comment\n( (wrapper.transformers.from_pretrained) # tail\n) )\n",
                );
                let tree = AstGrep::doc(doc.clone());
                let roots = tree
                    .root()
                    .dfs()
                    .filter(|n| {
                        matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                    })
                    .filter(|n| prove(n.clone()).is_ok())
                    .collect::<Vec<_>>();
                let proofs = roots
                    .iter()
                    .cloned()
                    .map(|n| prove(n).unwrap())
                    .collect::<Vec<_>>();
                let demands = proofs
                    .iter()
                    .flat_map(|p| {
                        [
                            key(0, 0, p, 0, Cell::Null).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.api))).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.incompatible))).unwrap(),
                        ]
                    })
                    .collect::<Vec<_>>();
                let cases = [
                    (1, 0, 256),
                    (0, 0, 4),
                    (3, 0, 256),
                    (0, 0, 256),
                    (1, 0, 0),
                    (0, 0, 3),
                    (0, 0, 8),
                    (0, 0, 12),
                    (0, 0, 20),
                    (0, 0, 40),
                    (1, 253, 256),
                    (0, 255, 256),
                    (3, 256, 256),
                    (0, 0, 256),
                ];
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let expected = matcher::with_file_memo(&tree, || {
                    cases
                        .iter()
                        .map(|&(root, level, work)| {
                            matcher::match_at_depth(&g, roots[root].clone(), level, work)
                        })
                        .collect::<Vec<_>>()
                });
                let atom = pattern(&g.matcher).unwrap();
                let AstMatcher::Pattern(atom) = atom else {
                    panic!("pattern")
                };
                exec::with_terminals(
                    vec![(doc, Default::default())],
                    terminals(atom, g.language),
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        let graph = Suffix::batch_mode(db, g.language, &demands, true);
                        let sql = query();
                        for ((root, level, work), native) in cases.into_iter().zip(expected) {
                            let p = &proofs[root];
                            let mut input = match seed {
                                None => Cell::Null,
                                Some("from_pretrained") => Cell::Integer(i64::from(p.api)),
                                _ => Cell::Integer(i64::from(p.incompatible)),
                            };
                            let mut remaining = work as i64;
                            let mut peak = 0;
                            let mut truth = 1;
                            for _ in 0..repeat {
                                let typed = key(
                                    0,
                                    0,
                                    p,
                                    u32::from(!matches!(input, Cell::Null)),
                                    input.clone(),
                                )
                                .unwrap();
                                let id = graph.root(db, &typed);
                                let row: (i64, i64, u32, Cell) = db
                                    .query_row(
                                        &sql,
                                        rusqlite::params![9600000, id, level, remaining],
                                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                                    )
                                    .unwrap();
                                truth = row.0;
                                remaining = row.1;
                                peak = peak.max(row.2);
                                input = row.3;
                                if truth != 1 {
                                    break;
                                }
                            }
                            assert_eq!(
                                (truth, remaining, peak.saturating_sub(level)),
                                (
                                    native.truth,
                                    native.work as i64,
                                    native.peak.saturating_sub(level)
                                ),
                                "seed={seed:?} repeat={repeat} root={root} level={level} work={work}"
                            );
                            if truth == 1 {
                                assert_eq!(input.single_node().unwrap(), Some(p.api));
                            }
                            assert_eq!(
                                db.query_row("SELECT relation_continuation_rows()", [], |r| r
                                    .get::<_, i64>(0))
                                    .unwrap(),
                                0
                            );
                        }
                    },
                );
            }
        }
    }
    #[derive(Debug, PartialEq)]
    struct MaterializedShape {
        nodes: Vec<Vec<rusqlite::types::Value>>,
        edges: Vec<[i64; 3]>,
        followers: Vec<Vec<rusqlite::types::Value>>,
    }
    fn shape(db: &rusqlite::Connection) -> MaterializedShape {
        let nodes=db.prepare("SELECT native_mode,utility,host,scope,node,kind,api,truth,capture,cost,peak FROM suffix_nodes ORDER BY utility,host,scope,node,kind,typeof(api),api").unwrap().query_map([],|r|(0..11).map(|i|r.get(i)).collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        let edges = db
            .prepare("SELECT parent,ordinal,child FROM suffix_edges ORDER BY parent,ordinal")
            .unwrap()
            .query_map([], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?]))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let followers=db.prepare("SELECT parent,after_ordinal,ordinal,utility,host,scope,node,kind,api,native_mode FROM suffix_failure_followers ORDER BY parent,after_ordinal,ordinal").unwrap().query_map([],|r|(0..10).map(|i|r.get(i)).collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        MaterializedShape {
            nodes,
            edges,
            followers,
        }
    }
    fn explain(db: &rusqlite::Connection, sql: &str) -> Vec<Value> {
        db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap().query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"parent":r.get::<_,i64>(1)?,"detail":r.get::<_,String>(3)?}))).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    }
    #[test]
    fn structural_spike_b615_edge_plans_preserve_native_finite_and_mode_contracts() {
        for three in [false, true] {
            for seed in [None, Some("transformers")] {
                let isolated = policy(3, seed);
                let g = if three {
                    original_context(3, seed)
                } else {
                    grammar_matchers(&isolated)[0].clone()
                };
                let doc = tape(
                    g.language,
                    "( # comment\n( (wrapper.transformers.from_pretrained) ) )\n",
                );
                let tree = AstGrep::doc(doc.clone());
                let roots = tree
                    .root()
                    .dfs()
                    .filter(|n| {
                        matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                    })
                    .filter(|n| prove(n.clone()).is_ok())
                    .collect::<Vec<_>>();
                let proofs = roots
                    .iter()
                    .cloned()
                    .map(|n| prove(n).unwrap())
                    .collect::<Vec<_>>();
                let demands = proofs
                    .iter()
                    .flat_map(|p| {
                        [
                            key(0, 0, p, 0, Cell::Null).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.api))).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(i64::from(p.incompatible))).unwrap(),
                            key(0, 0, p, 1, Cell::Integer(0)).unwrap(),
                        ]
                    })
                    .collect::<Vec<_>>();
                let cases = [
                    (1, 0, 256),
                    (0, 0, 4),
                    (3, 0, 256),
                    (0, 0, 256),
                    (0, 0, 0),
                    (0, 0, 8),
                    (1, 253, 256),
                    (0, 255, 256),
                    (3, 256, 256),
                    (0, 0, 256),
                ];
                let _scope =
                    matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
                let native = matcher::with_file_memo(&tree, || {
                    cases
                        .iter()
                        .map(|&(root, level, work)| {
                            matcher::match_at_depth(&g, roots[root].clone(), level, work)
                        })
                        .collect::<Vec<_>>()
                });
                let mut baseline = None;
                for plan in [
                    EdgePlan::Baseline,
                    EdgePlan::Unique,
                    EdgePlan::Ordered,
                    EdgePlan::Analyzed,
                    EdgePlan::ParentLinks,
                ] {
                    let atom = pattern(&g.matcher).unwrap();
                    let AstMatcher::Pattern(atom) = atom else {
                        panic!("pattern")
                    };
                    exec::with_terminals(
                        vec![(doc.clone(), Default::default())],
                        terminals(atom, g.language),
                        |db, batch, _| {
                            crate::code_rule_dsl::sql::relational::leaves::register(db, batch)
                                .unwrap();
                            let graph = Suffix::batch_plan(db, g.language, &demands, three, plan);
                            let rows = shape(db);
                            if let Some(expected) = &baseline {
                                assert_eq!(&rows, expected, "mode={three} plan={plan:?}")
                            } else {
                                baseline = Some(rows)
                            }
                            let absent =
                                graph.root(db, &key(0, 0, &proofs[0], 0, Cell::Null).unwrap());
                            let node_zero = graph
                                .root(db, &key(0, 0, &proofs[0], 1, Cell::Integer(0)).unwrap());
                            assert_ne!(
                                absent, node_zero,
                                "semantic tag distinguishes absent from Node0"
                            );
                            let sql = query();
                            for ((root, level, work), expected) in cases.into_iter().zip(&native) {
                                let p = &proofs[root];
                                let mut api = if seed.is_some() {
                                    Cell::Integer(i64::from(p.incompatible))
                                } else {
                                    Cell::Null
                                };
                                let mut left = work as i64;
                                let mut peak = 0;
                                let mut truth = 1;
                                for _ in 0..3 {
                                    let id = graph.root(
                                        db,
                                        &key(
                                            0,
                                            0,
                                            p,
                                            u32::from(!matches!(api, Cell::Null)),
                                            api.clone(),
                                        )
                                        .unwrap(),
                                    );
                                    let result: (i64, i64, u32, Cell) = db
                                        .query_row(
                                            &sql,
                                            rusqlite::params![9700000, id, level, left],
                                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                                        )
                                        .unwrap();
                                    truth = result.0;
                                    left = result.1;
                                    peak = peak.max(result.2);
                                    api = result.3;
                                    if truth != 1 {
                                        break;
                                    }
                                }
                                assert_eq!(
                                    (truth, left, peak.saturating_sub(level)),
                                    (
                                        expected.truth,
                                        expected.work as i64,
                                        expected.peak.saturating_sub(level)
                                    ),
                                    "plan={plan:?} mode={three} seed={seed:?} root={root} level={level} work={work}"
                                );
                                if truth == 1 {
                                    assert_eq!(api.single_node().unwrap(), Some(p.api));
                                }
                                assert_eq!(
                                    db.query_row("SELECT relation_continuation_rows()", [], |r| {
                                        r.get::<_, i64>(0)
                                    })
                                    .unwrap(),
                                    0
                                );
                            }
                        },
                    );
                }
            }
        }
    }
    #[test]
    fn structural_spike_b615_edge_join_uniqueness_and_stats_scaling() {
        let mut reports = vec![];
        for three in [false, true] {
            for depth in [8, 32] {
                for mode in ["outer", "all"] {
                    let isolated = policy(1, None);
                    let g = if three {
                        original_context(1, None)
                    } else {
                        grammar_matchers(&isolated)[0].clone()
                    };
                    let doc = tape(
                        g.language,
                        &format!(
                            "{}wrapper.transformers.from_pretrained{}\n",
                            "(".repeat(depth),
                            ")".repeat(depth)
                        ),
                    );
                    let tree = AstGrep::doc(doc.clone());
                    let all = tree
                        .root()
                        .dfs()
                        .filter(|n| {
                            matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute")
                        })
                        .filter(|n| prove(n.clone()).is_ok())
                        .collect::<Vec<_>>();
                    let selected = if mode == "outer" {
                        vec![0]
                    } else {
                        (0..all.len()).rev().collect::<Vec<_>>()
                    };
                    let proofs = selected
                        .iter()
                        .map(|&i| prove(all[i].clone()).unwrap())
                        .collect::<Vec<_>>();
                    let demands = proofs
                        .iter()
                        .flat_map(|p| {
                            [
                                key(0, 0, p, 0, Cell::Null).unwrap(),
                                key(0, 0, p, 1, Cell::Integer(i64::from(p.api))).unwrap(),
                            ]
                        })
                        .collect::<Vec<_>>();
                    let mut baseline = None;
                    for plan in [
                        EdgePlan::Baseline,
                        EdgePlan::Unique,
                        EdgePlan::Ordered,
                        EdgePlan::Analyzed,
                        EdgePlan::ParentLinks,
                    ] {
                        let atom = pattern(&g.matcher).unwrap();
                        let AstMatcher::Pattern(atom) = atom else {
                            panic!("pattern")
                        };
                        exec::with_terminals(
                            vec![(doc.clone(), Default::default())],
                            terminals(atom, g.language),
                            |db, batch, _| {
                                crate::code_rule_dsl::sql::relational::leaves::register(db, batch)
                                    .unwrap();
                                let (graph, build) = measured(db, || {
                                    Suffix::batch_plan(db, g.language, &demands, three, plan)
                                });
                                let rows = shape(db);
                                let counts = (rows.nodes.len(), rows.edges.len());
                                if let Some(expected) = &baseline {
                                    assert_eq!(&rows, expected)
                                } else {
                                    baseline = Some(rows)
                                }
                                let eqp = explain(db, &graph.edge_select);
                                let hf_eqp = explain(db, &graph.hf_select);
                                let sql = query();
                                let prepared = std::time::Instant::now();
                                let mut statement = db.prepare(&sql).unwrap();
                                let ids = demands
                                    .iter()
                                    .map(|key| graph.root(db, key))
                                    .collect::<Vec<_>>();
                                let prepare = prepared.elapsed().as_nanos() as u64;
                                let (_, consume) = measured(db, || {
                                    for id in &ids {
                                        let row: (i64, i64, u32) = statement
                                            .query_row(
                                                rusqlite::params![9800000, *id, 0, 10000],
                                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                                            )
                                            .unwrap();
                                        assert_eq!(row.0, 1);
                                    }
                                });
                                reports.push(json!({"plan":plan.name(),"ambientThreeValued":three,"depth":depth,"rootMode":mode,"distinctDemands":demands.len(),"canonicalNodes":counts.0,"canonicalEdges":counts.1,"buildCTASIndexesAndAnalyze":build,"edgeSelectSQL":graph.edge_select,"edgeSelectEQP":eqp,"hfProducerSQL":graph.hf_select,"hfProducerEQP":hf_eqp,"frontierPrepareWallNs":prepare,"frontierConsumption":consume,"totalVmSteps":build["vmSteps"].as_u64().unwrap()+consume["vmSteps"].as_u64().unwrap(),"semanticKey":"utility,host,scope,node,kind,COALESCE(api,0),mode; tag0/null and tag1/Node0 distinct","contract":"all plan rows equal baseline; separate original native finite/mode test covers every plan; all setup including ANALYZE counted; no speed threshold claim"}));
                            },
                        );
                    }
                }
            }
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.edge-plans.json"),
                serde_json::to_vec_pretty(&reports).unwrap(),
            )
            .unwrap();
        }
    }
    #[test]
    fn structural_spike_b615_sticky_failure_keeps_cached_follower_peaks() {
        let mut evidence = vec![];
        for three in [false, true] {
            let isolated = policy(1, None);
            let g = if three {
                original_context(1, None)
            } else {
                grammar_matchers(&isolated)[0].clone()
            };
            let doc = tape(g.language, "(((wrapper.transformers.from_pretrained)))\n");
            let tree = AstGrep::doc(doc.clone());
            let roots = tree
                .root()
                .dfs()
                .filter(|n| matches!(n.kind().as_ref(), "parenthesized_expression" | "attribute"))
                .filter(|n| prove(n.clone()).is_ok())
                .collect::<Vec<_>>();
            let proofs = roots
                .iter()
                .cloned()
                .map(|n| prove(n).unwrap())
                .collect::<Vec<_>>();
            let inputs = proofs
                .iter()
                .map(|p| key(0, 0, p, 0, Cell::Null).unwrap())
                .collect::<Vec<_>>();
            let _scope =
                matcher::FileScopeSlot::install(Default::default(), Default::default(), false);
            let native = matcher::with_file_memo(&tree, || {
                let warm = matcher::match_at_depth(&g, roots[1].clone(), 0, 256);
                assert_eq!(warm.truth, 1);
                matcher::match_at_depth(&g, roots[0].clone(), 0, 4)
            });
            let mut outcomes = vec![];
            for cleanup in [false, true] {
                let atom = pattern(&g.matcher).unwrap();
                let AstMatcher::Pattern(atom) = atom else {
                    panic!("pattern")
                };
                exec::with_terminals(
                    vec![(doc.clone(), Default::default())],
                    terminals(atom, g.language),
                    |db, batch, _| {
                        crate::code_rule_dsl::sql::relational::leaves::register(db, batch).unwrap();
                        let graph = Suffix::batch_mode(db, g.language, &inputs, three);
                        let sql = query_cleanup(cleanup);
                        let inner = graph.root(db, &inputs[1]);
                        let outer = graph.root(db, &inputs[0]);
                        let warm: (i64, i64, u32) = db
                            .query_row(&sql, rusqlite::params![9900000, inner, 0, 256], |r| {
                                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                            })
                            .unwrap();
                        assert_eq!(warm.0, 1);
                        let row: (i64, i64, u32) = db
                            .query_row(&sql, rusqlite::params![9900000, outer, 0, 4], |r| {
                                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                            })
                            .unwrap();
                        assert_eq!((row.0, row.1), (native.truth, native.work as i64));
                        if cleanup || !three {
                            assert_eq!(row.2, native.peak)
                        } else {
                            assert_eq!(
                                (row.2, native.peak),
                                (3, 6),
                                "preserve the demonstrated old abort-all counterexample"
                            );
                        }
                        assert_eq!(
                            db.query_row("SELECT relation_continuation_rows()", [], |r| r
                                .get::<_, i64>(0))
                                .unwrap(),
                            0
                        );
                        outcomes.push(json!({"cachedFollowerCleanup":cleanup,"truth":row.0,"work":row.1,"relativePeak":row.2}));
                    },
                );
            }
            evidence.push(json!({"ambientThreeValued":three,"order":"immediate inner warm256 then cold outer work4","nativeTruth":native.truth,"nativeWork":native.work,"nativePeak":native.peak,"outcomes":outcomes,"contract":"sticky failure follows only cached utility sites; no new body/work/captures/publication"}));
        }
        if let Ok(path) = std::env::var("RERAIL_B615_CAPTURED_RELATION_SPIKE_EVIDENCE") {
            std::fs::write(
                format!("{path}.sticky-failure.json"),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }
}
