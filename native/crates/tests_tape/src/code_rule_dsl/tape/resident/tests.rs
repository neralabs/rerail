    use super::*;

    /// A stored format 16 tape read in place, as the device reads it (the reads written once in CLIF, decision 0310),
    /// is the restored tape: each node's code through its language's table (`doc_codes`), its span, its subtree and
    /// its parent.
    #[test]
    fn an_image_read_in_place_is_the_restored_tape() {
        crate::code_rule_dsl::tape::formats::as_run(crate::code_rule_dsl::tape::formats::Format::F16, format_16_images_read_in_place);
    }

    fn format_16_images_read_in_place() {
        use rerail_tape_format::reads::f16;
        for (source, lang) in [
            ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
            ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
            ("fn main() { let x: Vec<u8> = vec![1, 2]; if let Some(y) = x.first() { println!(\"{y}\"); } }\n", GrammarDialect::Rust),
            ("def broken(:\n    pass\n", GrammarDialect::Python),
        ] {
            let grammar = Grammar::new(lang).unwrap();
            let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
            let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
            let tape = TapeDoc::restore(Arc::new(source.to_owned().into()), Arc::clone(&grammar), &image, &|| false).unwrap();
            let bytes = tape.resident_parts(None).unwrap().0;
            assert!(matches!(stored(&bytes), Ok(Stored::F16)), "{lang:?}: a format 16 image");
            assert!(std::ptr::eq(lang_of(&bytes).unwrap(), tape.doc_tape().image().lang().unwrap()), "{lang:?}: its language's table");
            let codes = doc_codes(&tape).unwrap();
            let (t, v) = tape.doc_tape().in_place().unwrap();
            assert_eq!(v.nodes as usize, tape.transducer_len());
            for i in 0..v.nodes {
                let code = codes[f16::number(t, &v, i) as usize];
                assert_eq!(code.kind, tape.transducer_kind(i), "{lang:?} node {i} kind");
                assert_eq!(code.field, tape.transducer_field(i), "{lang:?} node {i} field");
                assert_eq!(code.flags, tape.transducer_flags(i), "{lang:?} node {i} flags");
                assert_eq!(f16::start(t, &v, i) as usize..f16::end(t, &v, i) as usize, tape.transducer_span(i), "{lang:?} node {i} span");
                assert_eq!(i + 1..i + f16::size(t, &v, i), tape.transducer_descendants(i), "{lang:?} node {i} size");
                let parent = f16::parent(t, &v, i);
                assert_eq!((parent != NONE).then_some(parent), tape.transducer_parent(i), "{lang:?} node {i} parent");
                let classes = tape.transducer_supertypes().filter(|&(node, _)| node == i).map(|(_, class)| class).collect::<Vec<_>>();
                assert_eq!(&code.classes[..usize::from(code.class_count)], classes.as_slice(), "{lang:?} node {i} classes");
            }
        }
    }
