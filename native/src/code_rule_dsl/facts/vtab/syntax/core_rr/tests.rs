    use super::*;

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
