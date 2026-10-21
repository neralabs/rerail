//! A continuation-based recursive CTE, compiled from real logical operators. One recursive
//! reference carries the call stack, sparse capture environment, and per-lane memo. This is a
//! general scheduling feasibility baseline; specialized relational shapes can replace its work.
use crate::code_rule_dsl::sql::plan::{Op, SlotKind, Statement, Stop, Walk};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LeafPlan {
    Baseline,
    Kinds,
    Capture,
    Partitioned,
}
#[derive(Default)]
struct Compiler {
    code: Vec<Value>,
    patterns: BTreeMap<usize, usize>,
    regex: BTreeSet<usize>,
    facts: BTreeSet<usize>,
    eager_facts: BTreeSet<usize>,
    direct: BTreeMap<usize, (String, Option<Vec<u16>>)>,
}
impl Compiler {
    fn op(&mut self, op: &Op) -> Result<usize, String> {
        if let Op::Unknowable { op, .. } = op {
            return self.op(op);
        }
        let id = self.code.len();
        self.code.push(Value::Null);
        let value = match op {
            Op::Kinds(kinds) => json!({"tag":"kind","kinds":kinds}),
            Op::Regex(term) => {
                self.regex.insert(*term);
                json!({"tag":"regex","term":term})
            }
            Op::Never => json!({"tag":"never"}),
            Op::Pattern {
                term,
                env,
                vars,
                single_capture,
                ..
            } => {
                if *env && single_capture.is_none() {
                    return Err("recursive SQL: bound multi-capture patterns need batched environment demand".into());
                }
                if let Some((dialect, kinds)) = self.direct.get(term) {
                    json!({"tag":"capture","vars":vars,"bound":env,"dialect":dialect,"kinds":kinds})
                } else {
                    self.patterns.insert(*term, vars.len());
                    json!({"tag":"pattern","term":term,"vars":vars,"bound":env})
                }
            }
            Op::All(items) | Op::Seq(items) | Op::Any(items) => {
                let tag = match op {
                    Op::Any(_) => "any",
                    Op::Seq(_) => "seq",
                    _ => "all",
                };
                let kids = items
                    .iter()
                    .map(|op| self.op(op))
                    .collect::<Result<Vec<_>, _>>()?;
                json!({"tag":tag,"kids":kids})
            }
            Op::AllOpen(items) => {
                let kids = items
                    .iter()
                    .map(|t| self.op(&t.op))
                    .collect::<Result<Vec<_>, _>>()?;
                if items.iter().any(|t| {
                    t.mentions.len() > 1
                        || t.binds.len() > 1
                        || t.mentions.first().is_some_and(|m| *m < 0)
                        || t.binds.first().is_some_and(|m| *m < 0)
                }) {
                    return Err("recursive SQL: withheld mask wider than 62 captures".into());
                }
                let terms:Vec<_>=items.iter().map(|t|json!({"mentions":t.mentions.first().copied().unwrap_or(0),"binds":t.binds.first().copied().unwrap_or(0),"withholds":t.withholds})).collect();
                json!({"tag":"all","kids":kids,"terms":terms})
            }
            Op::AnyOpen(items) => {
                let kids = items
                    .iter()
                    .map(|(op, _)| self.op(op))
                    .collect::<Result<Vec<_>, _>>()?;
                json!({"tag":"any","kids":kids,"strict":items.iter().map(|i|i.1).collect::<Vec<_>>()})
            }
            Op::Fact {
                term,
                reads,
                binds,
                at_node: true,
            } if reads.is_empty() && binds.is_empty() && self.eager_facts.contains(term) => {
                self.facts.insert(*term);
                json!({"tag":"fact","term":term})
            }
            Op::Not(sub) => json!({"tag":"not","kids":[self.op(sub)?]}),
            Op::Recur {
                unit,
                slots: None,
                context: None,
            } => json!({"tag":"recur","unit":unit}),
            Op::Relation {
                walk, sub, need, ..
            }
            | Op::Search {
                walk, sub, need, ..
            } => {
                let (walk, field) = walk_code(walk)?;
                let strict = matches!(op, Op::Search { strict: true, .. });
                json!({"tag":"walk","kids":[self.op(sub)?],"walk":walk,"field":field,"need":need.kinds,"strict":strict})
            }
            Op::Count {
                walk,
                sub,
                min,
                max,
                bind,
                need,
            } => {
                let (walk, field) = walk_code(walk)?;
                json!({"tag":"count","kids":[self.op(sub)?],"walk":walk,"field":field,"need":need.kinds,"min":min,"max":max,"bind":bind.map(|b|b.0),"offset":bind.map_or(0,|b|b.1)})
            }
            _ => {
                return Err(format!(
                    "recursive SQL: leaf/scope contract not implemented for {op:?}"
                ));
            }
        };
        self.code[id] = value;
        Ok(id)
    }
}
fn walk_code(walk: &Walk) -> Result<(&'static str, Option<u16>), String> {
    Ok(match walk {
        Walk::Has(Stop::Neighbor) => ("child", None),
        Walk::Has(Stop::End) => ("desc", None),
        Walk::Inside(Stop::Neighbor) => ("parent", None),
        Walk::Inside(Stop::End) => ("ancestor", None),
        Walk::Precedes(Stop::Neighbor) => ("next", None),
        Walk::Precedes(Stop::End) => ("later", None),
        Walk::Follows(Stop::Neighbor) => ("prev", None),
        Walk::Follows(Stop::End) => ("earlier", None),
        Walk::FieldHas(field, Stop::Neighbor) => ("field", Some(*field)),
        _ => return Err("recursive SQL: field-end, stop-set, or semantic walk".into()),
    })
}
fn literal(value: &Value) -> String {
    format!("'{}'", value.to_string().replace('\'', "''"))
}
fn field(i: usize) -> String {
    format!("json_extract(f.value,'$[{i}]')")
}

