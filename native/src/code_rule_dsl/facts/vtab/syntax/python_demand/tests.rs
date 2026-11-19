    use super::super::{Document, Projection};
    use super::*;
use crate::code_rule_dsl::facts::producers::python::families as py;
    use crate::{
        code_rule_dsl::{facts::core, model::FileInput},
        content_store::ContentStore,
    };
    #[test]
    fn selective_batches_cache_only_admitted_nodes_and_preserve_document_identity() {
        let store_path = crate::code_rule_dsl::fixtures::temp();
        let store = ContentStore::open(&store_path).unwrap();
        let sources = [
            "import requests as r\nasync def f():\n r.get('url')\n irrelevant(1,2,3)\n",
            "import requests as r\nasync def f(r):\n r.get('url')\n",
            "async def f():\n irrelevant(1,2,3)\n",
        ];
        let docs = sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let path = format!("d{i}.py");
                let input = FileInput::new(
                    path.clone(),
                    "python".into(),
                    Arc::new(s.to_string()),
                    None,
                    None,
                    "core",
                )
                .unwrap();
                let core::Built::Facts(f) = core::build(&input).unwrap() else {
                    panic!("parse")
                };
                let facts = Arc::new(f);
                Document {
                    key: i as u32,
                    core: format!("/w#core:{path}"),
                    digest: core::digest(&facts),
                    path,
                    facts,
                    guest: None,
                    module: None,
                    tape: None,
                }
            })
            .collect();
        let (batch, _, _, _) = Batch::build(&store, docs, &Projection::default(), &[], &[], false).unwrap();
        let grammar = &batch.bound::<super::super::py_families::Py>().unwrap().grammar;
        let mut requests = Vec::new();
        let mut calls = Vec::new();
        for p in &batch.docs {
            for [call, callee] in py::structure(grammar, &p.facts).call {
                requests.push(p.global(p.local[callee as usize]));
                if p.key == 0 && p.facts.text(callee) == "r.get" {
                    calls.push(p.global(p.local[call as usize]));
                }
            }
        }
        let target = BTreeSet::from(["requests.get".into()]);
        let rows = batch
            .python_project("qualified", &requests, &target)
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "only the unshadowed first document qualifies"
        );
        assert!(matches!(rows[0][0],Value::Integer(n) if n>>32==0));
        for (i, _) in batch.docs.iter().enumerate() {
            let state = batch.bound::<super::super::py_families::Py>().unwrap().semantics[i].lock().unwrap();
            if i == 2 {
                assert!(state.is_none(), "screened document built scopes");
            } else {
                assert_eq!(
                    state.as_ref().unwrap().facts.len(),
                    1,
                    "unrelated call resolved"
                );
            }
        }
        assert_eq!(
            batch
                .python_project("qualified", &requests, &target)
                .unwrap(),
            rows
        );
        assert!(batch.bound::<super::super::py_families::Py>().unwrap().argument_rows.get().is_none());
        let args = batch
            .python_project("arguments", &calls, &BTreeSet::new())
            .unwrap();
        assert_eq!(args.len(), 1, "arguments from undemanded calls leaked");
        assert!(
            batch.bound::<super::super::py_families::Py>().unwrap().argument_rows.get().is_none(),
            "demand filled the global argument relation"
        );
        assert!(
            batch
                .python_project("arguments", &[], &target)
                .unwrap()
                .is_empty()
        );
    }
