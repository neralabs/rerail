//! The tree is well formed over every tree the oracle records, and ast-grep patterns match it.
//!
//! The invariant test builds trees from Forte's recorded parses (the oracle's cases; where a case
//! records only its digest, the port's parse that hashes to it); the pattern, field and error tests
//! parse with the port; the robustness test runs the tree, the view and the tape on input no
//! template is.
//!
//! Environment: `FORTE_SUITE` checks another suite file (e.g. the full dump) instead of the
//! checked-in one, `RERAIL_FORTE_CORPUS` adds the corpus dump.

use super::super::node::{FlatNode, NodeKind};
use super::super::tests::{
    digested_tree, oracle_errors, oracle_nodes, oracle_tokens, prefix_positions, read_json,
    read_oracle, robustness_inputs, source_of,
};
use super::super::token::Token;
use super::super::{ParseError, Parsed};
use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use ast_grep_core::{AstGrep, Pattern};
use serde_json::Value as Json;

fn tree_of(source: &[u8], tree: &Json) -> BladeTree {
    BladeTree::new(
        source.to_vec(),
        Ok(Parsed {
            tokens: oracle_tokens(tree),
            nodes: oracle_nodes(tree),
            errors: oracle_errors(tree),
        }),
    )
}

/// What breaks the tree's invariants, if anything: children within their parent, in order,
/// without overlap, links consistent, every reachable flat node once.
fn check(tree: &BladeTree, reachable: usize) -> Result<(), String> {
    let len = tree.source().len() as u32;
    let root = &tree.nodes[0];
    if root.kind != Kind::Document as u16
        || root.start != 0
        || root.end != len
        || root.parent != NONE
    {
        return Err("root is not the whole document".into());
    }
    for (index, rec) in tree.nodes.iter().enumerate() {
        if rec.start > rec.end || rec.end > len {
            return Err(format!("node {index}: span {}..{}", rec.start, rec.end));
        }
        let mut child = rec.first;
        let mut previous = NONE;
        let mut count = 0;
        while child != NONE {
            let c = &tree.nodes[child as usize];
            if c.parent != index as u32 || c.prev != previous {
                return Err(format!("node {child}: links"));
            }
            if c.start < rec.start || c.end > rec.end {
                return Err(format!(
                    "node {child} ({}..{}) outside its parent {index} ({}..{})",
                    c.start, c.end, rec.start, rec.end
                ));
            }
            if previous != NONE && tree.nodes[previous as usize].end > c.start {
                let p = &tree.nodes[previous as usize];
                return Err(format!(
                    "node {child} ({}..{}) overlaps or precedes its previous sibling {previous} ({}..{})",
                    c.start, c.end, p.start, p.end
                ));
            }
            previous = child;
            child = c.next;
            count += 1;
        }
        if count != rec.children {
            return Err(format!(
                "node {index}: {count} children linked, {} counted",
                rec.children
            ));
        }
    }
    let mut seen = vec![
        false;
        tree.forte
            .iter()
            .filter(|f| **f != NONE)
            .map(|f| *f as usize + 1)
            .max()
            .unwrap_or(0)
    ];
    for forte in tree.forte.iter().filter(|f| **f != NONE) {
        if std::mem::replace(&mut seen[*forte as usize], true) {
            return Err(format!("flat node {forte} twice"));
        }
    }
    if seen.iter().filter(|s| **s).count() != reachable {
        return Err("not every reachable flat node is a node".into());
    }
    Ok(())
}

/// The flat nodes linked from the root.
fn reachable(flat: &[FlatNode]) -> usize {
    let mut seen = vec![false; flat.len()];
    let mut stack = if flat.is_empty() {
        vec![]
    } else {
        vec![0usize]
    };
    let mut count = 0;
    while let Some(node) = stack.pop() {
        if std::mem::replace(&mut seen[node], true) {
            continue;
        }
        count += 1;
        let mut child = flat[node].first_child;
        while let Ok(index) = usize::try_from(child) {
            if index >= flat.len() || seen[index] {
                break;
            }
            stack.push(index);
            child = flat[index].next_sibling;
        }
    }
    count
}

/// How Forte's own records break the invariants, counted per tree.
#[derive(Default, Debug)]
struct ForteShape {
    trees: usize,
    /// Trees where a node's token range, unioned with its children, overlaps a sibling's.
    overlapping_siblings: usize,
    /// Trees where flat nodes, neither inside the other, claim a token.
    shared_tokens: usize,
    /// Trees with a node that has neither tokens nor children.
    empty_nodes: usize,
    /// Trees where a byte that is not whitespace is in no leaf of the tree.
    unleaved_bytes: usize,
    examples: Vec<String>,
}

