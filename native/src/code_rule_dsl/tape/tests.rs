use super::*;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use ast_grep_core::AstGrep;
use ast_grep_core::matcher::Pattern;
use ast_grep_core::tree_sitter::StrDoc;

#[test]
fn scala_contextual_aliases_preserve_native_node_identity() {
    let source = "object A { def f(x: Int) = g(x) }";
    let (native, _, image, grammar) = fixture(source, GrammarDialect::Scala);
    assert!(!image.kind_overrides.is_empty());
    let persisted = TapeImage::from_bytes(&image.to_bytes().unwrap(), &grammar).unwrap();
    let tape =
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &persisted, &|| false).unwrap();

    fn native_nodes<'a>(node: TsNode<'a>, output: &mut Vec<TsNode<'a>>) {
        output.push(node);
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                native_nodes(cursor.node(), output);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    let mut expected = Vec::new();
    native_nodes(native.tree.root_node(), &mut expected);
    let actual = tape.root_node().dfs().collect::<Vec<_>>();
    assert_eq!(expected.len(), actual.len());
    for (native, restored) in expected.into_iter().zip(actual) {
        assert_eq!(restored.kind_id(), native.kind_id());
        assert_eq!(restored.kind(), native.kind());
        assert_eq!(restored.is_named(), native.is_named());
        assert_eq!(restored.range(), native.byte_range());
    }
}

fn fixture(
    source: &str,
    lang: GrammarDialect,
) -> (StrDoc<GrammarDialect>, TapeDoc, TapeImage, Arc<Grammar>) {
    let grammar = Grammar::new(lang).unwrap();
    let native = StrDoc::new(source, lang);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
    let tape = TapeDoc::restore(
        Arc::new(source.to_owned()),
        Arc::clone(&grammar),
        &image,
        &|| false,
    )
    .unwrap();
    (native, tape, image, grammar)
}

type MatchResult = (Range<usize>, Vec<(String, Vec<Range<usize>>)>);

fn matches<D: Doc>(doc: D, pattern: &str, lang: GrammarDialect) -> Vec<MatchResult> {
    let matcher = Pattern::try_new(pattern, lang).unwrap();
    let root = AstGrep::doc(doc);
    root.root()
        .find_all(&matcher)
        .map(|found| {
            let mut names = found.get_env().get_matched_variables().collect::<Vec<_>>();
            names.sort_by_key(|name| format!("{name:?}"));
            let captures = names
                .into_iter()
                .filter_map(|name| match &name {
                    ast_grep_core::meta_var::MetaVariable::Capture(key, _) => Some((
                        key.clone(),
                        found
                            .get_env()
                            .get_match(key)
                            .into_iter()
                            .map(|node| node.range())
                            .collect(),
                    )),
                    ast_grep_core::meta_var::MetaVariable::MultiCapture(key) => Some((
                        key.clone(),
                        found
                            .get_env()
                            .get_multiple_matches(key)
                            .into_iter()
                            .filter(|node| node.is_named())
                            .map(|node| node.range())
                            .collect(),
                    )),
                    _ => None,
                })
                .collect();
            (found.get_node().range(), captures)
        })
        .collect()
}

#[test]
fn persisted_image_round_trips_without_parsing() {
    let source = "const value = call(é);\n";
    let (native, _tape, image, grammar) = fixture(source, GrammarDialect::TypeScript);
    let bytes = image.to_bytes().unwrap();
    let decoded = TapeImage::from_bytes(&bytes, &grammar).unwrap();
    assert_eq!(decoded.to_bytes().unwrap(), bytes);
    let restored =
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &decoded, &|| false).unwrap();
    assert_eq!(
        matches(native, "call($A)", GrammarDialect::TypeScript),
        matches(restored, "call($A)", GrammarDialect::TypeScript)
    );
    let digest = Sha256::digest(&bytes);
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // The bytes number kinds by the grammar's symbol table: a grammar that adds a token (Patch 4's
    // `out`, Patch 13's `_arrow_body_end`, Patch 14's `yield` name, Patch 21's
    // `_jsx_closing_tag_lt`, Patch 24's `_no_line_terminator_here`,
    // `grammars/tree-sitter-typescript/PATCHES.md`) renumbers them.
    assert_eq!(
        digest,
        "227bea89873bd43db3dae4b3a9d1af873c096e79a0b308b83959a967fbd183ca"
    );
}

/// A persisted image keeps each embedded program's root, dialect and apartness (`EmbeddedProgram`),
/// and rejects an entry that names no dialect the language embeds, a flag that is neither 0 nor
/// 1, and a root that is no child of the tape's root.
#[test]
fn persisted_image_keeps_and_checks_its_embedded_programs() {
    let source = "<script lang=\"tsx\">let x = <a />;</script>\n";
    let grammar = Grammar::new(GrammarDialect::Html).unwrap();
    let image = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let [program] = image.programs[..] else {
        panic!("one program: {:?}", image.programs);
    };
    assert_eq!(program.dialect, GrammarDialect::Tsx);
    assert!(program.apart);
    let bytes = image.to_bytes().unwrap();
    assert_eq!(
        TapeImage::from_bytes(&bytes, &grammar).unwrap().programs,
        image.programs
    );
    // The program entry follows the header and the tables before it.
    let at = 160
        + image.rec.len()
        + image.spans.len()
        + image.checkpoints.len()
        + 8 * (image.par_overflow.len() + image.size_overflow.len())
        + 4 * (image.extra.len() + image.error.len() + image.missing.len())
        + 8 * image.syntax_errors.len();
    assert_eq!(bytes[at..at + 4], program.root.to_le_bytes());
    for (offset, byte) in [(4, 3), (5, 2)] {
        let mut corrupt = bytes.clone();
        corrupt[at + offset] = byte;
        assert!(
            TapeImage::from_bytes(&corrupt, &grammar).is_err(),
            "{offset}"
        );
    }
    let mut unknown = image.clone();
    unknown.programs[0].dialect = GrammarDialect::Python;
    assert!(unknown.to_bytes().is_err());
    let mut nested = image.clone();
    nested.programs[0].root += 1;
    assert!(TapeDoc::restore(Arc::new(source.to_owned()), grammar, &nested, &|| false).is_err());
}

