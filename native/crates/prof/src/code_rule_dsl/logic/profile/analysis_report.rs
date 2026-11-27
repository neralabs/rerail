//! Connect production counters to the structural analyzer. No diagnostic query is executed.
use super::{
    State,
    analysis::{self, Cursor, Finding, Input, Join, Projection, Span, Sql},
    dropped,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn input(state: &State) -> Input {
    let mut i = Input {
        spans: state
            .spans
            .iter()
            .map(|(p, s)| Span {
                path: p.clone(),
                own_cpu: s.own,
                calls: s.calls,
            })
            .collect(),
        sql: state
            .sql
            .iter()
            .map(|((p, q), s)| Sql {
                path: p.clone(),
                text: q.clone(),
                runs: s.runs,
                vm: s.vm,
                autoindex: s.autoindex,
            })
            .collect(),
        joins: state
            .native
            .iter()
            .map(|((p, q), steps)| Join {
                path: p.clone(),
                plan: q.clone(),
                steps: steps.clone(),
                cpu: state.native_cpu.get(&(p.clone(), q.clone())).copied(),
            })
            .collect(),
        ..Input::default()
    };
    let reasons: Vec<_> = state
        .decisions
        .keys()
        .filter(|(topic, _)| topic == "projection provenance")
        .map(|(_, text)| text.clone())
        .collect();
    for (topic, text) in state.decisions.keys() {
        if let Some(attr) = topic.strip_suffix(" on request")
            && let Some(why) = text.strip_prefix("no, every node: ")
        {
            i.eager_attributes.push((attr.to_owned(), why.to_owned()));
        }
    }
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
                if w.len() == 10
                    && let (Ok(filters), Ok(batches), Ok(rows)) = (
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

/// The measured plans, as the rows-dropped analysis reads them: each step with its actual rows and CPU, and the head's
/// authored rules where the run's sources are known.
fn plans(state: &State) -> Vec<dropped::Plan> {
    let mut out = Vec::new();
    for ((stratum, kind, _), t) in &state.plans {
        if !t.measured || t.plan.head.is_empty() {
            continue;
        }
        let head_rel = t.plan.head.split('(').next().unwrap_or_default().trim_start_matches('!').to_owned();
        let name = super::author_joins::source_name(&head_rel, None);
        let steps = t
            .plan
            .steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (rows_in, rows_out) = t.actual.get(i).copied().unwrap_or((0, 0));
                dropped::Step {
                    literal: s.literal.clone(),
                    access: s.access.clone(),
                    source: s.source.clone(),
                    vars: s.vars.clone(),
                    est: s.rows,
                    basis: s.basis.clone(),
                    passed: s.passed.iter().map(|(l, r, _)| (l.clone(), *r)).collect(),
                    rows_in,
                    rows_out,
                    cpu: t.cpu.get(i).copied().unwrap_or(0.0),
                }
            })
            .collect();
        out.push(dropped::Plan {
            stratum: stratum.clone(),
            kind: kind.clone(),
            rule: t.rule,
            head: t.plan.head.clone(),
            authored: (name != head_rel).then_some(name),
            head_rel,
            head_vars: t.plan.head_vars.clone(),
            loops: t.loops,
            steps,
        });
    }
    out
}

/// What to do about each cause, beside the rewrite written out.
fn action(cause: dropped::Cause) -> &'static str {
    use dropped::Cause::*;
    match cause {
        DeadVariable => "Make the helper below and read it in place of the literals it is made from: it keeps one row for each value the rest of the rule reads.",
        InvariantInRecursion => "Make the helper below once, outside the recursion, and read it in the recursive rule: each round then joins its new rows to the helper's by the variables they share.",
        Mispredicted => "The planner's estimate is wrong here: fix its basis (statistics, decision 0203) rather than the rule. Reordering the rule's literals does not change the plan.",
        TestAfterWalk => "Give the test's value a relation the planner can look it up in, or bound the rows before the walk (below).",
        Dropped => "Read the plan below: the marked source makes the rows the marked step drops.",
        CrossProduct => "Bind one of the step's variables before it (below), unless the rule means the product.",
    }
}

