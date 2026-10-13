use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::value::rust::{RustValue, literal};
use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_core::{Doc, Node};

/// The first string literal of a macro call's source, decoded.
fn template_of(src: &str) -> Option<String> {
    fn find<D: Doc>(node: &Node<'_, D>) -> Option<String> {
        if matches!(
            node.kind().as_ref(),
            "string_literal" | "raw_string_literal"
        ) {
            return match literal(node) {
                RustValue::Str(text) => Some(text),
                _ => None,
            };
        }
        node.children().find_map(|child| find(&child))
    }
    let root = GrammarDialect::Rust.ast_grep(format!("fn main() {{ {src}; }}"));
    find(&root.root())
}

/// Whether rustc's error is the template parser's (`rustc_parse_format` or the trait check).
fn is_template_error(message: &str) -> bool {
    message.starts_with("invalid format string") || message.starts_with("unknown format trait")
}

#[test]
fn parse_matches_rustc_on_every_template() {
    let oracle: serde_json::Value = serde_json::from_str(crate::code_rule_dsl::oracle!(
        "rust/matcher/format_args/rust.expected.json"
    ))
    .expect("oracle JSON");
    let mut checked = 0usize;
    let mut refused = 0usize;
    for case in oracle["cases"].as_array().expect("cases") {
        if case["calls"] == false {
            continue;
        }
        let name = case["name"].as_str().expect("name");
        let src = case["src"].as_str().expect("src");
        let template = template_of(src).unwrap_or_else(|| panic!("{name}: no template in {src}"));
        let rustc_refuses = case["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| is_template_error(error["message"].as_str().expect("message")));
        let parsed = parse(&template);
        assert_eq!(
            parsed.is_err(),
            rustc_refuses,
            "{name}: {template:?} parsed as {parsed:?}, rustc errors {}",
            case["errors"]
        );
        checked += 1;
        refused += usize::from(rustc_refuses);
    }
    assert!(checked >= 140, "only {checked} templates checked");
    assert!(refused >= 13, "only {refused} refused templates");
}

#[test]
fn escapes_start_at_their_second_brace() {
    let pieces = parse("a{{b}}c{}").expect("parses");
    let ranges: Vec<Range<usize>> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Literal { range } | Piece::Field { range, .. } => range.clone(),
        })
        .collect();
    assert_eq!(ranges, vec![0..1, 2..4, 5..7, 7..9]);
    let text = "a{{b}}c{}";
    let printed: Vec<&str> = pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Literal { range } => Some(&text[range.clone()]),
            Piece::Field { .. } => None,
        })
        .collect();
    assert_eq!(printed, vec!["a", "{b", "}c"]);
}

#[test]
fn field_carries_argument_counts_and_spec() {
    let template = "{name:*^w$.*}";
    let pieces = parse(template).expect("parses");
    let [
        Piece::Field {
            range,
            argument,
            width,
            precision,
            spec_range,
            spec,
        },
    ] = pieces.as_slice()
    else {
        panic!("one field, got {pieces:?}");
    };
    assert_eq!(range, &(0..template.len()));
    assert_eq!(argument, &Argument::Name("name".to_string()));
    assert_eq!(width, &Some(Count::Name("w".to_string())));
    assert_eq!(precision, &Some(Count::Star));
    assert_eq!(&template[spec_range.clone()], "*^w$.*");
    assert_eq!(spec.fill, Some('*'));
    assert_eq!(spec.align, Some(Align::Center));
    assert_eq!(spec.ty, Trait::Display);
}

#[test]
fn zero_dollar_is_the_width_argument_not_the_flag() {
    let pieces = parse("{:0$}{:05}").expect("parses");
    let counts: Vec<(Option<Count>, bool)> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Field { width, spec, .. } => (width.clone(), spec.zero),
            Piece::Literal { .. } => panic!("no literal"),
        })
        .collect();
    assert_eq!(
        counts,
        vec![
            (Some(Count::Index(0)), false),
            (Some(Count::Literal(5)), true)
        ]
    );
}

#[test]
fn star_precision_takes_its_argument_before_the_value() {
    let pieces = parse("{:.*} {}").expect("parses");
    let binding = bind(&pieces, 3, &[]);
    let slots: Vec<(Slot, Option<Slot>)> = binding
        .fields
        .iter()
        .map(|field| (field.value.clone(), field.precision.clone()))
        .collect();
    assert_eq!(
        slots,
        vec![
            (Slot::Argument(1), Some(Slot::Argument(0))),
            (Slot::Argument(2), None)
        ]
    );
    assert!(binding.unused.is_empty() && binding.missing.is_empty());
}

#[test]
fn names_resolve_to_arguments_after_the_positional_ones() {
    let pieces = parse("{b} {a} {c} {:w$}").expect("parses");
    let named = vec!["a".to_string(), "b".to_string()];
    let binding = bind(&pieces, 1, &named);
    let values: Vec<Slot> = binding
        .fields
        .iter()
        .map(|field| field.value.clone())
        .collect();
    assert_eq!(
        values,
        vec![
            Slot::Argument(2),
            Slot::Argument(1),
            Slot::Capture("c".to_string()),
            Slot::Argument(0)
        ]
    );
    assert_eq!(
        binding.fields[3].width,
        Some(Slot::Capture("w".to_string()))
    );
    assert!(binding.unused.is_empty() && binding.missing.is_empty());
}

#[test]
fn missing_names_the_field_and_unused_the_argument() {
    let pieces = parse("{0} {3} {}").expect("parses");
    let binding = bind(&pieces, 2, &[]);
    assert_eq!(binding.missing, vec![2]);
    assert_eq!(binding.unused, vec![1]);
    assert_eq!(binding.fields[1].value, Slot::Missing(3));
}
