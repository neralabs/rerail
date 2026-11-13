    use super::*;
    use super::super::{
        behind_the_run, decision, device, executor, fallback, one_run, planned, reset, set, span, sql_in_wave, stratum, waves,
        RulePlan, StepPlan,
    };

    #[test]
    fn explain_shows_engines_plans_devices_and_decisions() {
        let _serial = one_run();
        set(true);
        reset();
        decision("xtopic", || "2 chunks".into());
        stratum("xstratum", "native set engine", || "every rule has a native step".into());
        stratum("xstratum", "native set engine", String::new);
        let plan = RulePlan {
            work: 10.0,
            rows: 2.0,
            subsets: 4,
            candidates: 6,
            steps: vec![
                StepPlan { literal: "a($x)".into(), access: "scan".into(), source: "input".into(), rows: 5.0, passed: Vec::new(), basis: "relation size".into() },
                StepPlan { literal: "b($x)".into(), access: "lookup on (c0)".into(), source: "derived".into(), rows: 2.0, passed: vec![("c($x)".into(), 7.0, "postings".into())], basis: "postings, containment 0.500".into() },
            ],
        };
        planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 0, Some(&[(1, 5), (5, 2)]), None);
        planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 1, Some(&[(1, 4), (4, 1)]), None);
        {
            let _s = span(|| "xspan".into());
            device("xkernel", 1, 0.002, 0.003, 100);
        }
        // A stratum that fell back to SQLite, and its span.
        stratum("xsql", "SQL wave kernel", || "the native engine declined".into());
        fallback("xsql", "aggregate join (string concatenation)".into());
        drop(span(|| "stratum xsql".into()));
        executor("xsql", "SQLite", Some("aggregate join (string concatenation)".into()), vec!["string join: string join".into()]);
        executor("xstratum", "native CPU", None, vec!["computed term: @sql_instr in a comparison".into()]);
        executor("xstratum", "Metal", None, Vec::new());
        // SQL before the waves is the run's beginning; inside them, from a wave's thread, a hazard; from the store's
        // writer (behind the run), none.
        sql_in_wave("run", "SELECT 1 FROM xbefore", 1_000);
        {
            let _w = waves();
            let _s = span(|| "xwave".into());
            sql_in_wave("store", "BEGIN IMMEDIATE", 2_000_000);
            std::thread::spawn(|| {
                behind_the_run();
                sql_in_wave("store", "INSERT INTO xbehind VALUES (1)", 1_000);
            })
            .join()
            .unwrap();
        }
        sql_in_wave("run", "SELECT 1 FROM xafter", 1_000);
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("explain");
        write(&dir, "header\n").unwrap();
        let report = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
        // The SQL waits under this test's span; other tests' runs may record theirs beside them (the profile is the
        // process's, and on while this test runs).
        let ours: Vec<_> = super::super::sql_waits().into_iter().filter(|(path, _, _, _)| path == "xwave").collect();
        set(false);
        assert!(report.contains("Stratum xstratum  [engine: native set engine]  (loops=2"), "{report}");
        assert!(report.contains("why: every rule has a native step"));
        // Counted over every run the profile saw: xsql's own line is below.
        assert!(report.contains("== Fell back to SQLite: "), "{report}");
        assert!(report.contains("== Forces SQL waits (hazards): "), "{report}");
        assert_eq!(ours, [("xwave".to_owned(), "store: BEGIN IMMEDIATE".to_owned(), 1, 2_000_000)], "{report}");
        assert!(report.contains("a write to the store on a wave's thread") && report.contains("xwave"), "{report}");
        assert!(!report.contains("xbefore") && !report.contains("xafter") && !report.contains("xbehind"), "{report}");
        assert!(report.contains("xsql: aggregate join (string concatenation)"), "{report}");
        // Other tests' strata may be recorded beside these (the profile is the process's): the heading alone.
        assert!(report.contains("== Executor hazards: what kept each stratum off Metal and off the native engine ("), "{report}");
        assert!(report.contains("native: aggregate join (string concatenation)") && report.contains("metal: computed term: @sql_instr in a comparison"), "{report}");
        assert!(report.contains("xstratum: Metal x1, native CPU x1; metal: computed term: @sql_instr in a comparison"), "{report}");
        assert!(report.contains("rule #0003 (seed xrule)") && report.contains("loops=2 replans=1"));
        assert!(report.contains("passed over c($x)") && report.contains("7.0 rows a binding  [postings]"), "the joins a step passed over: {report}");
        assert!(report.contains("[postings, containment 0.500]"), "the rule a step was priced by: {report}");
        // Actual rows add over the loops.
        assert!(report.contains("actual 9 -> 3"), "{report}");
        assert!(report.contains("xkernel  (xspan)"));
        assert!(report.contains("xtopic:\n  - 2 chunks"));
        let _ = std::fs::remove_dir_all(&dir);
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
        super::super::analysis_report::projection(&p, &std::collections::BTreeSet::from(["text".into()]));
        let decisions = super::super::decisions();
        super::super::set(false);
        assert!(
            decisions
                .keys()
                .any(|(topic, text)| topic == "projection provenance"
                    && text == "text <- selected <- result")
        );
    }
