    use super::*;
    use crate::code_rule_dsl::logic::plan::{corpus, output::Outputs};
    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
    use crate::code_rule_dsl::typing::file_local::Binding;

    const SOURCE: &str = "
        .decl lhs(a: number, k: number)
        .decl rhs(b: number, k: number)
        .decl enabled(b: number)
        .decl request(a: number, b: number)
        .decl wide(a: number, b: number)
        wide(a,b) :- lhs(a,k), rhs(b,k), enabled(b).
        lhs(1,10). lhs(1,20). lhs(2,10). lhs(3,99).
        rhs(1,10). rhs(1,20). rhs(2,99). rhs(3,10). rhs(7,77).
        enabled(1). enabled(2). enabled(7).
        request(1,1). request(2,1). request(3,1). request(4,1).
        request(5,1). request(6,1). request(7,1). request(8,1).
        request(3,2). request(1,3). request(1,7).
    ";

    fn parse(text: &str) -> Arc<Version> {
        crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()
    }

    /// `v` with the pass run on it: the version it leaves, and what it did.
    fn rewritten(v: &Arc<Version>) -> (Arc<Version>, Plans) {
        let mut p = Program::of(v);
        let plans = rewrite(&mut p, v, true, true).unwrap();
        (p.made.clone().unwrap_or_else(|| Arc::clone(v)), plans)
    }

    /// `v` with `rels` outputs too.
    fn with_outputs(v: &Arc<Version>, rels: &[&str]) -> Arc<Version> {
        use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
        let mut b = crate::code_rule_dsl::logic::program_image::Builder::on(v);
        for rel in rels {
            b.fields().add_output(rel);
        }
        b.finish().unwrap().0
    }

    fn execute(v: &Arc<Version>, case: &str) -> BTreeMap<String, Vec<Vec<String>>> {
        let root = crate::code_rule_dsl::fixtures::temp().with_extension(case);
        std::fs::create_dir_all(&root).unwrap();
        let out = root.join("out");
        corpus::run_read(v, &root, &out, "program", Binding::Cpu, 4096, &Default::default(), &serde_json::Value::Null).unwrap();
        let outputs = Outputs::open(&out).unwrap();
        let rows = F::output_names(&**v)
            .into_iter()
            .map(|rel| {
                let mut rows = outputs.get(&rel).map(|r| r.shown()).unwrap_or_default();
                rows.sort();
                (rel, rows)
            })
            .collect();
        drop(outputs);
        std::fs::remove_dir_all(root).unwrap();
        rows
    }

    fn declared(v: &Version, suffix: &str) -> String {
        F::declared_names(v).into_iter().find(|r| r.ends_with(suffix)).unwrap()
    }

    #[test]
    fn hot_and_cold_requests_share_only_their_own_answers() {
        let v = parse(&format!(
            "{SOURCE}
            .decl yes(a: number, b: number)
            .decl missing(a: number, b: number)
            .output yes, missing
            yes(a,b) :- request(a,b), wide(a,b).
            missing(a,b) :- request(a,b), !wide(a,b).
        "
        ));
        let expected = execute(&v, "deferred-control");
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.pairs, ["wide"]);
        assert_eq!(plans.shared.len(), 1);
        let (hot, factor) = (declared(&made, "_hot"), declared(&made, "_factor"));
        let made = with_outputs(&made, &[&hot, &factor]);
        let mut actual = execute(&made, "deferred-adaptive");
        assert_eq!(actual.remove(&hot).unwrap(), [["1"]]);
        assert_eq!(actual.remove(&factor).unwrap(), [["1", "10"], ["1", "20"]]);
        assert_eq!(actual, expected);
        assert_eq!(actual["yes"], [["1", "1"], ["2", "1"], ["3", "2"]]);
        assert_eq!(actual["missing"].len(), 8);
    }

    #[test]
    fn broad_readers_share_reduced_factors_and_fresh_witnesses() {
        let v = parse(&format!(
            "{SOURCE}
            .decl pairs(a: number, b: number)
            .decl diagonal(a: number)
            .output pairs, diagonal
            pairs(a,b) :- wide(a,b), enabled(fz1_k).
            diagonal(a) :- wide(a,a), wide(a,_).
        "
        ));
        let expected = execute(&v, "deferred-broad-control");
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.factors.len(), 1);
        assert!(F::columns_of(&*made, "wide").is_none());
        assert_eq!(execute(&made, "deferred-broad"), expected);
        assert_eq!(expected["pairs"].len(), 3);
    }

    #[test]
    fn one_requested_endpoint_can_stream_the_other_factor() {
        let v = parse(&format!(
            "{SOURCE}
            .decl selected(a: number,b: number)
            .output selected
            selected(a,b) :- enabled(a), wide(a,b).
        "
        ));
        let (made, plans) = rewritten(&v);
        assert_eq!(plans.one_sided, ["wide column 0"]);
        assert_eq!(plans.factors.len(), 1);
        assert!(F::columns_of(&*made, "wide").is_none());
        assert_eq!(execute(&made, "stream-endpoint")["selected"], [["1", "1"], ["2", "1"]]);
    }

    #[test]
    fn reused_product_cache_has_an_exact_witness_limit() {
        for size in [64, 65] {
            let mut src = String::from(
                "
                .decl lhs(a: number, k: number)
                .decl rhs(b: number, k: number)
                .decl wide(a: number, b: number)
                .decl lefts(a: number)
                .decl rights(b: number)
                .output lefts, rights
                wide(a,b) :- lhs(a,k), rhs(b,k).
                lefts(a) :- wide(a,_).
                rights(b) :- wide(_,b).
            ",
            );
            for n in 0..size {
                src.push_str(&format!("lhs({n},0).\n"));
            }
            for n in 0..64 {
                src.push_str(&format!("rhs({n},0).\n"));
            }
            let (made, plan) = rewritten(&parse(&src));
            assert_eq!(plan.factors.len(), 1);
            let guard = made.nullary_names().unwrap().into_iter().find(|r| r.starts_with("rr_pair_cache_")).unwrap();
            assert_eq!(F::columns_of(&*made, &guard).unwrap().len(), 1);
            assert!(made.rules().any(|r| made.rel_name(r.head().rel()) == guard && matches!(r.head().args().collect::<Vec<_>>().as_slice(), [TermRef::Int(1)])));
            let pairs = F::declared_names(&*made).into_iter().find(|r| r.starts_with("rr_pair_cache_") && r.ends_with("_pairs")).unwrap();
            let made = with_outputs(&made, &[&pairs]);
            let rows = execute(&made, &format!("bounded-pairs-{size}"));
            assert_eq!(rows[&pairs].len(), if size == 64 { 4096 } else { 0 });
            let expected = |n| (0..n).map(|i: usize| vec![i.to_string()]).collect::<BTreeSet<_>>();
            assert_eq!(rows["lefts"].iter().cloned().collect::<BTreeSet<_>>(), expected(size));
            assert_eq!(rows["rights"].iter().cloned().collect::<BTreeSet<_>>(), expected(64));
        }
    }

    /// A reused product whose readers, or the rules over them, read an answer only a node's document gives (`value`)
    /// is factorized but not cached: a cache's readers are the cross-file part's, which has no documents to answer
    /// it from. Decided from the program, so a run is never prepared again to find it (decision 0488).
    #[test]
    fn a_product_read_with_a_documents_answer_is_not_cached() {
        let src = |reader: &str| format!("
            .decl lhs(a: number, k: number)
            .decl rhs(b: number, k: number)
            .decl wide(a: number, b: number)
            .decl value(n: number, v: symbol)
            .decl lefts(a: number)
            .decl rights(b: number)
            .decl over(a: number)
            .output rights, over
            wide(a,b) :- lhs(a,k), rhs(b,k).
            {reader}
            rights(b) :- wide(_,b).
            over(a) :- lefts(a).
        ");
        let cached = |reader: &str| {
            let (made, plan) = rewritten(&parse(&src(reader)));
            assert_eq!(plan.factors.len(), 1, "{reader}: factorized");
            made.nullary_names().unwrap().into_iter().any(|r| r.starts_with("rr_pair_cache_"))
        };
        assert!(cached("lefts(a) :- wide(a,_)."), "a plain reader keeps its cache");
        assert!(!cached("lefts(a) :- wide(a,_), value(a, _)."), "a reader of `value`");
        assert!(!cached("lefts(a) :- wide(a,_).\n over(a) :- lefts(a), value(a, _)."), "a rule over a reader reads `value`");
    }

    /// A producer the chunks make is not restricted by a cross-file reader's requests: its rows are another file's
    /// (b.py's comment names a.py's function), and a chunk's request holds only its own. A reader within one
    /// document (its comment in the function's body) still drives it.
    #[test]
    fn a_cross_file_reader_does_not_restrict_a_file_local_producer() {
        let deferred = |reader: &str| {
            let v = parse(&format!(r#"
.decl kind(n: node, k: symbol)
.decl field(n: node, f: symbol, c: node)
.decl text(n: node, x: symbol)
.decl comment(c: node, x: symbol)
.decl line(n: node, l: number)
.decl col(n: node, k: number)
.input kind, field, text, comment, line, col
.decl defined(d: node, x: symbol)
defined(d, x) :- kind(d, "function_definition"), field(d, "name", n), text(n, x).
.decl noted(c: node, x: symbol)
noted(c, x) :- comment(c, x).
.decl remark(x: symbol, l: number, k: number)
{reader}
.output remark
"#));
            let (made, plan) = rewritten(&v);
            (F::declared_names(&*made).into_iter().any(|r| r.starts_with(DEFERRED_PAIRS)), plan.cross)
        };
        assert_eq!(deferred("remark(x, l, k) :- noted(c, x), defined(_, x), line(c, l), col(c, k)."), (false, vec!["defined".to_owned()]), "across files");
        assert_eq!(deferred("remark(x, l, k) :- noted(c, x), defined(d, x), field(d, \"body\", c), line(c, l), col(c, k)."), (true, Vec::new()), "within one document");
    }

    /// A constant's join (decision 0665: of facts and settings alone, React's `deps_on`) is left as written: a file's
    /// request would make the constant read every file's rows, while it is the same for every file and made once. The
    /// same join over a file's rows is driven by its readers.
    #[test]
    fn a_constants_join_is_not_restricted_by_its_readers_requests() {
        let deferred = |package_dep: &str| {
            let v = parse(&format!(r#"
.decl doc(d: number, path: symbol, module: symbol)
.decl package_for(path: symbol, dir: symbol)
.decl package_field(dir: symbol, key: symbol, v: symbol)
.decl kind(n: node, k: symbol)
.decl text(n: node, x: symbol)
.input doc, package_for, package_field, kind, text
.decl dep_package(m: symbol, r: symbol)
dep_package("react", "react"). dep_package("vue", "other").
.decl package_dep(dir: symbol, m: symbol)
{package_dep}
.decl deps_on(dir: symbol, r: symbol)
deps_on(dir, r) :- package_dep(dir, m), dep_package(m, r).
.decl other_jsx(d: number)
other_jsx(d) :- doc(d, p, _), package_for(p, dir), deps_on(dir, "other").
.output other_jsx
"#));
            let (made, plan) = rewritten(&v);
            (F::declared_names(&*made).into_iter().any(|r| r.starts_with(DEFERRED_PAIRS)), plan.constants)
        };
        assert_eq!(deferred("package_dep(dir, m) :- package_field(dir, \"dependencies\", m)."), (false, vec!["deps_on".to_owned()]), "settings alone");
        let (driven, constants) = deferred("package_dep(dir, m) :- kind(n, \"import\"), text(n, m), doc(_, dir, _).");
        assert!(constants.is_empty(), "{constants:?}");
        let _ = driven;
    }

    /// A reader that asks for every key the producer makes (through an atom of the producer's own rule) makes the
    /// request, a union over the readers, restrict nothing: the join is left as written. Readers that each narrow
    /// the keys still drive it.
    #[test]
    fn a_reader_asking_for_every_key_leaves_the_join_as_written() {
        let requested = |readers: &str| {
            let v = parse(&format!(r#"
.decl kind(n: node, k: symbol)
.decl field(n: node, f: symbol, c: node)
.decl text(n: node, x: symbol)
.decl marked(n: node)
.input kind, field, text, marked
.decl exc_call(c: node, q: symbol)
exc_call(c, q) :- kind(c, "call"), field(c, "function", f), text(f, q).
.decl a(c: node)
a(c) :- marked(c), exc_call(c, _).
.decl b(x: symbol)
{readers}
.output a, b
"#));
            let (made, _) = rewritten(&v);
            F::columns_of(&*made, &format!("{DEFERRED_PAIRS}exc_call")).is_some()
        };
        assert!(requested("b(x) :- marked(c), exc_call(c, \"p\"), field(c, \"body\", g), text(g, x)."), "every reader narrows the calls");
        assert!(!requested("b(x) :- exc_call(c, \"p\"), field(c, \"function\", g), text(g, x)."), "b asks for every call");
        assert!(requested("b(x) :- exc_call(c, \"p\"), field(c, \"function\", g), text(g, \"f\"), text(g, x)."), "b's constant narrows them");
    }

    #[test]
    fn aggregate_recursive_and_open_negative_readers_keep_their_relation() {
        for reader in ["out(a) :- enabled(a), !wide(_,_).", "out(a) :- a = count : { wide(_,_) }.", "wide(a,b) :- wide(a,k), rhs(b,k). out(a) :- wide(a,_)."] {
            let v = parse(&format!("{SOURCE} .decl out(a: number)\n.output out\n{reader}"));
            let mut p = Program::of(&v);
            let plans = rewrite(&mut p, &v, true, true).unwrap();
            assert!(plans.pairs.is_empty() && plans.factors.is_empty() && plans.shared.is_empty());
            assert!(p.first(), "the rules as they were");
        }
    }

    /// A request keeps the document its dropped read ties (decision 0652): `pair(a, f)` holds one call's two nodes in
    /// every row, and its reader joins `f`'s path with `a`'s text, a value. Asked for by `a` alone, the request drops
    /// `pair`, and with it the tie: every name met every path of the corpus (UP008's `super_first`). Given `pair`'s
    /// columns as one document's, the request says `@doc(a) = @doc(f)`, and the split places it in the chunks.
    #[test]
    fn a_request_keeps_the_document_its_dropped_read_ties() {
        let v = parse(
            r#"
            .decl node(d: number, n: node, kind: symbol, start: number, end: number)
            .decl child(p: node, c: node, i: number)
            .decl text(n: node, s: symbol)
            .input node, child, text
            .decl pair(a: node, f: node)
            pair(a, f) :- node(_, c, "call", _, _), child(c, a, 0), child(c, f, 1).
            .decl path(f: node, p: symbol)
            path(f, p) :- node(_, f, "class", _, _), text(f, p).
            .decl named(a: node, f: node)
            named(a, f) :- pair(a, f), path(f, p), text(a, p).
            .output named
        "#,
        );
        let first: &[usize] = &[0];
        let requests = vec![("pair".to_owned(), first)];
        let every = |_: &str, _: &[usize], _: &Version, _: &[upstream::Proof]| true;
        let request = format!("{DEFERRED_PAIRS}pair");
        let split = |p: &Program<'_>| crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(p.made.as_ref().unwrap()).unwrap();
        let mut tied = Program::of(&v);
        let same_doc = BTreeMap::from([("pair".to_owned(), rerail_blocks::vector![0, 1])]);
        assert_eq!(upstream::install_requests(&mut tied, &requests, DEFERRED_PAIRS, &same_doc, &every).unwrap(), [true]);
        let rules = tied.shown(Some(&request));
        assert!(!rules.is_empty() && rules.iter().all(|r| r.contains("@doc($a) = @doc($f)")), "{rules:#?}");
        assert!(split(&tied).local.contains(&request), "{:?}", split(&tied).causes.get(&request));
        let mut open = Program::of(&v);
        assert_eq!(upstream::install_requests(&mut open, &requests, DEFERRED_PAIRS, &Default::default(), &every).unwrap(), [true]);
        assert!(!split(&open).local.contains(&request), "without the tie the request meets every file's paths");
        assert_eq!(execute(tied.made.as_ref().unwrap(), "deferred-tied"), execute(&v, "deferred-tied-control"));
    }

    #[test]
    fn requests_installed_in_rounds_are_those_installed_one_at_a_time() {
        // `wide`'s one reader is `awide`'s producer, which names `wide`'s key only once `awide`'s install guards it: so
        // `wide` waits for a round after `awide`'s, and planned in `awide`'s round it would install nothing. `wide2`'s
        // plan nothing else's changes, and it joins `wide`'s round.
        let v = parse(&format!(
            "{SOURCE}
            .decl awide(a: number, b: number)
            .decl wide2(a: number, b: number)
            .decl yes(a: number, b: number)
            .decl yes2(a: number, b: number)
            .output yes, yes2
            awide(a,b) :- wide(a,b), enabled(a).
            wide2(a,b) :- lhs(a,k), rhs(b,k).
            yes(a,b) :- request(a,b), awide(a,b).
            yes2(a,b) :- request(a,b), wide2(a,b).
        "
        ));
        let both: &[usize] = &[0, 1];
        let requests: Vec<(String, &[usize])> = ["awide", "wide", "wide2"].iter().map(|r| (r.to_string(), both)).collect();
        let every = |_: &str, _: &[usize], _: &Version, _: &[upstream::Proof]| true;
        let mut rounds = Program::of(&v);
        let installed = upstream::install_requests(&mut rounds, &requests, DEFERRED_PAIRS, &Default::default(), &every).unwrap();
        let mut alone = Program::of(&v);
        let one_at_a_time: Vec<bool> =
            requests.iter().flat_map(|r| upstream::install_requests(&mut alone, std::slice::from_ref(r), DEFERRED_PAIRS, &Default::default(), &every).unwrap()).collect();
        assert_eq!(installed, one_at_a_time, "{:#?}", alone.shown(None));
        assert_eq!(installed, [true, true, true]);
        assert_eq!(rounds.shown(None), alone.shown(None));
        let made = rounds.made.clone().unwrap();
        assert_eq!(execute(&made, "deferred-rounds"), execute(&v, "deferred-rounds-control"));
    }