#[test]
fn persisted_image_rejects_wrong_grammar_and_noncanonical_bytes() {
    let source = "x = 1 # extra\n";
    let (_native, _tape, image, grammar) = fixture(source, GrammarDialect::Python);
    let bytes = image.to_bytes().unwrap();
    let wrong = Grammar::new(GrammarDialect::TypeScript).unwrap();
    assert!(TapeImage::from_bytes(&bytes, &wrong).is_err());

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(TapeImage::from_bytes(&trailing, &grammar).is_err());

    let mut header = bytes.clone();
    header[9] = 1;
    assert!(TapeImage::from_bytes(&header, &grammar).is_err());

    let mut noncanonical = image.clone();
    noncanonical.extra.push(0);
    noncanonical.extra.push(0);
    assert!(TapeImage::from_bytes(&noncanonical.to_bytes().unwrap(), &grammar).is_err());

    let decoded = TapeImage::from_bytes(&bytes, &grammar).unwrap();
    assert!(
        TapeDoc::restore(
            Arc::new("x = 2 # extra\n".to_owned()),
            grammar,
            &decoded,
            &|| false,
        )
        .is_err()
    );
}

/// Corrupt alias and override counts are rejected before a table is reserved for them: a count the
/// image's bytes cannot back neither panics on capacity overflow nor asks for a huge allocation.
#[test]
fn persisted_image_rejects_oversized_alias_counts_without_allocating() {
    let (_native, _tape, image, grammar) = fixture("x = 1\n", GrammarDialect::Python);
    let bytes = image.to_bytes().unwrap();
    // Header count slots: aliases at 120, alias bytes at 128, overrides at 136.
    for (offset, count) in [
        (120, u64::MAX >> 1),
        (120, 1 << 20),
        (120, 1 << 16 | 1),
        (136, u64::MAX >> 1),
        (136, 1 << 20),
    ] {
        let mut corrupt = bytes.clone();
        corrupt[offset..offset + 8].copy_from_slice(&count.to_le_bytes());
        take_table_reservation();
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            TapeImage::from_bytes(&corrupt, &grammar).is_err()
        }));
        assert!(
            matches!(rejected, Ok(true)),
            "count {count} at {offset}: expected an error, got {:?}",
            rejected.map_err(|_| "a panic")
        );
        assert!(
            take_table_reservation() <= bytes.len(),
            "count {count} at {offset}: reserved a table the image's bytes cannot back"
        );
    }
}

