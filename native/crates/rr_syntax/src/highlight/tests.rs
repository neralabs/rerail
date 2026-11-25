use super::{Class, highlight};

/// Each classed token of `src`, as its text and class.
fn classed(src: &str) -> Vec<(&str, Class)> {
    highlight(src).tokens.iter().map(|t| (&src[t.start..t.end], t.class)).collect()
}

const RULE: &str = r#".policy "demo" revision 2
.mode log
// what it finds
#include "lib/core.rerail"
.decl seen(n: node, k: symbol) inline
.violation Bad(at: node) code "X" "a message"
Bad(a) :- kind(a, "call"), match("^f[0-9]+$", a), !seen(a, _), strlen(s) > 3, a ~ `foo($X)`, n = count : { seen(_, _) }.
.test cases exactly
.end
"#;

#[test]
fn each_token_is_classed_by_its_place_in_the_tree() {
    let got = classed(RULE);
    use Class::*;
    let expected = [
        (".policy", Keyword),
        ("\"demo\"", String),
        ("revision", Value),
        ("2", Number),
        (".mode", Keyword),
        ("log", Value),
        ("// what it finds", Comment),
        ("#include", Keyword),
        ("\"lib/core.rerail\"", String),
        (".decl", Keyword),
        ("seen", RelationDeclaration),
        ("n", Parameter),
        ("node", Type),
        ("inline", Keyword),
        (".violation", Keyword),
        ("Bad", RelationDeclaration),
        ("code", Keyword),
        ("\"a message\"", String),
        ("Bad", Relation),
        (":-", Operator),
        ("kind", Relation),
        ("\"call\"", String),
        ("\"^f[0-9]+$\"", Regexp),
        ("!", Operator),
        ("_", Variable),
        ("strlen", Function),
        (">", Operator),
        ("~", Operator),
        ("`foo($X)`", Code),
        ("count", Keyword),
        (".test", Keyword),
        ("cases", Test),
        ("exactly", Keyword),
        (".end", Keyword),
    ];
    let mut rest = got.iter();
    for want in expected {
        assert!(rest.any(|have| *have == want), "{want:?} not found in order in {got:#?}");
    }
    assert!(highlight(RULE).errors.is_empty(), "{:?}", highlight(RULE).errors);
}

#[test]
fn a_statement_that_does_not_parse_is_an_error_and_the_rest_still_classed() {
    let src = ".decl ok(n: node)\nok(a) :- kind(a, .\nok(b) :- kind(b, \"x\").\nbad( :- .\n";
    let found = highlight(src);
    let errors: Vec<_> = found.errors.iter().map(|e| (&src[e.start..e.end], e.message.as_str())).collect();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(!errors[0].1.starts_with("line "), "{errors:?}");
    assert_eq!(errors[1], ("bad( :- .", "this statement does not parse"), "{errors:?}");
    // The clause between the two errors is classed as any other.
    let got = classed(src);
    assert!(got.contains(&("\"x\"", Class::String)), "{got:?}");
    assert!(got.windows(2).any(|w| w == [("ok", Class::Relation), ("b", Class::Variable)]), "{got:?}");
}

/// On every `.rerail` file of the repository: tokens in order, apart, on character boundaries; and errors only for a
/// file the engine's parser refuses.
#[test]
fn every_rerail_file_highlights() {
    let corpus = crate::lean::tests::corpus();
    assert!(corpus.len() > 100, "{} files", corpus.len());
    for (path, src) in &corpus {
        let found = highlight(src);
        let mut at = 0;
        for t in &found.tokens {
            assert!(at <= t.start && t.start < t.end && t.end <= src.len(), "{path}: {t:?} after {at}");
            assert!(src.is_char_boundary(t.start) && src.is_char_boundary(t.end), "{path}: {t:?}");
            at = t.end;
        }
        let parses = crate::syntax::Parser::new(src).and_then(|mut p| p.statements()).is_ok();
        assert_eq!(found.errors.is_empty(), parses, "{path}: {:?}", found.errors);
    }
}
