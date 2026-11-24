//! The lint of a native step's answer (t33 K3): what `.answer` reads and refuses.

use super::{Answer, answers, parse};

#[test]
fn an_answer_names_its_request_and_what_it_adds_rows_to() {
    assert_eq!(parse("rr_dir_req -> dir_entry").unwrap(), Answer { req: "rr_dir_req".into(), produces: vec!["dir_entry".into()] });
    assert_eq!(parse("c_cc_file_req -> c_compile_command, c_compile_arg").unwrap().produces, ["c_compile_command", "c_compile_arg"]);
    for wrong in ["rr_dir_req dir_entry", "-> dir_entry", "rr_dir_req ->", "a -> b c"] {
        assert!(parse(wrong).is_err(), "{wrong}");
    }
}

#[test]
fn the_lint_refuses_a_relation_answered_twice() {
    let decls = ".decl a(x: symbol)\n.decl b(x: symbol)\n.decl r(x: symbol)\n.output r\nr(x) :- a(x).\n";
    // Parsing runs the lint (`front`), so a refusal is the parse's, or the lint's on what parsed.
    let lint = |extra: &str| -> Result<Vec<Answer>, String> {
        let v = crate::code_rule_dsl::logic::rr::parse_version(&format!("{decls}{extra}")).map_err(|e| format!("{e:?}"))?;
        answers(&v).map_err(|e| format!("{e:?}"))
    };
    assert_eq!(lint(".answer q -> a, b\n").unwrap().len(), 1);
    assert_eq!(lint(".answer q -> c\n").unwrap().len(), 1, "an answer the program reads none of");
    assert!(lint(".answer q -> a\n.answer p -> a\n").unwrap_err().contains("answered by both"));
}
