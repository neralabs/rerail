//! Connect production counters to the structural analyzer. No diagnostic query is executed.
use super::{
    State,
    analysis::{self, Cursor, Input, Join, Projection, Span, Sql},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn input(state: &State) -> Input {
    let mut i = Input::default();
    i.spans = state
        .spans
        .iter()
        .map(|(p, s)| Span {
            path: p.clone(),
            own_cpu: s.own,
            calls: s.calls,
        })
        .collect();
    i.sql = state
        .sql
        .iter()
        .map(|((p, q), s)| Sql {
            path: p.clone(),
            text: q.clone(),
            runs: s.runs,
            vm: s.vm,
            autoindex: s.autoindex,
        })
        .collect();
    i.joins = state
        .native
        .iter()
        .map(|((p, q), steps)| Join {
            path: p.clone(),
            plan: q.clone(),
            steps: steps.clone(),
            cpu: state.native_cpu.get(&(p.clone(), q.clone())).copied(),
        })
        .collect();
    let reasons: Vec<_> = state
        .decisions
        .keys()
        .filter(|(topic, _)| topic == "projection provenance")
        .map(|(_, text)| text.clone())
        .collect();
    for ((topic, text), count) in &state.decisions {
        match topic.as_str() {
            "composed automatic reprepare" => {
                i.reprepares += text.parse::<u64>().unwrap_or(0) * count
            }
            "input tape cache" => match text.as_str() {
                "hit" => i.tape_hits += count,
                "miss" => i.tape_misses += count,
                _ => {}
            },
            "candidate spike rewrite" => i.late_candidate_rewrite = true,
            "composed input work" => {
                let w: Vec<_> = text.split_whitespace().collect();
                if w.len() == 10 {
                    if let (Ok(filters), Ok(batches), Ok(rows)) = (
                        w[1].parse::<u64>(),
                        w[3].parse::<u64>(),
                        w[5].parse::<u64>(),
                    ) {
                        let name = w[0].trim_end_matches(':');
                        if let Some(c) = i.cursors.iter_mut().find(|c| c.relation == name) {
                            c.filters += filters * count;
                            c.rows += rows * count;
                            c.batches += batches * count;
                        } else {
                            i.cursors.push(Cursor {
                                relation: name.into(),
                                filters: filters * count,
                                rows: rows * count,
                                batches: batches * count,
                            });
                        }
                    }
                }
            }
            "projection demand" => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                    // Unrestricted export/test projections have no declared read set.
                    // Do not label them unrequested based on absent instrumentation.
                    let reads = v["reads"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_else(|| analysis::SCALARS.iter().map(|s| (*s).into()).collect());
                    i.projections.push(Projection {
                        nodes: v["nodes"].as_u64().unwrap_or(0) * count,
                        scalar_nodes: v["scalarNodes"].as_u64().unwrap_or(0) * count,
                        strings: v["strings"].as_u64().unwrap_or(0) * count,
                        reads,
                        reasons: reasons.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    i
}

pub fn report(state: &State) -> (String, serde_json::Value) {
    let i = input(state);
    let findings = analysis::analyze(&i);
    let text = analysis::render(&findings);
    let json = serde_json::json!({"schema":"rerail-structural-analysis-v1",
        "costMeaning":"Attributed CPU, not recoverable savings; findings can overlap",
        "coverage":{"sqlStatements":i.sql.len(),"nativePlans":i.joins.len(),"projectionObservations":i.projections.len(),"tapeCacheObservations":i.tape_hits+i.tape_misses},
        "findings":findings.iter().map(|f|serde_json::json!({"code":f.code,"confidence":f.confidence,"location":f.location,"cpuSeconds":f.cpu,"evidence":f.evidence,"action":f.action})).collect::<Vec<_>>()});
    (text, json)
}

/// Explain scalar inputs using actual dependency edges, including native contracts.
/// Keep one shortest consumer path per requested scalar relation, with cycle guards.
pub(super) fn projection(
    prog: &crate::code_rule_dsl::logic::front::Program,
    reads: &BTreeSet<String>,
) {
    use crate::code_rule_dsl::logic::front::Literal;
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for rule in &prog.rules {
        for a in rule.body.iter().filter_map(Literal::atom) {
            users
                .entry(a.rel.to_string())
                .or_default()
                .insert(rule.head.rel.to_string());
        }
    }
    for (name, native) in &prog.natives {
        for read in native.reads() {
            users.entry(read.into()).or_default().insert(name.clone());
        }
    }
    let outputs: BTreeSet<_> = prog.outputs.iter().cloned().collect();
    for scalar in analysis::SCALARS.iter().filter(|r| reads.contains(**r)) {
        let mut queue = VecDeque::from([vec![(*scalar).to_owned()]]);
        let mut seen = BTreeSet::new();
        let mut found = None;
        while let Some(path) = queue.pop_front() {
            let node = path.last().unwrap();
            if !seen.insert(node.clone()) {
                continue;
            }
            if outputs.contains(node) {
                found = Some(path);
                break;
            }
            for user in users.get(node).into_iter().flatten() {
                let mut next = path.clone();
                next.push(user.clone());
                queue.push_back(next);
            }
        }
        let text = found.map_or_else(
            || format!("{scalar}: requested input; output path unavailable"),
            |p| p.join(" <- "),
        );
        super::decision("projection provenance", || text);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn production_adapter_emits_ranked_findings_and_json() {
        let mut s = State::default();
        s.decisions
            .insert(("input tape cache".into(), "hit".into()), 5725);
        s.decisions
            .insert(("composed automatic reprepare".into(), "1".into()), 71);
        s.spans.insert(
            "worker/facts from tape".into(),
            super::super::SpanTotal {
                own: 1.2,
                calls: 5725,
                ..Default::default()
            },
        );
        let (text, json) = report(&s);
        assert!(text.contains("CACHED_TAPE_REBUILD"));
        assert!(text.contains("REPREPARE"));
        assert_eq!(json["findings"][0]["code"], "CACHED_TAPE_REBUILD");
        assert_eq!(json["coverage"]["tapeCacheObservations"], 5725);
    }
    #[test]
    fn missing_projection_reads_are_unknown_not_empty() {
        let mut s = State::default();
        s.decisions.insert(
            (
                "projection demand".into(),
                r#"{"nodes":100,"scalarNodes":100,"strings":10,"reads":null}"#.into(),
            ),
            1,
        );
        assert!(!report(&s).0.contains("UNREQUESTED_SCALARS"));
    }
    #[test]
    fn projection_reason_traces_real_consumers_and_ignores_dead_scalars() {
        let _lock = super::super::one_run();
        super::super::reset();
        super::super::set(true);
        let p = crate::code_rule_dsl::logic::rr::parse(
            r#"
#include "core.rr"
.decl selected(n:node)
selected(n) :- text(n,"request").
.decl result(n:node)
result(n) :- selected(n).
.output result
"#,
        )
        .unwrap();
        projection(&p, &BTreeSet::from(["text".into()]));
        let decisions = super::super::decisions();
        super::super::set(false);
        assert!(
            decisions
                .keys()
                .any(|(topic, text)| topic == "projection provenance"
                    && text == "text <- selected <- result")
        );
    }
}
