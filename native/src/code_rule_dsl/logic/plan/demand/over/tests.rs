    use super::{R, base, disconnected, per_node_relations, view_of};
    use super::super::super::body::Placed;
    use super::super::super::fields::Fields;

    /// Whether rule `k` of `text` falls apart once its reads of `cut` are cut. Its variables are written `$x`, as the
    /// planner shows them; the `.rerail` text names them without the `$`.
    fn apart(text: &str, k: usize, cut: &str) -> bool {
        let p = Placed::whole(super::super::super::super::rr::parse_version(&text.replace('$', "")).unwrap());
        let r = R::of(&p, k);
        disconnected(&r, &|j| r.plain(j).is_some_and(|a| r.rel(a) == cut))
    }

    /// A relation of a row per node, through another (`mf_def` over `mf_up`), is per node; one that tests its key
    /// (`ctor`) or reads it off a per-node relation's key (`by_def`) is not.
    #[test]
    fn relations_of_a_row_per_node_are_found_through_others() {
        let p = r#"
            .decl node(d: number, n: node, k: number, s: number, e: number)
            .decl kind(n: node, k: symbol)
            .decl up(n: node, o: node)
            .decl def(n: node, f: node)
            .decl ctor(n: node)
            .decl by_def(f: node, n: node)
            .decl rr_nearest_ks1(n: node, a: node)
            .decl near(n: node, o: node)
            .input node, kind, rr_nearest_ks1
            .output def, ctor, by_def, near
            up(n, o) :- node(_, n, _, _, _), kind(o, "def").
            def(n, f) :- up(n, f), kind(f, "function_definition").
            ctor(c) :- up(c, _), kind(c, "call").
            by_def(f, n) :- def(n, f).
            near(n, o) :- rr_nearest_ks1(n, o), !kind(o, "decorator").
        "#;
        let prog = crate::code_rule_dsl::logic::rr::parse_version(p).unwrap();
        let placed = Placed::whole(prog.clone());
        let rules: Vec<R<'_>> = (0..placed.len()).map(|k| R::of(&placed, k)).collect();
        let got = per_node_relations(&rules);
        assert!(got.contains("up") && got.contains("def") && got.contains("near"), "{got:?}");
        assert!(!got.contains("ctor") && !got.contains("by_def"), "{got:?}");
    }

    /// A split whose kept atoms only the demanded read joins is refused: its `pre_<i>` would be their cross product.
    #[test]
    fn kept_atoms_that_only_the_cut_joins_are_disconnected() {
        let p = r#"
            .decl body(f: node, n: node)
            .decl unp(e: node, m: node)
            .decl d(n: node, e: node)
            .decl out(f: node, e: node)
            .input body, unp, d
            .output out
            out($f, $e) :- body($f, $n), unp($e, $m), d($n, $e).
            out($f, $e) :- body($f, $n), unp($e, $n), d($n, $e).
            "#;
        assert!(apart(p, 0, "d"), "joined by the read alone");
        assert!(!apart(p, 1, "d"), "joined by n");
    }

    /// Atoms joined only through their documents are apart (O136): cutting `uinst` would leave every annotation of
    /// every reached document against every `anc` row.
    #[test]
    fn atoms_joined_only_by_their_documents_are_disconnected() {
        let p = r#"
            .decl ann(f: node, x: number, t: node)
            .decl uinst(t: node, c: node)
            .decl anc(s: node, c: node)
            .decl docof(n: node, d: number)
            .decl rr_cx_reach(d: number, e: number)
            .decl bv(f: node, x: number, s: node)
            .input ann, uinst, anc, docof, rr_cx_reach
            .output bv
            bv($f, $x, $s) :- ann($f, $x, $t), uinst($t, $c), anc($s, $c), docof($f, $d9), docof($s, $e9), rr_cx_reach($d9, $e9).
            "#;
        assert!(apart(p, 0, "uinst"), "cutting uinst leaves a product by documents");
        assert!(!apart(p, 0, "rr_cx_reach"), "the rest still joins by t and c");
    }

    /// A view of an input's columns (Q37) is a base relation to the split, read as its input's columns: `kind(o, 7)`
    /// is `node(_, o, 7, _, _)`, whose postings bound the scan. Taken as derived, a definition's `tp_name` was large by
    /// its `field` atom alone, and demanded (work queue Q88's measurement).
    #[test]
    fn a_view_of_an_input_reads_as_the_inputs_columns() {
        let p = r#"
            .decl node(d: number, n: node, kind: number, s: number, e: number)
            .decl field(p: node, f: number, c: node)
            .decl kind(n: node, k: number)
            .decl named(o: node, c: node)
            .decl out(o: node)
            .input node, field
            .output out
            kind($n, $k) :- node(_, $n, $k, _, _).
            named($o, $c) :- kind($o, 7), field($o, 3, $c).
            out($o) :- named($o, _).
            "#;
        let placed = Placed::whole(super::super::super::super::rr::parse_version(&p.replace('$', "")).unwrap());
        let f = Fields::of(&placed).unwrap();
        let k = (0..placed.len()).find(|&k| R::of(&placed, k).head() == "named").unwrap();
        let r = R::of(&placed, k);
        let atom = |rel: &str| (0..r.lits.len()).find_map(|i| r.plain(i).filter(|a| r.rel(*a) == rel)).unwrap();
        assert_eq!(view_of(&f, atom("kind")), Some(("node".to_owned(), rerail_blocks::vector![1, 2])), "kind's columns are node's 1 and 2");
        assert!(base(&f, "kind", atom("kind")), "a view of an input is a base relation");
        assert_eq!(view_of(&f, atom("field")), None, "an input is no view");
        let n = (0..placed.len()).find(|&k| R::of(&placed, k).head() == "out").unwrap();
        let o = R::of(&placed, n);
        let named = (0..o.lits.len()).find_map(|i| o.plain(i)).unwrap();
        assert!(!base(&f, "named", named), "a derived relation is not");
    }

    /// A part that meets the rest only at a document the cut read binds is apart: cutting `docof` leaves `ex($d)` a
    /// row for every document against every `walk` row (python/calls' `builtin_ref`, its `exists_0($d)` simplify's).
    /// Kept, `docof` ties `$d` to `$n`, and the part only filters.
    #[test]
    fn a_part_that_meets_the_rest_only_at_a_cut_document_is_disconnected() {
        let p = r#"
            .decl walk(n: node, x: number, m: node)
            .decl builtin(x: number)
            .decl docof(n: node, d: number)
            .decl ex(d: number)
            .decl out(n: node, x: number)
            .input walk, builtin, docof, ex
            .output out
            out($n, $x) :- walk($n, $x, $m), builtin($x), docof($n, $d), ex($d).
            "#;
        assert!(apart(p, 0, "docof"), "cutting docof leaves ex($d) a product with every document");
        assert!(!apart(p, 0, "builtin"), "docof kept ties $d to $n");
    }

    /// A tie carries through an atom of documents only: `docof` ties `$d`, `rr_cx_reach($d, $e)` then ties `$e`, and
    /// `ex($e)` only filters. With `docof` cut, nothing ties `$d` and the chain is a part of its own.
    #[test]
    fn a_tie_carries_through_an_atom_of_documents_only() {
        let p = r#"
            .decl walk(n: node, x: number, m: node)
            .decl builtin(x: number)
            .decl docof(n: node, d: number)
            .decl rr_cx_reach(d: number, e: number)
            .decl ex(d: number)
            .decl out(n: node, x: number)
            .input walk, builtin, docof, rr_cx_reach, ex
            .output out
            out($n, $x) :- walk($n, $x, $m), builtin($x), docof($n, $d), rr_cx_reach($d, $e), ex($e).
            "#;
        assert!(!apart(p, 0, "builtin"), "docof ties $d, rr_cx_reach ties $e");
        assert!(apart(p, 0, "docof"), "nothing kept ties $d");
    }
