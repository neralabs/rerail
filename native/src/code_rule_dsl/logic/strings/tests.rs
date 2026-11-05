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