#[test]
fn valid_native_root_ranges_survive_taping() {
    for (lang, cases) in [
        (
            GrammarDialect::Python,
            [
                "",
                " \t \n",
                "\n\n",
                "\n# note\n\n",
                "\n\nx = foo(1)\n\n# end\n",
                "é = foo(1)\r\n\r\n",
                "\u{feff}x = foo(1)\n",
                "x = foo(1)",
            ],
        ),
        (
            GrammarDialect::TypeScript,
            [
                "",
                " \t \n",
                "\n\n",
                "\n// note\n\n",
                "\n\nconst x = foo(1);\n\n// end\n",
                "const é = foo(1);\r\n\r\n",
                "\u{feff}const x = foo(1);\n",
                "const x = foo(1);",
            ],
        ),
    ] {
        for source in cases {
            let (native, tape, _, _) = fixture(source, lang);
            let left = AstGrep::doc(native);
            let right = AstGrep::doc(tape);
            let native_nodes = left.root().dfs().collect::<Vec<_>>();
            let tape_nodes = right.root().dfs().collect::<Vec<_>>();
            assert_eq!(native_nodes.len(), tape_nodes.len(), "{lang:?} {source:?}");
            for (a, b) in native_nodes.iter().zip(tape_nodes.iter()) {
                assert_eq!(
                    (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
                    (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
                    "{lang:?} {source:?}"
                );
            }
        }
    }
}

/// A tape node's position is tree-sitter's point: only `\n` ends a row. A lone `\r`, which ends a
/// Ruff, PHP and ESLint line, and U+2028 and U+2029, which end an ESLint line, stay inside it; the
/// evaluator's line table, which places violations, still ends lines at them.
#[test]
fn positions_count_only_line_feeds_in_every_dialect() {
    for (lang, source) in [
        (
            GrammarDialect::Python,
            "x = 1\ry = '\u{2028}'\r\nz = 2  # c\u{2029}\n\rw = 3\r",
        ),
        (
            GrammarDialect::TypeScript,
            "let x = 1;\rlet y = '\u{2028}';\r\nlet z = 2; // c\u{2029}\n\rw;\r",
        ),
        (
            GrammarDialect::Tsx,
            "let x = <a>\r</a>;\rlet y = '\u{2028}';\r\n// c\u{2029}\n\rw;\r",
        ),
        (
            GrammarDialect::JavaScript,
            "let x = 1;\rlet y = '\u{2028}';\r\nlet z = 2; // c\u{2029}\n\rw;\r",
        ),
        (
            GrammarDialect::Php,
            "<?php\r$x = 1;\r$y = '\u{2028}';\r\n$z = 2; // c\u{2029}\n\r$w;\r",
        ),
        (
            GrammarDialect::Blade,
            "<p>{{ $x }}</p>\r@if($y)\u{2028}\r\n{{ $z }}\u{2029}\n\r@endif\r",
        ),
    ] {
        let grammar = Grammar::new(lang).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        let tree_sitter_point = |byte: usize| {
            let before = &source.as_bytes()[..byte];
            let row = before.iter().filter(|&&b| b == b'\n').count();
            let start = before
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |at| at + 1);
            (row, byte - start)
        };
        let restored = AstGrep::doc(tape.clone());
        for node in restored.root().dfs() {
            for (position, byte) in [
                (node.start_pos(), node.range().start),
                (node.end_pos(), node.range().end),
            ] {
                assert_eq!(
                    position.byte_point(),
                    tree_sitter_point(byte),
                    "{lang:?} {} at {byte}",
                    node.kind()
                );
            }
        }
        // A Blade tape is Forte's tree and its embedded PHP view's nodes; `corpus_parity`
        // compares those against the Forte tree and the view.
        if lang != GrammarDialect::Blade {
            let native = AstGrep::doc(StrDoc::new(source, lang));
            let native_nodes = native.root().dfs().collect::<Vec<_>>();
            let tape_nodes = restored.root().dfs().collect::<Vec<_>>();
            assert_eq!(native_nodes.len(), tape_nodes.len(), "{lang:?}");
            for (a, b) in native_nodes.iter().zip(&tape_nodes) {
                assert_eq!(
                    (a.range(), a.start_pos(), a.end_pos()),
                    (b.range(), b.start_pos(), b.end_pos()),
                    "{lang:?}"
                );
            }
        }
        // Violations keep the linter's lines: each lone `\r` before `y` ends one.
        let y = source.find('y').unwrap();
        let lone_returns = source[..y].matches('\r').count();
        assert_eq!(tape.line_table().point(y).0, lone_returns, "{lang:?}");
    }
}

#[test]
fn direct_field_ids_and_navigation_match_native() {
    use ast_grep_core::source::SgNode;
    for (lang, source) in [
        (GrammarDialect::Python, "x = foo(1)\ny = foo(2)\n"),
        (
            GrammarDialect::TypeScript,
            "const x = foo(1);\nconst y = foo(2);\n",
        ),
    ] {
        let (native, tape, _, grammar) = fixture(source, lang);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape);
        let native_nodes = left.root().dfs().collect::<Vec<_>>();
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(native_nodes.len(), tape_nodes.len());
        for (a, b) in native_nodes.iter().zip(tape_nodes.iter()) {
            assert_eq!(
                a.next_all().map(|node| node.range()).collect::<Vec<_>>(),
                b.next_all().map(|node| node.range()).collect::<Vec<_>>(),
                "{lang:?} next_all {:?}",
                a.range()
            );
            assert_eq!(
                a.prev_all().map(|node| node.range()).collect::<Vec<_>>(),
                b.prev_all().map(|node| node.range()).collect::<Vec<_>>(),
                "{lang:?} prev_all {:?}",
                a.range()
            );
            assert_eq!(
                a.next().map(|node| node.range()),
                b.next().map(|node| node.range())
            );
            assert_eq!(
                a.prev().map(|node| node.range()),
                b.prev().map(|node| node.range())
            );
        }
        for (a, b) in left
            .root()
            .get_inner_node()
            .dfs()
            .zip(right.root().get_inner_node().dfs())
        {
            for id in [
                None,
                Some(0),
                Some(1),
                Some(grammar.kinds.field_count() as u16),
                Some(u16::MAX),
            ] {
                assert_eq!(
                    a.field_children(id)
                        .map(|node| node.byte_range())
                        .collect::<Vec<_>>(),
                    b.field_children(id)
                        .map(|node| node.range())
                        .collect::<Vec<_>>(),
                    "{lang:?} field {id:?} {:?}",
                    a.byte_range()
                );
                if let Some(id) = id {
                    assert_eq!(
                        a.child_by_field_id(id).map(|node| node.byte_range()),
                        b.child_by_field_id(id).map(|node| node.range()),
                        "{lang:?} field {id:?} {:?}",
                        a.byte_range()
                    );
                }
            }
        }
    }
}

#[test]
fn single_capture_ignores_comments_before_and_after_argument() {
    for (lang, source) in [
        (
            GrammarDialect::Python,
            "foo(\n  # before\n  1 # after\n)\nfoo(2)\n",
        ),
        (
            GrammarDialect::TypeScript,
            "foo(\n  // before\n  1 // after\n);\nfoo(2);\n",
        ),
    ] {
        let (native, tape, _, _) = fixture(source, lang);
        let expected = matches(native, "foo($A)", lang);
        assert_eq!(expected.len(), 2, "{lang:?}");
        assert_eq!(matches(tape, "foo($A)", lang), expected, "{lang:?}");
    }
}

#[test]
fn representative_alias_fields_and_flags_match_native() {
    for (lang, source) in [
        (
            GrammarDialect::Python,
            "with open('x') as stream:\n    value = f'{stream}'\n",
        ),
        (
            GrammarDialect::TypeScript,
            "type Alias = { member: string }; const obj = { member }; obj.member;\n",
        ),
    ] {
        let (native, tape, _, grammar) = fixture(source, lang);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape);
        let native_nodes = left.root().dfs().collect::<Vec<_>>();
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(native_nodes.len(), tape_nodes.len(), "{lang:?}");
        for (a, b) in native_nodes.iter().zip(tape_nodes.iter()) {
            assert_eq!(
                (a.kind_id(), a.is_named(), a.is_extra(), a.range(), a.text()),
                (b.kind_id(), b.is_named(), b.is_extra(), b.range(), b.text()),
                "{lang:?}"
            );
            for id in 1..=grammar.kinds.field_count() as u16 {
                let name = grammar.kinds.field_name(id).unwrap();
                assert_eq!(
                    a.field(name).map(|node| node.range()),
                    b.field(name).map(|node| node.range()),
                    "{lang:?} {name}"
                );
                assert_eq!(
                    a.field_children(name)
                        .map(|node| node.range())
                        .collect::<Vec<_>>(),
                    b.field_children(name)
                        .map(|node| node.range())
                        .collect::<Vec<_>>(),
                    "{lang:?} {name}"
                );
            }
        }
    }
}

#[test]
fn native_and_tape_match_smart_extras_and_ordered_variadic_captures() {
    for (lang, source, pattern) in [
        (
            GrammarDialect::Python,
            "value = foo(\n    # note\n    1, 2\n)\nfoo(3, 4)\n",
            "foo($$$ARGS)",
        ),
        (
            GrammarDialect::TypeScript,
            "const value = foo(\n  // note\n  1, 2\n);\nfoo(3, 4);\n",
            "foo($$$ARGS)",
        ),
    ] {
        let (native, tape, _, _) = fixture(source, lang);
        let expected = matches(native, pattern, lang);
        assert_eq!(expected.len(), 2);
        assert_eq!(matches(tape, pattern, lang), expected);
    }
}