impl ForteShape {
    fn record(
        &mut self,
        label: &str,
        source: &[u8],
        tokens: &[Token],
        flat: &[FlatNode],
        tree: &BladeTree,
    ) {
        self.trees += 1;
        let range = |node: &FlatNode| {
            let start = usize::try_from(node.token_start)
                .unwrap_or(tokens.len())
                .min(tokens.len());
            start..(start + usize::try_from(node.token_count).unwrap_or(0)).min(tokens.len())
        };
        // Naive spans: a node's whole token range and its children.
        let mut naive: Vec<Option<(u32, u32)>> = vec![None; flat.len()];
        let mut parent_of = vec![usize::MAX; flat.len()];
        let mut order = Vec::new();
        let mut stack = vec![0usize];
        let mut seen = vec![false; flat.len()];
        while let Some(node) = stack.pop() {
            if std::mem::replace(&mut seen[node], true) {
                continue;
            }
            order.push(node);
            let mut child = flat[node].first_child;
            while let Ok(index) = usize::try_from(child) {
                if index >= flat.len() || seen[index] {
                    break;
                }
                parent_of[index] = node;
                stack.push(index);
                child = flat[index].next_sibling;
            }
        }
        let mut empty = false;
        for &node in order.iter().rev() {
            let own = range(&flat[node]);
            let mut span =
                (!own.is_empty()).then(|| (tokens[own.start].start, tokens[own.end - 1].end));
            let mut child = flat[node].first_child;
            while let Ok(index) = usize::try_from(child) {
                if index >= flat.len() || parent_of[index] != node {
                    break;
                }
                if let Some((s, e)) = naive[index] {
                    span = Some(span.map_or((s, e), |(a, b)| (a.min(s), b.max(e))));
                }
                child = flat[index].next_sibling;
            }
            empty |= span.is_none() && node != 0;
            naive[node] = span;
        }
        let mut overlapping = false;
        for &node in &order {
            let mut spans: Vec<(u32, u32)> = Vec::new();
            let mut child = flat[node].first_child;
            while let Ok(index) = usize::try_from(child) {
                if index >= flat.len() || parent_of[index] != node {
                    break;
                }
                spans.extend(naive[index]);
                child = flat[index].next_sibling;
            }
            spans.sort();
            overlapping |= spans.windows(2).any(|pair| pair[0].1 > pair[1].0);
        }
        // Tokens claimed by two nodes neither of which contains the other.
        let ancestor = |mut node: usize, of: usize| {
            while node != usize::MAX {
                if node == of {
                    return true;
                }
                node = parent_of[node];
            }
            false
        };
        let mut claimant: Vec<Vec<usize>> = vec![Vec::new(); tokens.len()];
        for &node in &order {
            if node != 0 && flat[node].kind != NodeKind::DirectiveBlock {
                for token in range(&flat[node]) {
                    claimant[token].push(node);
                }
            }
        }
        let shared = claimant.iter().any(|nodes| {
            nodes.iter().any(|a| {
                nodes
                    .iter()
                    .any(|b| a != b && !ancestor(*a, *b) && !ancestor(*b, *a))
            })
        });
        // Bytes no leaf holds.
        let mut leaved = vec![false; source.len()];
        for rec in tree.nodes.iter().filter(|rec| rec.children == 0) {
            leaved[rec.start as usize..rec.end as usize].fill(true);
        }
        let unleaved = source.iter().zip(&leaved).any(|(byte, leaved)| {
            !leaved && !super::super::scan::is_trim(*byte) && !super::super::scan::is_space(*byte)
        });
        self.overlapping_siblings += usize::from(overlapping);
        self.shared_tokens += usize::from(shared);
        self.empty_nodes += usize::from(empty);
        self.unleaved_bytes += usize::from(unleaved);
        if (overlapping || shared || empty || unleaved) && self.examples.len() < 12 {
            self.examples.push(format!(
                "{label} overlapping={overlapping} shared={shared} empty={empty} unleaved={unleaved} {:?}",
                String::from_utf8_lossy(&source[..source.len().min(160)])
            ));
        }
    }
}

