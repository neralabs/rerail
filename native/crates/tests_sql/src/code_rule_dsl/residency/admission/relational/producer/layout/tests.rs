    use super::*;
    use crate::code_rule_dsl::{
        model::FileInput,
        residency::admission::relational::{
            producer::{Root, Tree},
            syntax::Facts,
        },
    };
    use std::sync::atomic::AtomicUsize;
    fn demand() -> Demand {
        let file = FileInput::new(
            "a.py".into(),
            "python".into(),
            Arc::new("def f(): return 1\n".into()),
            None,
            None,
            "test",
        )
        .unwrap();
        let facts = Facts::build(&file).unwrap();
        let visible = Arc::new((0..facts.subtree_ends().len() as u32).collect());
        Demand {
            python_ast: None,
            roots: vec![Root {
                file: 0,
                node: 0,
                expand: 1,
            }],
            trees: vec![Tree { facts, visible }],
            paths: vec![file.path],
            work: Arc::new(AtomicUsize::new(100_000)),
        }
    }
    #[test]
    fn capacity_admission_is_transactional_and_shared_columns_are_counted_once() {
        let d = demand();
        let mut layout = Footprint::default();
        assert!(layout.admit(&d, usize::MAX));
        let first = layout.bytes;
        assert!(!layout.admit(&d, first + 23));
        assert_eq!(layout.bytes, first);
        assert!(layout.admit(&d, first + 24));
        assert_eq!(layout.bytes, first + 24);
        let different = demand();
        assert!(!layout.admit(&different, first + 48));
        assert_eq!(layout.bytes, first + 24);
    }