pub(super) fn compile(statement: &Statement) -> Result<String, String> {
    compile_batch(&[statement])
}

pub(super) fn compile_batch(statements: &[&Statement]) -> Result<String, String> {
    compile_batch_with_contract(statements, &[])
}

pub(super) fn compile_batch_with_contract(
    statements: &[&Statement],
    terminals: &[crate::code_rule_dsl::sql::terminals::Terminal],
) -> Result<String, String> {
    compile_variant(statements, terminals, LeafPlan::Baseline)
}

pub(super) fn compile_variant(
    statements: &[&Statement],
    terminals: &[crate::code_rule_dsl::sql::terminals::Terminal],
    variant: LeafPlan,
) -> Result<String, String> {
    compile_scoped(statements, terminals, variant, 0, false)
}

pub(super) fn compile_scoped(
    statements: &[&Statement],
    terminals: &[crate::code_rule_dsl::sql::terminals::Terminal],
    variant: LeafPlan,
    root_group: usize,
    binary: bool,
) -> Result<String, String> {
    let eager_facts = terminals
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t {
            crate::code_rule_dsl::sql::terminals::Terminal::Fact(f)
                if f.verdicts.spike_eager_safe() =>
            {
                Some(i)
            }
            _ => None,
        })
        .collect();
    let mut compiler = Compiler {
        eager_facts,
        direct: if matches!(variant, LeafPlan::Capture | LeafPlan::Partitioned) {
            terminals
                .iter()
                .enumerate()
                .filter_map(|(id, t)| match t {
                    crate::code_rule_dsl::sql::terminals::Terminal::Pattern(p)
                        if p.root_capture =>
                    {
                        Some((id, (p.dialect.to_string(), p.kinds.clone())))
                    }
                    _ => None,
                })
                .collect()
        } else {
            BTreeMap::new()
        },
        ..Compiler::default()
    };
    let mut seeds = Vec::new();
    let mut abstract_roots = Vec::new();
    for (rule, statement) in statements.iter().enumerate() {
        if statement
            .slots
            .iter()
            .any(|s| !matches!(s.kind, SlotKind::Node | SlotKind::Count))
        {
            return Err("recursive SQL: list/text capture contract".into());
        }
        if statement.units.iter().any(|u| u.following.is_some()) {
            return Err("recursive SQL: following-context contract".into());
        }
        let start = compiler.code.len();
        let entry = compiler.op(&statement.op)?;
        let units = statement
            .units
            .iter()
            .map(|u| compiler.op(&u.body))
            .collect::<Result<Vec<_>, _>>()?;
        for op in &mut compiler.code[start..] {
            if op["tag"] == "recur" {
                let unit = op["unit"].as_u64().unwrap() as usize;
                op["target"] = json!(units[unit]);
                op["key"] = json!(statement.units[unit].key);
            }
        }
        let kinds = statement
            .kinds
            .as_ref()
            .map(|k| k.iter().copied().collect::<BTreeSet<_>>());
        abstract_roots.push((entry, kinds.clone()));
        seeds.push(format!(
            "({rule},{entry},{})",
            kinds.map_or("NULL".into(), |k| literal(&json!(k)))
        ));
    }
    if seeds.is_empty() {
        return Err("recursive SQL: empty program".into());
    }
    let mut definitions = vec![
        format!(
            "code AS MATERIALIZED(SELECT CAST(key AS INTEGER) AS id,value AS instruction,value->>'$.tag' AS tag FROM json_each({}))",
            literal(&json!(compiler.code))
        ),
        format!(
            "roots(rule,entry,kinds) AS MATERIALIZED(VALUES {})",
            seeds.join(",")
        ),
    ];
    let domains = if matches!(variant, LeafPlan::Kinds | LeafPlan::Partitioned) {
        super::amplification::domains::analyze(&compiler.code, &abstract_roots)
    } else {
        BTreeMap::new()
    };
    let mut leaves = Vec::new();
    let mut probes = Vec::new();
    if !compiler.direct.is_empty() {
        definitions.push("dialects(file,dialect) AS MATERIALIZED(SELECT file,fused_file_dialect(file) FROM node WHERE id=0)".into());
    }
    for &term in &compiler.regex {
        let filter = super::amplification::domains::filter(&domains, "regex", term);
        probes.push(format!("WHEN p.tag='regex' AND p.instruction->>'$.term'={term} THEN(SELECT json_object('node',node,'verdict',1,'captures',json('[]')) FROM regex_{term} WHERE file=m.file AND node=m.node)"));
        definitions.push(format!("regex_{term}(file,node) AS MATERIALIZED(SELECT ek>>32,ek&4294967295 FROM text_match WHERE term={term} AND ek IN(SELECT file*4294967296+id FROM node{filter}))"));
        leaves.push(format!(
            "SELECT 'regex' AS tag,{term} AS term,file,node,'[]' AS captures,1 AS verdict FROM regex_{term}"
        ));
    }
    for (&term, &width) in &compiler.patterns {
        let values = (0..width)
            .map(|i| format!("v{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let filter = super::amplification::domains::filter(&domains, "pattern", term);
        probes.push(format!("WHEN p.tag='pattern' AND p.instruction->>'$.term'={term} THEN(SELECT json_object('node',node,'verdict',1,'captures',json(captures)) FROM pattern_{term} WHERE file=m.file AND node=m.node)"));
        definitions.push(format!("pattern_{term}(file,node,captures) AS MATERIALIZED(SELECT file,node,json_array({values}) FROM pattern WHERE term={term} AND nk IN(SELECT file*4294967296+id FROM node{filter}))"));
        leaves.push(format!(
            "SELECT 'pattern',{term},file,node,captures,1 FROM pattern_{term}"
        ));
    }
    for &term in &compiler.facts {
        let filter = super::amplification::domains::filter(&domains, "fact", term);
        probes.push(format!("WHEN p.tag='fact' AND p.instruction->>'$.term'={term} THEN(SELECT json_object('node',node,'verdict',verdict,'captures',json('[]')) FROM fact_{term} WHERE file=m.file AND node=m.node)"));
        definitions.push(format!("fact_{term}(file,node,captures,verdict) AS MATERIALIZED(SELECT ek>>32,ek&4294967295,'[]',1+unk FROM fact WHERE term={term} AND nk IN(SELECT file*4294967296+id FROM node{filter}))"));
        leaves.push(format!(
            "SELECT 'fact',{term},file,node,captures,verdict FROM fact_{term}"
        ));
    }
    if leaves.is_empty() {
        leaves.push("SELECT '',-1,-1,-1,'[]',0 WHERE 0".into());
    }
    if variant != LeafPlan::Partitioned {
        definitions.push(format!(
            "leaves(tag,term,file,node,captures,verdict) AS MATERIALIZED({})",
            leaves.join(" UNION ALL ")
        ));
    }
    let fpc = field(0);
    let fnode = field(1);
    let fenv = field(2);
    let index = field(3);
    let last = field(4);
    let count = field(5);
    let call_key = if binary {
        format!("json({})", field(6))
    } else {
        field(6)
    };
    let unknown = field(7);
    let withheld = field(8);
    let instruction = "p.instruction";
    let kids = format!("json_extract({instruction},'$.kids')");
    let tag = "p.tag";
    let target = format!("json_extract({instruction},'$.target')");
    let key = format!(
        "json_array(json_extract({instruction},'$.unit'),m.node,json((SELECT json_group_array(json_extract(m.env,'$.\"'||k.value||'\"')) FROM json_each(json_extract({instruction},'$.key')) k)))"
    );
    let key = if binary { format!("json({key})") } else { key };
    let memo_path = format!("'$.\"'||{key}||'\"'");
    let memo_hit = format!("json_extract(m.memo,{memo_path})");
    let stack_key = if binary {
        "json(json_extract(z.value,'$[6]'))"
    } else {
        "json_extract(z.value,'$[6]')"
    };
    let is_cycle = format!("EXISTS(SELECT 1 FROM json_each(m.stack) z WHERE {stack_key}={key})");
    let restore = "json_remove(m.stack,'$[#-1]')";
    let vars = format!("json_extract({instruction},'$.vars')");
    let bound_slot = format!("json_extract({vars},'$[0]')");
    let bound_node = format!("json_extract(m.env,'$.\"'||{bound_slot}||'\"')");
    let (leaf_node, leaf_captures, leaf_verdict, leaf_join) = if variant == LeafPlan::Partitioned {
        (
            "json_extract(a.value,'$.node')",
            "json_extract(a.value,'$.captures')",
            "json_extract(a.value,'$.verdict')",
            format!(
                " CROSS JOIN json_each(json_array({})) a",
                if probes.is_empty() {
                    "NULL".to_owned()
                } else {
                    format!("CASE {} ELSE NULL END", probes.join(" "))
                }
            ),
        )
    } else {
        (
            "a.node",
            "a.captures",
            "a.verdict",
            format!(
                " LEFT JOIN leaves a ON a.tag=p.tag AND a.term=json_extract({instruction},'$.term') AND a.file=m.file AND a.node=m.node"
            ),
        )
    };
    let patched = format!(
        "json_patch(m.env,COALESCE((SELECT json_group_object(v.value,json_extract({leaf_captures},'$['||v.key||']')) FROM json_each({vars}) v),'{{}}'))"
    );
    let bound_compatible = format!(
        "({bound_node} IS NULL OR {bound_node}=m.node OR {})",
        equality(&bound_node)
    );
    let leaf_from = format!(
        "FROM machine m JOIN code p ON p.id=m.pc LEFT JOIN node n ON n.file=m.file AND n.id=m.node{leaf_join} LEFT JOIN node b ON b.file=m.file AND b.id={bound_node}"
    );
    let leaf_from = if compiler.direct.is_empty() {
        leaf_from
    } else {
        format!("{leaf_from} LEFT JOIN dialects d ON d.file=m.file")
    };
    let capture_holds = format!(
        "n.named AND d.dialect=json_extract({instruction},'$.dialect') AND (json_extract({instruction},'$.kinds') IS NULL OR n.kind IN(SELECT value FROM json_each(json_extract({instruction},'$.kinds')))) AND (NOT COALESCE(json_extract({instruction},'$.bound'),0) OR {bound_compatible})"
    );
    let capture_holds = if compiler.direct.is_empty() {
        "0".to_owned()
    } else {
        capture_holds
    };
    let mut arms = Vec::new();
    let emit = |from: &str,
                condition: &str,
                pc: &str,
                node: &str,
                env: &str,
                stack: &str,
                memo: &str,
                result: &str,
                fault: &str| {
        format!(
            "SELECT m.rule,m.file,m.root,{pc},{node},{env},{stack},{memo},{result},{fault},m.steps+1 {from} WHERE m.fault=0 AND m.steps<200000 AND ({condition})"
        )
    };
    let leaf_result = format!(
        "CASE {tag} WHEN 'kind' THEN n.kind IN(SELECT value FROM json_each(json_extract({instruction},'$.kinds'))) WHEN 'regex' THEN {leaf_node} IS NOT NULL WHEN 'fact' THEN COALESCE({leaf_verdict},0) WHEN 'capture' THEN ({capture_holds}) WHEN 'pattern' THEN {leaf_node} IS NOT NULL AND (NOT COALESCE(json_extract({instruction},'$.bound'),0) OR {bound_compatible}) ELSE 0 END"
    );
    let blocked = "COALESCE((json_extract(m.stack,'$[#-1][8]') & (SELECT json_extract(c.instruction,'$.terms['||(json_extract(m.stack,'$[#-1][3]')-1)||'].mentions') FROM code c WHERE c.id=json_extract(m.stack,'$[#-1][0]')))<>0,0)";
    arms.push(emit(
        "FROM machine m",
        &format!("m.result=-1 AND ({blocked})"),
        "m.pc",
        "m.node",
        "m.env",
        "m.stack",
        "m.memo",
        "3",
        "0",
    ));
    arms.push(emit(
        &leaf_from,
        &format!(
            "m.result=-1 AND NOT({blocked}) AND p.tag IN('kind','regex','pattern','never','fact','capture')"
        ),
        "m.pc",
        "m.node",
        &format!("CASE WHEN p.tag='capture' AND ({capture_holds}) THEN json_set(m.env,'$.\"'||{bound_slot}||'\"',m.node) WHEN p.tag='pattern' AND {leaf_node} IS NOT NULL THEN {patched} ELSE m.env END"),
        "m.stack",
        "m.memo",
        &leaf_result,
        "0",
    ));
    let mut enter_from =
        "FROM machine m JOIN code p ON p.id=m.pc LEFT JOIN node n ON n.file=m.file AND n.id=m.node"
            .to_owned();
    enter_from.push_str(" CROSS JOIN json_each(json_array('[]')) f");
    let first_query = candidate(instruction, "m.node", "n", "NULL");
    enter_from.push_str(&format!(" CROSS JOIN json_each(json_array(CASE WHEN p.tag IN('walk','count') THEN ({first_query}) END)) choice"));
    let first = "choice.value";
    let child0 = format!("json_extract({kids},'$[0]')");
    let enter = format!(
        "CASE WHEN p.tag='recur' THEN {memo_hit} IS NULL AND NOT({is_cycle}) WHEN p.tag IN('walk','count') THEN ({first}) IS NOT NULL ELSE json_array_length({kids})>0 END"
    );
    let empty_count = format!(
        "(0>=json_extract({instruction},'$.min') AND (json_extract({instruction},'$.max') IS NULL OR 0<=json_extract({instruction},'$.max')))"
    );
    let enter_env = format!(
        "CASE WHEN p.tag='recur' AND {memo_hit} IS NOT NULL THEN json_patch(m.env,json_extract({memo_hit},'$[1]')) WHEN p.tag='count' AND NOT({enter}) AND json_extract({instruction},'$.bind') IS NOT NULL THEN json_set(m.env,'$.\"'||json_extract({instruction},'$.bind')||'\"',json_extract({instruction},'$.offset')) ELSE m.env END"
    );
    let pushed = format!(
        "json_insert(m.stack,'$[#]',json_array(p.id,m.node,json(m.env),1,CASE WHEN p.tag IN('walk','count') THEN ({first}) END,0,CASE WHEN p.tag='recur' THEN {key} END,0,0))"
    );
    arms.push(emit(&enter_from,&format!("m.result=-1 AND NOT({blocked}) AND p.tag IN('all','seq','any','not','recur','walk','count')"),&format!("CASE WHEN p.tag='recur' THEN {target} ELSE {child0} END"),&format!("CASE WHEN p.tag IN('walk','count') AND ({enter}) THEN ({first}) ELSE m.node END"),&enter_env,&format!("CASE WHEN ({enter}) THEN {pushed} ELSE m.stack END"),"m.memo",&format!("CASE WHEN ({enter}) THEN -1 WHEN p.tag='recur' THEN COALESCE(json_extract({memo_hit},'$[0]'),0) WHEN p.tag='count' THEN {empty_count} WHEN p.tag IN('all','seq') THEN 1 ELSE 0 END"),&format!("CASE WHEN p.tag='recur' AND ({is_cycle}) AND {memo_hit} IS NULL THEN 1 ELSE 0 END")));
    // Returning from a child uses its parent's instruction, never a second recursive read.
    let mut parent_from = format!(
        "FROM machine m CROSS JOIN json_each(json_array(COALESCE(json_extract(m.stack,'$[#-1]'),json('[]')))) f JOIN code p ON p.id={fpc} LEFT JOIN node n ON n.file=m.file AND n.id={fnode}"
    );
    let next_query = candidate(instruction, &fnode, "n", &last);
    parent_from.push_str(&format!(" CROSS JOIN json_each(json_array(CASE WHEN p.tag IN('walk','count') THEN ({next_query}) END)) choice"));
    let next = "choice.value";
    let more_children = format!("{index}<json_array_length({kids})");
    let unknown_strict =
        format!("COALESCE(json_extract({instruction},'$.strict['||({index}-1)||']'),0)");
    let strict_walk = format!("COALESCE(json_extract({instruction},'$.strict'),0)");
    let more = format!(
        "CASE WHEN p.tag='all' THEN m.result<>0 AND {more_children} WHEN p.tag='seq' THEN m.result=1 AND {more_children} WHEN p.tag='any' THEN (m.result=0 OR (m.result=2 AND NOT({unknown_strict}))) AND {more_children} WHEN p.tag='walk' THEN (m.result=0 OR (m.result=2 AND NOT({strict_walk}))) AND ({next}) IS NOT NULL WHEN p.tag='count' THEN ({next}) IS NOT NULL ELSE 0 END"
    );
    parent_from.push_str(&format!(
        " CROSS JOIN json_each(json_array({more})) control"
    ));
    let more = "control.value";
    let new_count = format!("({count}+(m.result=1))");
    let new_unknown = format!(
        "CASE WHEN p.tag='count' THEN {unknown}+(m.result=2) ELSE ({unknown} OR m.result>=2) END"
    );
    let binds =
        format!("COALESCE(json_extract({instruction},'$.terms['||({index}-1)||'].binds'),0)");
    let withholds =
        format!("COALESCE(json_extract({instruction},'$.terms['||({index}-1)||'].withholds'),0)");
    let new_withheld = format!(
        "({withheld} | CASE WHEN m.result=3 OR (m.result=2 AND {withholds}) THEN {binds} ELSE 0 END)"
    );
    let count_holds = format!(
        "((json_extract({instruction},'$.bind') IS NULL OR ({new_unknown})=0) AND {new_count}>=json_extract({instruction},'$.min') AND (json_extract({instruction},'$.max') IS NULL OR {new_count}+({new_unknown})<=json_extract({instruction},'$.max')))"
    );
    let count_fails = format!(
        "({new_count}+({new_unknown})<json_extract({instruction},'$.min') OR {new_count}>json_extract({instruction},'$.max'))"
    );
    let continue_stack = format!(
        "json_set(m.stack,'$[#-1][3]',{index}+1,'$[#-1][4]',CASE WHEN p.tag IN('walk','count') THEN ({next}) ELSE {last} END,'$[#-1][5]',{new_count},'$[#-1][7]',({new_unknown}),'$[#-1][8]',{new_withheld})"
    );
    let result = format!(
        "CASE WHEN ({more}) THEN -1 WHEN p.tag='not' THEN CASE WHEN m.result=2 THEN 2 ELSE 1-m.result END WHEN p.tag='count' THEN CASE WHEN {count_holds} THEN 1 WHEN {count_fails} THEN 0 ELSE 2 END WHEN p.tag='all' THEN CASE WHEN m.result=0 THEN 0 WHEN ({new_unknown}) THEN 2 ELSE 1 END WHEN p.tag IN('any','walk') THEN CASE WHEN m.result=1 THEN 1 WHEN ({new_unknown}) THEN 2 ELSE 0 END ELSE m.result END"
    );
    parent_from.push_str(&format!(
        " CROSS JOIN json_each(json_array({result})) verdict"
    ));
    let result = "verdict.value";
    let environment = format!(
        "CASE WHEN p.tag='count' AND ({result})=1 AND json_extract({instruction},'$.bind') IS NOT NULL THEN json_set({fenv},'$.\"'||json_extract({instruction},'$.bind')||'\"',{new_count}+json_extract({instruction},'$.offset')) WHEN p.tag IN('not','count') OR (p.tag='any' AND ({more})) OR (p.tag='walk' AND ({more})) OR (NOT({more}) AND ({result})<>1) THEN {fenv} ELSE m.env END"
    );
    let memo_env = format!(
        "(SELECT json_group_object(k.value,json_extract(m.env,'$.\"'||k.value||'\"')) FROM json_each(json_extract({instruction},'$.key')) k)"
    );
    let memo = format!(
        "CASE WHEN p.tag='recur' THEN json_set(m.memo,'$.\"'||{call_key}||'\"',json_array(m.result,json(COALESCE({memo_env},'{{}}')))) ELSE m.memo END"
    );
    arms.push(emit(&parent_from,"m.result>=0 AND json_array_length(m.stack)>0",&format!("CASE WHEN ({more}) THEN CASE WHEN p.tag IN('walk','count') THEN {child0} ELSE json_extract({kids},'$['||{index}||']') END ELSE p.id END"),&format!("CASE WHEN ({more}) AND p.tag IN('walk','count') THEN ({next}) ELSE {fnode} END"),&environment,&format!("CASE WHEN ({more}) THEN {continue_stack} ELSE {restore} END"),&memo,result,"0"));
    Ok(super::amplification::machine::render(
        definitions,
        arms,
        root_group,
        binary,
    ))
}

fn candidate(p: &str, origin: &str, n: &str, after: &str) -> String {
    let w = format!("json_extract({p},'$.walk')");
    let lower = format!(
        "CASE {w} WHEN 'child' THEN {origin}+1 WHEN 'desc' THEN {origin}+1 WHEN 'parent' THEN {n}.parent WHEN 'ancestor' THEN 0 WHEN 'next' THEN {n}.next WHEN 'later' THEN {n}.anext WHEN 'prev' THEN {n}.prev WHEN 'earlier' THEN 0 WHEN 'field' THEN {origin}+1 END"
    );
    let upper = format!(
        "CASE {w} WHEN 'child' THEN {n}.\"last\" WHEN 'desc' THEN {n}.\"last\" WHEN 'parent' THEN {n}.parent WHEN 'ancestor' THEN {origin}-1 WHEN 'next' THEN {n}.next WHEN 'later' THEN 4294967295 WHEN 'prev' THEN {n}.prev WHEN 'earlier' THEN {n}.aprev WHEN 'field' THEN {n}.\"last\" END"
    );
    let backwards = format!("{w} IN('ancestor','earlier','parent','prev')");
    format!(
        "SELECT CASE WHEN {backwards} THEN MAX(c.id) ELSE MIN(c.id) END FROM node c WHERE c.file=m.file AND c.id BETWEEN ({lower}) AND ({upper}) AND (CASE {w} WHEN 'child' THEN c.parent={origin} WHEN 'ancestor' THEN c.\"last\">={origin} WHEN 'later' THEN c.parent={n}.wpar WHEN 'earlier' THEN c.parent={n}.wpar WHEN 'field' THEN c.parent={origin} AND c.field=json_extract({p},'$.field') AND c.ffirst ELSE 1 END) AND ({after} IS NULL OR CASE WHEN {backwards} THEN c.id<{after} ELSE c.id>{after} END) AND (json_extract({p},'$.need') IS NULL OR c.kind IN(SELECT value FROM json_each(json_extract({p},'$.need'))))"
    )
}

/// Source decoding only. Unification's structure, control flow, and equality stay in SQL.
pub(super) fn register_text(
    connection: &rusqlite::Connection,
    batch: &std::sync::Arc<crate::code_rule_dsl::sql::exec::Batch>,
) -> rusqlite::Result<()> {
    struct Text(
        std::panic::AssertUnwindSafe<std::sync::Arc<crate::code_rule_dsl::sql::exec::Batch>>,
    );
    impl Text {
        fn read(&self, file: usize, node: u32) -> rusqlite::Result<String> {
            let entry = self
                .0
                .files
                .get(file)
                .filter(|f| node < f.facts.len())
                .ok_or_else(|| rusqlite::Error::ModuleError("text input outside batch".into()))?;
            Ok(crate::code_rule_dsl::matcher::node_at(&entry.root, node)
                .text()
                .into_owned())
        }
    }
    let dialects: Vec<_> = batch.files.iter().map(|f| f.dialect.to_string()).collect();
    connection.create_scalar_function(
        "fused_file_dialect",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            dialects
                .get(ctx.get::<i64>(0)? as usize)
                .cloned()
                .ok_or_else(|| rusqlite::Error::ModuleError("dialect input outside batch".into()))
        },
    )?;
    let text = Text(std::panic::AssertUnwindSafe(std::sync::Arc::clone(batch)));
    connection.create_scalar_function(
        "spike_node_text",
        2,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |ctx| text.read(ctx.get::<i64>(0)? as usize, ctx.get(1)?),
    )
}

