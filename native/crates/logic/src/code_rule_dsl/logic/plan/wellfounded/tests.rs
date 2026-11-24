    use super::*;

    const DECLS: &str = r#"
.decl node(d: number, n: node, kind: symbol, start: number, end_: number)
.decl child(p: node, c: node, idx: number)
.decl edge(a: symbol, b: symbol)
.decl link(a: node, b: node)
"#;

    fn check_of(text: &str, comp: &[&str]) -> Result<Measure, String> {
        let p = crate::code_rule_dsl::logic::rr::parse_version(&format!("{DECLS}{text}")).expect("parses");
        let placed = super::super::body::Placed::whole(p.clone());
        let f = placed.fields().expect("fields");
        let comp = Components::of_names([comp.iter().copied()]);
        check(&placed.v, &|r: &str| f.types(r).into_iter().map(str::to_owned).collect(), &comp[0])
    }

    /// A walk down the tree is proved: each row is read from its parent's, ancestors first.
    #[test]
    fn a_walk_down_the_tree_cannot_cycle() {
        let m = check_of(
            ".decl top(a: node)\ntop(a) :- node(_, a, \"function_definition\", _, _).\n\
             .decl below(a: node, c: node)\nbelow(a, c) :- top(a), child(a, c, _).\nbelow(a, c) :- below(a, b), child(b, c, _).\n",
            &["below"],
        )
        .expect("proved");
        assert_eq!((m.order, m.col["below"]), (Order::Down, 1));
    }

    /// A step through a helper the check summarizes from its own rules (`desc`: column 1 below column 0).
    #[test]
    fn a_step_through_a_summarized_helper() {
        let m = check_of(
            ".decl desc(a: node, c: node)\ndesc(a, c) :- child(a, c, _).\ndesc(a, c) :- desc(a, b), child(b, c, _).\n\
             .decl seed(a: node)\nseed(a) :- node(_, a, \"module\", _, _).\n\
             .decl t(c: node)\nt(c) :- seed(c).\nt(c) :- t(a), desc(a, c).\n",
            &["t"],
        )
        .expect("proved");
        assert_eq!(m.order, Order::Down);
    }

    /// A graph that can cycle is not: its relation has no node column, or its step is a relation the check does not
    /// read as a tree step.
    #[test]
    fn graphs_that_can_cycle_are_not_proved() {
        let reach = check_of(".decl reach(a: symbol, b: symbol)\nreach(a, b) :- edge(a, b).\nreach(a, c) :- reach(a, b), edge(b, c).\n", &["reach"]);
        assert_eq!(reach, Err("a relation with no node column".into()));
        let linked = check_of(".decl r(a: node, b: node)\nr(a, b) :- link(a, b).\nr(a, c) :- r(a, b), link(b, c).\n", &["r"]);
        assert!(linked.unwrap_err().starts_with("an unknown step: link"));
    }

    /// A component whose measures are past the bound is refused, however many: the product of 70 relations' four node
    /// columns each (2^140) overflowed before it was compared (python/calls with the typing run, on two threads).
    #[test]
    fn a_component_of_many_measures_is_refused() {
        let n = 70;
        let mut text = String::new();
        for i in 0..n {
            let next = (i + 1) % n;
            text.push_str(&format!(".decl r{i}(a: node, b: node, c: node, d: node)\nr{i}(a, b, c, d) :- r{next}(a, b, c, d), child(a, b, _), child(c, d, _).\n"));
        }
        let comp: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
        let comp: Vec<&str> = comp.iter().map(String::as_str).collect();
        assert_eq!(check_of(&text, &comp), Err("too many measures to try".into()));
    }

    /// Steps both ways, and a cycle at one node, fail.
    #[test]
    fn both_ways_or_a_cycle_at_one_node_fail() {
        let both = check_of(
            ".decl w(a: node)\nw(a) :- node(_, a, \"call\", _, _).\nw(c) :- w(a), child(a, c, _).\nw(a) :- w(c), child(a, c, _).\n",
            &["w"],
        );
        assert_eq!(both, Err("steps both ways".into()));
        let cycle = check_of(
            ".decl a(n: node)\n.decl b(n: node)\na(n) :- node(_, n, \"call\", _, _).\na(n) :- b(n).\nb(n) :- a(n).\n",
            &["a", "b"],
        );
        assert_eq!(cycle, Err("a cycle at one node".into()));
    }

    /// `nearest(n, a, set)` as the engine lowers it (`rr_nearest_<set>(n, a)`) is a step up: a walk out through
    /// scopes (calls.rerail's `walk`) is proved, nearer scopes first.
    #[test]
    fn a_walk_out_through_lowered_nearest_scopes() {
        let m = check_of(
            ".decl rr_nearest_fscope(n: node, a: node)\n.decl start(n: node, s: node)\nstart(n, s) :- rr_nearest_fscope(n, s).\n\
             .decl walk(n: node, s: node)\nwalk(n, s) :- start(n, s).\nwalk(n, p) :- walk(n, s), rr_nearest_fscope(s, p).\n",
            &["walk"],
        )
        .expect("proved");
        assert_eq!((m.order, m.col["walk"]), (Order::Up, 1));
    }

    /// The program's proofs answer for its components, and for no set that is not one.
    #[test]
    fn the_programs_proofs_answer_by_component() {
        let p = crate::code_rule_dsl::logic::rr::parse_version(&format!(
            "{DECLS}.decl below(a: node, c: node)\nbelow(a, c) :- child(a, c, _).\nbelow(a, c) :- below(a, b), child(b, c, _).\n\
             .decl reach(a: symbol, b: symbol)\nreach(a, b) :- edge(a, b).\nreach(a, c) :- reach(a, b), edge(b, c).\n.output below\n.output reach\n"
        ))
        .expect("parses");
        let placed = super::super::body::Placed::whole(p.clone());
        let proofs = crate::code_rule_dsl::logic::program_image::manager::analysis::<Proofs>(&placed.v).expect("proofs");
        let set = |r: &[&str]| Components::of_names([r.iter().copied()]);
        assert_eq!(proofs.counts(), (1, 1));
        assert_eq!(proofs.of(&set(&["below"])[0]).map(|m| m.order), Ok(Order::Down));
        assert!(proofs.of(&set(&["reach"])[0]).is_err());
        assert_eq!(proofs.of(&set(&["below", "reach"])[0]), Err("not the program's component".into()));
        // Kept by the version's identity (t33 K4): its rows read back as it.
        use crate::code_rule_dsl::logic::program_image::manager::Analysis as _;
        let rows = proofs.rows(&placed.v).expect("rows");
        assert_eq!(Proofs::of_rows(&placed.v, &rows).as_ref(), Some(&*proofs));
    }

    /// Same-node steps without a cycle are ranked: a relation after the ones it reads at its node.
    #[test]
    fn same_node_steps_are_ranked() {
        let m = check_of(
            ".decl a(n: node)\n.decl b(n: node)\na(n) :- node(_, n, \"call\", _, _).\nb(n) :- a(n).\na(c) :- b(p), child(p, c, _).\n",
            &["a", "b"],
        )
        .expect("proved");
        assert_eq!(m.order, Order::Down);
        assert!(m.rank["a"] < m.rank["b"], "{m:?}");
    }
