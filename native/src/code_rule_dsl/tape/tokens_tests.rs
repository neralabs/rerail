//! Named-only tapes (RER-131): which tokens the writer stores (`TokenGrammar::stored_children`).

use super::tokens::{TokenGrammar, grammar_json};
use super::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::AstGrep;
use ast_grep_core::language::Language as _;
use std::sync::Arc;

/// `source`'s tape, restored.
fn tape(lang: GrammarDialect, source: &str) -> TapeDoc {
    let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
    let grammar = Grammar::new(lang).unwrap();
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
}

/// The texts of the tape's anonymous nodes, in preorder.
fn stored_tokens(lang: GrammarDialect, source: &str) -> Vec<String> {
    let doc = AstGrep::doc(tape(lang, source));
    doc.root().dfs().filter(|node| !node.is_named()).map(|node| node.text().into_owned()).collect()
}

/// The texts of tree-sitter's anonymous nodes, in preorder.
fn tree_tokens(lang: GrammarDialect, source: &str) -> Vec<String> {
    let doc = AstGrep::doc(ast_grep_core::tree_sitter::StrDoc::new(source, lang));
    doc.root().dfs().filter(|node| !node.is_named()).map(|node| node.text().into_owned()).collect()
}

#[test]
fn every_grammar_builds() {
    for lang in [
        GrammarDialect::Python,
        GrammarDialect::JavaScript,
        GrammarDialect::TypeScript,
        GrammarDialect::Tsx,
        GrammarDialect::Sql,
        GrammarDialect::Vue,
        GrammarDialect::Rust,
        GrammarDialect::Php,
    ] {
        let language = Grammar::new(lang).unwrap().kinds.grammar().unwrap().clone();
        let grammar = TokenGrammar::from_json(grammar_json(lang).unwrap(), &language).unwrap();
        assert!(grammar.kind_count() > 20, "{lang:?}");
    }
}

/// An implied token is never a record; a token that says more than its kind and named children
/// is stored as tree-sitter has it.
#[test]
fn the_tape_stores_the_tokens_the_grammar_does_not_imply() {
    let py = GrammarDialect::Python;
    // A call's punctuation and a statement's keyword: implied. `def` is not left out: `async def`
    // holds it beside a choice, so it is kept in every gap (`tokens::kept`).
    assert_eq!(stored_tokens(py, "f(a, b)\n"), Vec::<String>::new());
    assert_eq!(stored_tokens(py, "def f():\n    return 1\n"), ["def"]);
    // A trailing comma: the call's shortest end is `)` alone, so both are stored.
    assert_eq!(stored_tokens(py, "f(a, b,)\n"), [",", ")"]);
    // An operator: as short as every other.
    assert_eq!(stored_tokens(py, "a + b\n"), ["+"]);
    assert_eq!(stored_tokens(py, "a < b\n"), ["<"]);
    // `async`: optional.
    assert_eq!(stored_tokens(py, "async def f():\n    pass\n"), ["async", "def"]);
    // A keyword-argument's `=`: implied in every gap it stands in, so never stored.
    assert_eq!(stored_tokens(py, "f(a=1)\n"), Vec::<String>::new());
    // A slice's `:` is no delimiter: kept where its gap is implied too.
    assert_eq!(stored_tokens(py, "x[a:b]\n"), [":"]);
    // `else` and `except` follow from their clauses' kinds.
    assert_eq!(stored_tokens(py, "if a:\n    b\nelse:\n    c\n"), Vec::<String>::new());
    // A bare `except`: its keyword and `:` stand beside a choice in some gap (`except E:`, `except
    // E as e:`), so they are kept in every gap.
    assert_eq!(stored_tokens(py, "try:\n    a\nexcept:\n    b\n"), ["except", ":"]);
    let js = GrammarDialect::JavaScript;
    // An array's separators are implied; a hole is not.
    assert_eq!(stored_tokens(js, "x = [1, 2];\n").iter().filter(|t| *t == ",").count(), 0);
    assert!(stored_tokens(js, "x = [1,, 2];\n").iter().filter(|t| *t == ",").count() >= 2);
}

