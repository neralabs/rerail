    use super::*;
    use crate::code_rule_dsl::{CodeRuleDslRuntime, budget::Context, model::FileInput};
    use std::{
        collections::BTreeSet,
        sync::{Arc, atomic::AtomicBool},
    };
    fn rule() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
        CodeRuleDslRuntime::compile(serde_json::json!({"format":"rerail.policy/v1","id":"gate","revision":1,"mode":"log","clauses":[{"id":"gate","relational":{"requireNonemptyInput":"gate","inputs":{"gate":{"evidence":{"as":"n","query":{"ast":{"language":"python","syntax":"valid","rule":{"kind":"module"}}},"project":{"path":"file.path"}}}},"query":"SELECT __path path,__start start_byte,__end end_byte FROM gate"},"violation":{"code":"GATE","message":"Gate","anchor":"witness.span"}}]}).to_string(),"json".into(),include_str!("../../../../capabilities.json").into()).unwrap()
    }
    fn cached() -> Inputs {
        Inputs {
            gate: Some(eval::evidence::Rows::default()),
            rows: None,
            facts: vec![],
            file_local: false,
            read_bytes: 0,
            snapshot: Snapshot {
                revision: 1,
                files: vec![
                    FileInput::new(
                        "a.py".into(),
                        "python".into(),
                        Arc::new("pass\n".into()),
                        None,
                        None,
                        "test",
                    )
                    .unwrap(),
                ],
            },
        }
    }
    #[test]
    fn published_empty_evidence_does_not_repeat_matching_and_unknown_evidence_fails() {
        let rule = rule();
        let clause = &rule.inner.clauses[0];
        let mut inputs = cached();
        let mut metrics = Metrics::default();
        assert!(!check(&rule.inner, clause, &mut inputs, &mut metrics).unwrap());
        assert_eq!(metrics.visited_nodes, 0);
        inputs.gate.as_mut().unwrap().incomplete = BTreeSet::from(["a.py".into()]);
        assert!(check(&rule.inner, clause, &mut inputs, &mut metrics).is_err());
        assert_eq!(metrics.visited_nodes, 0);
    }
    #[test]
    fn cancellation_is_not_successful_emptiness_and_retry_uses_a_new_context() {
        let rule = rule();
        let clause = &rule.inner.clauses[0];
        let mut inputs = cached();
        let mut metrics = Metrics::default();
        let mut cancelled = Context::work(&Arc::new(AtomicBool::new(true)), 100);
        assert!(
            cancelled
                .run(|| check(&rule.inner, clause, &mut inputs, &mut metrics))
                .is_err()
        );
        assert!(cancelled.observed());
        let mut retry = Context::work(&Arc::new(AtomicBool::new(false)), 100);
        assert!(
            !retry
                .run(|| check(&rule.inner, clause, &mut inputs, &mut metrics))
                .unwrap()
        );
        assert!(!retry.observed());
    }
