    use super::Reading;
    use crate::code_rule_dsl::language::GrammarDialect;

    #[test]
    fn independently_constructed_readings_have_distinct_cache_keys() {
        let first = Reading::new(GrammarDialect::TypeScript, None, None);
        let second = Reading::new(GrammarDialect::TypeScript, None, None);
        assert_ne!(first.cache_id, second.cache_id);
        assert_eq!(first.cache_id, first.clone().cache_id);
    }