/// What the tape stores is tree-sitter's tokens in order, implied ones left out.
#[test]
fn stored_tokens_are_tree_sitters_in_order() {
    for (lang, source) in [
        (GrammarDialect::Python, "async def f(a, b=1):  # c\n    return a if a is not b else [x for x in b]\nimport os, sys\nfrom . import (x, y,)\n"),
        (GrammarDialect::JavaScript, "const f = async (a, b) => { let x = [1,,2]; return a?.b ?? 0; };\nfor (const k of o) if (!k) break;\n"),
        (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\ninterface I { readonly a: string; }\n"),
        (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\n"),
        (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n"),
        (GrammarDialect::Sql, "SELECT a, b FROM t WHERE a = 1 AND b IN (1, 2);\n"),
    ] {
        let (stored, tree) = (stored_tokens(lang, source), tree_tokens(lang, source));
        assert!(stored.len() < tree.len(), "{lang:?}: {stored:?}");
        let mut rest = tree.iter();
        for token in &stored {
            assert!(rest.any(|t| t == token), "{lang:?}: {token} out of order in {stored:?}");
        }
    }
}

/// RER-131 census (ignored): per language, tree-sitter's nodes and tokens, and how many the tape
/// stores. `RERAIL_TOKEN_CENSUS=<dialect>=<file list>[:<max>];...`; with `RERAIL_TOKEN_IMPLIED=<dir>`, each
/// dialect's implied token texts and counts to `<dir>/<dialect>.tsv`.
#[test]
#[ignore = "corpora: RERAIL_TOKEN_CENSUS"]
fn token_census() {
    let spec = std::env::var("RERAIL_TOKEN_CENSUS").expect("RERAIL_TOKEN_CENSUS");
    for entry in spec.split(';').filter(|entry| !entry.trim().is_empty()) {
        let (label, list) = entry.split_once('=').expect("<dialect>=<list>");
        let (list, most) = list.split_once(':').map_or((list, usize::MAX), |(list, most)| (list, most.parse().expect("a count")));
        let lang = match label.trim() {
            "python" => GrammarDialect::Python,
            "javascript" => GrammarDialect::JavaScript,
            "typescript" => GrammarDialect::TypeScript,
            "tsx" => GrammarDialect::Tsx,
            "rust" => GrammarDialect::Rust,
            "php" => GrammarDialect::Php,
            "sql" => GrammarDialect::Sql,
            other => panic!("unknown dialect {other}"),
        };
        let (mut files, mut nodes, mut tokens, mut records, mut kept) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut by_text = std::collections::BTreeMap::<String, usize>::new();
        let mut implied = std::collections::BTreeMap::<String, usize>::new();
        for path in std::fs::read_to_string(list).expect("a file list").lines().take(most) {
            let Ok(source) = std::fs::read_to_string(path) else { continue };
            let native = AstGrep::doc(ast_grep_core::tree_sitter::StrDoc::new(&source, lang));
            nodes += native.root().dfs().count();
            tokens += native.root().dfs().filter(|n| !n.is_named()).count();
            let doc = AstGrep::doc(tape(lang, &source));
            records += doc.root().dfs().count();
            // Tree-sitter's token texts less the tape's, as multisets: what the grammar implied.
            let mut held = std::collections::BTreeMap::<String, usize>::new();
            for node in doc.root().dfs().filter(|n| !n.is_named()) {
                *held.entry(node.text().into_owned()).or_default() += 1;
            }
            for node in native.root().dfs().filter(|n| !n.is_named()) {
                let text = node.text().into_owned();
                match held.get_mut(&text) {
                    Some(left) if *left > 0 => *left -= 1,
                    _ => *implied.entry(text).or_default() += 1,
                }
            }
            for node in doc.root().dfs().filter(|n| !n.is_named()) {
                kept += 1;
                *by_text.entry(node.text().into_owned()).or_default() += 1;
            }
            files += 1;
        }
        if let Ok(dir) = std::env::var("RERAIL_TOKEN_IMPLIED") {
            let lines: String = implied.iter().map(|(text, count)| format!("{count}\t{}\n", text.replace('\n', "\\n"))).collect();
            std::fs::write(std::path::Path::new(&dir).join(format!("{label}.tsv")), lines).unwrap();
        }
        let mut top: Vec<_> = by_text.into_iter().collect();
        top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        eprintln!(
            "{label}: {files} files, {nodes} nodes ({tokens} tokens) -> {records} records ({:.3}x), {kept} tokens stored ({:.1}% of tokens); top {:?}",
            records as f64 / nodes.max(1) as f64,
            100.0 * kept as f64 / tokens.max(1) as f64,
            &top[..top.len().min(16)]
        );
    }
}

/// Every pack rule's patterns: a token the filter keeps only because the pattern's tree has an
/// error (`TokenGrammar::stored_children` keeps all under one), which a document's tape may not
/// store. Lists each such pattern.
#[test]
fn pack_patterns_keep_no_token_under_an_error() {
    use ast_grep_core::matcher::PatternNode;
    use crate::code_rule_dsl::language::{PatternSource, dialect_by_label, rule_pattern};
    fn tokens_under_error(node: &PatternNode, under: bool) -> bool {
        match node {
            PatternNode::Terminal { is_named, .. } => under && !is_named,
            PatternNode::MetaVar { .. } => false,
            PatternNode::Internal { kind_id, children } => {
                let under = under || *kind_id == u16::MAX;
                children.iter().any(|child| tokens_under_error(child, under))
            }
        }
    }
    fn walk(value: &serde_json::Value, lang: Option<GrammarDialect>, found: &mut Vec<(GrammarDialect, String)>) {
        let lang = value.get("language").and_then(|l| l.as_str()).and_then(dialect_by_label).or(lang);
        match value {
            serde_json::Value::Object(map) => {
                if let (Some(lang), Some(pattern)) = (lang, map.get("pattern")) {
                    match pattern {
                        serde_json::Value::String(text) => found.push((lang, text.clone())),
                        serde_json::Value::Object(p) => {
                            if let (Some(c), Some(s)) = (p.get("context").and_then(|v| v.as_str()), p.get("selector").and_then(|v| v.as_str())) {
                                found.push((lang, format!("{c}\u{0}{s}")));
                            }
                        }
                        _ => {}
                    }
                }
                map.values().for_each(|v| walk(v, lang, found));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, lang, found)),
            _ => {}
        }
    }
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("../packs");
    let mut found = Vec::new();
    for pack in std::fs::read_dir(&root).unwrap() {
        let Ok(rules) = std::fs::read_dir(pack.unwrap().path().join("rules")) else { continue };
        for rule in rules {
            let path = rule.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                walk(&serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap(), None, &mut found);
            }
        }
    }
    found.sort();
    found.dedup();
    let mut bad = Vec::new();
    for (lang, text) in &found {
        let source = match text.split_once('\u{0}') {
            Some((context, selector)) => PatternSource::Contextual { context, selector },
            None => PatternSource::Plain(text),
        };
        let Ok(pattern) = rule_pattern(source, *lang) else { continue };
        if tokens_under_error(&pattern.node, false) {
            bad.push(format!("{lang:?}: {text:?}"));
        }
    }
    eprintln!("{} patterns; {} keep a token under an error", found.len(), bad.len());
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The gaps of a tape's parent, interleaved with its stored children, are tree-sitter's children
/// (extras aside), by kind: every token a gap test reads is the one tree-sitter has there (t31).
/// The number of parents checked, or the first that differs.
fn gaps_are_tree_sitters(lang: GrammarDialect, source: &str) -> Result<usize, String> {
    use super::tokens::stored_gaps;
    use std::collections::HashMap;
    let language = Grammar::new(lang).unwrap().kinds.grammar().unwrap().clone();
    let grammar = TokenGrammar::of(lang, &language).unwrap();
    let native = AstGrep::doc(ast_grep_core::tree_sitter::StrDoc::new(source, lang));
    let mut by_place: HashMap<(usize, usize, String), Vec<Vec<String>>> = HashMap::new();
    for node in native.root().dfs().filter(|node| node.is_named()) {
        let children = node.children().filter(|child| !grammar.is_extra(&child.kind())).map(|child| child.kind().into_owned()).collect();
        by_place.entry((node.range().start, node.range().end, node.kind().into_owned())).or_default().push(children);
    }
    let doc = AstGrep::doc(tape(lang, source));
    let mut checked = 0;
    for node in doc.root().dfs().filter(|node| node.is_named()) {
        let Some((children, gaps)) = stored_gaps(&grammar, &node) else { continue };
        let mut sequence = Vec::new();
        for (child, before) in children.iter().zip(&gaps.before) {
            if grammar.is_extra(&child.kind()) {
                continue;
            }
            sequence.extend(before.iter().map(|(token, _)| (*token).to_owned()));
            sequence.push(child.kind().into_owned());
        }
        sequence.extend(gaps.end.iter().map(|(token, _)| (*token).to_owned()));
        let key = (node.range().start, node.range().end, node.kind().into_owned());
        let candidates = by_place.get(&key).ok_or_else(|| format!("{lang:?}: no tree-sitter node {key:?}"))?;
        if !candidates.contains(&sequence) {
            return Err(format!("{lang:?}: {} {:?}: gaps give {sequence:?}, tree-sitter {:?}", key.2, &source[key.0..key.1.min(key.0 + 80)], candidates[0]));
        }
        checked += 1;
    }
    Ok(checked)
}

#[test]
fn gaps_give_tree_sitters_tokens() {
    for (lang, source) in [
        (GrammarDialect::Python, "async def f(a, b=1, *c, **d) -> int:  # c\n    return a if a is not b else [x for x in b if x]\nimport os, sys\nfrom . import (x, y,)\nv = sys.version[:3]\ntry:\n    pass\nexcept (A, B) as e:\n    raise\nelse:\n    pass\nd = {'a': 1, **e}\nf(a, *b, k=v)\nlambda x: x\nwith a as b, c:\n    del a[1:2, ::3]\n"),
        (GrammarDialect::JavaScript, "const f = async (a, b) => { let x = [1,,2]; return a?.b ?? 0; };\nfor (const k of o) if (!k) break;\nlet o = { a: 1, b, [c]: 2, get d() { return 1; } };\nnew C(a, b);\nexport default function () {}\n"),
        (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\ninterface I { readonly a: string; m(x: number): void }\nclass C<T> extends B implements I { constructor(private x: T = 1) { super(); } }\ndeclare global { var g: number }\n"),
        (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\nstruct P { a: u8, b: u8 }\nfn g(P { a, b: c }: P) {}\n"),
        (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n$x = ['k' => 1, 2];\n"),
    ] {
        let checked = gaps_are_tree_sitters(lang, source).unwrap_or_else(|error| panic!("{error}"));
        assert!(checked > 10, "{lang:?}: {checked}");
    }
}

/// The same over corpora (ignored): `RERAIL_GAP_CORPUS=<dialect>=<file list>[:<max>];...`.
#[test]
#[ignore = "corpora: RERAIL_GAP_CORPUS"]
fn gaps_give_tree_sitters_tokens_on_corpora() {
    let spec = std::env::var("RERAIL_GAP_CORPUS").expect("RERAIL_GAP_CORPUS");
    let mut failures = Vec::new();
    for entry in spec.split(';').filter(|entry| !entry.trim().is_empty()) {
        let (label, list) = entry.split_once('=').expect("<dialect>=<list>");
        let (list, most) = list.split_once(':').map_or((list, usize::MAX), |(list, most)| (list, most.parse().expect("a count")));
        let lang = crate::code_rule_dsl::language::dialect_by_label(label.trim()).expect("a dialect");
        let (mut files, mut parents) = (0, 0);
        for path in std::fs::read_to_string(list).expect("a file list").lines().take(most) {
            let Ok(source) = std::fs::read_to_string(path) else { continue };
            match gaps_are_tree_sitters(lang, &source) {
                Ok(checked) => parents += checked,
                Err(error) => failures.push(format!("{path}: {error}")),
            }
            files += 1;
        }
        eprintln!("{label}: {files} files, {parents} parents checked");
    }
    for failure in failures.iter().take(20) {
        eprintln!("{failure}");
    }
    assert!(failures.is_empty(), "{} files differ", failures.len());
}