/// An authored head's label (`name (file:1, file:2, other:3)`) with each file named once: `name (file:1, 2; other:3)`.
fn sites(label: &str) -> String {
    let Some((name, rest)) = label.split_once(" (") else { return label.to_owned() };
    let mut files: Vec<(&str, Vec<&str>)> = Vec::new();
    for site in rest.trim_end_matches(')').split(", ") {
        let (file, line) = site.rsplit_once(':').unwrap_or((site, ""));
        match files.iter_mut().find(|(f, _)| *f == file) {
            Some((_, lines)) => lines.push(line),
            None => files.push((file, vec![line])),
        }
    }
    format!("{name} ({})", files.iter().map(|(f, l)| format!("{f}:{}", l.join(", "))).collect::<Vec<_>>().join("; "))
}

fn finding(d: &dropped::Dropped) -> Finding {
    Finding {
        code: d.cause.code(),
        confidence: "observed",
        location: format!("rule {}", d.authored.as_deref().map_or_else(|| d.head_rel.clone(), sites)),
        cpu: Some(d.lost_cpu),
        evidence: format!(
            "{}; {} rows made and dropped, over {} loops of {} in stratum {}:
{}",
            d.why,
            d.lost_rows,
            d.loops,
            super::rule_name(d.rule, &d.kind),
            d.stratum,
            dropped::shown(d).trim_end()
        ),
        action: action(d.cause),
        fix: d.fix.clone(),
    }
}

pub fn report(state: &State) -> (String, serde_json::Value) {
    let mut i = input(state);
    // The rules that made rows they dropped, each with its cause and fix. The heads whose plans it read are left out
    // of the analysis by native joins' row counts, which says less of them.
    let plans = plans(state);
    i.covered = plans.iter().map(|p| super::kinds::head(&p.head)).collect();
    let mut findings = analysis::analyze(&i);
    findings.extend(dropped::analyze(&plans).iter().map(finding));
    findings.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)).then(a.code.cmp(b.code)).then(a.location.cmp(&b.location)));
    let text = analysis::render(&findings);
    let json = serde_json::json!({"schema":"rerail-structural-analysis-v1",
        "costMeaning":"Attributed CPU, not recoverable savings; findings can overlap",
        "coverage":{"sqlStatements":i.sql.len(),"nativePlans":i.joins.len(),"projectionObservations":i.projections.len(),"tapeCacheObservations":i.tape_hits+i.tape_misses},
        "findings":findings.iter().map(|f|serde_json::json!({"code":f.code,"confidence":f.confidence,"location":f.location,"cpuSeconds":f.cpu,"evidence":f.evidence,"action":f.action,"fix":f.fix})).collect::<Vec<_>>()});
    (text, json)
}

/// Explains each scalar input read (`reads`) by one shortest path of readers to an output, with cycle guards: the
/// relations' readers (a relation's, or a native's, native contracts included) and the outputs found by its caller
/// (over a version, `logic::plan::text_demand`). Public for the report's tests, which sit in the crate above
/// (`logic::profile::report`).
pub fn projection_of(users: &BTreeMap<String, BTreeSet<String>>, outputs: &BTreeSet<String>, reads: &BTreeSet<String>) {
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
    /// A per-node attribute the lowering left eager (decision 0174) is a finding, with the reader's reason.
    #[test]
    fn an_eager_node_attribute_is_a_finding() {
        let mut s = State::default();
        s.decisions.insert(("node_text on request".into(), "no, every node: out reads `!node_text($n, \"x\")`, a negation or a constant node".into()), 1);
        let (text, json) = report(&s);
        assert!(text.contains("EAGER_NODE_ATTRIBUTE"), "{text}");
        assert_eq!(json["findings"][0]["location"], "input relation node_text");
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
}