#[test]
fn repeated_capture_and_line_continuation_follow_native_matching() {
    for (lang, source, pattern) in [
        (GrammarDialect::Python, "foo + foo\nfoo + bar\n", "$X + $X"),
        (
            GrammarDialect::TypeScript,
            "foo + foo;\nfoo + bar;\n",
            "$X + $X",
        ),
        (GrammarDialect::Python, "value = foo(\\\n  1)\n", "foo($A)"),
    ] {
        let (native, tape, _, _) = fixture(source, lang);
        let expected = matches(native, pattern, lang);
        assert_eq!(expected.len(), 1, "{source}");
        assert_eq!(matches(tape, pattern, lang), expected, "{source}");
    }
}

#[test]
fn punctuation_fields_ancestors_and_unicode_positions_match_native() {
    for (lang, source) in [
        (GrammarDialect::Python, "é = foo(1, 2)\n"),
        (GrammarDialect::TypeScript, "const é = foo(1, 2);\n"),
    ] {
        let (native, tape, _, _) = fixture(source, lang);
        let native_root = AstGrep::doc(native);
        let tape_root = AstGrep::doc(tape.clone());
        let native_nodes = native_root.root().dfs().collect::<Vec<_>>();
        let tape_nodes = tape_root.root().dfs().collect::<Vec<_>>();
        assert_eq!(native_nodes.len(), tape_nodes.len());
        for (left, right) in native_nodes.iter().zip(tape_nodes.iter()) {
            assert_eq!(
                (
                    left.kind(),
                    left.kind_id(),
                    left.is_named(),
                    left.is_extra(),
                    left.is_error(),
                    left.is_missing(),
                    left.range(),
                    left.start_pos(),
                    left.end_pos()
                ),
                (
                    right.kind(),
                    right.kind_id(),
                    right.is_named(),
                    right.is_extra(),
                    right.is_error(),
                    right.is_missing(),
                    right.range(),
                    right.start_pos(),
                    right.end_pos()
                )
            );
            assert_eq!(
                left.children()
                    .map(|child| child.kind().into_owned())
                    .collect::<Vec<_>>(),
                right
                    .children()
                    .map(|child| child.kind().into_owned())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                left.ancestors()
                    .map(|parent| parent.kind().into_owned())
                    .collect::<Vec<_>>(),
                right
                    .ancestors()
                    .map(|parent| parent.kind().into_owned())
                    .collect::<Vec<_>>()
            );
            for field in ["function", "arguments", "name", "body", "left", "right"] {
                assert_eq!(
                    left.field(field).map(|child| child.range()),
                    right.field(field).map(|child| child.range())
                );
                assert_eq!(
                    left.field_children(field)
                        .map(|child| child.range())
                        .collect::<Vec<_>>(),
                    right
                        .field_children(field)
                        .map(|child| child.range())
                        .collect::<Vec<_>>()
                );
            }
        }
        let start = source.find('é').unwrap();
        assert_eq!(tape.line_table().point(start), (0, start));
        assert_eq!(tape.line_table().point(source.len()), (1, 0));
    }
}

