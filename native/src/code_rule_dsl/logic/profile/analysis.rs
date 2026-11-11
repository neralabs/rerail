//! Structural diagnostics over observations, without running or rewriting a query.
use std::fmt::Write;
#[derive(Default, Debug)]
pub(super) struct Input {
    pub spans: Vec<Span>,
    pub joins: Vec<Join>,
    pub sql: Vec<Sql>,
    pub cursors: Vec<Cursor>,
    pub projections: Vec<Projection>,
    pub reprepares: u64,
    pub tape_hits: u64,
    pub tape_misses: u64,
    pub late_candidate_rewrite: bool,
}
#[derive(Default, Debug)]
pub(super) struct Span {
    pub path: String,
    pub own_cpu: f64,
    pub calls: u64,
}
#[derive(Default, Debug)]
pub(super) struct Join {
    pub path: String,
    pub plan: String,
    pub steps: Vec<(u64, u64)>,
    pub cpu: Option<f64>,
}
#[derive(Default, Debug)]
pub(super) struct Sql {
    pub path: String,
    pub text: String,
    pub runs: u64,
    pub vm: u64,
    pub autoindex: u64,
}
#[derive(Default, Debug)]
pub(super) struct Cursor {
    pub relation: String,
    pub filters: u64,
    pub rows: u64,
    pub batches: u64,
}
#[derive(Default, Debug)]
pub(super) struct Projection {
    pub nodes: u64,
    pub scalar_nodes: u64,
    pub strings: u64,
    pub reads: Vec<String>,
    pub reasons: Vec<String>,
}
#[derive(Debug)]
pub(super) struct Finding {
    pub code: &'static str,
    pub confidence: &'static str,
    pub location: String,
    pub cpu: Option<f64>,
    pub evidence: String,
    pub action: &'static str,
}
pub(super) const SCALARS: &[&str] = &[
    "text",
    "name",
    "str",
    "strprefix",
    "value",
    "value_type",
    "static_value",
    "comment",
    "spread",
    "op",
    "op_at",
    "op_t",
    "intval",
    "next_sibling",
    "str_part",
    "str_part_start",
    "str_part_word",
];
pub(super) fn analyze(i: &Input) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |code, confidence, location: String, cpu, evidence: String, action| {
        out.push(Finding {
            code,
            confidence,
            location,
            cpu,
            evidence,
            action,
        })
    };
    let scalar: u64 = i.projections.iter().map(|p| p.scalar_nodes).sum();
    let nodes: u64 = i.projections.iter().map(|p| p.nodes).sum();
    let strings: u64 = i.projections.iter().map(|p| p.strings).sum();
    let projection_cpu: f64 = i
        .spans
        .iter()
        .filter(|s| s.path.split('/').any(|p| p == "project and intern"))
        .map(|s| s.own_cpu)
        .sum();
    let reasons: std::collections::BTreeSet<_> = i
        .projections
        .iter()
        .flat_map(|p| p.reasons.iter().cloned())
        .collect();
    let reasons = reasons.into_iter().collect::<Vec<_>>().join("; ");
    let unread: u64 = i
        .projections
        .iter()
        .filter(|p| !p.reads.iter().any(|r| SCALARS.contains(&r.as_str())))
        .map(|p| p.scalar_nodes)
        .sum();
    if unread > 0 {
        add(
            "UNREQUESTED_SCALARS",
            "observed",
            "input projection".into(),
            None,
            format!("{unread} scalar node visits with no requested scalar relation"),
            "Omit scalar construction for this projection; retain node identities and required document metadata.",
        );
    }
    if scalar > 100_000 && (projection_cpu > 0.05 || i.late_candidate_rewrite) {
        add(
            if i.late_candidate_rewrite {
                "LATE_PROJECTION"
            } else {
                "BROAD_SCALAR_PROJECTION"
            },
            if i.late_candidate_rewrite {
                "observed"
            } else {
                "investigate"
            },
            "input projection".into(),
            Some(projection_cpu),
            format!(
                "{scalar} scalar node visits; {nodes} identities; {strings} interned batch strings. Consumer paths: {reasons}"
            ),
            "Check the named consumers for candidate restrictions before Projection::of; push only semantically valid demands into producers.",
        );
    }
    let construction: Vec<_> = i
        .spans
        .iter()
        .filter(|s| s.path.split('/').any(|p| p == "facts from tape" || p == "tape restore"))
        .collect();
    let cpu: f64 = construction.iter().map(|s| s.own_cpu).sum();
    if i.tape_hits > 0 && cpu > 0.05 {
        add(
            "CACHED_TAPE_REBUILD",
            "investigate",
            "input lifecycle".into(),
            Some(cpu),
            format!(
                "{} tape cache hits, {} misses; restore/core-fact construction: {}. Tape hits do not mean core facts were reused.",
                i.tape_hits,
                i.tape_misses,
                construction
                    .iter()
                    .map(|s| format!("{}: {} calls, {:.6} own CPU s", s.path, s.calls, s.own_cpu))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            "Measure immutable core-fact reuse keyed by source and grammar generation. Check ownership and invalidation before adding a cache; this does not prove duplicate construction.",
        );
    }
    // Sum exclusive child costs; parent exclusive time is not the phase total.
    let identity_cpu: f64 = i.spans.iter().filter(|s| s.path.split('/').any(|p| p == "identity maps")).map(|s| s.own_cpu).sum();
    if nodes > 1_000_000 && scalar == 0 && identity_cpu > 0.05 {
        add(
            "IDENTITY_PROJECTION",
            "investigate",
            "input projection".into(),
            Some(identity_cpu),
            format!("{nodes} node identities, zero scalar node visits, {strings} batch strings"),
            "Measure identity-map reuse or a narrower identity projection while preserving global node IDs; do not rebuild scalar facts.",
        );
    }
    if i.reprepares > 0 {
        add(
            "REPREPARE",
            "observed",
            "SQLite statement lifecycle".into(),
            None,
            format!("{} automatic reprepares during execution", i.reprepares),
            "Inspect function registration and schema changes between cached executions; keep input binding owners alive when removing invalidation.",
        );
    }
    for j in &i.joins {
        // A seed scan is not a fanout. Flag expansion only after the seed, and
        // late rejection only inside a join that actually expanded first.
        let expansion = j
            .steps
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, (a, b))| *a > 0 && *b > 100_000 && *b > a.saturating_mul(20));
        if let Some((step, (a, b))) = expansion {
            add(
                "JOIN_FANOUT",
                "observed",
                j.path.clone(),
                j.cpu,
                format!("step {step}: {a} -> {b} rows; plan: {}", j.plan),
                "Inspect demanded join/group keys before enumeration, including document and world keys. Compare the plan with consumer restrictions; fanout alone does not prove a lost key.",
            );
            if let Some((later, (input, output))) = j
                .steps
                .iter()
                .enumerate()
                .skip(step + 1)
                .find(|(_, (a, b))| *a > 100_000 && *a > b.saturating_add(1).saturating_mul(100))
            {
                add(
                    "LATE_REJECTION",
                    "investigate",
                    j.path.clone(),
                    j.cpu,
                    format!(
                        "expanded at step {step}; step {later} rejects {input} -> {output} rows; plan: {}",
                        j.plan
                    ),
                    "Check whether this restriction can precede the expansion. Preserve aggregate, negation, recursion, and world dependencies; do not move it blindly.",
                );
            }
        }
    }
    for q in &i.sql {
        if q.runs > 1000 && q.vm / q.runs < 1000 {
            add(
                "REPEATED_STATEMENTS",
                "observed",
                q.path.clone(),
                None,
                format!(
                    "{} executions, {} VM instructions; {}",
                    q.runs, q.vm, q.text
                ),
                "Compose relational stages before execution or reuse materialization structures; preparation alone does not remove executions.",
            );
        }
        if q.autoindex > 100_000 {
            add(
                "AUTOMATIC_INDEX",
                "investigate",
                q.path.clone(),
                None,
                format!("{} automatic-index rows; {}", q.autoindex, q.text),
                "Inspect demand selectivity and join order before adding an index. A transient index can be the correct plan.",
            );
        }
    }
    for c in &i.cursors {
        if c.filters > 10_000
            && c.rows > 0
            && c.rows <= c.filters.saturating_mul(4)
            && c.batches == 0
        {
            add(
                "POINT_PROBE_PATTERN",
                "investigate",
                c.relation.clone(),
                None,
                format!(
                    "{} cursor filters, {} candidate rows, no batch lookups",
                    c.filters, c.rows
                ),
                "Measure this source's lookup CPU before changing it. Compare selective probes with a genuine bulk traversal; fewer cursor calls alone do not establish a win.",
            );
        }
    }
    out.sort_by(|a, b| {
        b.cpu
            .unwrap_or(0.0)
            .total_cmp(&a.cpu.unwrap_or(0.0))
            .then(a.code.cmp(b.code))
            .then(a.location.cmp(&b.location))
    });
    out
}
pub(super) fn render(findings: &[Finding]) -> String {
    let mut out = String::from(
        "== Structural analysis\nCPU is attributed cost, not an estimate of recoverable savings. Findings can overlap.\n",
    );
    for f in findings {
        let _ = writeln!(
            out,
            "\n{} [{}] {}\n  CPU: {}\n  Evidence: {}\n  Action: {}",
            f.code,
            f.confidence,
            f.location,
            f.cpu.map_or("unattributed".into(), |c| format!("{c:.6} s")),
            f.evidence,
            f.action
        );
    }
    if findings.is_empty() {
        out.push_str("No supported structural hazard detected. This is not a completeness or performance guarantee.\n");
    }
    out
}
#[cfg(test)]
pub mod tests {
    use super::*;
    fn codes(i: &Input) -> Vec<&'static str> {
        analyze(i).iter().map(|f| f.code).collect()
    }
    #[test]
    fn finer_spans_preserve_phase_cost_without_misattributing_token_work() {
        let mut i = Input { tape_hits: 5725, projections: vec![Projection {
            nodes: 12_535_139, ..Default::default()
        }], ..Default::default() };
        for (path, own_cpu) in [
            ("worker/facts from tape", 0.01),
            ("worker/facts from tape/node columns", 0.6),
            ("worker/tape restore", 0.5),
            ("worker/project and intern", 0.01),
            ("worker/project and intern/projected/identity maps", 0.2),
            ("worker/project and intern/projected/token spans", 0.3),
        ] { i.spans.push(Span { path: path.into(), own_cpu, calls: 5725 }); }
        let fs = analyze(&i);
        assert!((fs.iter().find(|f| f.code == "CACHED_TAPE_REBUILD").unwrap().cpu.unwrap() - 1.11).abs() < 1e-9);
        assert_eq!(fs.iter().find(|f| f.code == "IDENTITY_PROJECTION").unwrap().cpu, Some(0.2));
        i.spans.retain(|s| !s.path.ends_with("identity maps"));
        assert!(!codes(&i).contains(&"IDENTITY_PROJECTION"), "legacy setup cost is not an identity-map measurement");
    }
    #[test]
    fn many_point_probes_are_an_unpriced_opportunity_not_a_speedup_claim() {
        let i = Input {
            cursors: vec![Cursor {
                relation: "py_attribute".into(),
                filters: 65399,
                rows: 31393,
                batches: 0,
            }],
            ..Default::default()
        };
        let f = analyze(&i).pop().unwrap();
        assert_eq!(f.code, "POINT_PROBE_PATTERN");
        assert_eq!(f.cpu, None);
        assert_eq!(f.confidence, "investigate");
        assert!(f.action.contains("CPU before"));
    }
    #[test]
    fn archived_production_profile_identifies_both_billion_row_joins() {
        let mut i = Input::default();
        for row in include_str!("fixtures/outer-binding.tsv").lines().skip(1) {
            let c: Vec<_> = row.split('\t').collect();
            let steps = c[1]
                .split(',')
                .map(|p| {
                    let (a, b) = p.split_once(':').unwrap();
                    (a.parse().unwrap(), b.parse().unwrap())
                })
                .collect();
            i.joins.push(Join {
                path: c[0].into(),
                plan: c[2].into(),
                steps,
                ..Default::default()
            });
        }
        assert_eq!(i.joins.len(), 2);
        let fs = analyze(&i);
        assert_eq!(
            fs.iter()
                .filter(|f| f.code == "JOIN_FANOUT" && f.evidence.contains("1342302187"))
                .count(),
            2
        );
        assert!(fs.iter().all(|f| f.cpu.is_none()));
    }
    #[test]
    fn historic_outer_binding_expansion_and_late_rejection_are_identified() {
        let i = Input {
            joins: vec![Join {
                path: "local/rr_agg59".into(),
                plan: "scope_ancestors -> bound_names -> nearest_scope".into(),
                steps: vec![(1, 19820), (19820, 1_342_302_187), (1_342_302_187, 1)],
                cpu: Some(217.614),
            }],
            ..Default::default()
        };
        let fs = analyze(&i);
        assert!(codes(&i).contains(&"JOIN_FANOUT"));
        assert!(codes(&i).contains(&"LATE_REJECTION"));
        assert!(
            fs.iter()
                .any(|f| f.evidence.contains("1342302187") && f.location.contains("rr_agg59"))
        );
    }
    #[test]
    fn late_sql_rewrite_cannot_hide_upstream_scalar_projection() {
        let i = Input {
            late_candidate_rewrite: true,
            projections: vec![Projection {
                nodes: 12_535_139,
                scalar_nodes: 12_535_139,
                strings: 1_000_000,
                reads: vec!["text".into()],
                reasons: vec!["text <- py_attribute <- async_site <- violation".into()],
            }],
            spans: vec![Span {
                path: "worker batch/project and intern".into(),
                own_cpu: 3.993,
                calls: 72,
            }],
            ..Default::default()
        };
        let f = analyze(&i)
            .into_iter()
            .find(|f| f.code == "LATE_PROJECTION")
            .unwrap();
        assert_eq!(f.cpu, Some(3.993));
        assert!(f.evidence.contains("violation"));
    }
    #[test]
    fn unread_scalars_are_a_confirmed_defect_but_requested_scalars_are_not() {
        let mut i = Input {
            projections: vec![Projection {
                nodes: 100,
                scalar_nodes: 100,
                reads: vec!["node".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(codes(&i).contains(&"UNREQUESTED_SCALARS"));
        i.projections[0].reads.push("text".into());
        assert!(!codes(&i).contains(&"UNREQUESTED_SCALARS"));
    }
    #[test]
    fn current_shape_clears_old_hazards_but_exposes_input_construction() {
        let i = Input {
            tape_hits: 5725,
            projections: vec![Projection {
                nodes: 12_535_139,
                scalar_nodes: 0,
                strings: 23777,
                ..Default::default()
            }],
            spans: vec![
                Span {
                    path: "tree-sitter and facts/facts from tape".into(),
                    own_cpu: 0.99,
                    calls: 5725,
                },
                Span {
                    path: "tree-sitter and facts/tape restore".into(),
                    own_cpu: 0.575,
                    calls: 5725,
                },
            ],
            ..Default::default()
        };
        let c = codes(&i);
        assert!(c.contains(&"CACHED_TAPE_REBUILD"));
        assert!(!c.contains(&"LATE_PROJECTION"));
        assert!(!c.contains(&"UNREQUESTED_SCALARS"));
        assert!(!c.contains(&"REPREPARE"));
    }
    #[test]
    fn recompilation_and_repeated_small_statements_have_distinct_actions() {
        let i = Input {
            reprepares: 71,
            sql: vec![Sql {
                path: "local".into(),
                text: "INSERT INTO tmp SELECT * FROM source".into(),
                runs: 4000,
                vm: 40000,
                ..Default::default()
            }],
            ..Default::default()
        };
        let c = codes(&i);
        assert!(c.contains(&"REPREPARE"));
        assert!(c.contains(&"REPEATED_STATEMENTS"));
    }
    #[test]
    fn empty_results_and_legitimate_large_scans_are_not_defects() {
        let i = Input {
            joins: vec![Join {
                steps: vec![(1, 1_000_000), (1_000_000, 1_000_000)],
                ..Default::default()
            }],
            sql: vec![Sql {
                runs: 1,
                vm: 5_000_000,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(analyze(&i).is_empty());
    }
    #[test]
    fn a_final_absence_result_is_not_evidence_of_a_movable_filter() {
        let i = Input {
            joins: vec![Join {
                steps: vec![(1, 1_000_000), (1_000_000, 0)],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(analyze(&i).is_empty());
    }
    #[test]
    fn automatic_index_does_not_assert_that_an_index_is_missing() {
        let i = Input {
            sql: vec![Sql {
                autoindex: 500000,
                text: "SELECT * FROM candidates JOIN facts USING (id)".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let f = analyze(&i).pop().unwrap();
        assert_eq!(f.code, "AUTOMATIC_INDEX");
        assert_eq!(f.confidence, "investigate");
        assert!(f.action.contains("demand"));
    }
}
