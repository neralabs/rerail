//! Native parity covers both format sides, incomplete candidates and reached work.
use super::publication_tests::check_work;
use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
use serde_json::{Value, json};
fn policy(rule: Value) -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    CodeRuleDslRuntime::compile(json!({"format":"rerail.policy/v1","id":"format-outcomes","revision":1,"mode":"steer","clauses":[{"id":"c0","forbid":{"ast":{"language":"python","rule":rule}},"violation":{"code":"C0","message":"match","anchor":"witness.span"}}]}).to_string(),"json".into(),CAPABILITIES.into()).unwrap()
}
#[test]
fn format_candidate_sides_match_native_with_splats_and_invalid_templates() {
    for (pattern, style, args, sources) in [
        (
            "$S.format($$$A)",
            "brace",
            "A",
            vec![
                "'{}'.format(1, 2)\n'{a}'.format(a=1,b=2)\n'{x}'.format(**dynamic)\n'{' .format(1)\n",
                "'{2}'.format(1,*xs)\n'{}'.format(1)\n'{}'.format()\n",
            ],
        ),
        (
            "$S % $R",
            "percent",
            "R",
            vec![
                "'%s' % (1, 2)\n'%(x)s' % {'y': 1}\n'%(x)s' % dynamic\n'%s' % ()\n",
                "'%*s' % (1,)\n'%s' % (1,)\n'%' % 1\n",
            ],
        ),
    ] {
        for side in [
            json!({"unused":{"regex":".*"}}),
            json!({"missing":{"regex":".*"}}),
            json!({"unused":{"regex":".*"},"missing":{"regex":".*"}}),
        ] {
            let mut format = side;
            format["style"] = json!(style);
            format["string"] = json!("S");
            format["args"] = json!(args);
            let rules = [json!({"pattern":pattern,"formatArgs":format})];
            let p = policy(rules[0].clone());
            for work in [0, 1, 2, 3, 8, 64, 1000] {
                check_work(&p, &rules, &sources, None, work);
            }
        }
    }
}
#[test]
fn format_unused_candidates_rollback_unknown_and_commit_first_known_captures() {
    for unused in [
        json!({"pattern":"$X"}),
        json!({"pattern":"$X","testUnknown":"^u"}),
        json!({"any":[{"all":[{"pattern":"$X"},{"regex":"^never$"}]},{"pattern":"$Y"}]}),
    ] {
        let rules = [
            json!({"pattern":"$S.format($$$A)","formatArgs":{"style":"brace","string":"S","args":"A","unused":unused}}),
        ];
        let p = policy(rules[0].clone());
        for work in [0, 1, 3, 16, 128] {
            check_work(
                &p,
                &rules,
                &["'{}'.format(1,u,a)\n'{}'.format(1,a,b)\n"],
                None,
                work,
            );
        }
    }
}
#[test]
fn format_candidate_count_scales_vm_work_without_sql_growth() {
    let rules = [
        json!({"pattern":"$S.format($$$A)","formatArgs":{"style":"brace","string":"S","args":"A","unused":{"regex":"^never$"}}}),
    ];
    let p = policy(rules[0].clone());
    let mut samples = Vec::new();
    for size in [8, 16, 32] {
        let source = format!("''.format({})\n", vec!["1"; size].join(","));
        let r = check_work(&p, &rules, &[&source], None, 10000);
        samples.push((
            r["sql"].as_str().unwrap().len(),
            r["metrics"]["execution"]["vmSteps"].as_u64().unwrap(),
        ));
    }
    for pair in samples.windows(2) {
        assert_eq!(pair[0].0, pair[1].0);
        assert!(pair[1].1 <= 3 * pair[0].1);
    }
}
