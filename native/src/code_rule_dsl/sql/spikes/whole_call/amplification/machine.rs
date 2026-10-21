//! Bound memo scope to a group of roots in one rule and file. Completed utility answers can
//! be reused by later roots, without an ever-growing program-wide JSON value. Binary JSON is
//! a separate experiment; externally visible captures and memo property names remain JSON text.
use super::super::lex;
use std::collections::HashMap;
pub(in super::super) fn render(
    mut definitions: Vec<String>,
    mut arms: Vec<String>,
    group: usize,
    binary: bool,
) -> String {
    let mut seed="SELECT r.rule,n.file,n.id,r.entry,n.id,'{}','[]','{}',-1,0,0 FROM roots r CROSS JOIN node n WHERE r.kinds IS NULL UNION ALL SELECT r.rule,n.file,n.id,r.entry,n.id,'{}','[]','{}',-1,0,0 FROM roots r CROSS JOIN json_each(r.kinds) k CROSS JOIN node n ON n.kind=CAST(k.value AS INTEGER) WHERE r.kinds IS NOT NULL".to_owned();
    if group > 0 {
        definitions.push(format!("starts(rule,file,root,pc,node,env,stack,memo,result,fault,steps) AS MATERIALIZED({seed})"));
        definitions.push("root_order AS MATERIALIZED(SELECT rule,file,root,pc,ROW_NUMBER() OVER(PARTITION BY rule,file ORDER BY root) AS ordinal,LEAD(root) OVER(PARTITION BY rule,file ORDER BY root) AS next_root FROM starts)".into());
        seed = format!(
            "SELECT s.* FROM starts s JOIN root_order r ON r.rule=s.rule AND r.file=s.file AND r.root=s.root WHERE (r.ordinal-1)%{group}=0"
        );
        arms.push(format!("SELECT m.rule,m.file,r.next_root,r.pc,r.next_root,'{{}}','[]',m.memo,-1,0,0 FROM machine m JOIN root_order r ON r.rule=m.rule AND r.file=m.file AND r.root=m.root WHERE m.fault=0 AND m.result>=0 AND json_array_length(m.stack)=0 AND r.next_root IS NOT NULL AND r.ordinal%{group}<>0"));
    }
    definitions.push(format!(
        "machine(rule,file,root,pc,node,env,stack,memo,result,fault,steps) AS({seed} UNION ALL {})",
        arms.join(" UNION ALL ")
    ));
    let payload = "json_object('result',result,'fault',CASE WHEN steps=200000 THEN 2 ELSE fault END,'env',json(env),'steps',steps)";
    let payload = if binary {
        format!("json({payload})")
    } else {
        payload.to_owned()
    };
    let query = format!(
        "WITH RECURSIVE {} SELECT rule AS clause,file,root AS node,{payload} AS captures FROM machine WHERE (result>=0 AND json_array_length(stack)=0) OR fault<>0 OR steps=200000 ORDER BY file,root",
        definitions.join(",\n")
    );
    if !binary {
        return query;
    }
    let names: HashMap<_, _> = [
        "array",
        "object",
        "extract",
        "set",
        "insert",
        "remove",
        "patch",
        "group_array",
        "group_object",
    ]
    .iter()
    .map(|n| (format!("json_{n}"), format!("jsonb_{n}")))
    .collect();
    lex::replace(&query, &names)
}
