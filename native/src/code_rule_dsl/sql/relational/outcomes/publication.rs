//! File-local failure publication. Only fallible clauses sequence roots.
//! Each SQL child completes before the next root in its clause/file pair is reached.
use super::ordered::nested::{bridge, program};
use super::{Demand, Plan, Slot, Slots, Sql, fields};
use serde_json::{Value, json};

pub(in crate::code_rule_dsl::sql::relational::outcomes) fn compile_jobs(
    plans: &[(&Plan, &[Slot], bool, &str)],
    work: i64,
) -> Result<(String, Value), String> {
    if work < 0 {
        return Err("publication: negative work allowance".into());
    }
    if plans.is_empty() || plans.len() > 32 {
        return Err("publication: expected 1..32 clauses in this bounded kernel".into());
    }
    let mut sql = Sql {
        definitions: vec![],
        profile: vec![],
        correlated: false,
    };
    let width = plans
        .iter()
        .map(|(plan, _, _, _)| plan.writes.len())
        .max()
        .unwrap_or(0);
    let mut raw_arms = vec![];
    let mut payload_cases = vec![];
    let mut clauses = vec![];
    for (clause, &(plan, slots, applies, root_filter)) in plans.iter().enumerate() {
        let roots = Demand {
            name: sql.relation("publication root demand",format!("SELECT ROW_NUMBER() OVER () AS k,n.file,n.id AS node,{work} AS work,ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS ordinal FROM node n WHERE {} AND {}",usize::from(applies),root_filter),true,"k,file,node,ordinal"),
            slots: Slots::new(),
        };
        let output = if plan.fallible() {
            let (query, profile) = program(
                plan,
                &roots,
                true,
                "1",
                &plan.writes,
                ("d.file", "d.node", "w.work"),
            )?;
            let walk = format!("outcome{}", sql.definitions.len());
            let empty = fields(&plan.writes, |_| "NULL".into());
            let values = plan
                .writes
                .iter()
                .enumerate()
                .map(|(i, _)| format!(",r.c{}", i + 2))
                .collect::<String>();
            sql.relation("reached roots per clause/file",format!("SELECT NULL AS k,file,NULL AS node,0 AS ordinal,0 AS truth,{work} AS work{empty} FROM {} GROUP BY file\nUNION ALL\nSELECT d.k,d.file,d.node,d.ordinal,r.c0,r.c1{values} FROM {walk} w CROSS JOIN {} d ON d.file=w.file AND d.ordinal=w.ordinal+1 {} WHERE w.truth>=0",roots.name,roots.name,bridge(&query,2+plan.writes.len())),false,"k,file,node,ordinal,truth,work,observable captures");
            let item = sql.profile.last_mut().unwrap();
            item["childPrograms"] = json!({"root":profile});
            item["recursiveColumns"] = json!(6 + plan.writes.len());
            walk
        } else {
            sql.emit(plan, &roots)?.name
        };
        let mut observed: Vec<_> = plan.writes.iter().copied().collect();
        observed.sort_by_key(|&s| &slots[s].name);
        let payload = observed
            .iter()
            .map(|&s| {
                if slots[s].count {
                    format!("CAST(o.c{s} AS TEXT)")
                } else {
                    format!("o.c{s}")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let columns = (0..width)
            .map(|i| {
                format!(
                    ",{} AS v{i}",
                    observed
                        .get(i)
                        .map_or("NULL".into(), |slot| format!("o.c{slot}"))
                )
            })
            .collect::<String>();
        let source = if plan.fallible() {
            format!("{output} o")
        } else {
            format!("{} d JOIN {output} o ON o.k=d.k", roots.name)
        };
        let owner = if plan.fallible() { "o" } else { "d" };
        raw_arms.push(format!("SELECT {clause} AS clause,{owner}.file,{owner}.node,o.truth,o.work{columns} FROM {source} WHERE o.truth=1 OR o.truth<0"));
        let published = observed
            .iter()
            .enumerate()
            .map(|(i, &slot)| {
                if slots[slot].count {
                    format!("CAST(m.v{i} AS TEXT)")
                } else {
                    format!("m.v{i}")
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        payload_cases.push(format!("WHEN {clause} THEN json_array({published})"));
        clauses.push(json!({"clause":clause,"outcomeDemand":roots.name,"outcomeOutput":output,"outcomePayload":payload,"captures":observed.iter().map(|&s|json!({"name":slots[s].name,"slot":s,"kind":if slots[s].count {"count"} else {"node"}})).collect::<Vec<_>>()}));
    }
    let raw = sql.relation(
        "typed provisional publication",
        raw_arms.join("\nUNION ALL\n"),
        true,
        "clause,file,node,truth,work,typed observed captures",
    );
    let failures = sql.relation(
        "failed clause/file pairs",
        format!("SELECT clause,file,node,truth,work FROM {raw} WHERE truth<0"),
        true,
        "clause,file,node,failure,work",
    );
    let query = format!(
        "WITH RECURSIVE\n{}\n-- final matches\nSELECT m.clause,m.file,m.node,CASE m.clause {} END AS payload,0 AS failure,m.work FROM {raw} m WHERE m.truth=1 AND NOT EXISTS (SELECT 1 FROM {failures} f WHERE f.clause=m.clause AND f.file=m.file)\nUNION ALL\nSELECT clause,file,node,'[]',truth,work FROM {failures}\nORDER BY 1,2,3",
        sql.definitions.join(",\n"),
        payload_cases.join(" ")
    );
    if query.len() > 256_000 {
        return Err("publication: SQL text exceeds 256000 bytes".into());
    }
    let mut profile = json!({"scope":"Workspace clause/file failure publication kernel","clauses":clauses,"relations":sql.profile,"outcomes":{"false":0,"true":1,"unknown":2,"depth":-1,"work":-2,"scope":-3,"value":-4},"failureColumn":4,"remainingWorkColumn":5,"initialWork":work,"publicationWidth":5+width,"nativeComposition":false,"flags":["Fallible clauses sequence roots within each file; total clauses retain set composition.","Provisional matches are withheld until the clause/file outcome is complete."],"limitations":["Work accounting covers the admitted outcome operators. Utility memo replay spans Boolean scopes at one root. Traversal and nested value-utility sharing remain unproved.","Native depth/stack budgets, decisions and enumeration remain unproved.","This publication contract is for workspace local clauses. Stateless query aggregation has a different failure scope."]});
    if clauses.len() == 1 {
        for key in [
            "outcomeDemand",
            "outcomeOutput",
            "outcomePayload",
            "captures",
        ] {
            profile[key] = clauses[0][key].clone();
        }
    }
    Ok((query, profile))
}
