    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::literal::python_string::decode_python;
    use ast_grep_core::tree_sitter::LanguageExt;

    fn decode(source: &str) -> Option<Decoded> {
        let root = GrammarDialect::Python.ast_grep(source);
        let node = root
            .root()
            .dfs()
            .find(|node| matches!(node.kind().as_ref(), "string" | "concatenated_string"))
            .expect("a string");
        match decode_python(&node) {
            Decode::Known(decoded) => Some(decoded),
            _ => None,
        }
    }

    /// Each decoded character with the source text it came from.
    fn pieces(source: &str) -> Vec<(char, String)> {
        let decoded = decode(source).expect("decodes");
        decoded
            .text
            .chars()
            .zip(&decoded.spans)
            .map(|(c, span)| (c, source[span.start as usize..span.end as usize].to_owned()))
            .collect()
    }

    #[test]
    fn escapes_map_to_the_whole_escape() {
        assert_eq!(
            pieces(r#"x = "a\x41\n\N{BULLET}é\101\q""#),
            [
                ('a', "a".to_owned()),
                ('A', r"\x41".to_owned()),
                ('\n', r"\n".to_owned()),
                ('•', r"\N{BULLET}".to_owned()),
                ('é', r"é".to_owned()),
                ('A', r"\101".to_owned()),
                ('\\', r"\".to_owned()),
                ('q', "q".to_owned()),
            ]
        );
        // A line continuation decodes to nothing; CRLF is one newline.
        assert_eq!(
            pieces("x = '''a\\\nb\r\nc'''"),
            [
                ('a', "a".to_owned()),
                ('b', "b".to_owned()),
                ('\n', "\r\n".to_owned()),
                ('c', "c".to_owned()),
            ]
        );
    }

    #[test]
    fn implicit_concatenation_chains_the_parts() {
        let source = "x = ('a' r'\\n' \"b\")";
        assert_eq!(
            pieces(source),
            [
                ('a', "a".to_owned()),
                ('\\', "\\".to_owned()),
                ('n', "n".to_owned()),
                ('b', "b".to_owned()),
            ]
        );
        let decoded = decode(source).expect("decodes");
        assert_eq!(decoded.simple, None);
        // A range maps from its first character's start to its last one's end.
        let range = decoded.host_range(1..3);
        assert_eq!(&source[range], "\\n");
        assert_eq!(&source[decoded.host_range(4..4)], "");
        assert_eq!(decoded.host_range(4..4).start, source.len() - 2);
    }

    #[test]
    fn simple_strings_are_single_parts_whose_source_is_their_value() {
        let decoded = decode("x: 'List[int]'").expect("decodes");
        assert_eq!(
            decoded.simple,
            Some(Simple {
                content: 4,
                triple: false
            })
        );
        assert_eq!(
            decode("x: \"\"\"int\"\"\"")
                .expect("decodes")
                .simple
                .map(|s| s.triple),
            Some(true)
        );
        assert_eq!(decode(r#"x: "a\tb""#).expect("decodes").simple, None);
        assert_eq!(
            decode(r#"x: r"a\tb""#)
                .expect("decodes")
                .simple
                .map(|s| s.content),
            Some(5)
        );
    }

    #[test]
    fn unknown_strings() {
        assert!(decode("x = f'{a}'").is_none());
        assert!(decode("x = b'a'").is_none());
        assert!(decode(r"x = '\N{NO SUCH NAME}'").is_none());
        assert!(decode(r"x = '\x4'").is_none());
        // An f-string without fields decodes its doubled braces.
        assert_eq!(
            pieces("x = f'{{a}}'"),
            [
                ('{', "{{".to_owned()),
                ('a', "a".to_owned()),
                ('}', "}}".to_owned()),
            ]
        );
    }
