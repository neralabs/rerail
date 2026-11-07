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
    let mut prog = crate::code_rule_dsl::logic::rr::parse(
        ".decl name(n: node, s: symbol)\n.input name\n.decl a(n: node)\n.decl b(n: node)\n.decl c(s: symbol)\n\
         a(n) :- name(n, s), (glob(\"_*\", s) ; glob(\"x*\", s)), !suffix(\"_async\", s).\n\
         b(n) :- name(n, s), (s = \"conn\" ; s = \"db\"), (glob(\"x*\", s) ; glob(\"_*\", s)).\n\
         c(s) :- name(_, t), s = @dot(t, \"x\"), prefix(\"_\", s).\n.output a, b, c\n",
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

/// `core.rr`'s text functors give the values its comments give as examples.
#[test]
fn text_functors_follow_core_rr() {
    use super::functions::*;
    assert_eq!(pkg_up("a.b.c", 1), "a.b");
    assert_eq!(pkg_up("a.b.c", 0), "a.b.c");
    assert_eq!(pkg_up("a", 5), "");
    assert_eq!(pkg_up("", 1), "");
    assert_eq!(join("src/a", "../c"), "src/c");
    assert_eq!(join("src/a", "./b/../c.ts"), "src/a/c.ts");
    assert_eq!(glob_sub("@/*", "src/*", "@/lib/x"), "src/lib/x");
    assert_eq!(glob_sub("@/*", "src/*", "lib/x"), "");
    assert_eq!(slice("_name", 1, 5), "name");
    assert_eq!(slice("_name", -4, 5), "name");
    assert_eq!(convert("max-age", "camelCase", "dash"), "maxAge");
    assert_eq!(convert("maxAge", "snakeCase", ""), "max_age");
    assert_eq!(convert("max_age", "pascalCase", "underscore"), "MaxAge");
    assert_eq!(json_str("a\"b\u{e9}\u{1f600}"), "\"a\\\"b\\u00e9\\ud83d\\ude00\"");
}

/// Nested alternatives, which globset refuses, are expanded to one level (`public-api-triples.rr`'s `hint`).
#[test]
fn globs_nest_alternatives() {
    let p = "{{versioned,}{databaseconnection,readtransaction},{lazyresult,lazyseq}}";
    for s in ["ReadTransaction", "VersionedReadTransaction", "LazySeq"] {
        assert!(super::test("iglob", s, p), "{s}");
    }
    for s in ["Versioned", "ReadTransactions", "x"] {
        assert!(!super::test("iglob", s, p), "{s}");
    }
    assert!(super::test("glob", "a,b", "{{a\\,b,c},d}"));
}

/// Data-layer text scanners copy complete logical lines through shared candidate relations.
/// These tests must still use the vocabulary batch, including predicates on those aliases.
#[test]
fn logical_line_predicates_and_aliases_use_the_vocabulary_batch() {
    let mut program = crate::code_rule_dsl::logic::rr::parse(r##"
.decl split_line(d: number, l: number, t: symbol)
.input split_line
.decl candidate(d: number, l: number, t: symbol)
.decl selected(d: number, l: number, t: symbol)
.decl finding(d: number, l: number)
candidate(d,l,t) :- split_line(d,l,t), !prefix("#",t).
selected(d,l,t) :- candidate(d,l,t).
finding(d,l) :- selected(d,l,t), contains("inspect",t).
.output finding
"##).unwrap();
    let vocabulary = as_relations(&mut program).unwrap();
    assert_eq!(vocabulary.tests.len(), 2, "both line predicates must be batched");
    let rendered = program.rules.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n");
    assert!(!rendered.contains("@prefix") && !rendered.contains("@contains"), "{rendered}");
    assert_eq!(vocabulary.rows(&["#inspect".into(), "inspect".into(), "clean".into()], &[11,12,13]),
               [vec![11], vec![11,12]]);
}

/// A second writer can construct strings that the prepared vocabulary never saw.
/// Optimising only the first writer would silently discard those findings.
#[test]
fn computed_or_fact_writers_prevent_alias_vocabulary_rewrites() {
    let mut program = crate::code_rule_dsl::logic::rr::parse(r##"
.decl split_line(d: number, l: number, t: symbol)
.input split_line
.decl combined(t: symbol)
.decl constant(t: symbol)
.decl finding(t: symbol)
combined(t) :- split_line(_,_,t).
combined(t) :- split_line(_,_,s), t = @dot(s,"added").
constant("invented").
constant(t) :- split_line(_,_,t).
finding(t) :- combined(t), suffix("added",t).
finding(t) :- constant(t), prefix("invented",t).
.output finding
"##).unwrap();
    let vocabulary = as_relations(&mut program).unwrap();
    assert!(vocabulary.tests.is_empty(), "all writers must supply prepared vocabulary symbols");
}

/// The repository's existing public signature verifies without changing signed artifacts.
#[test]
fn signed_allowlists_verify_existing_bytes_and_reject_tampering() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    functions::register(&db).unwrap();
    let key = "7e9776d87292ab633115bd6c07892a4bf56450fffb2dfcf13b3eba1866b9fbb6";
    let message = r#"{"allowlist":[],"linter":"lint_admin_async_purity","version":1}"#;
    let signature = "xN3ugqWFKXrfdfYHOWH14S9NfLPHCwnni7yf1errzrn17xHvVgd6C3y/QXR7mwfGPnFwpuq/HcCWAx9C2crzDg==";
    let verify = |k: &str, m: &str, s: &str| db.query_row("SELECT ed25519_ok(?1,?2,?3)",
        rusqlite::params![k,m,s], |row| row.get::<_, i64>(0)).unwrap();
    assert_eq!(verify(key,message,signature), 1);
    assert_eq!(verify(key,&message.replace("[]", "[\"added.py\"]"),signature), 0);
    assert_eq!(verify(key,message,"not base64"), 0);
    assert_eq!(verify("wrong key",message,signature), 0);
    assert_eq!(verify(key,message,&signature[..60]), 0);
}
