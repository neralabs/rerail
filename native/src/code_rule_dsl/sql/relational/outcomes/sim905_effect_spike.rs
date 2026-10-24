//! Utility-only numeric effect frontier for the SIM905 structural experiment.
//! Pure Boolean composition has no program counter here. Frames represent only
//! a reached memo transaction, a utility-bearing child search, or a fallback.
#[path = "sim905_source_spike.rs"]
mod source;
use super::{Compiled, Op, emit::bridge, lower::Lower};
use crate::code_rule_dsl::{
    matcher::{AstMatcher, GrammarMatcher, Semantic},
    sql::terminals::Terminal,
};
use serde_json::json;
pub(super) use source::validate_root;
use std::collections::HashSet;
pub(super) struct Frontier {
    pub query: String,
    pub terminals: Vec<Terminal>,
}
fn atom(m: &AstMatcher, seen: &mut HashSet<(usize, usize)>) -> Option<AstMatcher> {
    match m {
        AstMatcher::Semantic(Semantic::Value(_)) => Some(m.clone()),
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().find_map(|m| atom(m, seen)),
        AstMatcher::Util(u) => {
            let k = (u.table.as_ptr() as usize, u.index);
            if !seen.insert(k) {
                return None;
            }
            u.with_target(|m| atom(m, seen)).flatten()
        }
        AstMatcher::Has(r, _) => atom(r.rule(), seen),
        AstMatcher::Not(m) => atom(m, seen),
        _ => None,
    }
}
fn push(alias: &str, parent: &str, guard: &str, cells: &[&str]) -> String {
    let supplied = cells
        .iter()
        .enumerate()
        .map(|(i, v)| format!(" AND {alias}.v{i} IS ({v})"))
        .collect::<String>();
    format!(
        "LEFT JOIN relation_continuation {alias} ON {alias}.mode IS CASE WHEN {guard} THEN 1 END AND {alias}.parent_input IS {parent}{supplied}"
    )
}
/// The caller substitutes @file/@node/@helper/@level/@work with correlated
/// numeric SQL expressions. Helpers: 1=int32, 2=neg-int32, 3=maxsplit.
/// Serialize demands in native order, sharing one clause/file memo lifetime.
/// Output: truth, remainingWork, executionCost, replayCost, relativePeak.
/// The helper graph is the explicit experiment contract, not a production
/// grammar recognizer. The original atomic Value terminal is reused unchanged.
pub(super) fn frontier(grammar: &GrammarMatcher) -> Result<Frontier, String> {
    source::validate_numeric(grammar)?;

    if !grammar.three_valued {
        return Err("numeric frontier requires native three-valued helper context".into());
    }
    let value = atom(&grammar.matcher, &mut HashSet::new())
        .ok_or("numeric frontier needs original Value atom")?;
    let mut lower = Lower::new(grammar.language, true);
    let Op::Value(term) = lower.plan(&value)?.op else {
        return Err("numeric frontier needs capture-free Value".into());
    };
    let mut terminals = lower.pure.program.terminals;
    let minus = terminals.len();
    terminals.push(Terminal::Regex(regex::Regex::new("^-$").unwrap()));
    let integer = grammar.language.kinds().kind_id("integer");
    let paren = grammar.language.kinds().kind_id("parenthesized_expression");
    let unary = grammar.language.kinds().kind_id("unary_operator");
    let entrypush = push(
        "frame",
        "s.handle",
        "s.level<256 AND s.cached IS NULL",
        &[
            "0", "s.helper", "s.node", "s.level", "s.work", "s.fills", "s.token",
        ],
    );
    let entry = format!(
        r#"WITH seed AS MATERIALIZED (SELECT w.node,w.helper,w.level,w.work,w.handle,w.truth,w.phase,w.replay,w.peak WHERE w.phase=0),
found AS MATERIALIZED (SELECT s.*,m.truth cached,m.cost,m.peak cached_peak FROM seed s LEFT JOIN relation_memo m ON m.mode IS CASE WHEN s.level<256 THEN 0 END AND m.clause IS 0 AND m.utility IS (6000000+s.helper) AND m.file IS @file AND m.node IS s.node AND (s.level+m.peak)<=256),
opened AS MATERIALIZED (SELECT s.*,CASE WHEN s.level<256 AND s.cached IS NULL THEN relation_peak_open(s.level) END token,CASE WHEN s.level<256 AND s.cached IS NULL THEN relation_cache_fill_steps() END fills FROM found s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.level<256 THEN relation_peak_note(s.level+COALESCE(s.cached_peak,1)) ELSE 0 END noted FROM opened s)
SELECT s.node,s.helper,CASE WHEN s.level<256 AND s.cached IS NULL THEN s.level+1 ELSE s.level END,
CASE WHEN s.level>=256 THEN s.work WHEN s.cached IS NOT NULL THEN MAX(0,s.work-s.cost) ELSE MAX(0,s.work-1) END,
COALESCE(frame.handle,s.handle),CASE WHEN s.level>=256 THEN -1 WHEN s.cached IS NOT NULL THEN CASE WHEN s.work<s.cost THEN -2 ELSE s.cached END WHEN s.work=0 THEN -2 ELSE 0 END,
CASE WHEN s.level<256 AND s.cached IS NULL AND s.work>0 THEN 1 ELSE 2 END,
CASE WHEN s.cached IS NOT NULL THEN s.cost END,COALESCE(s.cached_peak,0) FROM noted s {entrypush} WHERE s.noted>=0"#
    );
    let bodypush = push(
        "frame",
        "s.handle",
        "s.action=3 OR (s.action=2 AND s.first IS NOT NULL)",
        &[
            "CASE WHEN s.action=3 THEN 2 ELSE 1 END",
            "CASE WHEN s.action=3 THEN s.node ELSE s.child_helper END",
            "CASE WHEN s.action=3 THEN s.level ELSE s.node END",
            "s.level",
            "s.first",
            "0",
        ],
    );
    let body = format!(
        r#"WITH seed AS MATERIALIZED (SELECT w.node,w.helper,w.level,w.work,w.handle,w.truth,w.phase,w.replay,w.peak WHERE w.phase=1),
selected AS MATERIALIZED (SELECT s.*,CASE WHEN s.helper=3 THEN 3 WHEN s.helper=1 AND n.kind={integer} THEN 1 WHEN n.kind={paren} OR (s.helper=2 AND n.kind={unary} AND EXISTS(SELECT 1 FROM node minus WHERE minus.file=@file AND minus.parent=s.node AND node_regex({minus},@file,minus.id))) THEN 2 ELSE 0 END action,CASE WHEN s.helper=2 AND n.kind={unary} THEN 1 ELSE s.helper END child_helper FROM seed s JOIN node n ON n.file=@file AND n.id=s.node),
picked AS MATERIALIZED (SELECT s.*,CASE WHEN s.action=2 THEN (SELECT c.id FROM node c WHERE c.file=@file AND c.parent=s.node ORDER BY c.id LIMIT 1) END first FROM selected s)
SELECT CASE WHEN s.action=3 THEN s.node WHEN s.action=2 AND s.first IS NOT NULL THEN s.first ELSE s.node END,
CASE WHEN s.action=3 THEN 1 ELSE s.child_helper END,s.level,COALESCE(v.work,s.work),COALESCE(frame.handle,s.handle),
CASE WHEN s.action=1 THEN v.truth ELSE 0 END,
CASE WHEN s.action=3 OR (s.action=2 AND s.first IS NOT NULL) THEN 0 ELSE 2 END,s.replay,s.peak FROM picked s LEFT JOIN relation_value_budget({term},@file,s.node,CASE WHEN s.action=1 THEN s.work END) v {bodypush}"#
    );
    let returnpush = push(
        "again",
        "s.parent",
        "s.more",
        &[
            "CASE WHEN s.c0=1 THEN 1 ELSE 3 END",
            "CASE WHEN s.c0=1 THEN s.c1 ELSE (s.truth=2) END",
            "s.c2",
            "s.c3",
            "s.next",
            "CASE WHEN s.c0=1 THEN (s.c5 OR s.truth=2) ELSE 0 END",
        ],
    );
    // Native Any/search still recalls later siblings after a sticky utility
    // failure. Each admitted numeric utility has positive replay cost, so a
    // fitting hit can only raise the peak: spend rejects before verdict/body.
    // Read those reached completed answers without charges or publication.
    let returned = format!(
        r#"WITH seed AS MATERIALIZED (SELECT w.node,w.helper,w.level,w.work,w.handle,w.truth,w.phase,w.replay,w.peak WHERE w.phase=2 AND w.handle IS NOT NULL),
popped AS MATERIALIZED (SELECT s.*,p.parent,p.c0,p.c1,p.c2,p.c3,p.c4,p.c5,p.c6 FROM seed s JOIN relation_continuation p ON p.mode IS CASE WHEN s.phase=2 AND s.handle IS NOT NULL THEN 2 END AND p.h IS s.handle),
closed AS MATERIALIZED (SELECT s.*,CASE WHEN s.c0=0 THEN relation_peak_close(s.c6) ELSE 0 END measured FROM popped s),
advancing AS MATERIALIZED (SELECT s.*,CASE WHEN s.c0=1 AND s.truth IN(0,2) THEN (SELECT c.id FROM node n JOIN node c ON c.file=n.file AND c.parent=s.c2 AND c.id>n."last" WHERE n.file=@file AND n.id=s.c4 ORDER BY c.id LIMIT 1) END next FROM closed s),
routed AS MATERIALIZED (SELECT s.*,((s.c0=1 AND s.next IS NOT NULL) OR (s.c0=2 AND s.truth IN(0,2))) more FROM advancing s)
SELECT CASE s.c0 WHEN 0 THEN s.c2 WHEN 1 THEN COALESCE(s.next,s.c2) WHEN 2 THEN s.c1 ELSE s.node END,
CASE WHEN s.c0=1 THEN s.c1 WHEN s.c0=2 AND s.more THEN 2 ELSE s.helper END,
CASE s.c0 WHEN 0 THEN s.c3 WHEN 1 THEN s.c3 WHEN 2 THEN s.c2 ELSE s.level END,s.work,COALESCE(again.handle,s.parent),
CASE WHEN s.more THEN 0 WHEN s.c0=1 AND s.truth IN(0,2) THEN CASE WHEN s.c5 OR s.truth=2 THEN 2 ELSE 0 END WHEN s.c0=3 AND s.truth=0 AND s.c1 THEN 2 ELSE COALESCE(m.truth,s.truth) END,
CASE WHEN s.more THEN 0 ELSE 2 END,
CASE WHEN s.c0=0 AND s.truth>=0 THEN MAX(0,s.c4-s.work-relation_cache_fill_delta(s.c5)) ELSE s.replay END,
CASE WHEN s.c0=0 THEN s.measured ELSE s.peak END
FROM routed s {returnpush} LEFT JOIN relation_memo m ON m.mode IS CASE WHEN s.c0=0 AND s.truth>=0 THEN 1 END AND m.clause IS 0 AND m.utility IS (6000000+s.c1) AND m.file IS @file AND m.node IS s.c2 AND m.vtruth IS s.truth AND m.vcost IS CASE WHEN s.c0=0 AND s.truth>=0 THEN MAX(0,s.c4-s.work-relation_cache_fill_delta(s.c5)) END AND m.vpeak IS s.measured WHERE s.measured>=0 AND (CASE WHEN s.truth<0 AND s.c0=2 THEN (SELECT CASE WHEN MAX(f.peak) IS NOT NULL THEN relation_peak_note(s.c2+MAX(f.peak)) ELSE 1 END FROM relation_memo f WHERE f.mode IS 0 AND f.clause IS 0 AND f.utility IS 6000002 AND f.file IS @file AND f.node IS s.c1 AND s.c2+f.peak<=256) WHEN s.truth<0 AND s.c0=1 THEN (SELECT CASE WHEN MAX(f.peak) IS NOT NULL THEN relation_peak_note(s.c3+MAX(f.peak)) ELSE 1 END FROM node previous JOIN node candidate ON candidate.file=previous.file AND candidate.parent=s.c2 AND candidate.id>previous."last" JOIN relation_memo f ON f.mode IS CASE WHEN candidate.id IS NOT NULL THEN 0 END AND f.clause IS 0 AND f.utility IS (6000000+s.c1) AND f.file IS @file AND f.node IS candidate.id WHERE previous.file=@file AND previous.id=s.c4 AND s.c3+f.peak<=256) ELSE 1 END)>=0"#
    );
    let terminal = "SELECT w.node,w.helper,w.level,w.work,w.handle,w.truth,3,w.replay,w.peak WHERE w.phase=2 AND w.handle IS NULL";
    let dispatch = format!(
        "WITH entered AS MATERIALIZED ({entry}),body AS MATERIALIZED ({body}),returned AS MATERIALIZED ({returned}) SELECT * FROM entered UNION ALL SELECT * FROM body UNION ALL SELECT * FROM returned UNION ALL {terminal}"
    );
    let query = format!(
        "WITH RECURSIVE machine(node,helper,level,work,handle,truth,phase,replay,peak) AS (SELECT @node,@helper,@level,@work,NULL,0,0,NULL,0 UNION ALL SELECT r.c0,r.c1,r.c2,r.c3,r.c4,r.c5,r.c6,r.c7,r.c8 FROM machine w {} WHERE w.phase<>3) SELECT truth,work,(@work-work) executionCost,replay replayCost,peak relativePeak FROM machine WHERE phase=3",
        bridge(&dispatch, 9)
    );
    Ok(Frontier { query, terminals })
}
pub(super) fn bind(
    query: &str,
    file: &str,
    node: &str,
    helper: &str,
    level: &str,
    work: &str,
) -> String {
    query
        .replace("@file", file)
        .replace("@node", node)
        .replace("@helper", helper)
        .replace("@level", level)
        .replace("@work", work)
}
/// Native-oracle harness for a root that directly invokes the selected helper.
pub(super) fn compile_helper(
    grammars: &[&GrammarMatcher],
    allowance: i64,
    helper: u32,
) -> Result<Compiled, String> {
    if grammars.len() != 1 || allowance < 0 || !(1..=3).contains(&helper) {
        return Err("numeric frontier harness needs one helper grammar".into());
    }
    let grammar = grammars[0];
    let f = frontier(grammar)?;
    let query = bind(
        &f.query,
        "d.file",
        "d.node",
        &helper.to_string(),
        "0",
        "w.work",
    );
    let sql = format!(
        r#"WITH RECURSIVE init AS MATERIALIZED (SELECT relation_memo_reset(0) ready), roots AS MATERIALIZED (SELECT n.file,n.id node,ROW_NUMBER() OVER(PARTITION BY n.file ORDER BY n.id) ordinal FROM node n CROSS JOIN init WHERE init.ready=1 AND {}),
walk(file,node,ordinal,truth,work) AS (SELECT file,NULL,0,0,{allowance} FROM roots GROUP BY file UNION ALL SELECT d.file,d.node,d.ordinal,r.c0,r.c1 FROM walk w JOIN roots d ON d.file=w.file AND d.ordinal=w.ordinal+1 {} WHERE w.truth>=0),failed AS MATERIALIZED(SELECT * FROM walk WHERE truth<0)
SELECT 0,w.file,w.node,'[]',0,w.work FROM walk w WHERE w.truth=1 AND NOT EXISTS(SELECT 1 FROM failed f WHERE f.file=w.file) UNION ALL SELECT 0,file,NULL,'[]',truth,work FROM failed ORDER BY 1,2,3"#,
        super::root_filter(grammar, None),
        bridge(&query, 5)
    );
    let terminal_count = f.terminals.len();
    Ok(Compiled {
        sql,
        terminals: f.terminals,
        relations: json!({"terminalOwners":vec![0;terminal_count],"effectContract":{"utilityOnlyFrontier":true,"pureBooleanPCs":0,"memoIdentityNamespace":6000000,"separateExecutionAndReplayCost":true}}),
    })
}
#[path = "sim905_effect_spike_tests.rs"]
mod tests;
