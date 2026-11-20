    use super::*;

    #[test]
    fn a_scoped_evaluation_reuses_its_tape_and_rejects_an_edited_source() {
        let language = crate::code_rule_dsl::language::dialect_by_label("python").unwrap();
        let mut file = FileInput::new(
            "a.py".into(),
            "python".into(),
            Arc::new("def first(): pass\n".into()),
            None,
            None,
            "test",
        )
        .unwrap();
        let _slot = TapeCacheSlot::install();
        let (_, first) = file_tape(language, &file).unwrap().unwrap();
        let (_, same) = file_tape(language, &file).unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        file.source = Arc::new("def second(): pass\n".into());
        let (_, edited) = file_tape(language, &file).unwrap().unwrap();
        assert!(!Arc::ptr_eq(&first, &edited));
    }
