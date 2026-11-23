    use super::*;
    use super::super::tests::parse;

    #[test]
    fn a_present_location_uses_the_edge_shared_by_its_producers() {
        let v = parse(
            r#"
            .decl field(p: node, name: symbol, c: node)
            .decl missing(p: node)
            .input field, missing
            .decl parts(p: node, a: loc)
            parts(p, a) :- field(p, "type", a).
            parts(p, a) :- missing(p), unset(a).
            .decl signature(p: node, a: loc)
            signature(p, a) :- parts(p, a).
            .decl present(a: loc)
            .output present
            present(a) :- signature(_, a), !unset(a).
        "#,
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert_eq!(restore(&mut p).unwrap(), 1);
        let present = p.shown(Some("present")).remove(0);
        assert!(present.contains("field($rr_present"), "{present}");
        assert!(present.contains("signature($rr_present"), "{present}");
    }

    #[test]
    fn an_alternative_location_without_the_edge_blocks_the_proof() {
        let v = parse(
            r#"
            .decl field(p: node, name: symbol, c: node)
            .decl other(p: node, c: node)
            .input field, other
            .decl parts(p: node, a: loc)
            parts(p, a) :- field(p, "type", a).
            parts(p, a) :- other(p, a).
            .decl present(a: loc)
            .output present
            present(a) :- parts(_, a), !unset(a).
        "#,
        )
        .unwrap();
        let mut p = Program::of(&v);
        assert_eq!(restore(&mut p).unwrap(), 0);
        assert!(!p.changed());
    }
