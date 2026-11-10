//! The lint of a cache (decision 0103): what `.cache C by K` refuses.

use super::{Cache, caches};

fn refused(text: &str) -> String {
    match crate::code_rule_dsl::logic::rr::parse(text) {
        Err(e) => e.to_string(),
        Ok(p) => caches(&p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {text}")),
    }
}

const DECLS: &str = ".decl src(a: symbol, b: symbol)\n.input src\n.decl k(a: symbol, v: symbol)\n.decl c(a: symbol, v: symbol, b: symbol)\n";

#[test]
fn a_cache_is_a_key_rule_with_a_version_tag_and_an_output_rule() {
    let p = crate::code_rule_dsl::logic::rr::parse(&format!("{DECLS}.cache c by k\nk(a, \"v1\") :- src(a, _).\nc(a, v, b) :- k(a, v), src(a, b).\n.output c\n")).unwrap();
    let got = caches(&p).unwrap();
    assert_eq!((got[0].out.as_str(), got[0].key.as_str(), got.len()), ("c", "k", 1));
}

#[test]
fn the_lint_refuses_a_key_without_a_version_and_an_output_that_does_not_read_it() {
    let with = |key: &str, rules: &str| format!("{DECLS}.cache c by k\n{key}{rules}.output c\n");
    let key = "k(a, \"v1\") :- src(a, _).\n";
    assert!(refused(&with("k(a, b) :- src(a, b).\n", "c(a, v, b) :- k(a, v), src(a, b).\n")).contains("no version tag"));
    assert!(refused(&with(key, "c(a, \"v1\", b) :- src(a, b).\n")).contains("does not read k"));
    assert!(refused(&with(key, "c(a, \"v1\", b) :- src(a, b), !k(a, \"v1\").\n")).contains("does not read k"));
    assert!(refused(&format!("{DECLS}.cache c\n{key}")).contains("write `.cache C by K`"));
    assert!(refused(&format!("{DECLS}.cache d by k\n{key}")).contains("d is not declared"));
}