fn equality(bound: &str) -> String {
    let equal_text = "CASE WHEN a.leaf AND b.leaf THEN a.atom=b.atom ELSE a.thash=b.thash AND spike_node_text(m.file,a.id)=spike_node_text(m.file,b.id) END";
    format!(
        "NOT EXISTS(WITH RECURSIVE pairs(a,b,top) AS(SELECT {bound},m.node,1 UNION ALL SELECT a.id+1,b.id+1,0 FROM pairs w JOIN node a ON a.file=m.file AND a.id=w.a JOIN node b ON b.file=m.file AND b.id=w.b WHERE a.id<>b.id AND NOT a.leaf AND NOT b.leaf AND a.kind=b.kind AND a.\"last\">a.id AND b.\"last\">b.id UNION ALL SELECT a.nsib,b.nsib,0 FROM pairs w JOIN node a ON a.file=m.file AND a.id=w.a JOIN node b ON b.file=m.file AND b.id=w.b WHERE w.top=0 AND a.nsib IS NOT NULL AND b.nsib IS NOT NULL) SELECT 1 FROM pairs w JOIN node a ON a.file=m.file AND a.id=w.a JOIN node b ON b.file=m.file AND b.id=w.b WHERE (w.top=0 AND (a.nsib IS NULL)<>(b.nsib IS NULL)) OR (a.id<>b.id AND CASE WHEN a.leaf OR b.leaf THEN NOT({equal_text}) ELSE a.kind<>b.kind OR (a.\"last\">a.id)<>(b.\"last\">b.id) END))"
    )
}
