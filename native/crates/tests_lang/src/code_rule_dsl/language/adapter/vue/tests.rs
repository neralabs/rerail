    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::Doc;
    use std::sync::Arc;

    /// A component's embedded program is a document in the dialect its script's `lang` names
    /// (`<script setup>`'s first).
    #[test]
    fn a_components_program_is_read_in_its_scripts_lang() {
        for (source, dialect) in [
            (
                "<script>\nlet x = 1;\n</script>\n",
                GrammarDialect::JavaScript,
            ),
            (
                "<script lang=\"jsx\">\nlet x = 1;\n</script>\n",
                GrammarDialect::JavaScript,
            ),
            (
                "<script lang=\"ts\">\nlet x = 1;\n</script>\n",
                GrammarDialect::TypeScript,
            ),
            (
                "<script lang='tsx'>\nlet x = <a />;\n</script>\n",
                GrammarDialect::Tsx,
            ),
            (
                "<script>\nlet y = 1;\n</script>\n<script setup lang=\"ts\">\nlet x = 1;\n</script>\n",
                GrammarDialect::TypeScript,
            ),
            ("<template>{{ x }}</template>\n", GrammarDialect::JavaScript),
        ] {
            let grammar = Grammar::new(GrammarDialect::Vue).expect("grammar");
            let image = TapeImage::parse(&grammar, source, &|| false)
                .expect("parses")
                .expect("encodes");
            let doc = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                .expect("restores");
            let dialects = doc
                .guests()
                .expect("owned")
                .iter()
                .map(|guest| *guest.doc.get_lang())
                .collect::<Vec<_>>();
            assert_eq!(dialects, [dialect], "{source:?}");
        }
    }
