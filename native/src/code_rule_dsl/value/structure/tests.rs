use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::tree_sitter::LanguageExt;

/// The structure of the one expression statement's expression (or the statement itself).
fn of(source: &str, equality: Equality) -> Option<Vec<Tok>> {
    let root = GrammarDialect::Python.ast_grep(source);
    let statement = root.root().children().next().expect("a statement");
    let node = if statement.kind() == "expression_statement" && operands(&statement).len() == 1 {
        operands(&statement).remove(0)
    } else {
        statement
    };
    let mut spend = || true;
    let mut context = Context::new(&mut spend);
    structure(&node, equality, &mut context)
}

fn same(left: &str, right: &str, equality: Equality) -> bool {
    let (left_tokens, right_tokens) = (of(left, equality), of(right, equality));
    assert!(left_tokens.is_some(), "{left:?} has a structure");
    assert!(right_tokens.is_some(), "{right:?} has a structure");
    left_tokens == right_tokens
}

#[test]
fn plain_equality_is_ruffs_comparable_expr() {
    let equal = [
        ("(1, 'a')", "(1, \"a\")"),
        ("-1", "-(1)"),
        ("[1, 0x2]", "[1, 2]"),
        ("[1, 2,]", "[1,2]"),
        ("f'{x}'", "f\"{x}\""),
        ("f'a{b}'", "f\"a{b}\""),
        ("'a' 'b'", "'ab'"),
        ("'\\x61'", "'a'"),
        ("'\\N{LATIN SMALL LETTER A}'", "'a'"),
        ("r'\\d'", "'\\\\d'"),
        ("1.5", "1.50"),
        ("1e-400", "0.0"),
        ("1e400", "2e400"),
        ("1j", "1.0j"),
        ("18_446_744_073_709_551_616", "18446744073709551616"),
        ("a.b(c, k=1)", "a . b(c, k=(1))"),
        ("f(*a, k=1)", "f(k=1, *a)"),
        ("x[1,]", "x[(1,)]"),
        ("x[1:2]", "x[1:2:]"),
        ("not a", "not (a)"),
        ("a and b and c", "(a and b and c)"),
        ("a  # note\n", "a"),
        ("f(x for x in y)", "f((x for x in y))"),
        ("f'{x!r:>{w}}'", "f\"{x!r:>{w}}\""),
    ];
    for (left, right) in equal {
        assert!(same(left, right, Equality::Plain), "{left} == {right}");
    }
    let different = [
        ("1", "1.0"),
        ("1", "True"),
        ("0", "0j"),
        ("0.0", "-0.0"),
        ("1 + 1", "2"),
        ("f'a'", "'a'"),
        ("'a' f'b'", "'ab'"),
        ("b'a'", "'a'"),
        ("0xffffffffffffffffffff", "0XFFFFFFFFFFFFFFFFFFFF"),
        ("0xffffffffffffffffffff", "1208925819614629174706175"),
        ("f'{x=}'", "f'{x = }'"),
        ("f'{x:}'", "f'{x}'"),
        ("f'{x!r}'", "f'{x!s}'"),
        ("(a and b) and c", "a and b and c"),
        ("a and (b and c)", "a and b and c"),
        ("a < b <= c", "(a < b) <= c"),
        ("x[1:]", "x[:1]"),
        ("x[1]", "x[1,]"),
        ("[1]", "(1,)"),
        ("f(a)", "f(b)"),
        ("a.b", "a.c"),
        ("-a", "+a"),
        ("t'a'", "'a'"),
    ];
    for (left, right) in different {
        assert!(!same(left, right, Equality::Plain), "{left} != {right}");
    }
}

#[test]
fn hash_equality_folds_numbers_as_python_hashes_them() {
    let equal = [
        ("1", "1.0"),
        ("1", "True"),
        ("0", "0j"),
        ("0.0", "-0.0"),
        ("-True", "-1"),
        ("True + 1j", "1 + 1j"),
        ("1.5 - 0j", "1.5"),
        ("(1, 2)", "(1.0, 2)"),
        ("((1))", "1"),
        ("-(-1)", "1"),
        ("'a' 'b'", "'ab'"),
        ("(a, 1)", "(a,1)"),
        ("18446744073709551616", "18_446_744_073_709_551_616"),
    ];
    for (left, right) in equal {
        assert!(same(left, right, Equality::Hash), "{left} == {right}");
    }
    let different = [
        ("1 + 1", "2"),
        ("0.1 + 0.2", "0.3"),
        ("(1, a)", "(1.0, a)"),
        ("~1", "-2"),
        ("f'a'", "'a'"),
        ("b'a'", "'a'"),
        ("18446744073709551616", "0x10000000000000000"),
        ("18446744073709551616.0", "18446744073709551616"),
        ("1e20", "100000000000000000000"),
        ("[1]", "[1.0]"),
    ];
    for (left, right) in different {
        assert!(!same(left, right, Equality::Hash), "{left} != {right}");
    }
    // A named expression compares its target structurally and folds its value.
    assert!(same("(x := 1)", "(x := 1.0)", Equality::Hash));
    assert!(!same("(x := 1)", "(x := 1.0)", Equality::Plain));
}

#[test]
fn parts_equality_compares_implicit_concatenation_part_by_part() {
    assert!(same("'a' 'b'", "'a' \"b\"", Equality::Parts));
    assert!(!same("'a' 'b'", "'ab' ''", Equality::Parts));
    assert!(!same("'a' 'b'", "'ab'", Equality::Parts));
    assert!(same("'ab'", "('ab')", Equality::Parts));
    assert!(same("b'a' b'b'", "b'a' b'\\x62'", Equality::Parts));
    // Anything but a plain string literal compares as `plain` does.
    assert!(same("1", "0x1", Equality::Parts));
    assert!(!same("1", "1.0", Equality::Parts));
    assert!(same("f'a' 'b'", "f'ab'", Equality::Parts));
}

#[test]
fn statements_compare_as_ruffs_comparable_stmt() {
    assert!(same("x = (1, 2)\n", "x = 1, 2\n", Equality::Plain));
    assert!(same("g(x, y=1)\n", "g(x, y=(1))\n", Equality::Plain));
    assert!(!same("x = 1\n", "x = 1.0\n", Equality::Plain));
    assert!(same(
        "for a, b in c: pass\n",
        "for (a, b) in c: pass\n",
        Equality::Plain
    ));
    assert!(same("a, b\n", "a, (b)\n", Equality::Plain));
    assert!(same(
        "if a:\n    return 1  # one\n",
        "if a :\n    return (1)\n",
        Equality::Plain
    ));
}

#[test]
fn a_node_with_errors_or_past_the_depth_has_no_structure() {
    assert_eq!(of("'\\N{NOT A NAME}'\n", Equality::Plain), None);
    assert_eq!(of("'\\ud800'\n", Equality::Plain), None);
    let nested = format!("{}1{}\n", "[".repeat(300), "]".repeat(300));
    assert_eq!(of(&nested, Equality::Plain), None);
    let shallow = format!("{}1{}\n", "[".repeat(100), "]".repeat(100));
    assert!(of(&shallow, Equality::Plain).is_some());
}
