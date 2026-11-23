    use super::*;
    #[test]
    fn empty_stores_keep_grammar_constants_distinct_from_interned_names() {
        let p=".decl node(d:number,n:node,k:symbol,start:number,end:number)\n.decl field(p:node,f:symbol,c:node)\n.input node,field\n.decl kinds(k:symbol)\nkinds(\"integer\"). kinds(\"identifier\").\n.decl fields(f:symbol)\nfields(\"name\").\n.decl labels(s:symbol)\nlabels(\"integer\").\n.decl selected(n:node)\nselected(n) :- node(_,n,k,_,_), kinds(k).\nselected(n) :- field(n,f,_), fields(f).\n.output kinds,fields,labels,selected\n";
        for cold in [false,true] {
            let dir=tempfile::tempdir().unwrap();
            let mut e=CodeRuleSqlExecutor::new(dir.path().join("s.sqlite").to_string_lossy().into()).unwrap();
            if cold {e.use_cold_oracle();}
            e.bind_snapshot(json!({"files":[]}).to_string()).unwrap();
            e.run(p.into(),"python".into(),None).unwrap();
            let types=e.verification_types();
            assert_eq!(types["kinds"],vec!["kind"]);
            assert_eq!(types["fields"],vec!["field"]);
            assert_eq!(types["labels"],vec!["sym"]);
            for rel in ["kinds","fields","labels"] {
                for row in e.verification_rows(&format!("SELECT * FROM {rel}")).unwrap() {
                    let cell=e.verification_cell(&types[rel][0],row[0]).unwrap();
                    if rel=="labels" {assert_eq!(cell,json!({"symbol":"integer"}));}
                    else {assert!(cell.is_string());}
                }
            }
        }
    }
    #[test]
    fn document_and_symbol_cells_compare_across_different_store_local_ids() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut one =
            CodeRuleSqlExecutor::new(a.path().join("s.sqlite").to_string_lossy().into()).unwrap();
        let mut two =
            CodeRuleSqlExecutor::new(b.path().join("s.sqlite").to_string_lossy().into()).unwrap();
        let file =
            |path: &str, source: &str| json!({"path":path,"language":"python","source":source});
        let first = file("a.py", "alpha = 1\n");
        let second = file("b.py", "beta = 2\n");
        two.bind_snapshot(json!({"files":[file("z.py","reserved = 9\n")]}).to_string())
            .unwrap();
        let p = ".decl doc(d:number,path:symbol,module:symbol)\n.decl name(n:node,s:symbol)\n.decl node(d:number,n:node,k:symbol,start:number,end:number)\n.decl kind(n:node,k:symbol)\n.input doc,name,node,kind\n.decl seen(d:number,p:symbol)\nseen(d,p) :- doc(d,p,_).\n.decl names(n:node,s:symbol)\nnames(n,s) :- name(n,s).\n.decl kinds(k:symbol)\nkinds(k) :- kind(_,k).\n.output seen,names,kinds\n";
        for e in [&mut one, &mut two] {
            e.bind_snapshot(json!({"files":[first,second]}).to_string())
                .unwrap();
            e.run(p.into(), "python".into(), None).unwrap();
        }
        assert_ne!(one.verification_rows("SELECT n FROM tape").unwrap(), two.verification_rows("SELECT n FROM tape").unwrap());
        assert_eq!(one.verification_cell("node", Some(-1)).unwrap(), json!("-1"));
        assert_eq!(&one.verification_types()["tape"][..2], ["node", "node"]);
        assert_ne!(
            one.verification_rows("SELECT k FROM node_order").unwrap(),
            two.verification_rows("SELECT k FROM node_order").unwrap(),
        );
        assert_eq!(one.verification_types()["node_order"], ["node", "node_order"]);
        for relation in ["doc", "seen", "names", "kind", "kinds", "tape", "node_order"] {
            let read = |e: &CodeRuleSqlExecutor| {
                let types = e.verification_types();
                let mut rows = e
                    .verification_rows(&format!("SELECT * FROM {relation}"))
                    .unwrap()
                    .into_iter()
                    .map(|r| {
                        r.into_iter()
                            .enumerate()
                            .map(|(c, v)| e.verification_cell(&types[relation][c], v).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                rows.sort_by_key(|r| serde_json::to_string(r).unwrap());
                rows
            };
            assert_eq!(read(&one), read(&two), "{relation}");
        }
    }
