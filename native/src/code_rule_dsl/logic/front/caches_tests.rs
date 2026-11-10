//! The form of a cache (decision 0103 §1): what `.cache C by K` refuses.

use super::{Cache, caches};

fn program(text: &str) -> crate::code_rule_dsl::logic::front::Program {
    crate::code_rule_dsl::logic::rr::parse(text).unwrap()
}

fn refused(text: &str) -> String {
    match crate::code_rule_dsl::logic::rr::parse(text) {
        Err(e) => e.to_string(),
        Ok(p) => caches(&p).err().map(|e| e.to_string()).unwrap_or_else(|| panic!("accepted: {text}")),
    }
}

const DECLS: &str = ".decl src(a: symbol, b: symbol)\n.input src\n.decl k(a: symbol)\n.decl c(a: symbol, b: symbol)\n";

#[test]
fn a_cache_is_a_key_rule_and_an_output_rule() {
    let p = program(&format!("{DECLS}.cache c by k\nk(a) :- src(a, _).\nc(a, b) :- k(a), src(a, b).\n.output c\n"));
    assert_eq!(caches(&p).unwrap(), vec![Cache { out: "c".into(), key: "k".into(), cols: vec![0] }]);
    // The key in another column of the output.
    let p = program(&format!("{DECLS}.cache c by k\nk(a) :- src(a, _).\nc(b, a) :- k(a), src(a, b).\n.output c\n"));
    assert_eq!(caches(&p).unwrap()[0].cols, vec![1]);
}

#[test]
fn a_broken_cache_is_refused() {
    let with = |rules: &str| format!("{DECLS}.cache c by k\nk(a) :- src(a, _).\n{rules}.output c\n");
    assert!(refused(&with("c(a, b) :- src(a, b).\n")).contains("reads k 0 times"));
    assert!(refused(&with("c(a, b) :- src(a, b), !k(a).\n")).contains("negated"));
    assert!(refused(&with("c(a, b) :- k(x), src(a, b).\n")).contains("does not have the key"));
    assert!(refused(&with("c(a, b) :- k(a), src(a, b).\nc(b, a) :- k(a), src(a, b).\n")).contains("other columns"));
    assert!(refused(&with("c(a, b) :- k(a), src(a, b).\nc(a, b) :- k(a), c(b, a).\n")).contains("reads itself"));
    assert!(refused(&format!("{DECLS}.cache c\nk(a) :- src(a, _).\n")).contains("write `.cache C by K`"));
}
