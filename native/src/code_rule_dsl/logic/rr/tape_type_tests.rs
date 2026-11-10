//! The `tape` column type: a tape stands only in tape columns.

use crate::code_rule_dsl::logic::rr::parse;

const DECLS: &str = ".decl q(s: symbol)\n.input q\n";

#[test]
fn a_tape_column_takes_a_parse_and_a_tape_of() {
    let ok = format!("{DECLS}.decl t(s: symbol, t: tape)\nt(s, t) :- q(s), parse_of(s, \"sql\", t).\n.decl d(d: number)\nd(d) :- t(_, t), tape_of(d, t).\n.decl u(t: tape)\nu(t) :- q(s), t = @parse(\"sql\", s).\n.output d, u\n");
    parse(&ok).unwrap();
}

#[test]
fn a_tape_in_a_symbol_or_number_column_is_refused() {
    let in_symbol = format!("{DECLS}.decl t(s: symbol, t: symbol)\nt(s, t) :- q(s), parse_of(s, \"sql\", t).\n.output t\n");
    let e = parse(&in_symbol).unwrap_err().to_string();
    assert!(e.contains("t is a tape in one column and a symbol in another"), "{e}");
    let functor = format!("{DECLS}.decl n(t: number)\nn(t) :- q(s), t = @parse(\"sql\", s).\n.output n\n");
    let e = parse(&functor).unwrap_err().to_string();
    assert!(e.contains("a tape in one column and a number"), "{e}");
}