/// Every recorded tree: the suite (`FORTE_SUITE` or the checked-in file, whose whole-template
/// digests stand for the port's trees that hash to them), the fixtures, the corpus samples and,
/// with `RERAIL_FORTE_CORPUS`, the corpus dump.
fn each_tree(mut visit: impl FnMut(&str, &[u8], &Json)) {
    let suite = match std::env::var("FORTE_SUITE") {
        Ok(path) => read_json(&path),
        Err(_) => read_oracle("suite.expected.json"),
    };
    let mut files = vec![suite, read_oracle("fixtures.expected.json")];
    for corpus in ["bookstack", "filament", "bagisto", "laravel"] {
        files.push(read_oracle(&format!("corpus/{corpus}.expected.json")));
    }
    if let Ok(path) = std::env::var("RERAIL_FORTE_CORPUS") {
        files.push(
            serde_json::from_str(&std::fs::read_to_string(path).expect("corpus"))
                .expect("corpus JSON"),
        );
    }
    for (file, oracle) in files.iter().enumerate() {
        for (index, case) in oracle["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .enumerate()
        {
            let source = source_of(case);
            let digested = digested_tree(&source, case, &oracle["registries"]);
            if let Some(tree) = case.get("tree").or(digested.as_ref()) {
                if tree.get("x").is_none() {
                    visit(&format!("file {file} case {index}"), &source, tree);
                }
            } else if let Some(trees) = case.get("trees").and_then(Json::as_array) {
                for (tree, length) in trees.iter().zip(prefix_positions(source.len())) {
                    if tree.get("x").is_none() {
                        visit(
                            &format!("file {file} case {index} prefix {length}"),
                            &source[..length],
                            tree,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn trees_are_well_formed() {
    let mut shape = ForteShape::default();
    let mut failures = Vec::new();
    each_tree(|label, source, recorded| {
        let tokens = oracle_tokens(recorded);
        let flat = oracle_nodes(recorded);
        let tree = tree_of(source, recorded);
        if let Err(error) = check(&tree, reachable(&flat))
            && failures.len() < 20
        {
            failures.push(format!(
                "{label}: {error}\n  {:?}",
                String::from_utf8_lossy(&source[..source.len().min(200)])
            ));
        }
        shape.record(label, source, &tokens, &flat, &tree);
    });
    eprintln!(
        "{} trees; Forte's records: {} with overlapping sibling ranges, {} with tokens two unrelated nodes claim, {} with empty nodes; {} with non-whitespace bytes in no leaf",
        shape.trees,
        shape.overlapping_siblings,
        shape.shared_tokens,
        shape.empty_nodes,
        shape.unleaved_bytes
    );
    for example in &shape.examples {
        eprintln!("  {example}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(shape.trees > 6_000, "{} trees", shape.trees);
}

/// What Rerail builds on the port finishes, without a panic and within its stack, on input no
/// template is (`robustness_inputs`) and on sub-templates nested 20,000 deep (`<script>` text that
/// runs to the end, holding a component tag or a `@php` block): the tree keeps its invariants, its
/// syntax errors stay inside the template, and the tape (tree and view) encodes and round-trips.
#[test]
fn trees_views_and_tapes_survive_random_and_truncated_templates() {
    let grammar = Grammar::new(GrammarDialect::Blade).expect("Blade's grammar");
    let mut inputs = robustness_inputs();
    for nested in ["<script><x-a :b=\"$c\"/>", "<script>@php $a; @endphp"] {
        inputs.push(nested.repeat(20_000).into_bytes());
    }
    let mut failures = Vec::new();
    for input in &inputs {
        let source = String::from_utf8_lossy(input);
        let bytes = source.as_bytes();
        let mut directives = Directives::with_defaults();
        directives.set_accept_all(true);
        let parsed = super::super::parse(
            bytes,
            directives,
            DepthLimits::default(),
            &mut Budget::for_template(bytes.len()),
        );
        let flat = parsed.as_ref().ok().map(|parsed| reachable(&parsed.nodes));
        let tree = BladeTree::new(bytes.to_vec(), parsed);
        let mut fail = |error: String| {
            if failures.len() < 20 {
                failures.push(format!(
                    "{error}\n  {:?}",
                    &source[..source.floor_char_boundary(200)]
                ));
            }
        };
        if let Some(reachable) = flat
            && let Err(error) = check(&tree, reachable)
        {
            fail(error);
        }
        if let Some(range) = tree
            .error_ranges()
            .iter()
            .find(|range| range.start > range.end || range.end > bytes.len())
        {
            fail(format!("syntax error {range:?} outside the template"));
        }
        let round_trip =
            TapeImage::encode_template(&grammar, &source, &|| false).and_then(|image| {
                let encoded = image.to_bytes()?;
                let decoded = TapeImage::from_bytes(&encoded, &grammar)?.to_bytes()?;
                (decoded == encoded)
                    .then_some(())
                    .ok_or_else(|| "the tape did not round-trip".to_owned())
            });
        if let Err(error) = round_trip {
            fail(error);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn lang() -> BladeLang {
    BladeLang
}

const TEMPLATE: &str = "<div>{{ $user->name }}</div>\n@include('partials.header')\n@include( $view , ['a' => 1])\n{!! $raw !!}\n<form>@csrf</form>\n@csrf";

/// Each match's kind, text and the text of `var`.
fn matches(pattern: &str, var: &str) -> Vec<(String, String, Option<String>)> {
    let root = AstGrep::doc(lang().doc(TEMPLATE.as_bytes()));
    let pattern = Pattern::try_new(pattern, lang()).expect("pattern");
    root.root()
        .find_all(&pattern)
        .map(|found| {
            let bound = found
                .get_env()
                .get_match(var)
                .map(|node| node.text().to_string());
            (found.kind().to_string(), found.text().to_string(), bound)
        })
        .collect()
}

#[test]
fn metavariables_bind_echo_expressions_and_directive_arguments() {
    assert_eq!(
        matches("{{ $X }}", "X"),
        [(
            "echo".into(),
            "{{ $user->name }}".into(),
            Some("$user->name".into())
        )]
    );
    assert_eq!(
        matches("@include($V)", "V"),
        [
            (
                "directive".into(),
                "@include('partials.header')".into(),
                Some("'partials.header'".into())
            ),
            (
                "directive".into(),
                "@include( $view , ['a' => 1])".into(),
                Some("$view , ['a' => 1]".into())
            ),
        ]
    );
}

#[test]
fn exact_blade_patterns_match() {
    let found = matches("@csrf", "_");
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found
            .iter()
            .all(|(kind, text, _)| kind == "directive_name" && text == "@csrf")
    );
}

#[test]
fn fields_name_the_parts() {
    let source =
        "<a href=\"x\">{{ $y }}</a>@if($a) b @else c @endif<?php echo 1; ?>@php $z = 2; @endphp";
    let root = AstGrep::doc(lang().doc(source.as_bytes()));
    let doc = root.root();
    let kinds: Vec<String> = doc
        .children()
        .map(|child| child.kind().to_string())
        .collect();
    assert_eq!(
        kinds,
        ["element", "directive_block", "php_tag", "php_block"]
    );
    let field = |node: &ast_grep_core::Node<'_, BladeDoc>, name: &str| {
        node.field(name).map(|n| n.text().to_string())
    };
    let element = doc.child(0).expect("element");
    let tree = &element.get_doc().tree;
    assert_eq!(tree.forte_index(element.node_id()), Some(1));
    assert_eq!(
        tree.forte_index(element.child(0).expect("<").node_id()),
        None
    );
    assert_eq!(field(&element, "name").as_deref(), Some("a"));
    assert_eq!(field(&element, "closing_name").as_deref(), Some("a"));
    let attribute = element
        .children()
        .find(|c| c.kind() == "attribute")
        .expect("attribute");
    assert_eq!(field(&attribute, "name").as_deref(), Some("href"));
    assert_eq!(field(&attribute, "value").as_deref(), Some("\"x\""));
    let echo = element
        .children()
        .find(|c| c.kind() == "echo")
        .expect("echo");
    assert_eq!(field(&echo, "expression").as_deref(), Some("$y"));
    let block = doc.child(1).expect("block");
    let start = block.field("start").expect("start");
    assert_eq!(field(&start, "name").as_deref(), Some("@if"));
    assert_eq!(field(&start, "arguments").as_deref(), Some("$a"));
    assert_eq!(
        field(&block.field("end").expect("end"), "name").as_deref(),
        Some("@endif")
    );
    assert_eq!(
        field(&doc.child(2).expect("tag"), "code").as_deref(),
        Some("echo 1;")
    );
    assert_eq!(
        field(&doc.child(3).expect("block"), "code").as_deref(),
        Some("$z = 2;")
    );
    // Two trees never share node ids.
    let other = AstGrep::doc(lang().doc(source.as_bytes()));
    assert_ne!(other.root().node_id(), doc.node_id());
}

#[test]
fn errors_are_ranges_not_nodes() {
    let tree = parse_tree(b"{{ $a");
    assert_eq!(tree.error_ranges(), vec![5..5]);
    let doc = BladeDoc::new(tree, lang());
    let root = AstGrep::doc(doc);
    assert!(root.root().dfs().all(|node| !node.is_error()));
    let failed = BladeTree::new(b"<div>".to_vec(), Err(ParseError::Budget));
    assert_eq!(failed.error_ranges(), vec![0..5]);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed.parse_error(), Some(&ParseError::Budget));
}
