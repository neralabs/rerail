    use super::*;
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
        for relation in ["doc", "seen", "names", "kind", "kinds"] {
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
