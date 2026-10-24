//! Full SIM905 structural reference: ordered numeric effect frontier plus
//! demand-keyed pure cost relations. No production compiler selects this file.
use super::{
    Compiled, emit::bridge, sim905_effect_spike as effects, sim905_pure_cost_spike as pure,
};
use crate::code_rule_dsl::matcher::GrammarMatcher;
use serde_json::json;
/// This experiment uses the fixed source manifest's normalized maxsplit, sep,
/// then receiver ordering. Tests compare ORIGINAL native root/callback traces.
pub(super) fn compile(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    compile_mode(grammars, work, false, false)
}
pub(super) fn compile_final(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    compile_mode(grammars, work, true, false)
}
fn compile_mode(
    grammars: &[&GrammarMatcher],
    work: i64,
    diagnostic: bool,
    fused: bool,
) -> Result<Compiled, String> {
    if grammars.len() != 1 || work < 0 {
        return Err("SIM905 reference needs one grammar and nonnegative allowance".into());
    }
    let g = grammars[0];
    effects::validate_root(g)?;
    fn order(
        m: &crate::code_rule_dsl::matcher::AstMatcher,
        g: &GrammarMatcher,
        out: &mut Vec<String>,
    ) {
        use crate::code_rule_dsl::matcher::AstMatcher;
        match m {
            AstMatcher::All(v) | AstMatcher::Any(v) => {
                for m in v {
                    order(m, g, out)
                }
            }
            AstMatcher::Not(m) => order(m, g, out),
            AstMatcher::Has(r, _)
            | AstMatcher::Inside(r, _)
            | AstMatcher::Follows(r, _)
            | AstMatcher::Precedes(r, _) => order(r.rule(), g, out),
            AstMatcher::Util(u) => {
                let name = g.utils.names[u.index].clone();
                if out.last() != Some(&name) {
                    out.push(name);
                }
            }
            _ => {}
        }
    }
    let mut normalized = vec![];
    order(&g.matcher, g, &mut normalized);
    if normalized != vec!["maxsplit", "sep", "receiver"] {
        return Err(format!(
            "SIM905 normalized effect order outside contract: {normalized:?}"
        ));
    }
    let reference = super::sim905_relation_spike::compile(g)?;
    // Reuse only structural metadata preceding the speculative integer relation.
    let prefix = reference
        .sql
        .split("\nroots AS MATERIALIZED")
        .next()
        .ok_or("SIM905 structural metadata missing")?
        .trim_end();
    if !prefix.ends_with(',') {
        return Err("SIM905 structural metadata boundary changed".into());
    }
    let mut terminals = reference.terminals;
    let equals=terminals.iter().position(|t|matches!(t,crate::code_rule_dsl::sql::terminals::Terminal::Regex(r) if r.as_str()=="^=$")).ok_or("equals terminal missing")?;
    let keyword = g.language.kinds().kind_id("keyword_argument");
    let splat = g.language.kinds().kind_id("list_splat");
    let dictionary = g.language.kinds().kind_id("dictionary_splat");
    let candidates = format!(
        r#"keyword_groups AS MATERIALIZED(SELECT a.file,a.arguments,a.id keyword,EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=a.file AND k.keyword=a.id AND k.role=1) sep_name,EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=a.file AND k.keyword=a.id AND k.role=2) max_name FROM argument_nodes a WHERE a.kind={keyword}),
helper_candidates AS MATERIALIZED(SELECT k.file,k.arguments,1 group_type,k.keyword group_id,c.id node,EXISTS(SELECT 1 FROM node eq WHERE eq.file=c.file AND eq.id=c.prev AND node_regex({equals},eq.file,eq.id)) valid_max,EXISTS(SELECT 1 FROM node eq WHERE eq.file=c.file AND eq.id=c.prev AND node_regex({equals},eq.file,eq.id)) valid_sep FROM keyword_groups k JOIN node c ON c.file=k.file AND c.parent=k.keyword
UNION ALL SELECT p.file,p.arguments,2,p.arguments,p.id,(NOT p.punct AND p.kind NOT IN({keyword},{splat},{dictionary}) AND p.previous_comma AND NOT p.previous_previous_excluded AND p.third_previous_open),(NOT p.punct AND p.kind NOT IN({keyword},{splat},{dictionary}) AND p.first_position) FROM positions p
UNION ALL SELECT a.file,a.arguments,2,a.arguments,a.id,0,0 FROM argument_nodes a WHERE NOT EXISTS(SELECT 1 FROM positions p WHERE p.file=a.file AND p.id=a.id))"#
    );
    let metadata_sql = format!(
        r#"CREATE TEMP TABLE spike_sim905_meta AS {prefix}{candidates}
SELECT file,NULL call,arguments,1 kind,group_type,group_id,node,valid_max,valid_sep,NULL sep_name,NULL max_name,NULL attribute,NULL receiver_guard,NULL method_guard FROM helper_candidates
UNION ALL SELECT file,NULL,arguments,2,NULL,keyword,NULL,NULL,NULL,sep_name,max_name,NULL,NULL,NULL FROM keyword_groups
UNION ALL SELECT a.file,a.call,NULL,3,NULL,NULL,c.id,NULL,NULL,NULL,NULL,a.id,EXISTS(SELECT 1 FROM receiver_roots r WHERE r.file=a.file AND r.attribute=a.id AND r.node=c.id),EXISTS(SELECT 1 FROM method_calls m WHERE m.file=a.file AND m.attribute=a.id) FROM attributes a LEFT JOIN attribute_children c ON c.file=a.file AND c.attribute=a.id
UNION ALL SELECT c.file,c.id,(SELECT a.id FROM arguments a WHERE a.file=c.file AND a.call=c.id ORDER BY a.id LIMIT 1),4,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL FROM calls c"#
    );
    let stored = r#"helper_candidates AS NOT MATERIALIZED(SELECT file,arguments,group_type,group_id,node,valid_max,valid_sep FROM spike_sim905_meta WHERE kind=1),keyword_groups AS NOT MATERIALIZED(SELECT file,arguments,group_id keyword,sep_name,max_name FROM spike_sim905_meta WHERE kind=2),keyword_names AS NOT MATERIALIZED(SELECT file,arguments,keyword,1 role FROM keyword_groups WHERE sep_name UNION ALL SELECT file,arguments,keyword,2 FROM keyword_groups WHERE max_name),arguments AS MATERIALIZED(SELECT file,call,arguments id FROM spike_sim905_meta WHERE kind=4 AND arguments IS NOT NULL),attributes AS MATERIALIZED(SELECT DISTINCT file,call,attribute id FROM spike_sim905_meta WHERE kind=3),attribute_children AS MATERIALIZED(SELECT file,call,attribute,node id FROM spike_sim905_meta WHERE kind=3 AND node IS NOT NULL),receiver_roots AS MATERIALIZED(SELECT file,attribute,node FROM spike_sim905_meta WHERE kind=3 AND receiver_guard),method_calls AS MATERIALIZED(SELECT DISTINCT file,call,attribute FROM spike_sim905_meta WHERE kind=3 AND method_guard)"#;
    let receiver = pure::compile(
        g,
        "receiver",
        if fused {
            "SELECT DISTINCT file,id node FROM attribute_children"
        } else {
            "SELECT DISTINCT file,node FROM spike_sim905_meta WHERE kind=3 AND node IS NOT NULL"
        },
        0,
    )?;
    let receiver_defs = receiver.relations["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            pure::offset(
                d.as_str().unwrap(),
                terminals.len(),
                receiver.terminals.len(),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let receiver_sql = pure::offset(
        receiver.relations["select"].as_str().unwrap(),
        terminals.len(),
        receiver.terminals.len(),
    );
    terminals.extend(receiver.terminals);
    let sep = pure::compile(
        g,
        "sep",
        if fused {
            "SELECT DISTINCT file,node FROM helper_candidates"
        } else {
            "SELECT DISTINCT file,node FROM spike_sim905_meta WHERE kind=1"
        },
        0,
    )?;
    let sep_defs = sep.relations["definitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| pure::offset(d.as_str().unwrap(), terminals.len(), sep.terminals.len()))
        .collect::<Vec<_>>()
        .join(",");
    let sep_sql = pure::offset(
        sep.relations["select"].as_str().unwrap(),
        terminals.len(),
        sep.terminals.len(),
    );
    terminals.extend(sep.terminals);
    let frontier = effects::frontier(g)?;
    let mut effect_query = pure::offset(&frontier.query, terminals.len(), frontier.terminals.len());
    effect_query = effect_query.replace(
        "relation_value_budget(0,",
        &format!("relation_value_budget({},", terminals.len()),
    );
    terminals.extend(frontier.terminals);
    let seed = "SELECT @node,@helper,@level,@work,NULL,0,0,NULL,0";
    if !effect_query.contains(seed) {
        return Err("effect frontier seed contract changed".into());
    }
    // Pure false maxsplit calls cannot reach Value or fill a cache. At root
    // utility level0 they have stable cost3, complete peak2; batch their gaps.
    // A work failure ends the root, so no enclosing utility can publish a
    // partial failed pure trace's peak. Standalone effect-frontier tests remain
    // unchanged and continue to compare precise cold/warm failure peaks.
    let run = "candidate.potential AND a.work>=3*candidate.gap";
    effect_query=effect_query.replace(seed,&format!("SELECT @node,@helper,@level,@work,NULL,CASE WHEN {run} THEN 0 WHEN a.work<3*candidate.gap THEN -2 ELSE 0 END,CASE WHEN {run} THEN 0 ELSE 3 END,NULL,0"));
    let effect_query = effects::bind(
        &effect_query,
        "d.file",
        "candidate.node",
        "3",
        "0",
        "MAX(0,a.work-3*candidate.gap)",
    );
    let integer = g.language.kinds().kind_id("integer");
    let unary = g.language.kinds().kind_id("unary_operator");
    let paren = g.language.kinds().kind_id("parenthesized_expression");
    // Helper runs BEFORE post-call follows/position guards in normalized AST.
    // Windowed targets retain each Value-capable candidate and one trailing
    // false gap. Consecutive pure false calls need no per-candidate return.
    let helper_search = format!(
        r#"WITH RECURSIVE candidates AS MATERIALIZED(SELECT s.node,s.valid_max,(n.kind IN({integer},{unary},{paren})) potential,ROW_NUMBER() OVER(ORDER BY s.node) ordinal FROM helper_candidates s JOIN node n ON n.file=s.file AND n.id=s.node WHERE d.needed=1 AND s.file=d.file AND s.arguments=d.arguments AND s.group_type=d.group_type AND s.group_id=d.group_id),
numeric AS MATERIALIZED(SELECT node,valid_max,ordinal,LAG(ordinal,1,0) OVER(ORDER BY ordinal) prior FROM candidates WHERE potential),
targets AS MATERIALIZED(SELECT node,valid_max,ordinal,prior,ordinal-prior-1 gap,1 potential FROM numeric UNION ALL SELECT NULL,0,COUNT(*)+1,COALESCE(MAX(CASE WHEN potential THEN ordinal ELSE 0 END),0),COUNT(*)-COALESCE(MAX(CASE WHEN potential THEN ordinal ELSE 0 END),0),0 FROM candidates),
answers(ordinal,truth,uncertain,work) AS(SELECT 0,0,0,d.work UNION ALL SELECT candidate.ordinal,CASE WHEN r.c0<0 THEN r.c0 WHEN candidate.valid_max THEN r.c0 ELSE 0 END,(a.uncertain OR (r.c0=2 AND candidate.valid_max)),r.c1 FROM answers a JOIN targets candidate ON candidate.prior=a.ordinal {} WHERE a.truth IN(0,2)),
last AS MATERIALIZED(SELECT * FROM answers ORDER BY ordinal DESC LIMIT 1)
SELECT CASE WHEN a.truth<0 OR a.truth=1 THEN a.truth WHEN a.uncertain THEN 2 ELSE 0 END truth,a.work FROM last a"#,
        bridge(&effect_query, 5)
    );
    let keyword_search = helper_search
        .replace("d.group_type", "1")
        .replace("d.group_id", "k.keyword")
        .replace("d.needed", "1")
        .replace("d.work", "a.work");
    let positional_search = helper_search
        .replace("d.group_type", "2")
        .replace("d.group_id", "d.arguments")
        .replace("d.needed", "(k.truth IN(0,2))")
        .replace("d.work", "k.work");
    let keyword_query = format!(
        r#"WITH RECURSIVE groups AS MATERIALIZED(SELECT *,ROW_NUMBER() OVER(ORDER BY keyword) ordinal FROM keyword_groups WHERE file=d.file AND arguments=d.arguments),
answers(ordinal,truth,uncertain,work) AS(SELECT 0,0,0,d.work UNION ALL SELECT k.ordinal,CASE WHEN r.c0<0 THEN r.c0 WHEN k.max_name THEN r.c0 ELSE 0 END,(a.uncertain OR (r.c0=2 AND k.max_name)),r.c1 FROM answers a JOIN groups k ON k.ordinal=a.ordinal+1 {} WHERE a.truth IN(0,2)),last AS MATERIALIZED(SELECT * FROM answers ORDER BY ordinal DESC LIMIT 1)
SELECT CASE WHEN a.truth<0 OR a.truth=1 THEN a.truth WHEN a.uncertain THEN 2 ELSE 0 END truth,a.work FROM last a"#,
        bridge(&keyword_search, 2)
    );
    let max_query = format!(
        r#"WITH keyword AS MATERIALIZED(SELECT r.c0 truth,r.c1 work FROM (SELECT 1) seed {}),
positional AS MATERIALIZED(SELECT k.truth prior,r.c0 truth,r.c1 work FROM keyword k {}),
finished AS MATERIALIZED(SELECT p.*,CASE WHEN p.truth<0 OR p.truth=1 THEN p.truth WHEN NOT EXISTS(SELECT 1 FROM helper_candidates h WHERE h.file=d.file AND h.arguments=d.arguments AND h.group_type=2 AND h.valid_max) THEN 1 ELSE p.truth END positional_truth FROM positional p)
SELECT CASE WHEN p.prior<0 OR p.prior=1 THEN p.prior WHEN p.positional_truth<0 THEN p.positional_truth WHEN EXISTS(SELECT 1 FROM keyword_names n WHERE n.file=d.file AND n.arguments=d.arguments AND n.role=2) THEN p.prior WHEN p.positional_truth=1 THEN p.positional_truth WHEN p.prior=2 OR p.positional_truth=2 THEN 2 ELSE 0 END,p.work FROM finished p"#,
        bridge(&keyword_query, 2),
        bridge(&positional_search, 2)
    );
    // Static pure sep uses the SAME nested candidate boundaries, but its
    // reached-prefix truth/cost can be computed in one relational batch.
    let receiver_source = if fused {
        receiver_sql.clone()
    } else {
        "SELECT * FROM spike_sim905_receiver".into()
    };
    let sep_source = if fused {
        sep_sql.clone()
    } else {
        "SELECT * FROM spike_sim905_sep".into()
    };
    let profile_definitions = format!(
        r#"receiver_summary AS MATERIALIZED({receiver_source}),sep_summary AS MATERIALIZED({sep_source}),
sep_children AS MATERIALIZED(SELECT c.*,CASE WHEN p.truth<0 THEN p.truth WHEN c.valid_sep THEN p.truth ELSE 0 END truth,p.cost,p.peak FROM helper_candidates c JOIN sep_summary p ON p.file=c.file AND p.node=c.node),
sep_child_order AS MATERIALIZED(SELECT *,COALESCE(SUM(truth=1 OR truth<0) OVER(PARTITION BY file,arguments,group_type,group_id ORDER BY node ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM sep_children),
sep_search AS MATERIALIZED(SELECT file,arguments,group_type,group_id,CASE WHEN MIN(truth)<0 THEN MIN(truth) WHEN MAX(truth=1) THEN 1 ELSE 0 END truth,SUM(cost) cost,MAX(peak) peak FROM sep_child_order WHERE blocked=0 GROUP BY file,arguments,group_type,group_id),
sep_keywords AS MATERIALIZED(SELECT k.*,CASE WHEN s.truth<0 THEN s.truth WHEN k.sep_name THEN COALESCE(s.truth,0) ELSE 0 END truth,COALESCE(s.cost,0) cost,COALESCE(s.peak,0) peak FROM keyword_groups k LEFT JOIN sep_search s ON s.file=k.file AND s.arguments=k.arguments AND s.group_type=1 AND s.group_id=k.keyword),
sep_keyword_order AS MATERIALIZED(SELECT *,COALESCE(SUM(truth=1 OR truth<0) OVER(PARTITION BY file,arguments ORDER BY keyword ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM sep_keywords),
sep_keyword_answer AS MATERIALIZED(SELECT file,arguments,CASE WHEN MIN(truth)<0 THEN MIN(truth) WHEN MAX(truth=1) THEN 1 ELSE 0 END truth,SUM(cost) cost,MAX(peak) peak FROM sep_keyword_order WHERE blocked=0 GROUP BY file,arguments),
sep_answers AS MATERIALIZED(SELECT a.file,a.id arguments,CASE WHEN k.truth<0 OR k.truth=1 THEN k.truth WHEN p.truth<0 THEN p.truth WHEN EXISTS(SELECT 1 FROM keyword_names n WHERE n.file=a.file AND n.arguments=a.id AND n.role=1) THEN 0 WHEN p.truth=1 THEN p.truth WHEN NOT EXISTS(SELECT 1 FROM helper_candidates h WHERE h.file=a.file AND h.arguments=a.id AND h.group_type=2 AND h.valid_sep) THEN 1 ELSE 0 END truth,
COALESCE(k.cost,0)+CASE WHEN COALESCE(k.truth,0)=0 THEN COALESCE(p.cost,0) ELSE 0 END cost,MAX(COALESCE(k.peak,0),CASE WHEN COALESCE(k.truth,0)=0 THEN COALESCE(p.peak,0) ELSE 0 END) peak FROM arguments a LEFT JOIN sep_keyword_answer k ON k.file=a.file AND k.arguments=a.id LEFT JOIN sep_search p ON p.file=a.file AND p.arguments=a.id AND p.group_type=2),
receiver_candidates AS MATERIALIZED(SELECT a.file,a.attribute,a.id node,CASE WHEN p.truth<0 THEN p.truth WHEN p.truth=1 AND EXISTS(SELECT 1 FROM receiver_roots rr WHERE rr.file=a.file AND rr.attribute=a.attribute AND rr.node=a.id) THEN 1 ELSE 0 END truth,p.cost,p.peak FROM attribute_children a JOIN receiver_summary p ON p.file=a.file AND p.node=a.id),
receiver_ordered AS MATERIALIZED(SELECT *,COALESCE(SUM(truth=1 OR truth<0) OVER(PARTITION BY file,attribute ORDER BY node ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM receiver_candidates),
attribute_answers AS MATERIALIZED(SELECT a.file,a.call,a.id attribute,CASE WHEN MIN(r.truth)<0 THEN MIN(r.truth) WHEN MAX(r.truth=1) AND EXISTS(SELECT 1 FROM method_calls m WHERE m.file=a.file AND m.attribute=a.id) THEN 1 ELSE 0 END truth,COALESCE(SUM(r.cost),0) cost,COALESCE(MAX(r.peak),0) peak FROM attributes a LEFT JOIN receiver_ordered r ON r.file=a.file AND r.attribute=a.id AND r.blocked=0 GROUP BY a.file,a.call,a.id),
attribute_ordered AS MATERIALIZED(SELECT *,COALESCE(SUM(truth=1 OR truth<0) OVER(PARTITION BY file,call ORDER BY attribute ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) blocked FROM attribute_answers),
attribute_results AS MATERIALIZED(SELECT file,call,CASE WHEN MIN(truth)<0 THEN MIN(truth) WHEN MAX(truth=1) THEN 1 ELSE 0 END truth,SUM(cost) cost,MAX(peak) peak FROM attribute_ordered WHERE blocked=0 GROUP BY file,call),
profile_rows AS MATERIALIZED(SELECT file,arguments key,1 phase,truth,cost,peak FROM sep_answers UNION ALL SELECT file,call,2,truth,cost,peak FROM attribute_results)"#
    );
    let profile_sql = format!(
        "CREATE TEMP TABLE spike_sim905_profiles AS WITH RECURSIVE {stored},{profile_definitions} SELECT * FROM profile_rows"
    );
    let receiver_stage = format!(
        "CREATE TEMP TABLE spike_sim905_receiver AS WITH RECURSIVE {receiver_defs} {receiver_sql}"
    );
    let sep_stage =
        format!("CREATE TEMP TABLE spike_sim905_sep AS WITH RECURSIVE {sep_defs} {sep_sql}");
    let stages=vec![metadata_sql,
        "CREATE INDEX spike_sim905_meta_arg ON spike_sim905_meta(kind,file,arguments,group_type,group_id,node)".into(),
        "CREATE INDEX spike_sim905_meta_keyword ON spike_sim905_meta(kind,file,arguments,group_id)".into(),
        "CREATE INDEX spike_sim905_meta_attr ON spike_sim905_meta(kind,file,attribute,node)".into(),
        "CREATE INDEX spike_sim905_meta_call ON spike_sim905_meta(kind,file,call)".into(),
        receiver_stage,"CREATE INDEX spike_sim905_receiver_key ON spike_sim905_receiver(file,node)".into(),
        sep_stage,"CREATE INDEX spike_sim905_sep_key ON spike_sim905_sep(file,node)".into(),
        profile_sql,"CREATE INDEX spike_sim905_profile_key ON spike_sim905_profiles(file,phase,key)".into()];
    if !fused && let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
        for (i, stage) in stages.iter().enumerate() {
            std::fs::write(format!("{path}.stage-{i}.sql"), stage).unwrap();
        }
    }
    let root_definition = if fused {
        "root_demand AS MATERIALIZED(SELECT c.file,c.id node,(SELECT a.id FROM arguments a WHERE a.file=c.file AND a.call=c.id ORDER BY a.id LIMIT 1) arguments,ROW_NUMBER() OVER(PARTITION BY c.file ORDER BY c.id) ordinal FROM calls c CROSS JOIN init WHERE init.ready=1)"
    } else {
        "root_demand AS MATERIALIZED(SELECT m.file,m.call node,m.arguments,ROW_NUMBER() OVER(PARTITION BY m.file ORDER BY m.call) ordinal FROM spike_sim905_meta m CROSS JOIN init WHERE m.kind=4 AND init.ready=1)"
    };
    let final_definitions =
        format!("init AS MATERIALIZED(SELECT relation_memo_reset(0) ready),{root_definition}");
    let definitions = if fused {
        // Single owner for every metadata/helper definition. No nested WITH,
        // duplicate stored-view aliases, temporary tables, or index statements.
        format!(
            "{prefix}{candidates},{receiver_defs},{sep_defs},{profile_definitions},{final_definitions}"
        )
    } else {
        format!(
            "WITH RECURSIVE {stored},sep_answers AS NOT MATERIALIZED(SELECT file,key arguments,truth,cost,peak FROM spike_sim905_profiles WHERE phase=1),attribute_results AS NOT MATERIALIZED(SELECT file,key call,truth,cost,peak FROM spike_sim905_profiles WHERE phase=2),{final_definitions}"
        )
    };
    let root_query = format!(
        r#"WITH max_reached AS MATERIALIZED(SELECT r.c0 truth,r.c1 work FROM (SELECT 1) seed {}),
sep_reached AS MATERIALIZED(SELECT m.truth prior,m.work before,CASE WHEN m.truth<0 OR m.truth=0 THEN m.truth WHEN d.arguments IS NULL THEN 0 WHEN COALESCE(s.cost,0)>m.work THEN -2 WHEN s.arguments IS NOT NULL THEN s.truth WHEN EXISTS(SELECT 1 FROM keyword_names k WHERE k.file=d.file AND k.arguments=d.arguments AND k.role=1) THEN 0 ELSE 1 END own,CASE WHEN m.truth<0 OR m.truth=0 THEN m.work ELSE MAX(0,m.work-COALESCE(s.cost,0)) END work FROM max_reached m LEFT JOIN sep_answers s ON s.file=d.file AND s.arguments=d.arguments),
args AS MATERIALIZED(SELECT CASE WHEN prior<0 OR own<0 THEN MIN(prior,own) WHEN prior=0 OR own=0 THEN 0 WHEN prior=2 OR own=2 THEN 2 ELSE 1 END truth,work FROM sep_reached),
final AS MATERIALIZED(SELECT CASE WHEN a.truth<0 OR a.truth=0 THEN a.truth WHEN COALESCE(p.cost,0)>a.work THEN -2 WHEN COALESCE(p.truth,0)<=0 THEN COALESCE(p.truth,0) WHEN a.truth=2 THEN 2 ELSE 1 END truth,CASE WHEN a.truth<0 OR a.truth=0 THEN a.work ELSE MAX(0,a.work-COALESCE(p.cost,0)) END work FROM args a LEFT JOIN attribute_results p ON p.file=d.file AND p.call=d.node) SELECT truth,work FROM final"#,
        bridge(&max_query.replace("d.work", "w.work"), 2)
    );
    let initial_files = if diagnostic {
        "SELECT n.file FROM node n CROSS JOIN init WHERE init.ready=1 GROUP BY n.file"
    } else {
        "SELECT file FROM root_demand GROUP BY file"
    };
    let walk = format!(
        r#",walk(file,node,ordinal,truth,work) AS(SELECT file,NULL,0,0,{work} FROM ({initial_files}) UNION ALL SELECT d.file,d.node,d.ordinal,r.c0,r.c1 FROM walk w JOIN root_demand d ON d.file=w.file AND d.ordinal=w.ordinal+1 {} WHERE w.truth>=0),failed AS MATERIALIZED(SELECT * FROM walk WHERE truth<0)"#,
        bridge(&root_query, 2)
    );
    let output = if diagnostic {
        "SELECT w.file,w.truth,w.work FROM walk w WHERE w.ordinal=(SELECT MAX(t.ordinal) FROM walk t WHERE t.file=w.file)"
    } else {
        "SELECT 0,w.file,w.node,'[]',0,w.work FROM walk w WHERE w.truth=1 AND NOT EXISTS(SELECT 1 FROM failed f WHERE f.file=w.file) UNION ALL SELECT 0,file,NULL,'[]',truth,work FROM failed ORDER BY 1,2,3"
    };
    let sql = format!("{definitions}{walk} {output}");
    if let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
        std::fs::write(format!("{path}.sql"), &sql).unwrap();
    }
    let count = terminals.len();
    let mut compiled = Compiled {
        sql,
        terminals,
        relations: json!({"terminalOwners":vec![0;count],"pureHelperLiteralProjection":{"receiver":receiver.relations["literalProjection"],"sep":sep.relations["literalProjection"]},"pureHelperMetadataStrata":{"receiver":receiver.relations["metadataStrata"],"sep":sep.relations["metadataStrata"]},"spikeStageOutputs":["spike_sim905_meta",null,null,null,null,"spike_sim905_receiver",null,"spike_sim905_sep",null,"spike_sim905_profiles",null],"spikeStagePurposes":["pure structural candidate metadata","candidate lookup index","keyword lookup index","attribute lookup index","root lookup index","receiver truth/cost/peak summaries","receiver lookup index","sep truth/cost/peak summaries","sep lookup index","ordered pure segment profiles","profile lookup index"],"spikeStageSqlBytes":stages.iter().map(String::len).collect::<Vec<_>>(),"spikeStages":stages,"captures":[],"failureColumn":4,"scope":"test-only full SIM905 result/cost/effect reference","contract":{"pureBooleanPCs":0,"pureFalseGapCost":3,"pureFalseGapDepth":0,"pureContinuationCells":0,"numericFrontier":"native utility memo/work/peak and reached Value order","rootOrder":normalized,"finalRemainingWorkDiagnostic":diagnostic},"limitations":["single explicit source manifest","single argument_list per call","pure acyclic bases restricted to demanded structural closure; boundary variants use requested levels","relational DFS path is temporary ordering storage","native physical stack-byte equivalence not qualified"]}),
    };
    if fused {
        let object = compiled.relations.as_object_mut().unwrap();
        for key in [
            "spikeStages",
            "spikeStageOutputs",
            "spikeStagePurposes",
            "spikeStageSqlBytes",
        ] {
            object.remove(key);
        }
        object.insert("physicalControl".into(),json!({"shape":"single owned flat metadata/helper graph","matchingStatements":1,"explicitIndexes":0}));
        if let Ok(path) = std::env::var("RERAIL_SIM905_FULL_SPIKE_EVIDENCE") {
            std::fs::write(format!("{path}.fused.sql"), &compiled.sql).unwrap();
        }
    }
    Ok(compiled)
}
#[path = "sim905_full_spike_tests.rs"]
mod tests;

/// Physical control with explicit graph ownership, not SQL parsing/rewriting.
pub(super) fn compile_fused(grammars: &[&GrammarMatcher], work: i64) -> Result<Compiled, String> {
    compile_mode(grammars, work, false, true)
}
fn compile_fused_mode(
    grammars: &[&GrammarMatcher],
    work: i64,
    diagnostic: bool,
) -> Result<Compiled, String> {
    compile_mode(grammars, work, diagnostic, true)
}
