use super::*;

#[test]
fn literal_tests() {
    assert!(test("prefix", "_private", "_"));
    assert!(!test("prefix", "public", "_"));
    assert!(test("suffix", "fetch_async", "_async"));
    assert!(test("contains", "SqlServerPlan", "Server"));
    assert!(!test("contains", "plan", "Plan"), "case matters");
}

#[test]
fn globs_respect_path_segments() {
    assert!(test("glob", "impl/common/fts/manager.py", "impl/**/*.py"));
    assert!(test("glob", "impl/x.py", "impl/**/*.py"), "** matches no segment too");
    assert!(test("glob", "impl/sqlite/fts/a/b.py", "impl/*/fts/**"));
    assert!(!test("glob", "impl/a/b/fts/c.py", "impl/*/fts/**"), "* is one segment");
    assert!(!test("glob", "lib/impl/x.py", "impl/**/*.py"), "anchored at the start");
    assert!(test("glob", "l2070_server_plan/x.py", "l2070_*/*.py"));
    assert!(!test("glob", "a.py", "[unclosed"), "an invalid glob matches nothing");
}

fn matcher(op: &str, patterns: &[&str]) -> Matcher {
    Matcher::new(op, &patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn sets_mean_any_of_their_patterns() {
    let m = matcher("glob", &["_*branch_tip*", "_*revision_cache*", "[A-Z]*Step", "conn"]);
    assert!(m.is_match("_old_branch_tip_map"));
    assert!(m.is_match("_revision_cache"));
    assert!(m.is_match("ServerPlanStep"));
    assert!(m.is_match("conn"));
    assert!(!m.is_match("branch_tip"), "the leading _ is literal");
    assert!(!m.is_match("serverPlanStep"), "a class is a class");
    assert!(!m.is_match("connection"));
    let p = matcher("glob", &["**/tests/**", "*.py"]);
    assert!(p.is_match("a/tests/b.py"));
    assert!(p.is_match("x.py"));
    assert!(!p.is_match("a/x.py"), "* stays in one segment, also behind a literal shortcut");
    assert!(test("glob", "ServerPlanStep", &format!("*Intent{SET_SEP}*Step")), "SQL's sets");
}

#[test]
fn prefix_suffix_contains_sets_and_case() {
    let m = matcher("prefix", &["Sql", "_Sql"]);
    assert!(m.is_match("SqlRaw") && m.is_match("_SqlPrebuilt") && !m.is_match("RawSql"));
    let s = matcher("suffix", &["_async", "Async"]);
    assert!(s.is_match("fetch_async") && !s.is_match("async_fetch"));
    let i = matcher("icontains", &["Cursor"]);
    assert!(i.is_match("raw_cursor") && i.is_match("CURSORS") && !i.is_match("cur"));
    let e = matcher("iequals", &["Conn"]);
    assert!(e.is_match("CONN") && !e.is_match("conns"));
    let g = matcher("iglob", &["*_INFO"]);
    assert!(g.is_match("field_info"));
}

#[test]
fn words_are_whole_and_phrases_in_order() {
    let m = matcher("iword", &["select", "on delete"]);
    assert!(m.is_match("SELECT x FROM t"));
    assert!(m.is_match("REFERENCES t ON\n   DELETE CASCADE"));
    assert!(!m.is_match("selected rows"), "a whole word");
    assert!(!m.is_match("delete on t"), "in order");
    assert!(!matcher("word", &["SELECT"]).is_match("select"), "case matters");
}

#[test]
fn tests_on_text_relations_become_relations() {
    let mut prog = crate::code_rule_dsl::logic::front::parse(
        ".decl name(n: node, s: sym)\n.input name\n.decl a(n: node)\n.decl b(n: node)\n.decl c(s: sym)\n\
         a($n) :- name($n, $s), @glob($s, {\"_*\", \"x*\"}), !@suffix($s, \"_async\").\n\
         b($n) :- name($n, $s), $s in {\"conn\", \"db\"}, @glob($s, {\"x*\", \"_*\"}).\n\
         c($s) :- name(_, $t), $s = @cat($t, \"x\"), @prefix($s, \"_\").\n.output a, b, c\n",
    )
    .unwrap();
    let v = as_relations(&mut prog).unwrap();
    // The glob set is one relation for both rules (patterns in any order); the computed string stays per row.
    assert_eq!(v.tests.len(), 3, "{:?}", v.tests.iter().map(|t| &t.0).collect::<Vec<_>>());
    let text = prog.rules.iter().map(|r| r.to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("!strtest_1($s)"), "{text}");
    assert!(text.contains("@prefix"), "{text}");
    let rows = v.rows(&["_x".into(), "x_async".into(), "db".into(), "y".into()], &[1, 2, 3, 4]);
    assert_eq!(rows, [vec![1, 2], vec![2], vec![3]]);
}

#[test]
fn regexes_and_their_rewrites_agree() {
    assert!(test("rx", "_branch_cache_x", r"^_\w*cache\w*$"));
    assert!(!test("rx", "branch_cache", r"^_\w*cache\w*$"));
    assert!(test("irx", "SELECT x", r"^select\b"));
    assert!(test("rx", "abca", r"(.).*\1"), "a backreference");
    assert!(!test("rx", "x", "("), "a pattern that does not parse matches nothing");
    assert!(test("iequals", "ConnectionId", "connectionid"));
    assert!(test("isuffix", "RelationshipSpec", "spec"));
    for (s, glob, rx) in [("impl/a/b.py", "impl/**/*.py", r"^impl/(?:.*/)?[^/]*\.py$"), ("fetch_async", "*_async", "_async$")] {
        assert_eq!(test("glob", s, glob), test("rx", s, rx), "{s} {glob} {rx}");
    }
}
