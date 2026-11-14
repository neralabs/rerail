    use super::*;

    #[test]
    fn lines_follow_each_languages_terminators() {
        let py = Lines::new("a\r\nb\rc\n\nd", true);
        assert_eq!(py.count(), 5);
        assert_eq!((1..=5).map(|l| py.span(l)).collect::<Vec<_>>(), [[0, 1], [3, 4], [5, 6], [7, 7], [8, 9]]);
        assert_eq!(py.line(5), 3);
        let js = Lines::new("a\r\nb\rc\n", false);
        assert_eq!(js.count(), 2);
        assert_eq!(js.span(1), [0, 1]);
        assert_eq!(js.span(2), [3, 6]);
        assert_eq!(Lines::new("", false).count(), 1);
        assert_eq!(Lines::new("", true).count(), 0);
        // A node ending at a line's start ends on the line before.
        assert_eq!(js.place([0, 3]), (1, 1, 0));
        assert_eq!(js.place([4, 5]), (2, 2, 1));
    }

    #[test]
    fn literals_decode_per_language() {
        let lit = |f, k, t| literal(f, k, t, false);
        assert_eq!(lit(Family::Python, "string", r#""a\nb""#).str.as_deref(), Some("a\nb"));
        assert_eq!(lit(Family::Python, "string", r#"r"a\nb""#).str.as_deref(), Some("a\\nb"));
        let f = lit(Family::Python, "string", r#"F"x""#);
        assert_eq!((f.prefix.as_deref(), f.str.as_deref(), f.value), (Some("f"), Some("x"), None));
        assert_eq!(literal(Family::Python, "string", r#"f"x{y}""#, true).str, None);
        assert_eq!(lit(Family::Python, "integer", "0xff").value, Some(("255".to_owned(), "int")));
        assert_eq!(lit(Family::Ecma, "string", r"'aA'").str.as_deref(), Some("aA"));
        assert_eq!(lit(Family::Ecma, "template_string", "`a\\tb`").value, Some(("a\tb".to_owned(), "string")));
        assert_eq!(lit(Family::Ecma, "number", "1_000").value, Some(("1000".to_owned(), "number")));
        assert_eq!(lit(Family::Rust, "raw_string_literal", r##"r#"a\n"#"##).str.as_deref(), Some("a\\n"));
        assert_eq!(lit(Family::Rust, "integer_literal", "0x10u8").value, Some(("16".to_owned(), "int")));
        assert_eq!(lit(Family::Php, "encapsed_string", r#""a\tb""#).str.as_deref(), Some("a\tb"));
        assert_eq!(name("attribute", "a . b"), Some("a.b".to_owned()));
        assert_eq!(name("member_expression", "a(b).c"), None);
        assert_eq!(comment_body("# noqa: E501 "), "noqa: E501");
        assert_eq!(comment_body("/* x */"), "x");
    }
