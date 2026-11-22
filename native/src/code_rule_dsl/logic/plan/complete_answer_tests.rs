    use super::*;

    #[test]
    fn complete_answers_resume_deferred_batches_before_consumers() {
        // Source witness: plan_scope_side_effects.scan_repo must load an external file link.
        // Its load requests are already complete, so loaded is in a later stratum, not their SCC.
        // Exercise that execution path with a whole demand batch, both all-held and mixed answers.
        for immediate in [0, 1] {
            use super::super::program_image::fields::FieldsMut as _;
            let read = super::super::rr::parse_version(".decl seed(x:int)\n.decl req(x:int)\n.decl got(x:int)\n.decl shown(x:int)\nreq(x) :- seed(x).\nshown(x) :- got(x).\n.output shown\n").unwrap();
            let mut b = super::super::program_image::Builder::on(&read);
            b.fields().set_facts("seed", &(0..1000).map(|n| super::super::front::List::from(vec![Value::Int(n)])).collect::<Vec<_>>());
            b.fields().set_field(super::super::program_facts::Field::Answers(&BTreeMap::from([("req".to_owned(), vec!["got".to_owned()])])));
            let (program, _) = b.finish().unwrap();
            let lowered = lower::Lowered::of_placed(body::Placed::whole(program.clone()), lower::domains_of(&program).unwrap(), [0; 3], crate::code_rule_dsl::language::GrammarDialect::Python);
            let path = crate::code_rule_dsl::fixtures::temp();
            let _store = crate::content_store::ContentStore::open(&path).unwrap();
            let db = Connection::open(&path).unwrap();
            let mut batches = Vec::new();
            let mut answer = |rel: &str, keys: &[Vec<i64>]| -> Result<Vec<(String, Vec<Vec<i64>>)>> {
                assert_eq!(rel, "req");
                batches.push(keys.to_vec());
                if batches.len() == 1 {
                    Ok(vec![("got".into(), keys[..immediate].to_vec()),
                        (format!("{}req", super::super::answers::HELD_BACK), keys[immediate..].to_vec())])
                } else { Ok(vec![("got".into(), keys.to_vec())]) }
            };
            let result = run_atoms_native_answering(&lowered, &db, "main", Stats::empty_of_lowered(&lowered).unwrap(), BTreeSet::new(), "program", &mut answer, None, None, None).unwrap();
            assert_eq!(result.outputs["shown"].len(), 1000, "later consumers must see all deferred positives");
            assert_eq!(result.outputs["shown"].shown().iter().map(|r| r[0].parse::<i64>().unwrap()).collect::<BTreeSet<_>>(), (0..1000).collect(), "every returned value stays exact");
            assert_eq!(batches.len(), 2, "held keys resume as one batch, never one callback per key");
            assert_eq!(batches[0].len(), 1000);
            assert_eq!(batches[1].len(), 1000-immediate);
            assert_eq!(batches[1], batches[0][immediate..], "the complete deferred payload resumes unchanged");
        }
    }
