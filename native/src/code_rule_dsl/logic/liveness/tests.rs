    /// On a small program: an output's rows are live; a row no output reads is dead; a row that blocks a
    /// negation is live; a read inside a count makes its group live.
    #[test]
    fn liveness_marks_what_outputs_read() {
        let src = r#"
.decl e(a: number, b: number)
e(1, 2). e(2, 3). e(3, 4). e(7, 8).
.decl stop(a: number)
stop(3). stop(9).
.decl path(a: number, b: number)
path(a, b) :- e(a, b), !stop(a).
path(a, c) :- path(a, b), e(b, c), !stop(b).
.decl unused(a: number)
unused(a) :- e(a, _).
.decl fan(a: number, n: number)
fan(a, n) :- path(a, _), n = count : { e(a, _) }.
.decl out(a: number, b: number)
out(a, b) :- path(a, b), a = 1.
.output out, fan
"#;
        let prog = crate::code_rule_dsl::logic::rr::parse(src).unwrap();
        let live = super::program(&prog);
        let text = format!("{src}\n{}", crate::code_rule_dsl::logic::rr::print(&live).unwrap());
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("liveness");
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let out = dir.join("out");
        crate::code_rule_dsl::logic::plan::corpus::run(&text, &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
        let counts = super::read_counts(&std::fs::read_to_string(out.join(format!("{}.csv", super::COUNTS))).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
        let get = |rel: &str, what: &str| counts.get(rel).and_then(|m| m.get(what)).copied().unwrap_or(0);
        // path is (1,2) (2,3) (7,8) (1,3); stop(3) blocks (3,4) and (1,4). fan reads every row of path.
        assert_eq!((get("path", "rows"), get("path", "live")), (4, 4), "{counts:?}");
        // e(3,4) only meets a blocked derivation: deleting it changes nothing. The others make live paths.
        assert_eq!((get("e", "rows"), get("e", "live"), get("e", "keys"), get("e", "live keys")), (4, 3, 4, 3), "{counts:?}");
        assert_eq!((get("unused", "rows"), get("unused", "live")), (4, 0), "{counts:?}");
        assert_eq!((get("stop", "rows"), get("stop", "live")), (2, 1), "stop(3) blocks path(3, 4); stop(9) blocks nothing: {counts:?}");
        assert_eq!((get("out", "rows"), get("out", "live")), (2, 2), "{counts:?}");
    }
