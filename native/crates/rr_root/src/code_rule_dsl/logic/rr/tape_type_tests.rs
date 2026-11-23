//! The content-keyed column types (`tape`, `digest`, `blob`): each value stands only in columns of its own type.

use crate::code_rule_dsl::logic::rr::parse_version as parse;

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

/// A digest column takes a digest functor's value and is read by another digest column; a blob column is read by a
/// blob column.
#[test]
fn a_digest_and_a_blob_column_take_their_own_values() {
    let ok = format!(
        "{DECLS}.decl h(s: symbol, h: digest)\nh(s, h) :- q(s), h = @sha256(s).\n.decl k(h: digest)\nk(h) :- h(_, h).\n\
         .decl b(s: symbol, x: blob)\n.input b\n.decl c(x: blob)\nc(x) :- b(_, x).\n.output k, c\n"
    );
    parse(&ok).unwrap();
}

/// A digest in a symbol or number column, a blob in a digest column, and a tape in a blob column are refused.
#[test]
fn a_digest_or_a_blob_in_another_types_column_is_refused() {
    let in_symbol = format!("{DECLS}.decl h(s: symbol, h: digest)\nh(s, h) :- q(s), q(h).\n.output h\n");
    let e = parse(&in_symbol).unwrap_err().to_string();
    assert!(e.contains("h is a digest in one column and a symbol in another: declare the column `digest`"), "{e}");
    let in_number = format!("{DECLS}.decl h(h: digest)\nh(h) :- q(s), h = @sha256(s).\n.decl n(n: number)\nn(n) :- h(n).\n.output n\n");
    let e = parse(&in_number).unwrap_err().to_string();
    assert!(e.contains("n is a digest in one column and a number"), "{e}");
    let blob_digest = format!("{DECLS}.decl b(x: blob)\n.input b\n.decl d(x: digest)\nd(x) :- b(x).\n.output d\n");
    let e = parse(&blob_digest).unwrap_err().to_string();
    assert!(e.contains("x is a") && e.contains("blob") && e.contains("digest"), "{e}");
    let tape_blob = format!("{DECLS}.decl b(t: blob)\nb(t) :- q(s), parse_of(s, \"sql\", t).\n.output b\n");
    let e = parse(&tape_blob).unwrap_err().to_string();
    assert!(e.contains("t is a") && e.contains("tape") && e.contains("blob"), "{e}");
}
