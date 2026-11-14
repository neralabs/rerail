    #[test]
    fn statistics_sampling_is_bounded_and_reuses_known_counts() {
        use rerail_logic::code_rule_dsl::facts::vtab::Source;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Synthetic { reads: AtomicUsize, known: bool }
        impl Source for Synthetic {
            fn columns(&self) -> Vec<String> { vec!["n".into(), "kind".into()] }
            fn len(&self) -> usize { 1_000_000 }
            fn estimated_matches(&self, _:usize, _:bool)->usize { 1 }
            fn indexed(&self,_:usize)->bool { false }
            fn lookup(&self,_:usize,_:&[i64])->Option<Vec<usize>> { None }
            fn value(&self,r:usize,c:usize)->i64 { self.reads.fetch_add(1,Ordering::Relaxed); if c==0 {r as i64} else {7} }
            fn statistics(&self)->Option<Vec<(String,i64,i64,i64,i64)>> {
                self.known.then(||vec![("kind".into(),1_000_000,1,1_000_000_000_000,1_000_000_000_000)])
            }
        }
        let source=Synthetic { reads:AtomicUsize::new(0),known:false };
        let measured=rerail_logic::code_rule_dsl::logic::plan::corpus::sampled_statistics(&source);
        assert!(source.reads.load(Ordering::Relaxed)<=2*rerail_logic::code_rule_dsl::logic::plan::corpus::STAT_SAMPLE_ROWS,"sampling must not scan an execution chunk");
        assert_eq!(measured[0],("n".into(),1_000_000,1_000_000,1_000_000,1_000_000));
        assert_eq!(measured[1].1,1_000_000);
        assert_eq!(measured[1].2,1);
        assert_eq!(measured[1].3,1_000_000_000_000);
        let source=Synthetic { reads:AtomicUsize::new(0),known:true };
        assert_eq!(rerail_logic::code_rule_dsl::logic::plan::corpus::sampled_statistics(&source),source.statistics().unwrap());
        assert_eq!(source.reads.load(Ordering::Relaxed),0,"metadata must bypass sampling");
    }