#[test]
fn malformed_and_mismatched_images_are_not_negative_matches() {
    let source = "a = foo(1)\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    let source = Arc::new(source.to_owned());
    let reject = |candidate: &TapeImage, grammar: Arc<Grammar>| {
        assert!(TapeDoc::restore(Arc::clone(&source), grammar, candidate, &|| false).is_err());
    };
    let mut broken = image.clone();
    broken.rec.pop();
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.spans.pop();
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.rec[2..4].copy_from_slice(&0u16.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.rec[0..2].copy_from_slice(&u16::MAX.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.spans[0] = 0xff;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.size_overflow.push((0, u32::MAX));
    reject(&broken, Arc::clone(&grammar));
    reject(&image, Grammar::new(GrammarDialect::TypeScript).unwrap());
    let mut broken = image.clone();
    broken.source_len += 1;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.checkpoints[0] ^= 1;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.spans.extend_from_slice(&[0, 0]);
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.rec[4..6].copy_from_slice(&SENTINEL.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.extra.extend_from_slice(&[0, 0]);
    reject(&broken, Arc::clone(&grammar));
    let altered = Arc::new("a = foo(2)\n".to_owned());
    assert!(TapeDoc::restore(altered, Arc::clone(&grammar), &image, &|| false).is_err());
}

#[test]
fn restore_rejects_child_subtree_escaping_parent() {
    let source = "x+y\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    let mut broken = image.clone();
    // The expression_statement has five preorder records. Shortening its subtree
    // to two leaves descendants outside its declared exclusive boundary.
    broken.rec[10..12].copy_from_slice(&2u16.to_le_bytes());
    assert!(TapeDoc::restore(Arc::new(source.to_owned()), grammar, &broken, &|| false).is_err());
}

#[test]
fn wrong_parsed_grammar_never_produces_a_tape() {
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let valid = "x = 1\n";
    let parsed = StrDoc::new(valid, GrammarDialect::TypeScript);
    assert!(TapeImage::encode(parsed.tree.root_node(), &grammar, valid, &|| false).is_err());
}

// Error-tolerant evaluation matches around syntax errors, so damaged trees are taped with their
// ERROR nodes and zero-width MISSING tokens, and every node flag matches the native tree.
#[test]
fn damaged_trees_keep_error_and_missing_nodes_like_native() {
    for (lang, source, errors, missing) in [
        (GrammarDialect::Python, "x = 1\ndef f(:\n", false, true),
        (GrammarDialect::Python, "x = 1\n) = 2\nx = 3\n", true, false),
        (
            GrammarDialect::TypeScript,
            "debugger;\nfunction f( {\n  debugger;\n",
            true,
            false,
        ),
        (
            GrammarDialect::TypeScript,
            "function f() {\n  debugger;\n  x = 1;\n",
            false,
            true,
        ),
    ] {
        let (native, tape, image, grammar) = fixture(source, lang);
        assert!(native.tree.root_node().has_error());
        assert_eq!(!image.error.is_empty(), errors, "{source}");
        assert_eq!(!image.missing.is_empty(), missing, "{source}");
        let native_root = AstGrep::doc(native);
        let tape_root = AstGrep::doc(tape.clone());
        let native_nodes = native_root.root().dfs().collect::<Vec<_>>();
        let tape_nodes = tape_root.root().dfs().collect::<Vec<_>>();
        assert_eq!(native_nodes.len(), tape_nodes.len());
        for (left, right) in native_nodes.iter().zip(tape_nodes.iter()) {
            assert_eq!(
                (
                    left.kind(),
                    left.kind_id(),
                    left.is_named(),
                    left.is_extra(),
                    left.is_error(),
                    left.is_missing(),
                    left.range(),
                    left.children().len(),
                ),
                (
                    right.kind(),
                    right.kind_id(),
                    right.is_named(),
                    right.is_extra(),
                    right.is_error(),
                    right.is_missing(),
                    right.range(),
                    right.children().len(),
                )
            );
        }
        let native_errors = super::super::soundness::SyntaxErrors::collect(
            native_root.root().get_doc().tree.root_node(),
        );
        let tape_errors =
            super::super::soundness::SyntaxErrors::from_ranges(tape.syntax_error_ranges());
        let lines = LineTable::new(source, Terminators::of(lang));
        assert_eq!(
            native_errors.partial_message(&lines),
            tape_errors.partial_message(&lines)
        );
        for start in 0..=source.len() {
            for end in start..=source.len() {
                assert_eq!(
                    native_errors.overlaps(&(start..end)),
                    tape_errors.overlaps(&(start..end)),
                    "{source:?} {start}..{end}"
                );
            }
        }
        let pattern = if lang == GrammarDialect::Python {
            "x = $A"
        } else {
            "debugger"
        };
        assert_eq!(
            matches(StrDoc::new(source, lang), pattern, lang),
            matches(tape.clone(), pattern, lang)
        );
        let source = Arc::new(source.to_owned());
        let reject = |candidate: &TapeImage| {
            assert!(
                TapeDoc::restore(
                    Arc::clone(&source),
                    Arc::clone(&grammar),
                    candidate,
                    &|| false
                )
                .is_err()
            );
        };
        if let Some(&index) = image.error.first() {
            let mut broken = image.clone();
            broken.error.clear();
            reject(&broken);
            let mut broken = image.clone();
            broken.error.push(index);
            reject(&broken);
        }
        if let Some(&index) = image.missing.first() {
            let mut broken = image.clone();
            broken.missing.clear();
            broken.missing.push(0);
            reject(&broken);
            let mut broken = image.clone();
            broken.error.push(index);
            broken.error.sort_unstable();
            reject(&broken);
        }
    }
}

#[test]
fn cancellation_interrupts_mid_encode_without_partial_image() {
    use std::cell::Cell;
    let source = "x = foo(1)\n".repeat(100);
    let (native, _, _, grammar) = fixture(&source, GrammarDialect::Python);
    let calls = Cell::new(0);
    let interrupt = || {
        calls.set(calls.get() + 1);
        calls.get() == 5
    };
    assert!(TapeImage::encode(native.tree.root_node(), &grammar, &source, &interrupt).is_err());
    assert_eq!(
        calls.get(),
        5,
        "encode must poll before the final source digest"
    );
    let image = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).unwrap();
    assert_eq!(
        TapeDoc::restore(Arc::new(source), grammar, &image, &|| false)
            .unwrap()
            .data
            .len(),
        image.node_count()
    );
}

#[test]
fn cancellation_interrupts_mid_restore_after_hashing() {
    use std::cell::Cell;
    let source = "x = foo(1)\n".repeat(100);
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    let calls = Cell::new(0);
    let interrupt = || {
        calls.set(calls.get() + 1);
        calls.get() == 5
    };
    assert!(
        TapeDoc::restore(
            Arc::new(source.clone()),
            Arc::clone(&grammar),
            &image,
            &interrupt
        )
        .is_err()
    );
    assert_eq!(calls.get(), 5, "restore must poll after the source digest");
    assert_eq!(
        TapeDoc::restore(Arc::new(source), grammar, &image, &|| false)
            .unwrap()
            .data
            .len(),
        image.node_count()
    );
}

#[test]
fn checkpoint_encoding_rejects_unrepresentable_offsets() {
    let mut bytes = Vec::new();
    assert!(checkpoint(CHECKPOINT_LIMIT + 1, &mut bytes).is_err());
    assert!(bytes.is_empty());
}

#[test]
fn checkpoint_stride_boundaries_preserve_native_nodes_and_matches() {
    for (expected_count, source) in [
        (15, "x\n".repeat(7)),
        (16, format!("{}#é\n", "x\n".repeat(7))),
        (17, "x\n".repeat(8)),
    ] {
        let (native, tape, image, _) = fixture(&source, GrammarDialect::Python);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape.clone());
        let native_nodes = left.root().dfs().collect::<Vec<_>>();
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(
            native_nodes.len(),
            expected_count,
            "{expected_count} native fixture"
        );
        assert_eq!(image.node_count(), expected_count);
        assert_eq!(image.checkpoints.len(), expected_count.div_ceil(16) * 6);
        for (index, (a, b)) in native_nodes.iter().zip(tape_nodes.iter()).enumerate() {
            assert_eq!(
                (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
                (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
                "{expected_count} node {index}"
            );
            assert_eq!(
                a.ancestors().map(|node| node.range()).collect::<Vec<_>>(),
                b.ancestors().map(|node| node.range()).collect::<Vec<_>>(),
                "{expected_count} ancestors {index}"
            );
            assert_eq!(
                a.prev().map(|node| node.range()),
                b.prev().map(|node| node.range()),
                "{expected_count} previous sibling {index}"
            );
        }
        let expected = matches(
            StrDoc::new(source.as_str(), GrammarDialect::Python),
            "x",
            GrammarDialect::Python,
        );
        assert_eq!(expected.len(), 7 + usize::from(expected_count == 17));
        assert_eq!(matches(tape, "x", GrammarDialect::Python), expected);
    }
}

#[test]
fn span_varint_width_boundaries_preserve_utf8_ranges_and_captures() {
    for (length, expected_width) in [(127, 1), (128, 2), (16_383, 2), (16_384, 3)] {
        let source = format!("#é{}\nfoo(1)\n", "a".repeat(length - 3));
        let (native, tape, image, _) = fixture(&source, GrammarDialect::Python);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape.clone());
        let native_nodes = left.root().dfs().collect::<Vec<_>>();
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(
            native_nodes.len(),
            tape_nodes.len(),
            "comment length {length}"
        );
        let comment = native_nodes.iter().find(|node| node.is_extra()).unwrap();
        assert_eq!(comment.range(), 0..length, "comment length {length}");
        assert_eq!(source.find("foo"), Some(length + 1));
        let mut at = 0;
        let mut saw_comment = false;
        let mut previous = 0i64;
        for (index, (a, b)) in native_nodes.iter().zip(tape_nodes.iter()).enumerate() {
            assert_eq!(
                (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
                (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
                "comment length {length} node {index}"
            );
            let encoded_delta = read_varint(&image.spans, &mut at).unwrap();
            let delta = ((encoded_delta >> 1) as i32) ^ -((encoded_delta & 1) as i32);
            previous += i64::from(delta);
            let width_start = at;
            let decoded_length = read_varint(&image.spans, &mut at).unwrap();
            assert_eq!(
                previous as usize,
                a.range().start,
                "comment length {length} node {index}"
            );
            assert_eq!(
                decoded_length as usize,
                a.range().len(),
                "comment length {length} node {index}"
            );
            if a.is_extra() {
                assert_eq!(at - width_start, expected_width, "comment length {length}");
                saw_comment = true;
            }
        }
        assert!(saw_comment, "comment length {length}");
        let expected = matches(
            StrDoc::new(source.as_str(), GrammarDialect::Python),
            "foo($A)",
            GrammarDialect::Python,
        );
        assert_eq!(expected.len(), 1, "comment length {length}");
        assert_eq!(
            matches(tape, "foo($A)", GrammarDialect::Python),
            expected,
            "comment length {length}"
        );
    }
}

#[test]
fn large_file_roundtrips_parent_and_subtree_overflow() {
    let source = "x = foo(1)\n".repeat(12_000);
    let (native, tape, image, grammar) = fixture(&source, GrammarDialect::Python);
    assert!(image.node_count() > u16::MAX as usize);
    assert!(!image.par_overflow.is_empty());
    assert!(!image.size_overflow.is_empty());
    assert_eq!(
        matches(tape, "foo($A)", GrammarDialect::Python),
        matches(native, "foo($A)", GrammarDialect::Python)
    );
    let mut broken = image.clone();
    broken.par_overflow[0].1 = u32::MAX;
    assert!(TapeDoc::restore(Arc::new(source), grammar, &broken, &|| false).is_err());
}

#[test]
fn sentinel_edges_preserve_native_root_children_and_last_parent() {
    for (extra_count, expected_count) in [(1, 65_534), (2, 65_535), (3, 65_536)] {
        let source = format!("{}{}", "x\n".repeat(32_766), "#é\n".repeat(extra_count));
        let (native, tape, image, grammar) = fixture(&source, GrammarDialect::Python);
        let native_root = native.tree.root_node();
        assert_eq!(native_root.child_count() as usize, 32_766 + extra_count);
        assert_eq!(
            image.node_count(),
            expected_count,
            "node count at {expected_count}"
        );
        assert_eq!(
            u16::from_le_bytes([image.rec[4], image.rec[5]]),
            if expected_count >= 65_535 {
                SENTINEL
            } else {
                expected_count as u16
            },
            "root size at {expected_count}"
        );
        let expected_size_overflow = [(0, expected_count as u32)];
        assert_eq!(
            image.size_overflow.as_slice(),
            if expected_count >= 65_535 {
                &expected_size_overflow[..]
            } else {
                &[][..]
            },
            "root overflow at {expected_count}"
        );
        let last = expected_count - 1;
        assert_eq!(
            u16::from_le_bytes([image.rec[last * WIDTH + 2], image.rec[last * WIDTH + 3]]),
            if last >= 65_535 {
                SENTINEL
            } else {
                last as u16
            },
            "last parent distance at {expected_count}"
        );
        let expected_parent_overflow = [(last as u32, last as u32)];
        assert_eq!(
            image.par_overflow.as_slice(),
            if last >= 65_535 {
                &expected_parent_overflow[..]
            } else {
                &[][..]
            },
            "parent overflow at {expected_count}"
        );
        let native_doc = AstGrep::doc(native);
        let tape_doc = AstGrep::doc(tape.clone());
        let a = native_doc.root().children().last().unwrap();
        let b = tape_doc.root().children().last().unwrap();
        assert_eq!(a.text(), "#é", "last native comment at {expected_count}");
        assert_eq!(
            (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
            (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
            "last child at {expected_count}"
        );
        assert_eq!(
            a.prev().map(|node| (node.kind_id(), node.range())),
            b.prev().map(|node| (node.kind_id(), node.range())),
            "last sibling at {expected_count}"
        );
        assert_eq!(
            a.ancestors().map(|node| node.range()).collect::<Vec<_>>(),
            b.ancestors().map(|node| node.range()).collect::<Vec<_>>(),
            "last ancestors at {expected_count}"
        );
        if expected_count == 65_536 {
            let mut missing_parent = image.clone();
            missing_parent.par_overflow.clear();
            assert!(
                TapeDoc::restore(
                    Arc::new(source.clone()),
                    Arc::clone(&grammar),
                    &missing_parent,
                    &|| false
                )
                .is_err(),
                "sentinel parent without entry"
            );
            let mut missing_size = image.clone();
            missing_size.size_overflow.clear();
            assert!(
                TapeDoc::restore(Arc::new(source), grammar, &missing_size, &|| false).is_err(),
                "sentinel subtree without entry"
            );
        }
    }
}

#[test]
fn restore_rejects_noncanonical_side_tables_and_checkpoint_boundaries() {
    let source = format!("{}#é\n", "x\n".repeat(8));
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    assert_eq!(image.node_count(), 18);
    assert_eq!(image.extra.len(), 1);
    let reject = |label: &str, candidate: &TapeImage| {
        assert!(
            TapeDoc::restore(
                Arc::new(source.clone()),
                Arc::clone(&grammar),
                candidate,
                &|| false
            )
            .is_err(),
            "{label}"
        );
    };
    let mut broken = image.clone();
    broken.checkpoints.pop();
    reject("truncated checkpoint", &broken);
    let mut broken = image.clone();
    broken.checkpoints[6] ^= 1;
    reject("second checkpoint byte offset", &broken);
    let mut broken = image.clone();
    broken.checkpoints[9] ^= 1;
    reject("second checkpoint source start", &broken);
    let mut broken = image.clone();
    broken.par_overflow = vec![(0, 65_535), (0, 65_536)];
    reject("duplicate parent overflow", &broken);
    let mut broken = image.clone();
    broken.size_overflow = vec![(1, 65_535), (0, 65_536)];
    reject("out-of-order subtree overflow", &broken);
    let mut broken = image.clone();
    broken
        .par_overflow
        .push((image.node_count() as u32, 65_535));
    reject("out-of-range parent overflow", &broken);
    let mut broken = image.clone();
    broken.size_overflow.push((0, 65_535));
    reject("unused subtree overflow", &broken);
    let mut broken = image.clone();
    broken.par_overflow.push((0, 65_535));
    reject("unused parent overflow", &broken);
    let mut broken = image.clone();
    broken.extra.push(image.extra[0]);
    reject("duplicate extra node", &broken);
    let mut broken = image.clone();
    broken.extra = vec![image.extra[0], 0];
    reject("out-of-order extra nodes", &broken);
    let mut broken = image.clone();
    broken.extra = vec![image.node_count() as u32];
    reject("out-of-range extra node", &broken);
}

#[test]
fn restore_rejects_overwide_spans_and_non_utf8_or_parent_ranges() {
    let source = "é = foo(1)\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    let reject = |label: &str, candidate: &TapeImage| {
        assert!(
            TapeDoc::restore(
                Arc::new(source.to_owned()),
                Arc::clone(&grammar),
                candidate,
                &|| false
            )
            .is_err(),
            "{label}"
        );
    };
    let mut broken = image.clone();
    broken.spans.splice(0..1, [0x80, 0x80, 0x80, 0x80, 0x10]);
    reject("fifth varint byte exceeds u32", &broken);
    let mut broken = image.clone();
    broken.spans.splice(0..1, [0x80, 0x80, 0x80, 0x80, 0x80]);
    reject("unterminated fifth varint byte", &broken);
    let mut at = 0;
    let mut previous = 0i64;
    let leaf_length_offset = (0..image.node_count())
        .find_map(|index| {
            let delta = read_varint(&image.spans, &mut at).unwrap();
            let delta = ((delta >> 1) as i32) ^ -((delta & 1) as i32);
            let start = previous + i64::from(delta);
            previous = start;
            let offset = at;
            let length = read_varint(&image.spans, &mut at).unwrap();
            (index != 0
                && index % CHECKPOINT_STRIDE != 0
                && start == 0
                && length == 2
                && image.rec[index * WIDTH + 4..index * WIDTH + 6] == [1, 0])
            .then_some(offset)
        })
        .expect("non-checkpoint UTF-8 leaf inside the unchanged root and parent");
    let mut broken = image.clone();
    assert_eq!(broken.spans[leaf_length_offset], 2, "one-byte leaf length");
    broken.spans[leaf_length_offset] = 1;
    assert!(
        TapeDoc::restore(
            Arc::new(source.to_owned()),
            Arc::clone(&grammar),
            &image,
            &|| false
        )
        .is_ok()
    );
    reject(
        "leaf ends inside UTF-8 codepoint with valid enclosing spans",
        &broken,
    );

    let source = "x+y\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    let mut broken = image.clone();
    assert_eq!(broken.spans[0], 0);
    assert_eq!(broken.spans[1], source.len() as u8);
    broken.spans[1] = 2;
    assert!(
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &broken, &|| false).is_err(),
        "child span escapes shortened parent but remains within source"
    );
}

/// Every node, looked up from every node of its tree, is the node a lookup from the root finds:
/// `find_near` starts below the root only where siblings never overlap, so it agrees with `find`
/// on native and taped trees alike, through wrappers of the same range and syntax errors.
#[test]
fn a_lookup_from_a_nearby_node_finds_what_a_lookup_from_the_root_finds() {
    use crate::code_rule_dsl::value::NodeRef;

    fn agree<D: Doc>(doc: D, label: &str) {
        let root = AstGrep::doc(doc);
        let nodes = root.root().dfs().collect::<Vec<_>>();
        for target in &nodes {
            let reference = NodeRef::of(target);
            let expected = reference.find(root.root()).map(|node| node.node_id());
            let at = (target.kind(), target.range());
            if !target.range().is_empty() {
                assert!(expected.is_some(), "{label}: {at:?}");
            }
            for near in &nodes {
                let found = reference
                    .find_near(root.root(), near)
                    .map(|node| node.node_id());
                assert_eq!(found, expected, "{label}: {at:?} from {:?}", near.range());
            }
        }
    }

    let sources = [
        (
            GrammarDialect::Php,
            "<?php\nclass A {\n  function f($x) { $y = [1, $x]; return fn() => $y[0]; }\n  function g() { $z = $this->f(2); echo \"a{$z}b\"; }\n}\nif (! function_exists('e')) { function e($v) { return h($v); } }\n",
        ),
        (
            GrammarDialect::Php,
            "<?php\nfunction f( { $a = ; }\n$b = 1;\n",
        ),
        (
            GrammarDialect::Python,
            "def f(x):\n    y = (x, [1])\n    return g(y)\n\nclass C:\n    z = f(1)\n",
        ),
        (
            GrammarDialect::TypeScript,
            "const a = { b: [1, 2] };\nfunction f(x: number) { return a.b[x] ?? g(x); }\n",
        ),
    ];
    for (lang, source) in sources {
        let (native, tape, _, _) = fixture(source, lang);
        agree(native, &format!("{lang:?} native"));
        agree(tape, &format!("{lang:?} tape"));
    }
}

/// A PHP template's tape is its view's tree (`php_view`) over the template: no syntax errors, every
/// span reads the template, and inline text holds no node.
#[test]
fn a_php_template_tapes_its_php_view() {
    let cases: [(&str, &[&str], &[&str]); 9] = [
        (
            "<?php\nprint(1);\n?>\n<script>\nx = eval('document.getElementById (option)');\n</script>\n",
            &["<script>\nx = eval('document.getElementById (option)');\n</script>"],
            &[],
        ),
        (
            "<?php\neval('$a = 1;');\n?>\n<p>hi</p>\n<?php eval('$b = 2;'); ?>\n",
            &["<p>hi</p>"],
            &["eval('$a = 1;')", "eval('$b = 2;')"],
        ),
        (
            "<html><?php foo(); ?></html>\n",
            &["<html>", "</html>"],
            &["foo()"],
        ),
        (
            "<?php if ($a): ?>\n<p>x</p>\n<?php endif; ?>\n",
            &["<p>x</p>"],
            &[],
        ),
        ("<p><?= $x ?></p>\n", &["<p>", "</p>"], &[]),
        (
            "<p><?php $s = '?>'; $t = <<<EOT\n?>\nEOT;\nf($t) // c ?></p>\r\n<?php if ($a) { ?>é<?php } ?>\n",
            &["</p>", "é"],
            &["f($t)"],
        ),
        (
            "<?php $s = \"{$o->{\"?>\"}}\"; $x = function() {} ?>html<?php eval(1);\n",
            &["html"],
            &["eval(1)"],
        ),
        (
            "<p><?php $__halt_compiler = static fn() => 0; $__halt_compiler(); eval('1'); ?></p>\n",
            &["<p>", "</p>"],
            &["$__halt_compiler()", "eval('1')"],
        ),
        (
            "<?php $a = 1; ?>html<?$x = eval('1');?>tail<?php $b = 2; ?>\n",
            &["html", "tail"],
            &["eval('1')"],
        ),
    ];
    for (source, inline, calls) in cases {
        let grammar = Grammar::new(GrammarDialect::Php).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        assert_eq!(
            tape.syntax_error_ranges(),
            Vec::<Range<usize>>::new(),
            "{source:?}"
        );
        let inline = inline
            .iter()
            .map(|text| {
                let start = source.find(text).unwrap();
                start..start + text.len()
            })
            .collect::<Vec<_>>();
        let root = AstGrep::doc(tape);
        // The view's tree, node for node, its texts read from the template.
        let view = AstGrep::doc(StrDoc::new(
            &super::super::php_view::view(source),
            GrammarDialect::Php,
        ));
        let native = view
            .root()
            .dfs()
            .map(|node| (node.kind_id(), node.range()))
            .collect::<Vec<_>>();
        let taped = root
            .root()
            .dfs()
            .map(|node| (node.kind_id(), node.range()))
            .collect::<Vec<_>>();
        assert_eq!(native, taped, "{source:?}");
        let mut found = Vec::new();
        for node in root.root().dfs() {
            let range = node.range();
            assert_eq!(node.text(), &source[range.clone()], "{source:?}");
            if node.children().next().is_none() {
                assert!(
                    inline
                        .iter()
                        .all(|text| range.end <= text.start || text.end <= range.start),
                    "{source:?}: {} at {range:?} is inline text",
                    node.kind()
                );
            }
            if node.kind() == "function_call_expression" {
                found.push(node.text().into_owned());
            }
        }
        assert_eq!(found, calls, "{source:?}");
    }
}

/// A lookup in a taped tree reads no child list one child at a time: an element of a 20,000-wide
/// list, looked up from its first element, is found by searching the list's subtree, not by
/// reading the elements before it (once per binding occurrence, that held one clause on jison's
/// generated parser tables for seconds while it spent almost none of its work budget).
#[test]
fn a_taped_lookup_reads_no_wide_child_list_child_by_child() {
    use crate::code_rule_dsl::value::{LINEAR_CHILDREN, NodeRef};

    let source = format!(
        "x = [{}]\n",
        (0..20_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let (_, tape, _, _) = fixture(&source, GrammarDialect::Python);
    let root = AstGrep::doc(tape);
    let numbers = root
        .root()
        .dfs()
        .filter(|node| node.kind() == "integer")
        .collect::<Vec<_>>();
    assert_eq!(numbers.len(), 20_000);
    let (first, last) = (&numbers[0], &numbers[19_999]);
    LINEAR_CHILDREN.set(0);
    let found = NodeRef::of(last)
        .find_near(root.root(), first)
        .expect("the element");
    assert_eq!(found.node_id(), last.node_id());
    assert_eq!(LINEAR_CHILDREN.get(), 0, "no child was read one by one");
    // A node of another kind at the same range is in no taped tree: the search says so, and does
    // not read the list child by child to find out.
    let missing = NodeRef {
        kind: last.kind_id().wrapping_add(1),
        ..NodeRef::of(last)
    };
    LINEAR_CHILDREN.set(0);
    assert!(missing.find_near(root.root(), first).is_none());
    assert_eq!(LINEAR_CHILDREN.get(), 0, "a miss read children one by one");
}
