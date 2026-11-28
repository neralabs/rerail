use super::*;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::StrDoc;

/// A parse-cost measurement's step: its name, and what it does to a file (its count, kept from being optimized away).
type CostStep<'a> = (&'static str, Box<dyn Fn(&str) -> usize + 'a>);

/// A step bound to the measured files.
type CostPass<'a> = (&'static str, Box<dyn Fn() -> usize + 'a>);

/// The engine's own parsers against tree-sitter over repositories and suites (`lean_parsers_survey`).
mod survey;

#[test]
fn a_tape_persists_abstract_tree_sitter_supertypes() {
    let source = "value = left + right\n";
    let (_, tape, image, grammar) = fixture(source, GrammarDialect::Python);
    let bytes = image.to_bytes().unwrap();

    let memberships = tape.transducer_supertypes().collect::<Vec<_>>();
    assert!(!memberships.is_empty());
    assert!(memberships.iter().any(|&(node, kind)| {
        tape.transducer_has_supertype(node, kind) && grammar.kind_name(kind) == Some("expression")
    }));

    let persisted = TapeImage::from_bytes(&bytes, &grammar).unwrap();
    let restored = TapeDoc::restore(
        Arc::new(source.to_owned().into()),
        Arc::clone(&grammar),
        &persisted,
        &|| false,
    )
    .unwrap();
    assert_eq!(
        restored.transducer_supertypes().collect::<Vec<_>>(),
        memberships
    );

    // Memberships live in the codes, which are the language's table's: a node's number names them, and a stored
    // tape holds no table of its own to corrupt.
    assert!(image.lang().is_some(), "the language's table");
}

#[test]
fn packed_backend_accessors_read_text_named_leaves_and_structure() {
    let source = "f(a); g(b); h(a, b);\n";
    let (_, tape, _, grammar) = fixture(source, GrammarDialect::TypeScript);
    let calls: Vec<u32> = (0..tape.transducer_len() as u32)
        .filter(|&node| grammar.kind_name(tape.transducer_kind(node)) == Some("call_expression"))
        .collect();
    assert_eq!(calls.len(), 3);
    for node in 0..tape.transducer_len() as u32 {
        assert_eq!(
            tape.transducer_text(node),
            tape.transducer_text_str(node).as_bytes()
        );
        let leaf = tape.transducer_children(node).next().is_none();
        let name = grammar.kind_name(tape.transducer_kind(node));
        if name == Some("identifier") {
            assert!(tape.transducer_is_named_leaf(node));
        }
        if !leaf {
            assert!(!tape.transducer_is_named_leaf(node));
        }
    }
    let keys: Vec<_> = calls
        .iter()
        .map(|&call| tape.transducer_structural_key(call))
        .collect();
    assert_eq!(keys[0], keys[1], "same shape, different identifiers");
    assert_ne!(keys[0], keys[2], "an extra argument changes the structure");
}

#[test]
fn scala_contextual_aliases_preserve_native_node_identity() {
    let source = "object A { def f(x: Int) = g(x) }";
    let (native, _, image, grammar) = fixture(source, GrammarDialect::Scala);
    assert!(image.aliased_nodes() > 0);
    let persisted = TapeImage::from_bytes(&image.to_bytes().unwrap(), &grammar).unwrap();
    let tape =
        TapeDoc::restore(Arc::new(source.to_owned().into()), grammar, &persisted, &|| false).unwrap();

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
    // The nodes a tape holds: the tokens the grammar implies (`def`, `:`, `}` here) left out, as
    // Scala's token automata (0305) decide (`stored_children_for_test`, as `stored` reads them).
    let mut kept = std::collections::HashSet::new();
    let mut stack = vec![native.tree.root_node()];
    while let Some(node) = stack.pop() {
        kept.insert(node.id());
        stack.extend(
            stored_children_for_test(GrammarDialect::Scala, node)
                .into_iter()
                .map(|(child, _)| child),
        );
    }
    expected.retain(|node| kept.contains(&node.id()));
    let actual = tape.root_node().dfs().collect::<Vec<_>>();
    assert!(
        actual.iter().any(|n| n.kind() == "arguments"),
        "the contextual alias is kept"
    );
    assert_eq!(expected.len(), actual.len());
    for (native, restored) in expected.into_iter().zip(actual) {
        assert_eq!(restored.kind_id(), native.kind_id());
        assert_eq!(restored.kind(), native.kind());
        assert_eq!(restored.is_named(), native.is_named());
        assert_eq!(restored.range(), native.byte_range());
    }
}

/// The nodes of tree-sitter's tree a tape holds, in preorder: the tokens the grammar implies left
/// out (`stored_children_for_test`).
fn stored<'r>(
    native: &'r AstGrep<StrDoc<GrammarDialect>>,
) -> Vec<ast_grep_core::Node<'r, StrDoc<GrammarDialect>>> {
    let kept = kept(native);
    native
        .root()
        .dfs()
        .filter(|node| kept.contains(&node.get_inner_node().id()))
        .collect()
}

/// The ids of the tree-sitter nodes a tape holds (`stored`).
fn kept(native: &AstGrep<StrDoc<GrammarDialect>>) -> std::collections::HashSet<usize> {
    let lang = *native.root().get_doc().get_lang();
    let mut kept = std::collections::HashSet::new();
    let mut stack = vec![native.root().get_doc().tree.root_node()];
    while let Some(node) = stack.pop() {
        kept.insert(node.id());
        stack.extend(
            stored_children_for_test(lang, node)
                .into_iter()
                .map(|(child, _)| child),
        );
    }
    kept
}

/// `image`'s nodes as format 16's writer takes them (`f16::write`), each passed through `edit` (its index and its
/// cells), written again: a tree no parser writes, for the admission tests. `Err` where the writer refuses the cells
/// itself.
fn rewritten(image: &TapeImage, source: &str, edit: impl Fn(usize, &mut rerail_tape_format::f16::Node)) -> Result<TapeImage, String> {
    use rerail_tape_format::tape::Tape;
    let lang = image.lang().ok_or("a tape without its language's table")?;
    let tag = crate::code_rule_dsl::language::tape_tag(image.language).ok_or("a language without a tag")?;
    let nodes: Vec<rerail_tape_format::f16::Node> = rerail_tape_format::with_tape!(image.tape(), |t| {
        let mut s = 0;
        (0..t.len())
            .map(|i| {
                s = t.start_next(i, s);
                rerail_tape_format::f16::Node { number: t.code(i), start: s, end: t.end_with_start(i, s), subtree_end: t.subtree_end(i) }
            })
            .collect()
    });
    let terminators = crate::code_rule_dsl::lines::terminators(image.language);
    let tape = rerail_tape_format::f16::write(lang, nodes.len(), source.as_bytes(), terminators, tag, false, |i| {
        let mut x = nodes[i];
        edit(i, &mut x);
        x
    })?;
    TapeImage::written_16(rerail_tape_format::f16::Finished { tape }, image.language, source, None, &|| false)
}

/// `image`'s stored bytes edited, then read back: `Err` where the format's reading refuses them.
fn bytes_edited(image: &TapeImage, grammar: &Arc<Grammar>, edit: impl FnOnce(&mut Vec<u8>)) -> Result<TapeImage, String> {
    let mut bytes = image.to_bytes().map_err(|e| format!("{e:?}"))?;
    edit(&mut bytes);
    TapeImage::from_bytes(&bytes, grammar).map_err(|e| format!("{e:?}"))
}

/// Whether `candidate` (a tape, or the reason none was made) never restores a document of `source`.
fn refused(candidate: Result<TapeImage, String>, grammar: &Arc<Grammar>, source: &str) -> bool {
    !candidate.is_ok_and(|c| TapeDoc::restore(Arc::new(source.to_owned().into()), Arc::clone(grammar), &c, &|| false).is_ok())
}

/// The last node of `image` in preorder whose second child ends before it does, and its first child's subtree end:
/// the node ended there leaves its other children to its parent, the next of them ending inside its span (siblings
/// are in the order of their ends).
fn two_children(image: &TapeImage) -> (usize, usize) {
    use rerail_tape_format::tape::Tape;
    rerail_tape_format::with_tape!(image.tape(), |t| {
        (0..t.len())
            .rev()
            .find_map(|i| {
                let second = t.subtree_end(i + 1);
                (i + 1 < t.subtree_end(i) && second < t.subtree_end(i) && t.end(second) < t.end(i)).then_some((i, second))
            })
            .expect("a node of two children, the second ending before it")
    })
}

/// A leaf of `image` spanning `start..end`.
fn leaf_at(image: &TapeImage, start: u64, end: u64) -> usize {
    use rerail_tape_format::tape::Tape;
    rerail_tape_format::with_tape!(image.tape(), |t| {
        (0..t.len()).find(|&i| t.subtree_end(i) == i + 1 && t.start(i) == start && t.end(i) == end).expect("the leaf")
    })
}

fn fixture(
    source: &str,
    lang: GrammarDialect,
) -> (StrDoc<GrammarDialect>, TapeDoc, TapeImage, Arc<Grammar>) {
    let grammar = Grammar::new(lang).unwrap();
    let native = StrDoc::new(source, lang);
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false)
        .unwrap()
        .primary;
    let tape = TapeDoc::restore(
        Arc::new(source.to_owned().into()),
        Arc::clone(&grammar),
        &image,
        &|| false,
    )
    .unwrap();
    (native, tape, image, grammar)
}

#[test]
fn known_field_edges_match_native_first_child_without_larger_records() {
    for (lang, source) in [
        (
            GrammarDialect::Python,
            "import alpha, beta, gamma\ndef f(a: int, b=2): return a + b\n",
        ),
        (
            GrammarDialect::TypeScript,
            "const x = f(a, b); if (x) { g(x); }",
        ),
    ] {
        let (native, tape, _, grammar) = fixture(source, lang);
        let native = AstGrep::doc(native);
        let restored = AstGrep::doc(tape);
        for (left, right) in stored(&native).into_iter().zip(restored.root().dfs()) {
            for field in 0..=grammar.kinds.field_count() as u16 {
                let expected = left
                    .parent()
                    .and_then(|parent| parent.child_by_field_id(field))
                    .is_some_and(|first| first.node_id() == left.node_id());
                assert_eq!(
                    right.get_inner_node().is_first_field(field),
                    expected,
                    "{lang:?} {:?} field {field}",
                    right.range()
                );
            }
        }
    }
}

type MatchResult = (Range<usize>, Vec<(String, Vec<Range<usize>>)>);

/// `pattern`'s matches over `doc`: a tape's pattern over a tape, the unfiltered one over tree-sitter's
/// tree (`test_support::pattern_for`).
fn matches<D: Doc + 'static>(doc: D, pattern: &str, lang: GrammarDialect) -> Vec<MatchResult> {
    let matcher = crate::code_rule_dsl::test_support::pattern_for::<D>(pattern, lang);
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
        TapeDoc::restore(Arc::new(source.to_owned().into()), grammar, &decoded, &|| false).unwrap();
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
    // `_jsx_closing_tag_lt`, Patch 24's `_no_line_terminator_here`, Patch 28's
    // `_member_name_here`, Patch 29's `_template_escape_without_digits`, Patch 30's `using`
    // declarations, `grammars/tree-sitter-typescript/PATCHES.md`) renumbers them; so does the language's format 16
    // table (its codes, from the grammar, decision 0631, and their numbering, by a corpus's counts: revision 5), and a
    // change of format 16's layout (decision 0434) or of what its home holds (its auxiliary index, decision 0656).
    assert_eq!(
        digest,
        "7a5af35f41d747bf64e59f1f34fe3b9d04cd6d435a9180cd6a9a592d5f2f6ef5"
    );
}

/// A file that embeds a program is two documents (`FileImage`): its own tape, which holds its
/// tree alone, and beside it the derived program's text, tape, source map and host errors, kept
/// as the store keeps them (`file_documents`) and restored as the file's guest. A derived
/// document is refused whose embedding its parent's dialect has not, whose tape is of a dialect
/// its parent does not embed, whose map reaches past its text, whose role is not its map's, or
/// that is out of order.
#[test]
fn file_documents_persist_apart_and_are_checked() {
    let source = "<script lang=\"tsx\">let x = <a />;</script>\n";
    let grammar = Grammar::new(GrammarDialect::TestHtml).unwrap();
    let image = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let [document] = &image.derived[..] else {
        panic!("one program: {}", image.derived.len());
    };
    assert_eq!(document.tape.language, GrammarDialect::Tsx);
    assert_eq!(document.parent, 0);
    // The test embedder writes its program as well as copying it.
    assert_eq!(document.role, DocumentRole::Generated);
    // The file's tape is its own tree alone: a format 16 tape of the host's nodes, nothing after it (a byte past a
    // tape is refused).
    let primary = image.primary.to_bytes().unwrap();
    assert!(super::resident::stored(&primary).is_ok(), "a format 16 tape");
    assert_eq!(TapeImage::from_bytes(&primary, &grammar).unwrap().node_count(), image.primary.node_count());
    let read = image.round_trip(&grammar).unwrap();
    assert_eq!(read.primary.to_bytes().unwrap(), primary);
    let tape = document.tape.to_bytes().unwrap();
    assert_eq!(read.derived[0].tape.to_bytes().unwrap(), tape);
    let restored = TapeDoc::restore(
        Arc::new(source.to_owned().into()),
        Arc::clone(&grammar),
        &read,
        &|| false,
    )
    .unwrap();
    let guests = restored.guests().unwrap();
    assert_eq!(
        guests[0].via,
        crate::code_rule_dsl::language::Via::TestScript
    );
    assert_eq!(guests[0].doc.get_source().as_str(), document.text.as_str());

    let source_map = document.source_map_bytes();
    let host_errors = document.host_error_bytes();
    let via = via_code(document.via);
    let role = document.role.code();
    let read = |ordinal: i64, parent: i64, role: i64, via: i64, tape: &[u8], map: &[u8]| {
        FileImage::read(
            &primary,
            &grammar,
            &[StoredDerived {
                ordinal,
                parent,
                role,
                via,
                text: document.text.as_bytes(),
                source_map: map,
                host_errors: &host_errors,
                tape,
                at: None,
            }],
        )
    };
    assert!(read(1, 0, role, via, &tape, &source_map).is_ok());
    assert!(
        read(2, 0, role, via, &tape, &source_map).is_err(),
        "numbered from 1"
    );
    assert!(
        read(1, 1, role, via, &tape, &source_map).is_err(),
        "an earlier parent"
    );
    let embedded = DocumentRole::Embedded.code();
    assert!(
        read(1, 0, embedded, via, &tape, &source_map).is_err(),
        "its map's role"
    );
    assert!(
        read(1, 0, 0, via, &tape, &source_map).is_err(),
        "never the primary's"
    );
    assert!(
        read(1, 0, role, via ^ 1, &tape, &source_map).is_err(),
        "an embedding of HTML"
    );
    let mut python = tape.clone();
    python[8] = crate::code_rule_dsl::language::tape_tag(GrammarDialect::Python).unwrap();
    assert!(
        read(1, 0, role, via, &python, &source_map).is_err(),
        "HTML embeds no Python"
    );
    let mut past = source_map.clone();
    past[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        read(1, 0, role, via, &tape, &past).is_err(),
        "a segment past the text"
    );
}

/// A derived document is embedded where its text is one byte-exact slice of its parent's (one
/// copied segment over the whole text), and generated otherwise.
#[test]
fn a_derived_documents_role_is_its_source_maps() {
    let map = |segments: &[(usize, usize, Range<usize>, bool)]| {
        let mut map = SegmentMap::new();
        for (at, end, anchor, copied) in segments {
            map.push(*at, *end, anchor.clone(), *copied, false);
        }
        map.index();
        map
    };
    let text = "let x = 1;";
    assert_eq!(
        DocumentRole::derived(text, &map(&[(0, 10, 20..30, true)])),
        DocumentRole::Embedded
    );
    assert_eq!(
        DocumentRole::derived(text, &map(&[(0, 10, 20..30, false)])),
        DocumentRole::Generated,
        "written over its anchor"
    );
    assert_eq!(
        DocumentRole::derived(text, &map(&[(0, 5, 20..25, true), (5, 10, 25..30, true)])),
        DocumentRole::Generated,
        "two stretches"
    );
    assert_eq!(
        DocumentRole::derived(text, &map(&[(0, 9, 20..29, true), (9, 10, 29..29, false)])),
        DocumentRole::Generated,
        "a written end"
    );
    assert_eq!(
        DocumentRole::derived("", &map(&[])),
        DocumentRole::Generated
    );
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

    // A stored tape holds no source digest (the store pairs a tape with its source), so a source of another length
    // is what restore can tell apart.
    let decoded = TapeImage::from_bytes(&bytes, &grammar).unwrap();
    assert!(
        TapeDoc::restore(
            Arc::new("x = 22 # extra\n".to_owned().into()),
            grammar,
            &decoded,
            &|| false,
        )
        .is_err()
    );
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
            let native_nodes = stored(&left);
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
            TapeDoc::restore(Arc::new(source.to_owned().into()), grammar, &image, &|| false).unwrap();
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
            let native_nodes = stored(&native);
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
        // Tree-sitter's navigation over the nodes the tape holds.
        let ids = kept(&left);
        let held = |node: &ast_grep_core::Node<'_, StrDoc<GrammarDialect>>| {
            ids.contains(&node.get_inner_node().id())
        };
        let native_nodes = stored(&left);
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(native_nodes.len(), tape_nodes.len());
        for (a, b) in native_nodes.iter().zip(tape_nodes.iter()) {
            assert_eq!(
                a.next_all()
                    .filter(held)
                    .map(|node| node.range())
                    .collect::<Vec<_>>(),
                b.next_all().map(|node| node.range()).collect::<Vec<_>>(),
                "{lang:?} next_all {:?}",
                a.range()
            );
            assert_eq!(
                a.prev_all()
                    .filter(held)
                    .map(|node| node.range())
                    .collect::<Vec<_>>(),
                b.prev_all().map(|node| node.range()).collect::<Vec<_>>(),
                "{lang:?} prev_all {:?}",
                a.range()
            );
            assert_eq!(
                std::iter::successors(a.next(), |node| node.next())
                    .find(held)
                    .map(|node| node.range()),
                b.next().map(|node| node.range()),
                "{lang:?} next {:?}",
                a.range()
            );
            assert_eq!(
                std::iter::successors(a.prev(), |node| node.prev())
                    .find(held)
                    .map(|node| node.range()),
                b.prev().map(|node| node.range()),
                "{lang:?} prev {:?}",
                a.range()
            );
        }
        let held = |node: &tree_sitter::Node<'_>| ids.contains(&node.id());
        for (a, b) in native_nodes
            .iter()
            .map(|node| node.get_inner_node())
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
                        .filter(held)
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
                        a.field_children(Some(id))
                            .find(held)
                            .map(|node| node.byte_range()),
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
        let native_nodes = stored(&left);
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
        let ids = kept(&native_root);
        let held = |node: &ast_grep_core::Node<'_, StrDoc<GrammarDialect>>| {
            ids.contains(&node.get_inner_node().id())
        };
        let native_nodes = stored(&native_root);
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
                    .filter(held)
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
                    left.field_children(field)
                        .find(held)
                        .map(|child| child.range()),
                    right.field(field).map(|child| child.range())
                );
                assert_eq!(
                    left.field_children(field)
                        .filter(held)
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
    // The tree written again as it is restores: each case below breaks one thing.
    assert!(!refused(rewritten(&image, source, |_, _| {}), &grammar, source));
    let n = image.node_count();
    let len = source.len() as u64;
    for (label, candidate) in [
        ("a byte short", bytes_edited(&image, &grammar, |b| { b.pop(); })),
        ("a byte past the tape", bytes_edited(&image, &grammar, |b| b.push(0))),
        ("another magic", bytes_edited(&image, &grammar, |b| b[0] ^= 0x20)),
        ("the root's subtree short of the tape", rewritten(&image, source, |i, x| if i == 0 { x.subtree_end = 1 })),
        ("the last node past the source", rewritten(&image, source, |i, x| if i == n - 1 { (x.start, x.end) = (len + 2, len + 3) })),
        ("the last node ending before it starts", rewritten(&image, source, |i, x| if i == n - 1 { x.start = x.end + 1 })),
        ("the root ending past the source", rewritten(&image, source, |i, x| if i == 0 { x.end = len + 1 })),
    ] {
        assert!(refused(candidate, &grammar, source), "{label}");
    }
    let mut broken = image.clone();
    broken.source_len += 1;
    assert!(refused(Ok(broken), &grammar, source), "the source's length");
    let mut broken = image.clone();
    broken.source_digest.as_mut().expect("the image holds its source's digest")[0] ^= 1;
    assert!(refused(Ok(broken), &grammar, source), "the source's digest");
    assert!(refused(Ok(image.clone()), &Grammar::new(GrammarDialect::TypeScript).unwrap(), source), "another grammar");
    assert!(refused(Ok(image), &grammar, "a = foo(2)\n"), "another source");
}

#[test]
fn restore_rejects_child_subtree_escaping_parent() {
    let source = "x+y\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    // A node whose subtree ends after its first child: its other children, inside its span, are its parent's.
    let (node, end) = two_children(&image);
    assert!(refused(rewritten(&image, source, |i, x| if i == node { x.subtree_end = end }), &grammar, source));
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
        let (native, tape, image, _) = fixture(source, lang);
        assert!(native.tree.root_node().has_error());
        assert_eq!(!image.error_nodes().is_empty(), errors, "{source}");
        assert_eq!(!image.missing_nodes().is_empty(), missing, "{source}");
        let native_root = AstGrep::doc(native);
        let tape_root = AstGrep::doc(tape.clone());
        let ids = kept(&native_root);
        let held = |node: &ast_grep_core::Node<'_, StrDoc<GrammarDialect>>| {
            ids.contains(&node.get_inner_node().id())
        };
        let native_nodes = stored(&native_root);
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
                    left.children().filter(held).count(),
                ),
                (
                    right.kind(),
                    right.kind_id(),
                    right.is_named(),
                    right.is_extra(),
                    right.is_error(),
                    right.is_missing(),
                    right.range(),
                    right.children().count(),
                )
            );
        }
        let native_errors = super::super::soundness::SyntaxErrors::collect(
            native_root.root().get_doc().tree.root_node(),
        );
        let tape_errors =
            super::super::soundness::SyntaxErrors::from_ranges(tape.syntax_error_ranges());
        let lines = LineTable::new(source, crate::code_rule_dsl::lines::terminators(lang));
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
        // The statement: `debugger` alone is natively its keyword, a token the grammar implies, which
        // no tape stores (RER-131).
        let pattern = if lang == GrammarDialect::Python {
            "x = $A"
        } else {
            "debugger;"
        };
        assert_eq!(
            matches(StrDoc::new(source, lang), pattern, lang),
            matches(tape.clone(), pattern, lang)
        );
        // A node's flags are its code's, in its language's table (`CODE_ERROR`, `CODE_MISSING`).
        if let Some(&index) = image.error_nodes().first() {
            assert!(tape.transducer_flags(index) & 8 != 0, "an error node");
        }
        if let Some(&index) = image.missing_nodes().first() {
            assert!(tape.transducer_flags(index) & 4 != 0, "a missing node");
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
    // A record per stored node: the tokens the grammar implies are in no tape (RER-131).
    let nodes = native.tree.root_node().descendant_count();
    let doc = TapeDoc::restore(Arc::new(source.into()), grammar, &image, &|| false).unwrap();
    assert_eq!(doc.transducer_len(), doc.image.node_count());
    assert_eq!(image.primary.node_count(), doc.transducer_len());
    assert!(doc.transducer_len() < nodes);
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
            Arc::new(source.clone().into()),
            Arc::clone(&grammar),
            &image,
            &interrupt
        )
        .is_err()
    );
    assert_eq!(calls.get(), 5, "restore must poll after the source digest");
    let doc = TapeDoc::restore(Arc::new(source.into()), grammar, &image, &|| false).unwrap();
    assert_eq!(doc.transducer_len(), doc.image.node_count());
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
        let (native, tape, _, _) = fixture(&source, GrammarDialect::Python);
        // The document's tree image: the stored one is named-only (RER-131).
        let image = Arc::clone(&tape.image);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape.clone());
        let native_nodes = stored(&left);
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(
            native_nodes.len(),
            expected_count,
            "{expected_count} native fixture"
        );
        assert_eq!(image.node_count(), expected_count);
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
        let (native, tape, _, _) = fixture(&source, GrammarDialect::Python);
        // The document's tree image: the stored one is named-only (RER-131).
        let image = Arc::clone(&tape.image);
        let left = AstGrep::doc(native);
        let right = AstGrep::doc(tape.clone());
        let native_nodes = stored(&left);
        let tape_nodes = right.root().dfs().collect::<Vec<_>>();
        assert_eq!(
            native_nodes.len(),
            tape_nodes.len(),
            "comment length {length}"
        );
        let comment = native_nodes.iter().find(|node| node.is_extra()).unwrap();
        assert_eq!(comment.range(), 0..length, "comment length {length}");
        assert_eq!(source.find("foo"), Some(length + 1));
        let _ = expected_width;
        let span = |i: usize| rerail_tape_format::with_tape!(image.tape(), |t| {
            use rerail_tape_format::tape::Tape;
            (t.start(i) as usize, t.end(i) as usize)
        });
        let mut saw_comment = false;
        for (index, (a, b)) in native_nodes.iter().zip(tape_nodes.iter()).enumerate() {
            assert_eq!(
                (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
                (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
                "comment length {length} node {index}"
            );
            assert_eq!(
                span(index).0,
                a.range().start,
                "comment length {length} node {index}"
            );
            if a.is_extra() {
                assert_eq!(
                    span(index).1 - span(index).0,
                    a.range().len(),
                    "comment length {length}"
                );
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
    assert_eq!(
        matches(tape, "foo($A)", GrammarDialect::Python),
        matches(native, "foo($A)", GrammarDialect::Python)
    );
    let len = source.len() as u64;
    assert!(refused(rewritten(&image, &source, |i, x| if i == 0 { x.end = len + 1 }), &grammar, &source));
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
        // The root's subtree is the node count.
        assert_eq!(
            rerail_tape_format::with_tape!(image.tape(), |t| rerail_tape_format::tape::Tape::subtree_end(&t, 0)),
            expected_count,
            "root size at {expected_count}"
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
            assert!(
                refused(rewritten(&image, &source, |i, x| if i == 0 { x.subtree_end = expected_count - 1 }), &grammar, &source),
                "root size short of the node count"
            );
        }
    }
}

/// A node of a tape's second block (format 16's checkpoints, every 16 nodes) out of its place is refused: before its
/// predecessor, past the source, or ending inside a codepoint.
#[test]
fn restore_rejects_nodes_out_of_place_past_a_block_boundary() {
    let source = format!("{}#é\n", "x\n".repeat(8));
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    assert_eq!(image.node_count(), 18);
    let comment = leaf_at(&image, 16, 19);
    assert!(comment >= 16, "the comment is in the second block");
    let len = source.len() as u64;
    for (label, candidate) in [
        ("before its predecessor", rewritten(&image, &source, |i, x| if i == comment { x.start = 0 })),
        ("past the source", rewritten(&image, &source, |i, x| if i == comment { x.end = len + 1 })),
        ("ending inside a codepoint", rewritten(&image, &source, |i, x| if i == comment { x.end = 18 })),
    ] {
        assert!(refused(candidate, &grammar, &source), "{label}");
    }
}

#[test]
fn restore_rejects_overwide_spans_and_non_utf8_or_parent_ranges() {
    let source = "é = foo(1)\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    let len = source.len() as u64;
    // A two-byte leaf at 0 (`é`) whose end moves inside the codepoint while the root and its parent stay valid.
    let leaf = leaf_at(&image, 0, 2);
    assert!(!refused(rewritten(&image, source, |_, _| {}), &grammar, source));
    for (label, candidate) in [
        ("start beyond the source", rewritten(&image, source, |i, x| if i == leaf { (x.start, x.end) = (len + 10, len + 10) })),
        ("root ends beyond the source", rewritten(&image, source, |i, x| if i == 0 { x.end = len + 100 })),
        ("leaf ends inside UTF-8 codepoint with valid enclosing spans", rewritten(&image, source, |i, x| if i == leaf { x.end = 1 })),
    ] {
        assert!(refused(candidate, &grammar, source), "{label}");
    }
    let source = "x+y\n";
    let (_, _, image, grammar) = fixture(source, GrammarDialect::Python);
    // The root ends at 2, inside its last child's span, which stays within the source.
    assert!(
        refused(rewritten(&image, source, |i, x| if i == 0 { x.end = 2 }), &grammar, source),
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
            TapeDoc::restore(Arc::new(source.to_owned().into()), grammar, &image, &|| false).unwrap();
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
        let native = stored(&view)
            .into_iter()
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

/// A Blade tape's ERROR leaves are the template's syntax errors as Blade reads them: arguments
/// Forte read after a line break are text to Blade, so an unclosed string in them is no error of
/// the template's, with or without a view.
#[test]
fn a_blade_tapes_errors_are_blades_reading() {
    for source in [
        "@csrf\n(it's required)\n<p>{{ $x }}</p>\n",
        "@csrf\n(it's required)\n<p>x</p>\n",
    ] {
        let tree = crate::code_rule_dsl::blade::forte::tree::parse_tree(source.as_bytes())
            .expect("Forte parses it");
        assert!(
            !tree.error_ranges().is_empty(),
            "Forte reads the parenthesis as arguments: {source:?}"
        );
        let grammar = Grammar::new(GrammarDialect::Blade).expect("grammar");
        let image = TapeImage::parse(&grammar, source, &|| false)
            .expect("parses")
            .expect("encodes");
        let tape = TapeDoc::restore(Arc::new(source.to_owned().into()), grammar, &image, &|| false)
            .expect("restores");
        assert_eq!(
            tape.syntax_error_ranges(),
            Vec::<Range<usize>>::new(),
            "{source:?}"
        );
    }
}

/// Every node's restored supertype memberships, after a round trip through the stored bytes, are
/// exactly what the parser's cursor reports for it, on a corpus; and every node's kind, namedness,
/// flags, range, field and parent agree with the native tree. `RERAIL_TAPE_SIZE='<lang> <dir>'`.
#[test]
#[ignore = "corpus validation; RERAIL_TAPE_SIZE='<lang> <dir>', --ignored --nocapture"]
fn stored_memberships_are_the_cursors_on_a_corpus() {
    let spec = std::env::var("RERAIL_TAPE_SIZE").expect("RERAIL_TAPE_SIZE='<lang> <dir>'");
    let (lang, dir) = spec.split_once(' ').expect("<lang> <dir>");
    let mut files = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if !matches!(
                    name.to_str(),
                    Some(
                        "node_modules"
                            | ".git"
                            | "target"
                            | ".venv"
                            | "venv"
                            | "__pycache__"
                            | "dist"
                            | "site-packages"
                    )
                ) {
                    stack.push(path);
                }
            } else if path.to_str().and_then(crate::code_rule_dsl::language::label_of_path).is_some_and(|label| (if label == "tsx" { "typescript" } else { label }) == lang) && !path.to_string_lossy().ends_with(".blade.php") {
                files.push(path);
            }
        }
    }
    files.sort();
    let (mut nodes, mut memberships, mut documents) = (0_usize, 0_usize, 0_usize);
    let mut checked_detects = false;
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let dialect = crate::code_rule_dsl::language::dialect_by_label(
            crate::code_rule_dsl::language::label_of_path(path.to_str().unwrap()).unwrap(),
        )
        .unwrap();
        let grammar = Grammar::new(dialect).unwrap();
        let native = StrDoc::new(&source, dialect);
        let Some(encoded) =
            TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).ok()
        else {
            continue;
        };
        let image = TapeImage::from_bytes(&encoded.primary.to_bytes().unwrap(), &grammar).unwrap();
        let tape = TapeDoc::restore(
            Arc::new(source.clone().into()),
            Arc::clone(&grammar),
            &image,
            &|| false,
        )
        .unwrap();
        // The cursor's memberships and the native nodes, in preorder.
        let mut expected = Vec::new();
        let mut native_nodes = Vec::new();
        let mut cursor = native.tree.root_node().walk();
        let mut parents: Vec<usize> = Vec::new();
        let mut parent_of = Vec::new();
        loop {
            let index = native_nodes.len();
            let (classes, count) = cursor_supertypes(&cursor);
            let mut own = classes[..count].to_vec();
            own.sort_unstable();
            own.dedup();
            expected.extend(own.into_iter().map(|class| (index as u32, class)));
            native_nodes.push((cursor.node(), cursor.field_id().map_or(0, |id| id.get())));
            parent_of.push(parents.last().copied());
            if cursor.goto_first_child() {
                parents.push(index);
                continue;
            }
            let mut done = false;
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    done = true;
                    break;
                }
                parents.pop();
            }
            if done {
                break;
            }
        }
        let path = path.display();
        assert_eq!(
            tape.transducer_len(),
            native_nodes.len(),
            "{path}: node count"
        );
        assert_eq!(
            tape.transducer_supertypes().collect::<Vec<_>>(),
            expected,
            "{path}: memberships"
        );
        for (index, ((node, field), parent)) in native_nodes.iter().zip(&parent_of).enumerate() {
            let at = index as u32;
            let kind = if node.is_error() {
                ERROR_SYMBOL
            } else {
                node.kind_id()
            };
            assert_eq!(
                (
                    tape.transducer_kind(at),
                    tape.transducer_field(at),
                    tape.transducer_span(at),
                    tape.transducer_parent(at)
                ),
                (kind, *field, node.byte_range(), parent.map(|p| p as u32)),
                "{path}: node {index}"
            );
            let flags = tape.node(at);
            assert_eq!(
                (
                    flags.is_named(),
                    flags.is_extra(),
                    flags.is_error(),
                    flags.is_missing()
                ),
                (
                    node.is_named(),
                    node.is_extra(),
                    node.is_error(),
                    node.is_missing()
                ),
                "{path}: node {index} flags"
            );
        }
        // The comparison is not vacuous: a document with memberships.
        checked_detects |= !expected.is_empty();
        nodes += native_nodes.len();
        memberships += expected.len();
        documents += 1;
    }
    assert!(documents > 0 && checked_detects);
    println!(
        "{documents} documents, {nodes} nodes, {memberships} memberships: all equal the cursor's"
    );
}

/// The Bitlex parser's tape of a JSON file, as stored: `None` where it refuses the file.
fn bitlex_bytes(source: &str) -> Option<Vec<u8>> {
    Some(bitlex_image(source)?.to_bytes().unwrap())
}

/// A JSON file's image from the Bitlex parser, or `None` where it refuses the file.
fn bitlex_image(source: &str) -> Option<TapeImage> {
    let written = crate::code_rule_dsl::transducer::packed::json_parser::image(source)?;
    TapeImage::written_16(written, GrammarDialect::Json, source, None, &|| false).ok()
}

/// A JSON file's tape from the Bitlex parser, and from tree-sitter-json: `None` where the Bitlex
/// parser refuses the file (`Some` of each tape's bytes otherwise).
fn json_tapes(source: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    let grammar = Grammar::new(GrammarDialect::Json).unwrap();
    let ours = bitlex_bytes(source)?;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
        .unwrap()
        .primary
        .to_bytes()
        .unwrap();
    Some((ours, theirs))
}

/// The repository's `.rerail` files (outside `.git`, `target` and `node_modules`), with their text.
fn rr_files() -> Vec<(String, String)> {
    let root = crate::source_tree::native()
    .join("..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| {
                [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref())
            });
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rerail") {
                files.extend(
                    std::fs::read_to_string(&p)
                        .ok()
                        .map(|t| (p.display().to_string(), t)),
                );
            }
        }
    }
    files.sort();
    files
}

/// A `.rerail` file's tape as it was made before the parsers wrote rows: the statement parser's tree, walked in
/// preorder through the engine's tree ops (`TapeImage::walked`, as tree-sitter's cursor walk writes).
fn packed_rr_tape(grammar: &Grammar, source: &str) -> Vec<u8> {
    let nodes = rerail_rr_syntax::engine_tape(source);
    TapeImage::walked(grammar, source, nodes.len(), &|| false, |out| {
        let mut open: Vec<u32> = Vec::new();
        for n in &nodes {
            while open.last().is_some_and(|&o| Some(o) != n.parent) {
                open.pop();
                out.close();
            }
            let error = n.is_error();
            let kf = if error { ERROR_CODE } else { n.kind | (n.field << 10) };
            let visit = Visit { kf, start: n.start, end: n.end, extra: n.extra, error, missing: false, kind_alias: None };
            open.push(out.open(visit, n.parent, &[])?);
        }
        for _ in open {
            out.close();
        }
        Ok(())
    })
    .unwrap()
    .to_bytes()
    .unwrap()
}

/// A `.rerail` file's tape from the lean parser's bytes (the engine's path, `encode_rr`, decision 0140) equals the
/// statement parser's tree packed by the engine's own passes, byte for byte: every `.rerail` file of the repository.
#[test]
fn rr_tapes_from_the_lean_parser_are_the_statement_parsers() {
    let grammar = Grammar::new(GrammarDialect::Rr).unwrap();
    let mut checked = 0;
    for (path, source) in rr_files() {
        let packed = packed_rr_tape(&grammar, &source);
        let lean = TapeImage::encode_rr(&grammar, &source, &|| false)
            .unwrap()
            .to_bytes()
            .unwrap();
        assert!(
            lean == packed,
            "{path}: the lean parser's tape is not the statement parser's"
        );
        checked += 1;
    }
    eprintln!("{checked} files");
    assert!(checked > 1000, "{checked}");
}

/// What a `.rerail` tape costs, in instructions retired and thread CPU time, over the repository's `.rerail` files
/// the compiled parser takes: the lean parser's rows and its tape (the engine's path), the compiled Bitlex
/// parser's rows, and the statement parser's tree and its tape. With `RERAIL_RR_COST_LOOP` set to a step's name, runs that step
/// alone for 20 s (for a profiler).
#[test]
#[ignore = "a measurement: run by name with --nocapture"]
fn rr_parse_costs() {
    // The `.rerail` parser (rerail-rr-parser): the Bitlex grammar compiled.
    use rerail_rr_parser as rr_parser;
    let grammar = Grammar::new(GrammarDialect::Rr).unwrap();
    let mut files: Vec<String> = rr_files().into_iter().map(|(_, t)| t).collect();
    let all = files.len();
    files.retain(|f| rr_parser::compiled_tape(f).is_some());
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files.iter().map(|f| rerail_rr_syntax::tape(f).map_or(0, |t| t.len())).sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    #[allow(clippy::type_complexity)]
    let steps: Vec<CostStep<'_>> = vec![
        ("lean: rows", Box::new(|f| rerail_rr_syntax::tape(f).map_or(0, |t| t.len()))),
        (
            "lean: the tape (the engine's path)",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(TapeImage::encode_rr(&grammar, f, &|| false)).is_ok(),
                )
            }),
        ),
        (
            "bitlex: rows (tiers 1 and 2, JIT)",
            Box::new(|f| rr_parser::compiled_tape(f).unwrap().len()),
        ),
        (
            "statement parser: tree",
            Box::new(|f| rerail_rr_syntax::engine_tape(f).len()),
        ),
        (
            "statement parser: the tape, packed (before)",
            Box::new(|f| packed_rr_tape(&grammar, f).len()),
        ),
    ];
    if let Ok(only) = std::env::var("RERAIL_RR_COST_LOOP") {
        let (_, step) = steps
            .iter()
            .find(|(name, _)| *name == only)
            .expect("a step's name");
        let t0 = cpu();
        let mut n = 0;
        while cpu() - t0 < 20.0 {
            files.iter().for_each(|f| n += step(f));
        }
        std::hint::black_box(n);
        return;
    }
    let mut best = vec![(f64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for ((_, step), best) in steps.iter().zip(&mut best) {
            let (i0, t0) = (retired().0, cpu());
            let n: usize = files.iter().map(|f| step(f)).sum();
            let (i1, t1) = (retired().0, cpu());
            std::hint::black_box(n);
            *best = (best.0.min(t1 - t0), best.1.min(i1 - i0));
        }
    }
    let mb = bytes as f64 / 1e6;
    for ((name, _), (t, i)) in steps.iter().zip(best) {
        eprintln!(
            "{name:44} {:8.1} M instructions {:6.0} a node {:7.1} ms {:7.1} MB/s",
            i as f64 / 1e6,
            i as f64 / nodes as f64,
            t * 1e3,
            mb / t
        );
    }
    eprintln!(
        "{} of {all} files (the compiled parser takes them), {mb:.1} MB, {nodes} nodes",
        files.len()
    );
}

/// The Bitlex parser's JSON tapes are tree-sitter-json's, byte for byte: kinds, fields, spans,
/// extras (comments) and the `_value` supertype; and an invalid file is refused, so it goes to
/// tree-sitter-json.
#[test]
fn json_tapes_from_the_bitlex_parser_are_tree_sitters() {
    assert_eq!(crate::code_rule_dsl::transducer::packed::json_parser::compile_error(), None);
    for source in [
        "{}",
        "[]",
        "null\n",
        "{\"a\": 1, \"b\": [true, false, null], \"c\": {\"d\": \"e\\n\\u00e9\"}}\n",
        "[1, -2.5e10, 0, 3.0E-2]",
        "// a comment\n{\n  \"a\": 1, /* inline */ \"b\": 2\n}\n// trailing\n",
        "\u{feff}{\"bom\": true}",
        "{\"nested\": [[[{\"x\": [1, {\"y\": []}]}]]]}",
        "  \n\t{ \"spaced\" :\t[ 1 ,2 ] }  \n",
    ] {
        let (ours, theirs) = json_tapes(source).unwrap_or_else(|| panic!("refused {source:?}"));
        assert!(ours == theirs, "the tapes of {source:?} differ");
    }
    // JSON allows an exponent's sign; tree-sitter-json refuses `+`, so these tapes are the Bitlex parser's alone.
    for source in ["1e+2", "[1.5E+3]"] {
        assert!(
            crate::code_rule_dsl::transducer::packed::json_parser::image(source).is_some(),
            "refused {source:?}"
        );
    }
    for invalid in ["{", "[1,]", "{\"a\" 1}", "tru", ""] {
        assert!(
            crate::code_rule_dsl::transducer::packed::json_parser::image(invalid).is_none(),
            "accepted {invalid:?}"
        );
    }
}

/// A JSON file's tape from simdjson, and from tree-sitter-json: `Err` with simdjson's reason where it
/// refuses the file (`Ok` of each tape's bytes otherwise).
fn simdjson_tapes(source: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let grammar = Grammar::new(GrammarDialect::Json).unwrap();
    let ours = crate::code_rule_dsl::transducer::packed::json_parser::simdjson::encode(
        &grammar,
        source,
        &|| false,
    )
    .map_err(|why| why.to_string())?
    .unwrap();
    let ours = keyless(&ours);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    // A trailing comma is passed over (JSONC, as TypeScript reads it): tree-sitter-json's tape of the text
    // without it, which is as long.
    let text =
        crate::code_rule_dsl::transducer::packed::json_parser::fallback::without_trailing_commas(source);
    let tree = parser.parse(&text, None).unwrap();
    let theirs = keyless(&TapeImage::encode(tree.root_node(), &grammar, &text, &|| false).unwrap().primary);
    Ok((ours, theirs))
}

/// An image's bytes, its auxiliary index's key zeroed: the index is keyed by its source's digest (decision 0656
/// step 3), and a JSONC file's tape is tree-sitter's of its text without its trailing commas, another source of
/// the same tape.
fn keyless(image: &TapeImage) -> Vec<u8> {
    let mut b = image.to_bytes().unwrap();
    if let Some(aux) = image.aux_bytes() {
        let at = b.len() - aux.len();
        b[at + 24..at + 32].fill(0);
    }
    b
}

/// The product reads JSON with simdjson: a JSON file's image through the engine's parse
/// (`TapeImage::parse`, the parsers the library installs as it loads) is simdjson's, its digest the
/// one simdjson's stage 1 computed (the source's SHA-256); a file simdjson refuses goes to tree-sitter-json.
#[test]
fn the_product_reads_json_with_simdjson() {
    use crate::code_rule_dsl::transducer::packed::json_parser as jp;
    use sha2::Digest;
    let grammar = Grammar::new(GrammarDialect::Json).unwrap();
    assert!(
        tree_parser(GrammarDialect::Json).is_some(),
        "JSON's parser is installed"
    );
    let source = "\u{feff}{\"a\": [1, true, \"x\\ny\"], \"b\": {\"c\": null}}\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let simdjson = jp::simdjson::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        simdjson.to_bytes().unwrap()
    );
    assert_eq!(
        product.primary.source_digest,
        Some(<[u8; 32]>::from(sha2::Sha256::digest(source.as_bytes())))
    );
    // Refused by simdjson, as JSON and as JSONC (two commas): tree-sitter-json's tape.
    let jsonc = "// c\n{\"a\": 1,,}\n";
    assert!(jp::simdjson::encode(&grammar, jsonc, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, jsonc, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// Each language with a lean parser has it as its top tier: the router takes it for the dialect, at
/// rank 0, and no other implementation this host runs ranks with it (a file it refuses goes to the
/// dialect's tree-sitter grammar, which is no registered implementation).
#[test]
fn the_lean_parsers_are_their_languages_top_tier() {
    let lean = [
        (GrammarDialect::Python, "lean Python parser"),
        (GrammarDialect::JavaScript, "lean JavaScript parser"),
        (GrammarDialect::TypeScript, "lean TypeScript parser"),
        (GrammarDialect::Tsx, "lean TSX parser"),
        (GrammarDialect::Rust, "lean Rust parser"),
        (GrammarDialect::Java, "lean Java parser"),
        (GrammarDialect::Go, "lean Go parser"),
        (GrammarDialect::C, "lean C parser"),
        (GrammarDialect::Cpp, "lean C++ parser"),
        (GrammarDialect::CSharp, "lean C# parser"),
        (GrammarDialect::Php, "lean PHP parser"),
        (GrammarDialect::Html, "lean HTML parser"),
        (GrammarDialect::Css, "lean CSS parser"),
        (GrammarDialect::Markdown, "lean Markdown parser"),
        (GrammarDialect::MarkdownInline, "lean Markdown inline parser"),
        (GrammarDialect::Sql, "lean SQL parser"),
        (GrammarDialect::Vue, "lean Vue parser"),
    ];
    let routes = parser_routes();
    for (dialect, name) in lean {
        let mine: Vec<_> = routes.iter().filter(|(r, _)| r.dialect == dialect).collect();
        let taken: Vec<_> = mine.iter().filter(|(_, taken)| *taken).collect();
        assert_eq!(taken.len(), 1, "{dialect:?}: one implementation taken");
        assert_eq!(taken[0].0.name, name, "{dialect:?}: the lean parser is taken");
        assert_eq!(taken[0].0.rank, 0, "{dialect:?}: at the top rank");
        assert!(mine.iter().all(|(r, t)| *t || !r.runs || r.rank > 0), "{dialect:?}: another implementation ranks with the lean parser");
        assert!(tree_parser(dialect).is_some(), "{dialect:?}: routed");
    }
}

/// The parser router takes, for each dialect, the best implementation this host runs: for JSON the first
/// kernel `Kernel::ALL` lists that this CPU runs; each other implementation this CPU runs writes the same
/// tape, the DOM parser and the index writer included.
#[test]
fn the_router_takes_the_best_json_parser_this_cpu_runs() {
    let routes: Vec<_> = parser_routes()
        .into_iter()
        .filter(|(r, _)| r.dialect == GrammarDialect::Json)
        .collect();
    let listing: Vec<String> = routes
        .iter()
        .map(|(r, taken)| {
            format!(
                "{}{}{}",
                r.name,
                if r.runs { "" } else { " (not here)" },
                if *taken { " <- taken" } else { "" }
            )
        })
        .collect();
    eprintln!("JSON's implementations: {listing:#?}");
    let taken: Vec<_> = routes.iter().filter(|(_, taken)| *taken).collect();
    assert_eq!(taken.len(), 1, "one implementation taken");
    let best = rerail_simdjson::Kernel::best().expect("a kernel this CPU runs");
    assert_eq!(taken[0].0.name, best.name());
    assert!(
        routes
            .iter()
            .all(|(r, _)| !r.runs || r.rank >= taken[0].0.rank),
        "a better implementation runs here"
    );
    let grammar = Grammar::new(GrammarDialect::Json).unwrap();
    let source = "{\"a\": [1, -2.5e3, true, false, null, \"x\\u00e9y\"], \"b\": {}}";
    let want = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap()
        .primary
        .to_bytes()
        .unwrap();
    for (r, _) in routes.iter().filter(|(r, _)| r.runs) {
        let image = r
            .parser
            .parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(image.primary.to_bytes().unwrap(), want, "{}", r.name);
    }
    // Every kernel's name is registered, whether this CPU runs it or not.
    for kernel in rerail_simdjson::Kernel::ALL {
        assert!(
            routes.iter().any(|(r, _)| r.name == kernel.name()),
            "{:?}",
            kernel
        );
    }
}

/// JSON's tapes from simdjson (the engine's JSON parser) are tree-sitter-json's, byte for byte: kinds,
/// fields, spans and the `_value` supertype, on hand-written cases and on every `.json` file of the
/// repository simdjson accepts. What it refuses (comments, a trailing comma, invalid JSON) goes to
/// tree-sitter-json; the refused files are only counted.
#[test]
fn json_tapes_from_simdjson_are_tree_sitters() {
    for source in [
        "{}",
        "[]",
        "null\n",
        "true",
        " -0.5e-3 ",
        "\"\"",
        "{\"a\": 1, \"b\": [true, false, null], \"c\": {\"d\": \"e\\n\\u00e9\"}}\n",
        "[1, -2.5e10, 0, 3.0E-2]",
        "\u{feff}{\"bom\": true}",
        "{\"nested\": [[[{\"x\": [1, {\"y\": []}]}]]]}",
        "  \n\t{ \"spaced\" :\t[ 1 ,2 ] }  \n",
        "[\"\\\\\", \"\\\\\\\"\", \"a\\tb\\u0041c\", \"\\ud83d\\ude00\", \"é \\/ ü\", \"  \"]",
        "{\"\": {\"\": []}, \"k\\\"\": \"v\"}",
        "[[],{},[{}],{\"a\":[]}]\r\n",
        // JSONC: comments where tree-sitter-json places them (extras of the lowest node holding the tokens on
        // either side), read again after the strict read refused them.
        "// c\n{}",
        "// a\n{ /*b*/ \"k\" /*c*/ : /*d*/ 1 /*e*/ , /*f*/ \"j\": [ /*g*/ 2 /*h*/ , 3 /*i*/ ] /*j*/ } // k\n",
        "{\"a\": {} /*x*/, \"b\": [ /*y*/ ], \"c\": { /*z*/ }}",
        "{\"a\": 1 // c\r\n}",
        "/* a **/ /** b */1",
        "\u{feff}// x\n{}",
        "1 // eof",
        "[\"/*x*/\", 1] /* a \" quote */",
        "{\n  // The compiler's options.\n  \"compilerOptions\": {\n    \"strict\": true, // always\n    /* \"lib\": [\"dom\"], */\n    \"target\": \"es2022\"\n  }\n}\n",
        // Trailing commas, passed over: tree-sitter-json's tape of the text without them.
        "[1,]",
        "{\"a\": 1,}",
        "// c\n{\"a\": 1,}",
        "[[1,],]",
        "{\"a\": {\"b\": 1,},}",
        "[{\"a\": 1},\n]",
        "[\"x\", // c\n]",
        "{\"a\": [1, 2,], \"b\": 3, /* last */ }",
        "{\n  \"compilerOptions\": {\n    \"strict\": true,\n    \"lib\": [\"es2022\", \"dom\",],\n  },\n}\n",
    ] {
        let (ours, theirs) =
            simdjson_tapes(source).unwrap_or_else(|why| panic!("refused {source:?}: {why}"));
        assert!(ours == theirs, "the tapes of {source:?} differ");
    }
    for invalid in [
        "{",
        "{\"a\" 1}",
        "tru",
        "",
        "[1,,]",
        "[,1]",
        "{,}",
        "// c\n{\"a\": 1,,}",
        "[1 / 2]",
        "/* open",
    ] {
        assert!(simdjson_tapes(invalid).is_err(), "accepted {invalid:?}");
    }
    // JSON allows an exponent's sign; tree-sitter-json refuses `+`, so these tapes are simdjson's alone.
    for source in ["1e+2", "[1.5E+3]"] {
        let grammar = Grammar::new(GrammarDialect::Json).unwrap();
        assert!(
            crate::code_rule_dsl::transducer::packed::json_parser::simdjson::encode(
                &grammar,
                source,
                &|| false
            )
            .is_ok_and(|t| t.is_ok()),
            "refused {source:?}"
        );
    }
    let root = crate::source_tree::native()
    .join("..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| {
                [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref())
            });
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "json")
                && e.metadata().is_ok_and(|m| m.len() <= 1 << 20)
            {
                files.push(p);
            }
        }
    }
    files.sort();
    let (mut equal, mut refused, mut corrected, mut bytes) = (0, 0, 0, 0);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        match simdjson_tapes(&source) {
            Err(_) => refused += 1,
            Ok((ours, theirs)) if ours == theirs => {
                equal += 1;
                bytes += source.len();
            }
            // tree-sitter-json finds an error in a file simdjson accepts (it refuses an exponent's `+`, which JSON
            // allows: `1e+2`). The tape is then the Bitlex parser's, a reading of its own.
            Ok((ours, _)) => {
                assert!(
                    parser
                        .parse(&source, None)
                        .is_some_and(|t| t.root_node().has_error()),
                    "the tapes of {} differ",
                    path.display()
                );
                let bitlex = bitlex_image(&source).map(|i| keyless(&i));
                assert!(
                    bitlex.is_none_or(|b| b == ours),
                    "{}: tree-sitter errs, and simdjson's tape is not the Bitlex parser's",
                    path.display()
                );
                corrected += 1;
            }
        }
    }
    eprintln!(
        "{equal} files ({bytes} bytes) equal tree-sitter-json's tapes; {corrected} where tree-sitter finds an error; {refused} refused"
    );
    assert!(equal > 1000, "{equal} files");
}

/// [`json_tapes_from_the_bitlex_parser_are_tree_sitters`] over files: `RERAIL_JSON_FILES`, a file
/// listing one path a line. Each file the Bitlex parser accepts has tree-sitter-json's tape; the
/// refused files are counted.
#[test]
#[ignore = "needs files: run by name with RERAIL_JSON_FILES"]
fn json_tapes_over_files() {
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_JSON_FILES").expect("set RERAIL_JSON_FILES"))
            .unwrap();
    let (mut equal, mut refused, mut corrected, mut differ) =
        (0, Vec::new(), Vec::new(), Vec::new());
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        match json_tapes(&source) {
            None => refused.push(path.to_owned()),
            Some((ours, theirs)) if ours == theirs => equal += 1,
            // tree-sitter-json finds an error in a file the Bitlex parser accepts (it refuses an exponent's `+`,
            // which JSON allows: `1e+2`): the Bitlex tree is the right one.
            Some(_)
                if parser
                    .parse(&source, None)
                    .is_some_and(|t| t.root_node().has_error()) =>
            {
                corrected.push(path.to_owned())
            }
            Some(_) => differ.push(path.to_owned()),
        }
    }
    eprintln!(
        "{equal} equal, {} refused, {} where tree-sitter finds an error, {} differ",
        refused.len(),
        corrected.len(),
        differ.len()
    );
    for p in refused.iter().take(20) {
        eprintln!("refused: {p}");
    }
    for p in corrected.iter().take(20) {
        eprintln!("tree-sitter's error: {p}");
    }
    assert!(differ.is_empty(), "tapes differ:\n{}", differ.join("\n"));
}

/// [`json_tapes_from_simdjson_are_tree_sitters`] over files: `RERAIL_JSON_FILES`, a file listing one path
/// a line (say a `node_modules`' `.json` files, its `tsconfig.json`s with comments). Each file simdjson
/// reads, strictly or as JSONC, has tree-sitter-json's tape; what it refuses is counted, by whether it
/// has a comment.
#[test]
#[ignore = "needs files: run by name with RERAIL_JSON_FILES"]
fn simdjson_tapes_over_files() {
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_JSON_FILES").expect("set RERAIL_JSON_FILES"))
            .unwrap();
    let (mut strict, mut jsonc, mut refused, mut differ) = (0, 0, Vec::new(), Vec::new());
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    let kinds = &crate::code_rule_dsl::transducer::packed::json_parser::simdjson::kinds()
        .unwrap()
        .0;
    let kernel = rerail_simdjson::Kernel::best().unwrap();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        match simdjson_tapes(&source) {
            Err(_) => refused.push(path.to_owned()),
            Ok((ours, theirs)) if ours == theirs => {
                if rerail_simdjson::events_with(kernel, source.as_bytes(), kinds, false, |_, _| ())
                    .is_ok()
                {
                    strict += 1
                } else {
                    jsonc += 1
                }
            }
            // tree-sitter-json's error in valid JSON (an exponent's `+`): not this test's.
            Ok(_)
                if parser
                    .parse(&source, None)
                    .is_some_and(|t| t.root_node().has_error()) => {}
            Ok(_) => differ.push(path.to_owned()),
        }
    }
    eprintln!(
        "{strict} strict and {jsonc} JSONC files equal tree-sitter-json's tapes; {} refused, {} differ",
        refused.len(),
        differ.len()
    );
    for p in refused.iter().take(40) {
        eprintln!("refused: {p}");
    }
    assert!(differ.is_empty(), "tapes differ:\n{}", differ.join("\n"));
}

#[test]
fn direct_facts_reject_corruption_through_the_shared_admission_contract() {
    use crate::code_rule_dsl::facts::core;
    let text = "é = foo(1)\n";
    let source = Arc::new(text.to_owned());
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    let lines = |text: &str| Arc::new(LineTable::new(text, crate::code_rule_dsl::lines::terminators(GrammarDialect::Python)));
    // Refused by restore and by the facts read off the tape alike.
    let reject = |label: &str, candidate: Result<TapeImage, String>, text: &str| {
        assert!(refused(candidate.clone(), &grammar, text), "{label}: restored");
        assert!(candidate.map_or(true, |image| core::from_image(Arc::new(text.into()), &grammar, image, lines(text)).is_err()), "{label}: facts");
    };
    let mut corrupt = image.clone();
    corrupt.source_digest.as_mut().expect("the image holds its source's digest")[0] ^= 1;
    reject("the source's digest", Ok(corrupt), text);
    reject("a byte short", bytes_edited(&image, &grammar, |b| { b.pop(); }), text);
    reject("the root's subtree short of the tape", rewritten(&image, text, |i, x| if i == 0 { x.subtree_end = 1 }), text);
    reject("the root ending past the source", rewritten(&image, text, |i, x| if i == 0 { x.end = text.len() as u64 + 5 }), text);
    let leaf = leaf_at(&image, 0, 2);
    reject("a leaf ending inside a codepoint", rewritten(&image, text, |i, x| if i == leaf { x.end = 1 }), text);
    // A nested subtree ended early, whose byte span still fits, cannot leave its children to its parent.
    let nested = "f(g(x), y)\n";
    let (_, _, image, _) = fixture(nested, GrammarDialect::Python);
    let (node, end) = two_children(&image);
    reject("a nested subtree ended early", rewritten(&image, nested, |i, x| if i == node { x.subtree_end = end }), nested);
}

/// What reading the repository's JSON costs, in instructions retired and thread CPU time, over the files
/// both simdjson and the Bitlex parser accept: each parser's parse, and its tape (the product's path).
/// With `RERAIL_JSON_COST_LOOP` set to a step's name, runs that step alone for 20 s (for a profiler).
#[test]
#[ignore = "a measurement: run by name with --nocapture"]
fn json_parse_costs() {
    use crate::code_rule_dsl::transducer::packed::json_parser as jp;
    let root = crate::source_tree::native()
    .join("..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| {
                [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref())
            });
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "json")
                && e.metadata().is_ok_and(|m| m.len() <= 1 << 20)
            {
                files.extend(std::fs::read_to_string(&p).ok());
            }
        }
    }
    let all = files.len();
    files.retain(|f| {
        rerail_simdjson::structurals(f.as_bytes(), |_| ()).is_ok() && jp::image(f).is_some()
    });
    let bytes: usize = files.iter().map(String::len).sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&GrammarDialect::Json.get_ts_language())
        .unwrap();
    let grammar = Grammar::new(GrammarDialect::Json).unwrap();
    let kinds = &jp::simdjson::kinds().unwrap().0;
    let nodes: usize = files
        .iter()
        .map(|f| {
            rerail_simdjson::walk_from_index(f.as_bytes(), kinds)
                .unwrap()
                .node_count()
        })
        .sum();
    let kernel = rerail_simdjson::Kernel::best().expect("a kernel this CPU runs");
    let product = tree_parser(GrammarDialect::Json).expect("JSON's parser");
    let steps: Vec<CostStep<'_>> = vec![
        (
            "simdjson: stage 1 alone",
            Box::new(|f| kernel.stage1(f.as_bytes(), false) as usize),
        ),
        (
            "simdjson: stage 1, SHA-256 in it",
            Box::new(|f| kernel.stage1(f.as_bytes(), true) as usize),
        ),
        (
            "simdjson: parse and index",
            Box::new(|f| rerail_simdjson::structurals(f.as_bytes(), <[u32]>::len).unwrap()),
        ),
        (
            "simdjson: DOM parse, events from index",
            Box::new(|f| {
                let mut n = 0;
                rerail_simdjson::events_from_index(f.as_bytes(), kinds, |_, w| n += w.len() / 3)
                    .unwrap();
                n
            }),
        ),
        (
            "simdjson: stage 1 (hashed), stage 2",
            Box::new(|f| {
                let mut n = 0;
                rerail_simdjson::events_with(kernel, f.as_bytes(), kinds, false, |_, w| {
                    n += w.len() / 3
                })
                .unwrap();
                n
            }),
        ),
        // Through the router's `TreeParser::parse`, as the product calls it (the tape's generic code is made in the parser's crate).
        (
            "simdjson: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(product.parse(&grammar, f, &|| false))
                        .is_some_and(|t| t.is_ok()),
                )
            }),
        ),
        (
            "bitlex: parse (the image)",
            Box::new(|f| jp::image(f).unwrap().tape.extent().len()),
        ),
        (
            "bitlex: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(jp::tape::Bitlex.parse(&grammar, f, &|| false))
                        .is_some_and(|t| t.is_ok()),
                )
            }),
        ),
        (
            "tree-sitter-json: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter-json: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
        (
            "the tape's source digest (SHA-256)",
            Box::new(|f| {
                use sha2::Digest;
                std::hint::black_box(sha2::Sha256::digest(f.as_bytes()))[0] as usize
            }),
        ),
    ];
    if let Ok(only) = std::env::var("RERAIL_JSON_COST_LOOP") {
        let (_, step) = steps
            .iter()
            .find(|(name, _)| *name == only)
            .expect("a step's name");
        let t0 = cpu();
        let mut n = 0;
        while cpu() - t0 < 20.0 {
            files.iter().for_each(|f| n += step(f));
        }
        std::hint::black_box(n);
        return;
    }
    let mut best = vec![(f64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for ((_, step), best) in steps.iter().zip(&mut best) {
            let (i0, t0) = (retired().0, cpu());
            let n: usize = files.iter().map(|f| step(f)).sum();
            let (i1, t1) = (retired().0, cpu());
            std::hint::black_box(n);
            *best = (best.0.min(t1 - t0), best.1.min(i1 - i0));
        }
    }
    let mb = bytes as f64 / 1e6;
    for ((name, _), (t, i)) in steps.iter().zip(best) {
        eprintln!(
            "{name:36} {:8.1} M instructions {:6.0} a node {:7.1} ms {:7.1} MB/s",
            i as f64 / 1e6,
            i as f64 / nodes as f64,
            t * 1e3,
            mb / t
        );
    }
    eprintln!(
        "{} of {all} files (both parsers accept them), {mb:.1} MB, {nodes} nodes; {}",
        files.len(),
        kernel.name()
    );
}

/// The lean `.rerail` parser writes the engine's tape (decision 0140): its bytes are the engine's
/// image of every `.rerail` file in the repository, and of a prefix of each (an unfinished file).
/// The language tag it writes is the registry's.
#[test]
fn lean_rr_tape_is_the_engines_image() {
    assert_eq!(
        crate::code_rule_dsl::language::tape_tag(GrammarDialect::Rr),
        Some(rerail_tape_format::RR_TAG)
    );
    let grammar = Grammar::new(GrammarDialect::Rr).expect("grammar");
    let root = crate::source_tree::native()
    .join("..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| {
                [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref())
            });
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rerail") {
                files.extend(
                    std::fs::read_to_string(&p)
                        .ok()
                        .map(|t| (p.display().to_string(), t)),
                );
            }
        }
    }
    assert!(files.len() > 1000, "{} files", files.len());
    // And tapes of long strings and comments, and of many nodes.
    let long = "x".repeat(70_000);
    files.push(("a 70,000-byte string".into(), format!("r(\"{long}\").\n")));
    files.push((
        "a 70,000-byte comment".into(),
        format!("r(1).\n/* {long} */\nr(2).\nr(3).\n"),
    ));
    files.push(("20,000 facts".into(), "r(1).\n".repeat(20_000)));
    for (path, text) in &files {
        let mut half = text.len() / 2;
        while !text.is_char_boundary(half) {
            half -= 1;
        }
        for src in [text.as_str(), &text[..half]] {
            let engine = packed_rr_tape(&grammar, src);
            let lean = TapeImage::encode_rr(&grammar, src, &|| false).unwrap().to_bytes().unwrap();
            if lean != engine {
                let k = lean
                    .iter()
                    .zip(&engine)
                    .position(|(a, b)| a != b)
                    .unwrap_or(lean.len().min(engine.len()));
                panic!(
                    "{path} ({} bytes): tapes of {} / {} bytes differ from byte {k}",
                    src.len(),
                    lean.len(),
                    engine.len(),
                );
            }
        }
    }
}

/// Python's tapes as tree-sitter-python's trees encode them, over the files `RERAIL_PY_FILES` lists: each code
/// (kind, field, flags, supertypes) and how often, the anonymous tokens stored under each parent kind against
/// that kind's count, and the aliased nodes. The census the lean Python parser is written against.
#[test]
#[ignore = "a census over files on disk: RERAIL_PY_FILES"]
fn python_tape_census() {
    use std::collections::BTreeMap;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Python.get_ts_language())
        .unwrap();
    let name = |kind: u16| grammar.kind_name(kind).unwrap_or("?").to_owned();
    let field = |f: u16| {
        if f == 0 {
            String::new()
        } else {
            grammar.field_name(f).unwrap_or("?").to_owned()
        }
    };
    let (mut files, mut errors, mut aliased, mut nodes, mut bytes) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut stored: BTreeMap<(String, String), usize> = BTreeMap::new();
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let tree = parser.parse(&source, None).unwrap();
        if tree.root_node().has_error() {
            errors += 1;
            continue;
        }
        files += 1;
        bytes += source.len();
        let image = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
            .unwrap()
            .primary;
        aliased += image.aliased_nodes();
        let n = image.node_count();
        let cells = cells(&image);
        nodes += n;
        let mut open: Vec<(usize, String)> = Vec::new();
        for i in 0..n {
            while open.last().is_some_and(|&(end, _)| end <= i) {
                open.pop();
            }
            let code = &cells[i];
            let kind = code.kind;
            let classes: Vec<String> = code.classes.iter().map(|&c| name(c)).collect();
            let text = format!(
                "{} [{}] flags={} {{{}}}",
                name(kind),
                field(code.field),
                code.flags,
                classes.join(",")
            );
            *codes.entry(text).or_default() += 1;
            let k = name(kind);
            *kinds.entry(k.clone()).or_default() += 1;
            if !grammar.kind_is_named(kind) {
                let parent = open.last().map(|(_, p)| p.clone()).unwrap_or_default();
                *stored
                    .entry((parent, format!("{k} [{}]", field(code.field))))
                    .or_default() += 1;
            }
            open.push((i + image.subtree_size(i) as usize, k));
        }
    }
    eprintln!(
        "{files} files ({bytes} bytes, {nodes} nodes), {errors} with errors left out, {aliased} aliased nodes"
    );
    eprintln!("== codes ({})", codes.len());
    for (c, n) in &codes {
        eprintln!("{n:>9}  {c}");
    }
    eprintln!("== stored anonymous tokens: parent, token, count, parent's count");
    for ((p, t), n) in &stored {
        eprintln!(
            "{n:>9} / {:>9}  {p}  {t}",
            kinds.get(p).copied().unwrap_or(0)
        );
    }
}

/// The engine's tape of a Python file, as tree-sitter-python's tree encodes it, printed as an indented tree: each
/// node's kind, field, flags, supertypes and span. `RERAIL_PY_SRC` names the file; the snippets the lean Python
/// parser is written against are read this way.
#[test]
#[ignore = "prints the tape of RERAIL_PY_SRC"]
fn python_tape_dump() {
    let path = std::env::var("RERAIL_PY_SRC").expect("set RERAIL_PY_SRC");
    let source = std::fs::read_to_string(path).unwrap();
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Python.get_ts_language())
        .unwrap();
    // Each case, cut at lines holding only `#---`, is printed on its own.
    for case in source.split("#---\n") {
        let tree = parser.parse(case, None).unwrap();
        let image = TapeImage::encode(tree.root_node(), &grammar, case, &|| false)
            .unwrap()
            .primary;
        println!(
            "=== {:?}{}",
            case,
            if tree.root_node().has_error() {
                "  (has errors)"
            } else {
                ""
            }
        );
        print!("{}", python_tape_text(&image, &grammar, case));
    }
}

/// A node of a tape as the tape API reads it: its code's kind, field, flags and classes, its span, and one past its
/// subtree.
struct Cells {
    kind: u16,
    field: u16,
    flags: u8,
    classes: Vec<u16>,
    start: u64,
    end: u64,
    subtree_end: usize,
}

/// Each node of `image` in preorder (`Cells`).
fn cells(image: &TapeImage) -> Vec<Cells> {
    use rerail_tape_format::tape::Tape;
    rerail_tape_format::with_tape!(image.tape(), |t| {
        (0..t.len())
            .map(|i| {
                let e = t.table().entry(t.code(i));
                Cells { kind: e.kind, field: e.field, flags: e.flags, classes: e.classes.to_vec(), start: t.start(i), end: t.end(i), subtree_end: t.subtree_end(i) }
            })
            .collect()
    })
}

/// A Python tape as text: a node a line, indented by depth, its kind, field, flags, supertypes and span.
pub(crate) fn python_tape_text(image: &TapeImage, grammar: &Grammar, source: &str) -> String {
    let cells = cells(image);
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..image.node_count() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let code = &cells[i];
        let kind = code.kind;
        let f = code.field;
        let classes: Vec<&str> = code
            .classes
            .iter()
            .map(|&c| grammar.kind_name(c).unwrap_or("?"))
            .collect();
        let (s, end, len) = (code.start as usize, code.end as usize, code.subtree_end - i);
        let text: String = source.get(s..end).unwrap_or("").chars().take(30).collect();
        out.push_str(&format!(
            "{}{}{}{}{} [{s}..{end}] {:?}\n",
            "  ".repeat(open.len()),
            grammar.kind_name(kind).unwrap_or("?"),
            if f == 0 {
                String::new()
            } else {
                format!(" {}:", grammar.field_name(f).unwrap_or("?"))
            },
            if code.flags == 0 {
                String::new()
            } else {
                format!(" flags={}", code.flags)
            },
            if classes.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", classes.join(","))
            },
            text
        ));
        open.push(i + len);
    }
    out
}

/// For the lean Python parser's oracle (`rerail-python-lean`'s `oracle` test): each file of
/// `RERAIL_PY_FILES` (a list of paths), its tape as `python_tape_plain` writes it, to
/// `RERAIL_PY_OUT/<n>.txt` (the path on the first line; then an `ERROR` line where
/// tree-sitter-python's tree has an error).
/// The tokens the tapes of JavaScript, TypeScript and TSX leave out, per kind (the automata's
/// `implied_tokens`): `rerail-ecma-lean`'s `gen_kinds.py` reads them into its tables, so the lean
/// parser stores the tokens the tape does, by the grammar, not by hand.
#[test]
#[ignore = "writes the ECMAScript grammars' implied tokens to RERAIL_ECMA_IMPLIED"]
fn ecma_implied_tokens() {
    let out = std::env::var("RERAIL_ECMA_IMPLIED").expect("set RERAIL_ECMA_IMPLIED");
    let mut text = String::new();
    // With `RERAIL_IMPLIED_DIALECTS` (comma-separated: `rust`, `java`, `csharp`, ...), those dialects'
    // rows instead, for another lean parser's table.
    let all = [
        ("javascript", GrammarDialect::JavaScript),
        ("typescript", GrammarDialect::TypeScript),
        ("tsx", GrammarDialect::Tsx),
        ("rust", GrammarDialect::Rust),
        ("java", GrammarDialect::Java),
        ("csharp", GrammarDialect::CSharp),
        ("php", GrammarDialect::Php),
        ("html", GrammarDialect::Html),
        ("css", GrammarDialect::Css),
        ("markdown", GrammarDialect::Markdown),
        ("markdown-inline", GrammarDialect::MarkdownInline),
        ("sql", GrammarDialect::Sql),
        ("kotlin", GrammarDialect::Kotlin),
        ("swift", GrammarDialect::Swift),
        ("scala", GrammarDialect::Scala),
        ("go", GrammarDialect::Go),
        ("c", GrammarDialect::C),
        ("cpp", GrammarDialect::Cpp),
    ];
    let wanted = std::env::var("RERAIL_IMPLIED_DIALECTS")
        .unwrap_or_else(|_| "javascript,typescript,tsx".into());
    for (name, dialect) in all
        .into_iter()
        .filter(|(name, _)| wanted.split(',').any(|w| w == *name))
    {
        let grammar = tokens::TokenGrammar::of_dialect(dialect).expect("a token grammar");
        for (kind, dropped) in grammar.implied_tokens() {
            for (token, field) in dropped {
                text.push_str(&format!("{name}\t{kind}\t{token}\t{field}\n"));
            }
        }
    }
    std::fs::write(out, text).unwrap();
}

/// Writes a dialect's token automata (`TokenGrammar::automata_text`) to `RERAIL_AUTOMATA_OUT`, for a lean
/// parser's tables: `RERAIL_AUTOMATA_DIALECT=sql` writes `native/crates/sql_lean/automata.tsv`,
/// `markdown-inline` `native/crates/markdown_inline_lean/automata.tsv`, `vue`
/// `native/crates/vue_lean/automata.tsv`; `kotlin`, `swift` and `scala` those lean parsers'.
#[test]
#[ignore = "writes a lean parser's automata table; needs RERAIL_AUTOMATA_OUT"]
fn token_automata_text() {
    let out = std::env::var("RERAIL_AUTOMATA_OUT").expect("set RERAIL_AUTOMATA_OUT");
    let dialect = match std::env::var("RERAIL_AUTOMATA_DIALECT").as_deref() {
        Ok("sql") | Err(_) => GrammarDialect::Sql,
        Ok("markdown-inline") => GrammarDialect::MarkdownInline,
        Ok("vue") => GrammarDialect::Vue,
        Ok("kotlin") => GrammarDialect::Kotlin,
        Ok("swift") => GrammarDialect::Swift,
        Ok("scala") => GrammarDialect::Scala,
        Ok(other) => panic!("no automata table for {other}"),
    };
    std::fs::write(
        out,
        tokens::TokenGrammar::of_dialect(dialect)
            .expect("a token grammar")
            .automata_text(),
    )
    .unwrap();
}

#[test]
#[ignore = "writes the oracle's expected tapes; needs RERAIL_PY_FILES and RERAIL_PY_OUT"]
fn python_tape_expect() {
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let out = std::path::PathBuf::from(std::env::var("RERAIL_PY_OUT").expect("set RERAIL_PY_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    // With `RERAIL_PY_DIALECT` (`javascript`, `typescript`, `tsx`, `rust`, `java`, `csharp`, `php`, `vue`, ...),
    // another dialect's tapes, for that lean parser's oracle (`rerail-ecma-lean`, ...). PHP's are of
    // each file's view (`parse_input`, the bytes tree-sitter-php parses), written to `views/` beside
    // the tapes and named in them. `blade-views` writes, instead of tapes, each Blade template's
    // compiled program (`blade::view`, the PHP its embedding parses) to `RERAIL_PY_OUT`, a file each.
    if std::env::var("RERAIL_PY_DIALECT").as_deref() == Ok("blade-views") {
        let mut n = 0;
        for path in list.lines().filter(|l| !l.is_empty()) {
            let Ok(source) = std::fs::read_to_string(path) else {
                continue;
            };
            let Ok(tree) = crate::code_rule_dsl::blade::forte::tree::parse_tree(source.as_bytes())
            else {
                continue;
            };
            let Ok(read) = crate::code_rule_dsl::blade::view::View::read(&tree, &source, &|| false)
            else {
                continue;
            };
            if let Some(view) = read.view {
                std::fs::write(out.join(format!("{n:05}.php")), view.text()).unwrap();
                n += 1;
            }
        }
        eprintln!("{n} compiled views");
        return;
    }
    // `php-raw`: PHP's tapes of the files as they are (a Blade template's compiled program, which
    // its embedding parses with no `<?php` tag).
    let php = std::env::var("RERAIL_PY_DIALECT").as_deref() == Ok("php");
    let dialect = match std::env::var("RERAIL_PY_DIALECT").as_deref() {
        Ok("php") | Ok("php-raw") => GrammarDialect::Php,
        Ok("javascript") => GrammarDialect::JavaScript,
        Ok("typescript") => GrammarDialect::TypeScript,
        Ok("tsx") => GrammarDialect::Tsx,
        Ok("rust") => GrammarDialect::Rust,
        Ok("java") => GrammarDialect::Java,
        Ok("csharp") => GrammarDialect::CSharp,
        Ok("html") => GrammarDialect::Html,
        Ok("css") => GrammarDialect::Css,
        Ok("markdown") => GrammarDialect::Markdown,
        Ok("markdown-inline") => GrammarDialect::MarkdownInline,
        Ok("sql") => GrammarDialect::Sql,
        Ok("vue") => GrammarDialect::Vue,
        Ok("kotlin") => GrammarDialect::Kotlin,
        Ok("swift") => GrammarDialect::Swift,
        Ok("scala") => GrammarDialect::Scala,
        Ok("go") => GrammarDialect::Go,
        Ok("c") => GrammarDialect::C,
        Ok("cpp") => GrammarDialect::Cpp,
        _ => GrammarDialect::Python,
    };
    let grammar = Grammar::new(dialect).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&dialect.get_ts_language()).unwrap();
    // With `RERAIL_PY_CUTS=k`, also each file cut at `k` places, as a file is while it is written:
    // `<n>-<j>.txt`, the path and the cut on the first line, and an `ERROR` line before the tape
    // where tree-sitter-python's tree has an error.
    let cuts: usize = std::env::var("RERAIL_PY_CUTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let edits: usize = std::env::var("RERAIL_PY_EDITS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let (mut written, mut errors) = (0usize, 0usize);
    for (n, path) in list.lines().filter(|l| !l.is_empty()).enumerate() {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let view_path;
        let (source, path) = if php {
            let view = crate::code_rule_dsl::php_view::view(&source).into_owned();
            std::fs::create_dir_all(out.join("views")).unwrap();
            view_path = out
                .join("views")
                .join(format!("{n:05}.php"))
                .to_string_lossy()
                .into_owned();
            std::fs::write(&view_path, &view).unwrap();
            (view, view_path.as_str())
        } else {
            (source, path)
        };
        let tree = parser.parse(&source, None).unwrap();
        if tree.root_node().has_error() {
            // Its tape with tree-sitter's recovery (`ERROR` nodes, missing tokens), after an
            // `ERROR` line: what a forgiving lean parser is to write.
            let image = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
                .unwrap()
                .primary;
            std::fs::write(
                out.join(format!("{n:05}.txt")),
                format!("{path}\nERROR\n{}", python_tape_plain(&image, &grammar)),
            )
            .unwrap();
            errors += 1;
            continue;
        }
        let image = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
            .unwrap()
            .primary;
        let text = format!("{path}\n{}", python_tape_plain(&image, &grammar));
        std::fs::write(out.join(format!("{n:05}.txt")), text).unwrap();
        written += 1;
        for j in 0..cuts {
            let mut cut = source.len() * (j + 1) / (cuts + 1);
            while !source.is_char_boundary(cut) {
                cut -= 1;
            }
            let text = &source[..cut];
            let tree = parser.parse(text, None).unwrap();
            let tape = python_tape_plain(
                &TapeImage::encode(tree.root_node(), &grammar, text, &|| false)
                    .unwrap()
                    .primary,
                &grammar,
            );
            let body = if tree.root_node().has_error() {
                format!("ERROR\n{tape}")
            } else {
                tape
            };
            std::fs::write(
                out.join(format!("{n:05}-{j}.txt")),
                format!("{path}\t{cut}\n{body}"),
            )
            .unwrap();
        }
        // With `RERAIL_PY_EDITS=k`, also `k` lines each cut short in the middle of the file, the rest
        // kept, as a line is while it is typed: `<n>-e<j>.txt`, the path and `@<cut>-<line end>` on
        // the first line (the text is the file's bytes before the cut and from the line's end on).
        for j in 0..edits {
            let lines: Vec<usize> = std::iter::once(0)
                .chain(source.match_indices('\n').map(|(k, _)| k + 1))
                .collect();
            if lines.len() < 3 {
                break;
            }
            let l = lines.len() * (j + 1) / (edits + 1);
            let (from, to) = (
                lines[l.min(lines.len() - 1)],
                lines.get(l + 1).map_or(source.len(), |&e| e - 1),
            );
            let indent = source[from..to].len() - source[from..to].trim_start().len();
            if to <= from + indent + 1 {
                continue;
            }
            let mut cut = from + indent + (to - from - indent) / 2;
            while !source.is_char_boundary(cut) {
                cut -= 1;
            }
            let text = format!("{}{}", &source[..cut], &source[to..]);
            let tree = parser.parse(&text, None).unwrap();
            let tape = python_tape_plain(
                &TapeImage::encode(tree.root_node(), &grammar, &text, &|| false)
                    .unwrap()
                    .primary,
                &grammar,
            );
            let body = if tree.root_node().has_error() {
                format!("ERROR\n{tape}")
            } else {
                tape
            };
            std::fs::write(
                out.join(format!("{n:05}-e{j}.txt")),
                format!("{path}\t@{cut}-{to}\n{body}"),
            )
            .unwrap();
        }
    }
    eprintln!("{written} tapes written, {errors} files with errors left out");
}

/// A Python tape as plain text, for the lean parser's oracle: a node a line, indented by depth, its
/// kind, field, flags, supertypes and span (`rerail_python_lean::tape_text` writes the same).
pub(crate) fn python_tape_plain(image: &TapeImage, grammar: &Grammar) -> String {
    let cells = cells(image);
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for i in 0..image.node_count() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let code = &cells[i];
        let kind = code.kind;
        let f = code.field;
        let classes: Vec<&str> = code
            .classes
            .iter()
            .map(|&c| grammar.kind_name(c).unwrap_or("?"))
            .collect();
        let (s, end, len) = (code.start, code.end, code.subtree_end - i);
        out.push_str(&format!(
            "{}{}{}{}{} [{s}..{end}]\n",
            "  ".repeat(open.len()),
            grammar.kind_name(kind).unwrap_or("?"),
            if f == 0 {
                String::new()
            } else {
                format!(" {}:", grammar.field_name(f).unwrap_or("?"))
            },
            if code.flags == 0 {
                String::new()
            } else {
                format!(" flags={}", code.flags)
            },
            if classes.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", classes.join(","))
            },
        ));
        open.push(i + len);
    }
    out
}

/// Whether two writers' tapes of one source say the same: in format 16 the same bytes; in 16z the same nodes, read
/// in a batch of their own, as 16z's piece order is the order its blocks were finished, which follows the writer's
/// commits (a lean parser commits at its statements, the conversion at each node; decision 0434 §2).
fn same_tape(ours: &TapeImage, theirs: &TapeImage) -> bool {
    use rerail_tape_format::tape::Tape as _;
    if ours.format() != super::formats::Format::F16Z || theirs.format() != super::formats::Format::F16Z {
        return ours.to_bytes().unwrap() == theirs.to_bytes().unwrap();
    }
    let (ca, cb) = (ours.own_cache(), theirs.own_cache());
    let (a, b) = (ours.tape_in(ca.as_ref()), theirs.tape_in(cb.as_ref()));
    rerail_tape_format::with_tape!(a, |a| rerail_tape_format::with_tape!(b, |b| {
        a.len() == b.len()
            && (0..a.len()).all(|i| {
                a.table().entry(a.code(i)) == b.table().entry(b.code(i))
                    && (a.start(i), a.end(i), a.subtree_end(i), a.parent(i)) == (b.start(i), b.end(i), b.subtree_end(i), b.parent(i))
            })
    }))
}

/// Python's tapes from the lean parser (the product's Python parser) are tree-sitter-python's, byte for
/// byte, on hand-written cases; a text tree-sitter reads with an error the lean parser refuses (its
/// file goes to tree-sitter). The corpus comparison is `rerail-python-lean`'s `oracle`.
#[test]
fn python_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::python_lean;
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Python.get_ts_language())
        .unwrap();
    for source in [
        "",
        "\n\n",
        "# only a comment\n",
        "x = 1  # c\n",
        "import os\nfrom . import (a, b,)\n",
        "def f(a, /, b: int = 1, *c, d, **e) -> None:\n    \"\"\"Doc.\"\"\"\n    return [x async for x in a if x]\n",
        "@dec\nclass A[T](B, metaclass=M):\n    x: list[int] | None = None\n    # c\n\n# d\ny = f'{a!r:>{w}} {b=}'\n",
        "if a:\n    b\nelif c:\n    d\nelse:\n    e\n",
        "try:\n    a\nexcept (A, B) as e:\n    raise C from e\nfinally:\n    pass\n",
        "with (a as b, c as d):\n    pass\nwith x if y else z as w:\n    pass\n",
        "match p:\n    case [1, *rest] if rest:\n        pass\n    case Point(x=0) | {'k': v}:\n        pass\n    case _:\n        pass\n",
        "a, *b = c\nx = *d, e\ny = [*f]\nz = lambda: (yield)\n",
        "x = not a and b or c < d is not e not in f\ny = -a ** -b | c @ d\n",
        "print \"py2\", x\nexec code in ns\nx = `y`\n",
        "x = r'\\\\' + rb'\\d' + b'\\N{x}' + '\\u00e9\\q'\n",
        "x = 1 \\\n    + 2\n",
        // A line end where tree-sitter's scanner gives no NEWLINE (the parser takes none): the line
        // goes on in the next (`_ = b = 1`, `col: message: str`).
        "with a as _:\n    _ =\n    b = 1\n",
        "class A:\n    col:\n    message: str\n",
        "def f(x: a: b): pass\n",
        // `:=` after a string is a format specifier.
        "print(f\"{' a ':=^8}\")\n",
        // Chains: each operator or postfix wraps the chain so far (held by the writer until it ends).
        "x = a + b * c - d.e.f(g)[h] + i - j ** k ** l + (m + n) * o.p\n",
        "x = a or b and c or not d and e.f(g or h)[i and j] or k\n",
        "x = a.b(c + d.e(f * g + h)[i - j].k + l).m\n",
        // A with item's parentheses around one `as` and a `,`: a tuple.
        "with (a as b,):\n    pass\n",
        // A walrus's value is no conditional: the conditional wraps the named expression.
        "if (y := f(x)) if c else d:\n    pass\n",
        "x = [y := a or b, z := lambda: 1]\n",
        // `type` subscripted is a name; `type X = …` an alias.
        "type[1] = 2\n",
        "type X = int\ntype Y[T] = list[T]\n",
        // A splat's form by its place: a statement's primary, a pattern's, an expression's.
        "*a.b.c\n",
        "*a, = b\n",
        "x = a, *b.c\n",
        "f(*a.b, *c)\n",
        "[*a for x in y]\n",
        // `_` in a union and a splat is stored; alone, it is not.
        "match p:\n    case A(x) | _:\n        pass\n    case [*_, _]:\n        pass\n    case {\"k\": _}:\n        pass\n",
        "match p:\n    case _ | _:\n        pass\n",
        "match p:\n    case 1 | -2 | x:\n        pass\n",
        "for x in lambda: y:\n    pass\n",
        "def f(*async): pass\n",
        // A line below the block inside brackets: no DEDENT where a closer may follow, a string
        // starts the line, or an f-string's interpolation is open (the scanner's
        // `inside_interpolated_string`, set as the f-string starts).
        "def f():\n    x = (a +\n'b')\n",
        "def f():\n    x = f\"\"\"{(a +\nb)}\"\"\"\n",
        "def f():\n    x = f\"\"\"{(f\"a\" +\n'b')}\"\"\"\n",
        "def f():\n    x = f\"\"\"{(f\"a\" + f\"{c +\nd}\")}\"\"\"\n",
        "def f():\n    x = f\"{g(a,\nb)}\"\n",
    ] {
        let ours = python_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter-python's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Long chains: linear in the writer (`Writer::open_chain`), and tree-sitter's tape.
    let terms = |sep: &str| {
        (0..3000)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(sep)
    };
    for source in [
        format!("x = {}\n", terms(" + ")),
        format!("x = {}\n", terms(".")),
        format!("x = {}\n", terms(" or b.c(d) and ")),
    ] {
        let ours = python_lean::encode(&grammar, &source, &|| false)
            .unwrap_or_else(|why| panic!("refused a chain: {why:?}"))
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let theirs = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
            .unwrap()
            .primary;
        assert!(
            same_tape(&ours, &theirs),
            "the tapes of a chain of {} bytes differ",
            source.len()
        );
    }
    for source in [
        format!("x = {}a\n", "a + ".repeat(200_000)),
        format!("x = a{}\n", ".b(c)".repeat(100_000)),
    ] {
        assert!(
            python_lean::encode(&grammar, &source, &|| false).is_ok_and(|t| t.is_ok()),
            "refused a chain of {} bytes",
            source.len()
        );
    }
    // Any syntax error: refused, for tree-sitter-python, which recovers as it recovers. The lean
    // parser has one behavior: tree-sitter's tape, or a refusal.
    for invalid in [
        "if a\n    b\n",
        "x = 'open\n",
        "f(a b)\n",
        "x = (1,\ny = 2\n",
        // tree-sitter puts in the missing `)` before the `:` and reads on (the workspace's broken revision,
        // `tape_workspace_matches_native_after_edit_noop_and_invalid_revision`).
        "def broken(:\n",
        // Texts that end in a statement (a file being written).
        "x = (1,\n",
        "x = [1, 2\n",
        "def f(x):\n    return foo(a, b",
        "import os\nfrom a.b import (c,",
        "class A:\n    def f(self):\n        return 1\n\n    x = g(y.z",
        "# c\nif x:\n    y = 1\n    # d\n    z = [a for",
        "from os.path impor",
        // A block's first statement left open.
        "def f(scope: PlanScope):\n    return (\n",
        "@p.\ndef f():\n    pass\n",
        // After a bare name, tree-sitter's lexer reads `:=` in an interpolation: a walrus.
        "f\"{x:=}\"\n",
        // `async` and `await` are keywords as a definition's or a parameter's name.
        "def await(): pass\n",
        "class async: pass\n",
        "def f(async=1): pass\n",
        "lambda await: 1\n",
        // A line below the block where no closer may follow inside brackets: the scanner's DEDENT,
        // in an interpolation too once a nested string has ended (its end clears the flag).
        "def f():\n    (bar.\nbaz)\n",
        "def f():\n    (a +\nbaz)\n",
        "def f():\n    x = (a +\nf'b')\n",
        "def f():\n    x = f\"\"\"{(f\"a\" +\nb)}\"\"\"\n",
        "def f():\n    x = f\"\"\"{('a' + f\"{c}\" + (d +\ne))}\"\"\"\n",
        "def f():\n    x = f\"\"\"{f\"a\" +\nb}\"\"\"\n",
    ] {
        let tree = parser.parse(invalid, None).unwrap();
        assert!(
            tree.root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            python_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
        // Through the engine's parse: tree-sitter-python's tape, its error with it.
        let product = TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .unwrap()
            .primary;
        let theirs = TapeImage::encode(tree.root_node(), &grammar, invalid, &|| false)
            .unwrap()
            .primary;
        assert!(
            product.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the engine's tape of {invalid:?} is not tree-sitter's"
        );
    }
}

/// The product reads Python with the lean parser: a Python file's image through the engine's parse
/// (`TapeImage::parse`, the parsers the library installs as it loads) is the lean parser's; a file it
/// refuses goes to tree-sitter-python.
#[test]
fn the_product_reads_python_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::python_lean;
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    assert!(
        tree_parser(GrammarDialect::Python).is_some(),
        "Python's parser is installed"
    );
    let source = "def f(x):\n    return x + 1  # one\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = python_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    // Refused (a syntax error): tree-sitter-python's tape, with its error.
    let invalid = "def f(:\n    pass\n";
    assert!(python_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Python file's tape costs, by parser: the lean parser (`tape::python_lean`, the product's)
/// against tree-sitter-python, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read (the lean parser's refusals
/// are counted apart).
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn python_parse_costs() {
    use crate::code_rule_dsl::tape::python_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&GrammarDialect::Python.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| python_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            python_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: tokens alone",
            Box::new(|f| rerail_python_lean::token_count(f.as_bytes()).unwrap_or(0)),
        ),
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_python_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(python_lean::encode(&grammar, f, &|| false)).is_ok(),
                )
            }),
        ),
        (
            "tree-sitter-python: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter-python: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "{} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// JavaScript's, TypeScript's and TSX's tapes from the lean parser (the product's parser for the
/// three, decision 0184) are tree-sitter's, byte for byte, on hand-written cases; a text tree-sitter
/// reads with an error the lean parser refuses (its file goes to tree-sitter). The corpus comparison
/// is `rerail-ecma-lean`'s `oracle`.
#[test]
fn ecma_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::ecma_lean;
    use rerail_ecma_lean::Dialect;
    let common = [
        "",
        "// only a comment\n",
        "import a, { b as c, d } from \"e\";\nexport { f as default };\nexport * from './g';\n",
        "const x = { a, b: [1, , 2,], ...c, [d]: e, get f() { return 1 }, async *g() {} };\n",
        "function f(a, { b = 1 }, ...c) {\n  if (a) return\n  b++\n  for (const k in o) {}\n  for await (x of y);\n}\n",
        "class A extends B { static #x = 1; get y() { return this.#x } static { z() } }\n",
        "label: while (a) { try { break label } catch { } finally { continue } }\n",
        "x = a ? b : c ?? d || e && f | g ^ h & i == j < k << l + m * n ** o\n",
        "let s = `a${b}c` + tag`d` + /re[/]g/.source; a = b\n(c)\n",
        "x = async (a) => { await a }\ny = a => a?.b?.[c]?.(d)\n",
        "a\n++b\nreturn\n",
        "debugger;\nif (a) debugger\n",
        // A declarator's `undefined` is a name; `yield` as a parameter, and as `void`'s operand.
        "var a = 1, undefined;\nlet undefined\nfunction f(undefined) {}\n",
        "function f(yield, b = yield) { void yield; }\nx = function* (yield) {}\n",
        // A class body's `{` is stored before a `;`.
        "class A { ; m() {} }\nx = class { ;; }\n",
        // A comment on a `}`'s line, before the automatic semicolon the grammar allows there: in
        // the block (or class) where `||` may follow it (an empty block, a function's body), and
        // where the scanner gives no semicolon at the `}` (a line follows, not `,` or `(`).
        "if (a) {} // c\nx\n{ a } // c\nx\nfunction f() {} // c\n// d\nx = function () {} // c\n+ y\n",
        "class B {} // c\nx\nf(() => {} // c\n)\ntry {} // c\ncatch (e) {}\n{} // c\n(x)\nf(function () {} // c\n, 1)\n",
        "{} /* a */ // c\n.5\nif (a) { b } // c\nelse {} // d\nx\n",
        "function f() {} // c",
        "42 /* c */ // d",
        // Chains: each operator or postfix wraps the chain so far (held by the writer until it ends).
        "x = a + b * c - d.e.f(g)[h] + i - j ** k ** l + (m + n) * o.p\n",
        "x = a /* c */ + b // d\n  + c.d /* e */ .f\n  (g)\n",
        "x = a.b(c + d.e(f * g + h)[i - j].k + l).m\n",
        "x = new a.b[c + d].e(f + g.h) + new i.j\n",
        // `await (…)` and a postfix: JavaScript calls a name `await`; TypeScript awaits.
        "async function f() { await (a)(); await (a).b; await (a)[0]; await (a)?.b; await (a)`t`; await (a)++; await (a)\n++b; await (a); f()++ }\n",
        // `let` before a line break and a name or `{` is a name's statement (the scanner's `;`);
        // before `[`, a declaration.
        "let\nx\nif (1) let\n{}\nL: let // c\n{}\nlet\n[a] = b;\n",
    ];
    // Arguments no expression reads: a call of `await` in JavaScript, errors in TypeScript.
    // U+0085 is no blank: a name's character.
    let common = [
        common.as_slice(),
        &["x =\u{85}1\ny = a\u{85}b\nz = 1;\u{85}\n"],
    ]
    .concat();
    let js = [
        "async function f() { await (...a); await (); await (a, b)() }\n",
        // A hashbang ends at `\n` only.
        "#!/usr/bin/env node\r\nx = 1\n",
        "#! a\r{\n",
        // `yield` as an operand is a `yield_expression`, as a pattern's name an identifier
        // holding its token; a catch clause's `undefined` is a name.
        "x = a * yield;\nvar {a: yield} = b;\n[yield] = c;\nvar [d, ...yield] = e;\ntry {} catch (undefined) {}\n",
    ];
    let jsx = [
        "const e = <A.B c=\"d\" {...e} f={1}>text &amp; {g}<h:i /><></></A.B>;\n",
        // A tag's comments are in it, before its `>` or `/>`.
        "x = <A b={c} /* d */ />\ny = <A b={c} // d\n />\nz = < /*a*/ ></ /*b*/>\nw = <A /* d */></A /* e */>\n",
    ];
    let ts = [
        "let x: A | B & C[] = y as const;\nlet z = <T>w;\n",
        "interface I<T extends U = V> extends J { a?: number; readonly b: string; (x: T): void; new (): I<T>; [k: string]: any }\n",
        "type T<K> = { [P in keyof K]-?: K[P] } & (() => void) | typeof import('./a').B;\n",
        "abstract class A<T> extends B<T> implements C { private readonly x!: T; abstract m(): void; constructor(public y?: number) { super() } }\n",
        "enum E { A = 1, B, }\nconst enum F {}\ndeclare module 'm' { export function f(): void; }\nnamespace N.M { }\n",
        "function f<T>(this: Window, a?: T, ...b: T[]): a is T;\nfunction f() {}\nexport = f;\n",
        "const q = a + b as T + c.d!.e<F>(g) satisfies H;\nconst r = a * b as U | V;\n",
        "async function f() { await (a)!; await (a)<T>(); await (a as B).c() }\n",
        "const g = f<string>(x)!.y satisfies Z;\nimport q = require('q');\nimport type { R } from 'r' with { type: 'json' };\n",
        // `x as A.B.C` is `(x as A.B).C` but where the `as` starts a parenthesized expression's,
        // an array's or an object's first element.
        "f(a as B.C.D)\nx = (b, a as B.C.D)\ny = [b, a as B.C.D]\nz = { j: 1, k: f(a as B.C.D) }\n",
        "(a as B.C.D)\nx = [...a as B.C.D]\ny = { k: a as B.C.D }\nz = (c + a as B.C.D).e\n",
        // A tagged template with type arguments.
        "x = f<T>`t`\ny = a.f<{ a: string }>`t ${b}`\n",
        // A modifier at its line's end names the member, but `static` before a name and `get`.
        "class A {\n  public\n  m() {}\n  static\n  *g() {}\n  readonly\n  r: string\n}\n",
        "class B {\n  static\n  m() {}\n  get\n  n() {}\n  static\n  x = 1\n}\n",
        // Modifiers in the grammar's order.
        "abstract class A {\n  private static override readonly async m() {}\n  protected abstract override n(): void;\n  declare public static override readonly p: number;\n  private readonly abstract q: string;\n  accessor r = 1\n  constructor(public override readonly s: T) { super() }\n}\n",
        // A `(` or `<` on a new line starts an object type's call signature.
        "interface I {\n  a\n  (): T\n  b?\n  <U>(): U\n}\n",
        // A reserved identifier before `in` is a computed name's expression, not a mapped type's.
        "type M = { [type in K]: B; [string in K]?: C };\nlet b: bigint.A;\nfunction f(a?: number = 1) {}\n",
        // `as A.B.C` splits its last name off before `[k]`, `|`, `&` and after `keyof`, not
        // before type arguments or `[]`; `typeof a[k]` is a lookup over the query, `typeof f<T>`
        // an instantiation's query; `new a!.B()`.
        "x = a as A.B.C['l'];\ny = a as keyof A.B.C;\nz = a as A.B.C | undefined;\nw = a as A.B.C[];\nv = a as A.B.C<T>;\nlet t: typeof a[0];\nlet u: typeof p<'a'>;\nq = g<typeof a[0]>(b);\nr = new m!.A();\n",
        // `f<T>` then a line break and an operand: comparisons; `interface` takes its name across
        // a line break; `namespace N` before a line break and `{` has no body; an accessor's
        // `get` across a line break; a class body's `}` after a `,`.
        "let x = f<true>\ny\ninterface\nF {}\nnamespace M\n{}\ninterface I {\n  get\n  foo(): string;\n}\ndeclare class C {\n  m(): T,\n}\n",
    ];
    let tsx = [
        "const C = <T,>(p: T) => <Table<T> rows={p} />;\n",
        // TSX reads `this` in a tag's name as an identifier.
        "x = <this.a></this.a>\ny = <this.a.b />\n",
    ];
    let cases: [(Dialect, Vec<&str>); 3] = [
        (
            Dialect::JavaScript,
            common.iter().chain(&js).chain(&jsx).copied().collect(),
        ),
        (
            Dialect::TypeScript,
            common.iter().chain(&ts).copied().collect(),
        ),
        // `<T>w` is an element's start in TSX, not a type assertion.
        (
            Dialect::Tsx,
            common
                .iter()
                .chain(&ts[1..])
                .chain(&jsx)
                .chain(&tsx)
                .copied()
                .collect(),
        ),
    ];
    for (dialect, sources) in cases {
        let gd = match dialect {
            Dialect::JavaScript => GrammarDialect::JavaScript,
            Dialect::TypeScript => GrammarDialect::TypeScript,
            Dialect::Tsx => GrammarDialect::Tsx,
        };
        let grammar = Grammar::new(gd).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&gd.get_ts_language()).unwrap();
        for source in sources {
            let ours = ecma_lean::encode(dialect, &grammar, source, &|| false)
                .unwrap_or_else(|why| panic!("{dialect:?} refused {source:?}: {why:?}"))
                .unwrap();
            let tree = parser.parse(source, None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "{dialect:?}: tree-sitter's tree of {source:?} has an error"
            );
            let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
                .unwrap()
                .primary;
            assert!(
                ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
                "{dialect:?}: the tapes of {source:?} differ:\n{}\n{}",
                python_tape_plain(&ours, &grammar),
                python_tape_plain(&theirs, &grammar)
            );
        }
        // Long chains: linear in the writer (`Writer::open_chain`), and tree-sitter's tape.
        let terms = |sep: &str| {
            (0..3000)
                .map(|i| format!("a{i}"))
                .collect::<Vec<_>>()
                .join(sep)
        };
        for source in [
            format!("x = {};\n", terms(" + ")),
            format!("x = {};\n", terms(".")),
            format!("x = {} * y.z(w);\n", terms(" - b.c + ")),
        ] {
            let ours = ecma_lean::encode(dialect, &grammar, &source, &|| false)
                .unwrap_or_else(|why| panic!("{dialect:?} refused a chain: {why:?}"))
                .unwrap();
            let tree = parser.parse(&source, None).unwrap();
            let theirs = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
                .unwrap()
                .primary;
            assert!(
                same_tape(&ours, &theirs),
                "{dialect:?}: the tapes of a chain of {} bytes differ",
                source.len()
            );
        }
        // Nested parentheses in parameter defaults, as minifiers write (`E = (E = (E = …`): each `(`
        // is tried as an arrow function once, not again for each enclosing one (once 2^40 tries).
        for source in [
            format!("x = {}1{};\n", "(a = ".repeat(40), ")".repeat(40)),
            format!("x = {}1{};\n", "async(a = ".repeat(40), ")".repeat(40)),
        ] {
            let ours = ecma_lean::encode(dialect, &grammar, &source, &|| false)
                .unwrap_or_else(|why| panic!("{dialect:?} refused {source:?}: {why:?}"))
                .unwrap();
            let tree = parser.parse(&source, None).unwrap();
            let theirs = TapeImage::encode(tree.root_node(), &grammar, &source, &|| false)
                .unwrap()
                .primary;
            assert!(
                ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
                "{dialect:?}: the tapes of {source:?} differ"
            );
        }
        // Past what tree-sitter's tape is compared at: the lean parser's own, not refused (no chain limit).
        for source in [
            format!("x = {}a;\n", "a + ".repeat(200_000)),
            format!("x = a{};\n", ".b(c)".repeat(100_000)),
        ] {
            assert!(
                ecma_lean::encode(dialect, &grammar, &source, &|| false).is_ok_and(|t| t.is_ok()),
                "{dialect:?} refused a chain of {} bytes",
                source.len()
            );
        }
        // Any syntax error: refused, for tree-sitter, which recovers as it recovers. The lean parser
        // has one behavior: tree-sitter's tape, or a refusal.
        let mut invalid = vec![
            "f(a b)\n",
            "x = (1,\n",
            "if (a {\n",
            "const s = 'open\n",
            "class A { m( }\n",
            "let x = [1, 2",
            "import {b} from",
        ];
        // A NUL anywhere: tree-sitter reads none, in a token or between them.
        invalid.extend(["x = 'a\0b'\n", "x = 1 // a\0b\n", "x = 1;\0\n"]);
        if dialect == Dialect::JavaScript {
            // tree-sitter-javascript's reserved words are no names; `yield` is no arrow's parameter
            // and takes no `/` after it.
            invalid.extend([
                "var var = 1;\n",
                "import { default } from 'a'\n",
                "x = yield => 1\n",
                "function* g() { yield / 2 }\n",
            ]);
        }
        if dialect != Dialect::JavaScript {
            invalid.extend([
                // Modifiers out of the grammar's order, or twice.
                "class A { public public a; }\n",
                "class A { static public m() {} }\n",
                "class A { override static m() {} }\n",
                "abstract class A { abstract async m(); }\n",
                "class A { static declare p: T; }\n",
                "class A { declare m() {} }\n",
                "class A { constructor(readonly public a: T) {} }\n",
                "class A { a?!: T }\n",
                // A `(` or `<` on a new line after a class member's name.
                "class A {\n  a\n  () {}\n}\n",
                "class A {\n  a\n  <T>() {}\n}\n",
                // Words tree-sitter-typescript reads as their expressions' or operators'.
                "function f(null: T) {}\n",
                "x = (class) => 1\n",
                "x = async (await) => 1\n",
                "let as = 1;\n",
                "let satisfies = 1;\n",
                // No member of a predefined type, a literal's or `this`; `asserts` takes its name.
                "let v: void.x;\n",
                "let v: this.x;\n",
                "function f(v: unknown): asserts {}\n",
                "type M = { [type in keyof A]: B };\n",
                // A pattern a function type's `(` reads as an object type.
                "type F = ({ a: string }: O) => any;\n",
                "type F = ([{ a: b }]: O) => any;\n",
                // A default after `?` with no type; a rest parameter's default.
                "function f(a? = 1) {}\n",
                "class A { m(...a = 0) {} }\n",
                "export import a = require('a');\n",
                // `type.a<T>`, `get` then a property's name on the next line.
                "let t: [type.cast<unknown>, '?'];\n",
                "interface I {\n  get\n  foo: string;\n}\n",
            ]);
        }
        for invalid in invalid {
            assert!(
                parser.parse(invalid, None).unwrap().root_node().has_error(),
                "{dialect:?}: {invalid:?} reads without an error"
            );
            assert!(
                ecma_lean::encode(dialect, &grammar, invalid, &|| false).is_err(),
                "{dialect:?} accepted {invalid:?}"
            );
        }
        // Valid, but tree-sitter reads `export` before a line break as a name's statement, and
        // JavaScript's `await [a]` and a postfix, and `for await (let [x] of …)`, as subscripts of
        // names: refused, not read as what they are.
        let mut odd = vec![
            "export\nconst a = 1;\n",
            "async function f() { for await (let [x] of y) {} }\n",
        ];
        if dialect != Dialect::JavaScript {
            // Readings tree-sitter-typescript decides past these rules: `as A.B.C | D.E`, a
            // trailing dotted name in an `as` type, `f<() => { a: T }>(c)` (comparisons), type
            // arguments with a type not read, `<T, U>(a) => {}` then `<`, `namespace as {}`,
            // `new` before a line break, `/ /* c */a /`.
            odd.extend([
                "x = a as A.B.C | D.E.F;\n",
                "x = a as () => A.B.C;\n",
                "x = f<() => { a: T }>(c);\n",
                "x = f<typeof a['k'].c>(b);\n",
                "<T, U>(a: T) => {}\n<V>(b: V) => {}\n",
                "namespace as {}\n",
                "const u = new\n  URL(a);\n",
                "var r = / /**/a /;\n",
                // `using` as a parameter's name: a name since #735's grammar (Patch 30), which the
                // lean parser refuses, as every `using`.
                "f = (a, using?: string) => 1;\n",
            ]);
        }
        if dialect == Dialect::JavaScript {
            odd.extend([
                "async function f() { x = await [1].y; }\n",
                "f(await [1]?.[0]);\n",
            ]);
        }
        for odd in odd {
            assert!(
                !parser.parse(odd, None).unwrap().root_node().has_error(),
                "{dialect:?}: {odd:?} reads with an error"
            );
            assert!(
                ecma_lean::encode(dialect, &grammar, odd, &|| false).is_err(),
                "{dialect:?} read {odd:?}"
            );
        }
    }
}

/// The product reads JavaScript, TypeScript and TSX with the lean parser: a file's image through the
/// engine's parse (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter.
#[test]
fn the_product_reads_ecma_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::ecma_lean;
    use rerail_ecma_lean::Dialect;
    for (dialect, gd, source) in [
        (
            Dialect::JavaScript,
            GrammarDialect::JavaScript,
            "function f(x) {\n  return x + 1 // one\n}\n",
        ),
        (
            Dialect::TypeScript,
            GrammarDialect::TypeScript,
            "function f(x: number): number {\n  return x + 1 // one\n}\n",
        ),
        (
            Dialect::Tsx,
            GrammarDialect::Tsx,
            "const f = (x: number) => <b>{x}</b>;\n",
        ),
    ] {
        let grammar = Grammar::new(gd).unwrap();
        assert!(tree_parser(gd).is_some(), "{gd:?}'s parser is installed");
        let product = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        let lean = ecma_lean::encode(dialect, &grammar, source, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(
            product.primary.to_bytes().unwrap(),
            lean.to_bytes().unwrap(),
            "{gd:?}"
        );
        // Refused (a syntax error): tree-sitter's tape, with its error.
        let invalid = "function f( {\n";
        assert!(ecma_lean::encode(dialect, &grammar, invalid, &|| false).is_err());
        assert!(
            TapeImage::parse(&grammar, invalid, &|| false)
                .unwrap()
                .is_ok()
        );
    }
}

/// `using` as a name (#735's grammars, Patch 18 and Patch 30) reads without a syntax error on the product's path:
/// the lean parser refuses every `using`, and tree-sitter, which once read it as a declaration's keyword and left
/// every TypeScript clause of such a file source-incomplete (src/cli/rule-draft.ts), reads it as a name.
#[test]
fn using_as_a_name_reads_without_an_error_on_the_products_path() {
    use crate::code_rule_dsl::tape::ecma_lean;
    use rerail_ecma_lean::Dialect;
    let sources = [
        "const using = [1]; using.filter(x => x)\n",
        "function f(using: number[]) { return using.length; }\n",
    ];
    for (dialect, gd) in [
        (Dialect::JavaScript, GrammarDialect::JavaScript),
        (Dialect::TypeScript, GrammarDialect::TypeScript),
        (Dialect::Tsx, GrammarDialect::Tsx),
    ] {
        let grammar = Grammar::new(gd).unwrap();
        for source in sources {
            // A parameter's type is TypeScript's: JavaScript reads the first only.
            if dialect == Dialect::JavaScript && source.contains(": number") {
                continue;
            }
            let product = TapeImage::parse(&grammar, source, &|| false)
                .expect("read to its end")
                .unwrap_or_else(|e| panic!("{gd:?}: {source:?}: {e}"));
            let doc = TapeDoc::restore(
                Arc::new(source.to_owned().into()),
                Arc::clone(&grammar),
                &product.primary,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                doc.syntax_error_ranges(),
                Vec::<std::ops::Range<usize>>::new(),
                "{gd:?}: {source:?}"
            );
            // Accepted, the lean parser's tape is the product's.
            if let Ok(Ok(lean)) = ecma_lean::encode(dialect, &grammar, source, &|| false) {
                assert_eq!(
                    product.primary.to_bytes().unwrap(),
                    lean.to_bytes().unwrap(),
                    "{gd:?}: {source:?}"
                );
            }
        }
    }
}

/// What a JavaScript, TypeScript or TSX file's tape costs, by parser: the lean parser
/// (`tape::ecma_lean`, the product's) against tree-sitter, over `RERAIL_PY_FILES` (a list of paths)
/// in `RERAIL_PY_DIALECT` (`javascript`, the default; `typescript`; `tsx`): instructions retired,
/// cycles and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn ecma_parse_costs() {
    use crate::code_rule_dsl::tape::ecma_lean;
    use rerail_ecma_lean::Dialect;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let (dialect, gd) = match std::env::var("RERAIL_PY_DIALECT").as_deref() {
        Ok("typescript") => (Dialect::TypeScript, GrammarDialect::TypeScript),
        Ok("tsx") => (Dialect::Tsx, GrammarDialect::Tsx),
        _ => (Dialect::JavaScript, GrammarDialect::JavaScript),
    };
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| ecma_lean::encode(dialect, &grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            ecma_lean::encode(dialect, &grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_ecma_lean::tape(f.as_bytes(), dialect).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(ecma_lean::encode(dialect, &grammar, f, &|| false))
                        .is_ok(),
                )
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "{dialect:?}: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Rust's tapes from the lean parser (the product's parser for Rust, decision 0193) are
/// tree-sitter-rust's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-rust-lean`'s `oracle`.
#[test]
fn rust_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::rust_lean;
    let sources = [
        "",
        // A byte-order mark: passed, the root starting after it and the white space.
        "\u{feff}",
        "\u{feff}  #![a]\nfn f() {}\n",
        "// only a comment\n//! inner doc\n/// outer doc\nfn f() {}\n",
        "use a::{b as c, d::*, self};\nuse ::e;\npub(crate) mod m { pub(super) struct S; }\n",
        "#[derive(Debug)]\npub struct P<'a, T: Clone + 'a, const N: usize> where T: Default { pub x: &'a [T; N], y: Option<Box<dyn Fn(u8) -> u8>> }\n",
        "enum E { A, B(u8, String), C { x: i32 } = 3, }\nunion U { a: u32 }\n",
        "impl<T> ::std::fmt::Debug for X<T> { fn fmt(&self, f: &mut Formatter<'_>) -> Result { Ok(()) } }\nimpl ! {}\n",
        "trait T: Sized { type A: Iterator<Item = u8>; const C: u8 = 1; fn m(&mut self, _: u8) -> impl '_ + Iterator; }\n",
        "fn f(x: u8) -> u8 {\n    let mut y = x as u16 + 1;\n    y <<= 2;\n    if a && let Some(z) = w { return z; }\n    match y { 0 | 1 => {} n if n > 2 => n, _ => unreachable!(), }\n}\n",
        "fn g() { for c in a..*b {} x(..=100); loop { break (a, b); } let v: Vec<Vec<u8>> = vec![]; }\n",
        "fn h() { let s = r#\" raw \"#; let c = 'c'; let b = b\"bytes\"; let l = |a, b: u8| -> u8 { a + b }; a.b::<T>().await?; }\n",
        "macro_rules! m { ($x:expr, $($y:tt)*) => { $x + m!($($y)*) }; () => {} }\nmacro_rules! set( { $($k:expr),* } => {}; );\n",
        "unsafe extern \"C\" fn v(n: i32, mut args: ...) {}\ntype F = for<'a> fn(&'a str, _) -> &'a str;\nstatic S: Option<&str> = None;\n",
        "#!/usr/bin/env run\nfn f<'r#a>(x: &'r#a u8) -> u8 { return { 1 }; }\n",
        "pub(in crate::a) fn f() {}\npub(self) fn g() {}\nextern \"C\" { fn v(a: i32, _: ...); }\n",
        "fn f() { let c = |x| _; match x { | _ => 1 } let b = a as u8 < c; let r = 0..; let x @ 1 = 2; }\n",
        "fn f() where T: A, &'a T: B, (u8,): C, [u8; 1]: D { let g = a::<T>::b(); let d = ::a::b; }\n",
        "impl X { const default fn f() {} default const unsafe fn g() {} }\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Rust).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Rust.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = rust_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers. The lean parser has
    // one behavior: tree-sitter's tape, or a refusal.
    for invalid in [
        "fn f(a b) {}\n",
        "fn f() { let x = (1,\n",
        "struct S {\n",
        "fn f() { 'open }\n",
        "pub impl X {}\n",
        "fn f(_: fn(&_)) {}\n",
        "fn f() { x.1e0; }\n",
        "fn f() where (): A {}\n",
        "type T = Box<dyn 'a + A>;\n",
        "fn f() { str![[a]]; }\n",
        "default impl T for F {}\n",
        "struct S where A: B;\n",
        "fn f() {\u{2028}}\n",
        "fn f() { let c = |a, | a; }\n",
        "fn f() { a as T < b; }\n",
        "fn f() { u8::<T>(); }\n",
        "type T = A::<B>;\n",
        "fn f() { let u8 @ 1 = 2; }\n",
        "type T = self;\n",
        "type T = A<u8 = B>;\n",
        "impl X { const default: u8 = 1; }\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            rust_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads Rust with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-rust.
#[test]
fn the_product_reads_rust_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::rust_lean;
    let grammar = Grammar::new(GrammarDialect::Rust).unwrap();
    assert!(
        tree_parser(GrammarDialect::Rust).is_some(),
        "Rust's parser is installed"
    );
    let source = "fn f(x: u8) -> u8 {\n    x + 1 // one\n}\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = rust_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    // Refused (a syntax error): tree-sitter's tape, with its error.
    let invalid = "fn f( {\n";
    assert!(rust_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Rust file's tape costs, by parser: the lean parser (`tape::rust_lean`, the product's)
/// against tree-sitter-rust, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn rust_parse_costs() {
    use crate::code_rule_dsl::tape::rust_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Rust;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| rust_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            rust_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_rust_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(rust_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Rust: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Java's tapes from the lean parser (the product's parser for Java, decision 0195) are
/// tree-sitter-java's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-java-lean`'s `oracle`.
#[test]
fn java_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::java_lean;
    let sources = [
        "",
        "// only a comment\n/** doc */\nclass A {}\n",
        "package a.b;\nimport java.util.*;\nimport static a.B.c;\n@A(x = 1, y = {2, 3}) public final class C<T extends D & E> extends F<T> implements G {}\n",
        "class A { private int x = 1, y[] = {1, 2,}; A() { this(1); } <T> T f(T t) throws X { return t; } }\n",
        "enum E { A, B(1), C { void f() {} }, ; void g() {} }\nrecord R(int a, String... b) implements I { R { } }\n@interface N { int x() default 1; }\n",
        "class A { void f() { int[] a = new int[3]; var l = new ArrayList<String>(); x = (int) y; r = (a, b) -> a + b; m = List<String>::size; c = A.class; } }\n",
        "class A { void f() { if (o instanceof String s && !s.isEmpty()) {} else ; for (int i = 0; i < n; i++) {} for (var e : m) {} l: while (x) break l; } }\n",
        "class A { int f() { return switch (x) { case 1, 2 -> 3; case P(int a, _) when a > 0 -> { yield a; } default -> throw new E(); }; } }\n",
        "class A { void f() { switch (x) { case 1: case 2: f(); break; default: g(); } try (var r = open()) {} catch (E | F e) {} finally {} } }\n",
        "class A { String s = \"a\\n\\u0041\" + \"\" + \"\"\"\n  block \\\"\"\"\"; char c = '\\''; long n = 0x1FL + 017 + 0b1 + 1_000; double d = .5e-3 + 0x1.8p1; }\n",
        "module m.n { requires transitive a.b; exports c to d, e; opens f; uses g.H; provides i.J with k.L, m.N; }\n",
        "sealed interface S permits A {} non-sealed class A implements S { void f(A this) { a = b & c += 4; } }\n",
        "\n",
        "\u{feff}class A { void f(int i, A this) {} void g(A A.this) {} }\n",
        "x = (byte) i++; y = (T)(Q) v++; z = (short)+ v--; w = (A & B) + c; v = (A & B) (c); u = -(byte) v++;\n",
        "x = (a.b) + c; y = d + (a.b) + c; z = d * (a.b) + c; w = (a) + b + c; v = -(a.b) + c;\n",
        "String @A [] @B [] x;\nObject c = new @J String [1] @B(5) [];\nvoid f(String [] @B [] a) {}\n",
        "s = \"\"\"\n  /* \"c\" */ x\n  // \"d\"\n  /* e */ y\n\"\"\";\n",
        "switch (x) { case A -> r -> r.m; case B, C -> (r) -> s -> t; case D -> r -> { }; };\n",
        "x = switch (t) { case A -> r -> { f(); }; default -> 1; };\nclass _ {}\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Java).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Java.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = java_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers. The lean parser has
    // one behavior: tree-sitter's tape, or a refusal.
    for invalid in [
        "class A { void f( }\n",
        "class A { int x = (1,\n",
        "class A {\n",
        "class A { void f() { a(b,); } }\n",
        "class A { char c = 'ab; }\n",
        "class A { void f() { x = 1 }\n",
        "class A {\u{3000}}\n",
        "class A { // a\0b\n}\n",
        "class A { String s = \"a\0b\"; }\n",
        "class A { void f(int i, A A.this) {} }\n",
        "class A { void _() {} }\n",
        "x = (A & B) ~c;\n",
        "switch (x) { default -> a; }.f();\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            java_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads Java with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-java.
#[test]
fn the_product_reads_java_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::java_lean;
    let grammar = Grammar::new(GrammarDialect::Java).unwrap();
    assert!(
        tree_parser(GrammarDialect::Java).is_some(),
        "Java's parser is installed"
    );
    let source = "class A {\n    int f(int x) { return x + 1; } // one\n}\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = java_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    // Refused (a syntax error): tree-sitter's tape, with its error.
    let invalid = "class A { void f( {\n";
    assert!(java_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Java file's tape costs, by parser: the lean parser (`tape::java_lean`, the product's)
/// against tree-sitter-java, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn java_parse_costs() {
    use crate::code_rule_dsl::tape::java_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Java;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| java_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            java_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_java_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(java_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Java: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Go's tapes from the lean parser (the product's parser for Go, decision 0314) are tree-sitter-go's,
/// byte for byte, on hand-written cases; a text tree-sitter reads with an error the lean parser
/// refuses (its file goes to tree-sitter). The corpus comparison is `rerail-go-lean`'s `oracle`.
#[test]
fn go_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::go_lean;
    let sources = [
        "",
        "// only a comment\n/* block */\npackage a\n",
        "package main\n\nimport (\n\t\"fmt\"\n\t. \"strings\"\n\t_ \"embed\"\n\tm \"math\"\n)\n\nimport \"os\"\n",
        "package a\n\nconst (\n\tA = iota\n\tB\n\tC, D = 1, 2\n)\n\nvar x, y int = 1, 2\nvar (\n\tz = []int{1, 2,}\n\tw map[string]chan<- *T\n)\n\ntype T struct {\n\tA, B int `json:\"a\"`\n\t*E\n\tpkg.F\n}\n\ntype I interface {\n\tM(a int) (b string, err error)\n\t~int | ~string\n}\n\ntype U = T\n",
        "package a\n\nfunc F[T any, U comparable](x T, ys ...U) (r T) {\n\treturn x\n}\n\nfunc (r *R[T]) M() {}\n",
        "package a\n\nfunc f() {\n\tx := 1\n\tx += 2\n\tx++\n\tc <- x\n\tgo g()\n\tdefer h()\n\tif x := f(); x > 0 {\n\t} else if y {\n\t} else {\n\t}\n\tfor i := 0; i < n; i++ {\n\t}\n\tfor k, v := range m {\n\t}\n\tfor range c {\n\t}\nL:\n\tfor {\n\t\tbreak L\n\t}\n\tgoto L\n}\n",
        "package a\n\nfunc f() {\n\tswitch x := v.(type) {\n\tcase int, nil:\n\tdefault:\n\t}\n\tswitch {\n\tcase a > b:\n\t\tfallthrough\n\tcase c:\n\t}\n\tselect {\n\tcase v := <-c:\n\tcase c <- 1:\n\tdefault:\n\t}\n}\n",
        "package a\n\nvar s = []string{\"a\\n\\x41\\u00e9\", `raw\n`, \"\"}\nvar r = 'x' + '\\''\nvar n = 0x1F + 0o17 + 0b1 + 1_000 + .5e-3 + 1i + 0x1p-2\n",
        "package a\n\nvar (\n\ta = F[int](x)\n\tb = pkg.F[T](x)\n\tc = f[i](x, y)\n\td = G[*pkg.T](x, y)\n\te = (*T)(p)\n\tf = (*S[A])(p)\n\tg = *(*[]T)(p)\n\th = T{A: 1, B: {2}}\n\ti = func(x int) int { return x }(1)\n\tj = interface{}(x)\n\tk = new(T)\n\tl = make([]int, 0, n)\n\tm = a[1:2:3]\n\tn = x.(T)\n)\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Go).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Go.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = go_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for invalid in [
        "package a\nfunc f( {\n",
        "package a\nvar x = (1,\n",
        "package a\nfunc f() {\n",
        "package a\nvar s = \"ab\n",
        "package a\nfunc f() { x := }\n",
        "package a\nvar x = 1 +\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            go_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads Go with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-go.
#[test]
fn the_product_reads_go_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::go_lean;
    let grammar = Grammar::new(GrammarDialect::Go).unwrap();
    assert!(
        tree_parser(GrammarDialect::Go).is_some(),
        "Go's parser is installed"
    );
    let source = "package a\n\nfunc f(x int) int { return x + 1 } // one\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = go_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let invalid = "package a\nfunc f( {\n";
    assert!(go_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Go file's tape costs, by parser: the lean parser (`tape::go_lean`, the product's)
/// against tree-sitter-go, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn go_parse_costs() {
    use crate::code_rule_dsl::tape::go_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Go;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| go_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            go_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_go_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(go_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Go: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// What a C or C++ file's tape costs, by parser: the lean parser (`tape::c_lean`, the product's)
/// against tree-sitter-c or tree-sitter-cpp (`RERAIL_C_DIALECT=c`, else C++), over
/// `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles and CPU time a node, the best
/// of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn cpp_parse_costs() {
    use crate::code_rule_dsl::tape::c_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let c = std::env::var("RERAIL_C_DIALECT").as_deref() == Ok("c");
    let gd = if c {
        GrammarDialect::C
    } else {
        GrammarDialect::Cpp
    };
    let dialect = if c {
        rerail_c_lean::Dialect::C
    } else {
        rerail_c_lean::Dialect::Cpp
    };
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| c_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            c_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_c_lean::tape(f.as_bytes(), dialect).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(c_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "{gd:?}: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// C's and C++'s tapes from the lean parser (the product's parser for both, decision 0317) are
/// tree-sitter-c's and tree-sitter-cpp's, byte for byte, on hand-written cases; a text tree-sitter
/// reads with an error the lean parser refuses (its file goes to tree-sitter). The corpus
/// comparison is `rerail-c-lean`'s `oracle`.
fn c_lean_matches_tree_sitter(dialect: GrammarDialect, sources: &[&str], invalid: &[&str]) {
    use crate::code_rule_dsl::tape::c_lean;
    let grammar = Grammar::new(dialect).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&dialect.get_ts_language()).unwrap();
    for &source in sources {
        let ours = c_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for &bad in invalid {
        assert!(
            parser.parse(bad, None).unwrap().root_node().has_error(),
            "{bad:?} reads without an error"
        );
        assert!(
            c_lean::encode(&grammar, bad, &|| false).is_err(),
            "accepted {bad:?}"
        );
    }
}

#[test]
fn c_tapes_from_the_lean_parser_are_tree_sitters() {
    let sources = [
        "",
        "// only a comment\n/* block */\n",
        "#include <stdio.h>\n#include \"a.h\"\n#define N 10\n#define F(x, y) ((x) + (y))\n#ifdef A\nint a;\n#elif B\nint b;\n#else\nint c;\n#endif\n",
        "struct S { int a, *b; unsigned c : 3; struct { char d; } e; };\ntypedef struct S S;\nenum E { A = 1, B, };\nunion U { int i; float f; };\n",
        "static const char *s[] = { \"a\" \"b\", [2] = \"c\" };\nint (*fp)(int, char *);\nlong long x = 0x1Fu + 1e-3f + 'a' + L'\\n';\n",
        "int f(int a, ...) {\n  for (int i = 0; i < a; i++) {\n    if (a > 1) continue; else break;\n  }\n  while (a--) a += 2;\n  do { a = a ? a : -a; } while (0);\n  switch (a) { case 1: return 1; default: break; }\n  goto end;\nend:\n  return sizeof(int) + sizeof a + (int)a;\n}\n",
        "void g(void) { struct S s = { .a = 1 }; s.b->c[1] = &s; int *p = (int *)malloc(4); f(p, *p); }\n",
    ];
    let invalid = [
        "int f( {\n",
        "int x = (1,\n",
        "void f() {\n",
        "char *s = \"ab\n",
        "int x = 1 +;\n",
    ];
    c_lean_matches_tree_sitter(GrammarDialect::C, &sources, &invalid);
}

#[test]
fn cpp_tapes_from_the_lean_parser_are_tree_sitters() {
    let sources = [
        "",
        "#include <vector>\nnamespace a::b {\nusing std::vector;\nusing T = vector<int>;\ninline namespace v1 { int x; }\n}\n",
        "template <typename T, int N = 3>\nclass A : public B<T>, virtual C {\npublic:\n  A() : x_(0) {}\n  explicit A(int x) noexcept;\n  virtual ~A() = default;\n  T get() const override { return x_; }\n  static constexpr int n = N;\nprivate:\n  T x_;\n};\n",
        "int A::f(std::unique_ptr<B> p, const std::string& s) const {\n  auto l = [&, this](int y) -> int { return y + x_; };\n  for (auto& [k, v] : m) { std::cout << k << v; }\n  if constexpr (sizeof(T) > 4) { throw std::runtime_error(\"x\"); }\n  try { g(); } catch (const E& e) { } catch (...) { }\n  return static_cast<int>(l(1)) + new int[3]{1, 2, 3}[0];\n}\n",
        "enum class E : unsigned char { A, B };\nstruct S { int a{}; S& operator=(const S&) = delete; operator bool() const; };\nconstexpr auto s = R\"(raw\\n)\"sv;\n",
        "template <class T> concept C = requires(T t) { t.f(); };\ntemplate <C T> void h(T&& t) { std::forward<T>(t).f(); }\n",
    ];
    let invalid = [
        "int f( {\n",
        "class A {\n",
        "auto x = [&] -> int { return 1; };\n",
        "int x = (a = b, c);\n",
        "void f() { p->A::~A(); }\n",
    ];
    c_lean_matches_tree_sitter(GrammarDialect::Cpp, &sources, &invalid);
    // Where tree-sitter-cpp's GLR forks past its 6 stack versions in nested template arguments,
    // it reads some as expressions by version order (decision 0317): refused, for tree-sitter.
    use crate::code_rule_dsl::tape::c_lean;
    let grammar = Grammar::new(GrammarDialect::Cpp).unwrap();
    for capped in [
        "[[nodiscard]] std::pair<A, B<C>> f(int);\n",
        "void f() { x = *static_cast<A<B<C<double>>>*>(p); }\n",
    ] {
        assert!(
            c_lean::encode(&grammar, capped, &|| false).is_err(),
            "accepted {capped:?}"
        );
    }
}

/// The product reads C and C++ with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter.
#[test]
fn the_product_reads_c_and_cpp_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::c_lean;
    for (dialect, source) in [
        (GrammarDialect::C, "int f(int x) { return x + 1; } // one\n"),
        (
            GrammarDialect::Cpp,
            "namespace a { int f(int x) { return x + 1; } } // one\n",
        ),
    ] {
        let grammar = Grammar::new(dialect).unwrap();
        assert!(
            tree_parser(dialect).is_some(),
            "{dialect:?}'s parser is installed"
        );
        let product = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        let lean = c_lean::encode(&grammar, source, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(
            product.primary.to_bytes().unwrap(),
            lean.to_bytes().unwrap()
        );
        let invalid = "int f( {\n";
        assert!(c_lean::encode(&grammar, invalid, &|| false).is_err());
        assert!(
            TapeImage::parse(&grammar, invalid, &|| false)
                .unwrap()
                .is_ok()
        );
    }
}

/// C#'s tapes from the lean parser (the product's parser for C#, decision 0196) are
/// tree-sitter-c-sharp's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-csharp-lean`'s `oracle`.
#[test]
fn csharp_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::csharp_lean;
    let sources = [
        "",
        "// only a comment\n/// <summary>doc</summary>\nclass A {}\n",
        "using System;\nusing static System.Math;\nusing L = System.Collections.Generic.List<int>;\nnamespace N.M;\n[Serializable] public sealed partial class C<T> : B<T>, I where T : class, new() { }\n",
        "class A { private int x = 1, y; public int P { get; private set; } = 2; A() : base(1) { } T F<T>(T t) where T : struct => t; ~A() { } }\n",
        "enum E : byte { A, B = 2, }\nrecord R(int A, string B) : S(A);\nstruct S { public static S operator +(S a, S b) => a; public static implicit operator int(S s) => 0; }\ninterface I { void F(); int this[int i] { get; } event System.Action E; }\n",
        "class A { void F() { var a = new int[3]; var l = new List<string> { \"a\" }; x = (int)y; Func<int, int> f = x => x + 1; var t = (1, \"b\"); var o = new { A = 1 }; c = typeof(A); s = a?.b ?? c; } }\n",
        "class A { void F() { if (o is string s && s.Length > 0) { } else ; for (int i = 0; i < n; i++) { } foreach (var e in m) { } while (x) break; do { } while (y); using (var r = Open()) { } lock (o) { } } }\n",
        "class A { int F(object x) => x switch { 1 or 2 => 3, int i when i > 0 => i, { Length: 0 } => 0, [1, .., _] => 1, _ => throw new E() }; }\n",
        "class A { async Task F() { switch (x) { case 1: case 2: G(); break; default: return; } try { await H(); } catch (E e) when (e.X) { } finally { } yield return 1; } }\n",
        "class A { string s = \"a\\n\" + @\"b\"\"c\" + $\"x{y,3:F2}z{{\" + \"\"\"\n  raw\n  \"\"\"; char c = '\\''; long n = 0x1FL + 1_000UL; double d = .5e-3 + 1.5m; }\n",
        "#region R\n#if DEBUG\nclass A { }\n#elif X\nclass B { }\n#else\nclass C { }\n#endif\n#endregion\n#pragma warning disable CS1591\n",
        "unsafe class A { void F(int* p, delegate*<int, void> g) { int* q = &x; fixed (byte* b = a) { } Span<int> s = stackalloc int[2]; } }\n",
        // A parenthesized pattern holds a name chain or a pattern; a literal, an operator or a call
        // is a constant's parenthesized expression.
        "class A { void F() { a = x is not (\"a\"); b = x is (1) or (A); c = x is (A.B.M()); d = x is (a + 1); e = x is (1 or 2); f = x is (var v); g = x is ((1)); h = x is (this.P); } }\n",
        // `not` takes a whole pattern; `A.M()` and `global::A.B` after `is` are constants.
        "class A { void F() { a = x is not 1 and not 2; b = x is 1 or not 2 and 3; c = x is A.M(); d = x is global::A.B; } }\n",
        // A switch arm's `(…)` before `=>` is a pattern, not a lambda's parameters.
        "class A { int F() => x switch { (1) => 2, (\"a\") => 3, (A) => 4, _ => 0 }; }\n",
        // After `is` in a chain with a left operand, the constant stands alone where the chain's
        // last operand is a literal, `typeof` or another `is`, and takes the operators after it
        // where that operand is a name or an access; a relational pattern takes a conditional.
        "class A { void F() { a = p || b is null || c > 1; b = p || b is X || c; c = p && b is X && c == 1; d = b is X || c > 1; e = b is X ? 1 : 2; f = c is > 0 ? 1 : 2; g = p || b is X || c.d; h = p && b is X && c is Y; i = p || b is X || typeof(T); } }\n",
        // `await` in an argument; a tuple type's array; a type's `,`s after `is` in parentheses;
        // positional patterns (by their first element) and tuple constants; a list pattern's `,`
        // before a discard; `switch` over a tuple in parentheses; a namespace's `;`.
        // The left operand's leftmost operand decides too (a call's or `!a`'s lets the constant
        // take the operators), as do operators all tighter than the one before the `is`; a
        // relational pattern takes `and`'s conditional; keywords as a declaration's name after a
        // type in arguments and brackets; casts to sized arrays and of collections; a
        // parenthesized lvalue's class at a statement's start; `nameof(C)` in a case label.
        "class A { void F() { a = p && b is X == 1; b = f() && b is X && 2; c = !a && r is V && p is W; d = (n == 1) && p[0] is L && 2; e = r is > 0 and < 5 ? 1 : 2; f(a * null); g = x[a * b]; h = (T[])[1]; i = (a[0]) - b; (t).ToString(); switch (y) { case nameof(C): break; } using (await t) { } j = y is 4 && b is 5; } }\n",
        // `as` after an `is` with no operator after it wraps the `is`; with one, it is the
        // constant's. A pointer indirection takes no postfix operator (`*p[0]` is `(*p)[0]`); a
        // creation's later rank with sizes is an element access; `..` is right-associative.
        "unsafe class A { void F() { a = y is null as O; b = y is A as O; c = y is 1 as O ?? q; d = p && y is not null as O; e = *p[0].q; *p[0] = 1; f = **p[0]; g = new int[4][2][3]; h = new int[4][]; i = 1..3..5; j = a[..3..]; } }\n",
        // A bare `this` before `with` or `switch` stores the gap's `{`; `orderby`'s `,` after a
        // direction is stored; `ref readonly` and `scoped ref` in a lambda's parameter are
        // modifiers; an `#if` in a top-level `#if` is a statement.
        "class A { void F() { a = this with { P = 1 }; b = this switch { { } => 1 }; c = y with { }; d = from b in c orderby b.P descending, b.D select b; e = (ref readonly int p) => 0; f = (scoped ref int q) => 0; } }\n",
        "#if A\n#if B\nusing X;\nclass C { }\n#endif\n#endif\n",
        // A text of a BOM or blanks alone: the root starts at its end.
        "\u{feff}",
        "  \n",
        "namespace N { class A { void F() { f(await t); x = new (A a, B b)[3]; y = (c is D<K, V> d && d.E); if (x is (_, 1) or (1, _)) { } if (x is (A a, B b) z) { } if (x is [1, _] or [_, _, 3]) { } switch ((a, b)) { case (_, _) when c: break; } } } };\n",
    ];
    let grammar = Grammar::new(GrammarDialect::CSharp).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::CSharp.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = csharp_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers. The lean parser has
    // one behavior: tree-sitter's tape, or a refusal.
    for invalid in [
        "class A { void F( }\n",
        "class A { int x = (1,\n",
        "class A {\n",
        "class A { void F() { G(a,); } }\n",
        "class A { char c = 'ab; }\n",
        "class A { void F() { x = 1 } }\n",
        // tree-sitter-c-sharp's own failures on valid C#: `$""`, `with` and attribute targets
        // lexed as keywords, `from _`, a pragma's trailing `,`, a region's text of `#region`, a
        // lambda parameter's name alone after modifiers.
        "class A { string s = $\"\"; }\n",
        "class A { void F() { T with = new T(); } }\n",
        "class A { object o = new D { [type] = 0 }; }\n",
        "class A { object o = s?[field]; }\n",
        "class A { object o = from _ in a select 1; }\n",
        "#pragma warning restore CS0618,\nclass A { }\n",
        "#region #region\n#endregion\n",
        "class A { object o = (ref readonly p) => 0; }\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            csharp_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
    // Valid, but tree-sitter reads its type arguments as comparisons (`(new S < A) > ((c) { })`,
    // elements `new S < A` and `B > { }` between an initializer's `,`s, a declaration's type split
    // at its `,` in a tuple, `(x is A) < T > &&f`), a dotted constant as a member access over its
    // `is`, `.. a ?? []` as a range, `[a * b]` as an element binding, or reads a chain whose
    // grouping its parse versions decide past these rules (a comment in it): refused, not read as
    // what it is.
    for odd in [
        "class A { object o = new S<A>(c) { }; }\n",
        "class A { object o = new List<(A a, B b)>(q); }\n",
        "class A { object[] o = { new S<A, B> { } }; }\n",
        "class A { object o = new S<A>(new L<E> { e }); }\n",
        "class A { object o = (c is D<K, V> d, 1); }\n",
        "class A { bool b = p || x is A.B || 1; }\n",
        "class A { bool b = x is A.B ? 1 : 2; }\n",
        "class A { object o = (P)S.W<R>(o); }\n",
        "class A { object o = f<(A a, B b)>(c); }\n",
        "class A { bool b = x is A<T> && f == 1; }\n",
        "class A { bool b = r is var (a, b); }\n",
        "class A { object o = [.. a ?? []]; }\n",
        "class A { bool b = x is [.., var w] && w.K == k; }\n",
        "class A { object o = [a * b]; }\n",
        "class A { void F() { foreach (L<V<T>> b in c) { } } }\n",
        "class A { bool b = a && y is not null && // c\n 1; }\n",
        "class A { bool b = y is 4 && b is S.O; }\n",
        // `x is 1 + 2 as O` (`as` after a tighter operator), `x is A.B as O` (a member access
        // over the `is`), `(r = new X<A, B>(c))` (a tuple of comparisons).
        "class A { object o = y is 1 + 2 as O; }\n",
        "class A { object o = y is A.B as O; }\n",
        "class A { object o = (r = new X<A, B>(c)); }\n",
    ] {
        assert!(
            !parser.parse(odd, None).unwrap().root_node().has_error(),
            "{odd:?} reads with an error"
        );
        assert!(
            csharp_lean::encode(&grammar, odd, &|| false).is_err(),
            "read {odd:?}"
        );
    }
}

/// The product reads C# with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-c-sharp.
#[test]
fn the_product_reads_csharp_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::csharp_lean;
    let grammar = Grammar::new(GrammarDialect::CSharp).unwrap();
    assert!(
        tree_parser(GrammarDialect::CSharp).is_some(),
        "C#'s parser is installed"
    );
    let source = "class A {\n    int F(int x) { return x + 1; } // one\n}\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = csharp_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    // Refused (a syntax error): tree-sitter's tape, with its error.
    let invalid = "class A { void F( {\n";
    assert!(csharp_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// PHP's tapes from the lean parser (the product's parser for PHP, decision 0197) are
/// tree-sitter-php's, byte for byte, on hand-written cases (of each file's view, `parse_input`, as
/// the product parses it); a text tree-sitter reads with an error the lean parser refuses (its file
/// goes to tree-sitter). The corpus comparison is `rerail-php-lean`'s `oracle`.
#[test]
fn php_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::php_lean;
    let sources = [
        "",
        "<?php\n",
        "<?php\n// a comment\n# another\n/** doc */\nnamespace App\\Models;\nuse Foo\\{Bar, Baz as Q};\nuse function strlen;\n",
        "<?php\ndeclare(strict_types=1);\nfinal class A extends B implements C, D {\n    use T { f as protected g; }\n    public const int X = 1, Y = 2;\n    private static ?array $p = null;\n    public function __construct(private readonly string $s = 'x', int ...$rest) {}\n    abstract protected function f(int|string $a, &$b): static;\n}\n",
        "<?php\nenum Suit: string { case Hearts = 'H'; case Spades = 'S'; public function color(): string { return match ($this) { self::Hearts => 'Red', default => 'Black' }; } }\ninterface I { const C = 1; }\ntrait T { abstract public function t(): void; }\n",
        "<?php\n$a = [1, 'b' => 2, ...$c, &$d];\n[$x, [$y, $z]] = $a;\nlist('k' => $v, , $w) = $b;\n$f = fn($x) => $x * 2;\n$g = static function () use (&$a, $b): int { return $a <=> $b; };\n$o = new class(1) extends B { public $p; };\n",
        "<?php\n$s = 'it\\'s' . \"a $b {$c->d} ${e} $f[0] $g->h \\n\\x41\\u{1F600}\";\n$h = <<<EOT\n  Hello $name\n  {$x['y']}\n\n  EOT;\n$n = <<<'NOW'\nraw $text\nNOW;\n$c = `ls -l $dir`;\n",
        "<?php\nif ($a): echo 1; elseif ($b): echo 2; else: echo 3; endif;\nforeach ($xs as $k => [$p, $q]): endforeach;\nwhile (true) { break 2; }\nfor ($i = 0, $j = 1; $i < 10; $i++, --$j);\nswitch ($x) { case 1: case 2: goto end; default: continue; }\nend:\ntry { throw new E(); } catch (A | B $e) { } finally { }\n",
        "<?php\n$x = (int) $y + @f() ?? $z ?: -1 ** 2 instanceof Foo;\n$y = $a?->b?->c() ?? A::$b::C::d();\n$z = !$a = $b and $c or $d xor $e;\nprint clone $o;\nunset($a[1], $b->c);\nglobal $g;\nstatic $s = 1;\nexit(0);\n",
        "<?php\n#[Attr(1, name: 'x')]\nfunction f(#[SensitiveParameter] $p): never { throw new \\Exception(); }\n$r = f(...);\n$v = strlen(string: 'x');\n",
        "<html><?php echo $title; ?></html>\n",
        "<?php\necho 1 ?>\n",
        // A bare heredoc word's start takes the blanks after `<<<`; a nowdoc's first string starts
        // past the empty lines after its opening line.
        "<?php\n$a = <<< E\na\nE;\n$b = <<<'E'\n\n\n    a\n\nb\nE;\n",
        // An indented heredoc's comment that closes early, and a keyword as a method's or a
        // named argument's name where it starts no expression.
        "<?php\n$x = <<<EOF\n  q\n  /*\n  */\n  r\n  EOF;\nf(class: 1, echo: 2);\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Php).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Php.get_ts_language())
        .unwrap();
    let parse = |parser: &mut tree_sitter::Parser, source: &str| {
        parser
            .parse(
                crate::code_rule_dsl::language::parse_input(GrammarDialect::Php, source).as_ref(),
                None,
            )
            .unwrap()
    };
    for source in sources {
        let ours = php_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parse(&mut parser, source);
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers. The lean parser has
    // one behavior: tree-sitter's tape, or a refusal.
    for invalid in [
        "<?php\nfunction f( {\n",
        "<?php\n$a = [1,\n",
        "<?php\nclass A {\n",
        "<?php\n$a[$b->c\n\ncontinue;\n",
        "<?php\n$s = 'abc;\n",
        "<?php\nclass A { public const FALSE = 1; }\n",
        // A keyword that starts an expression as a named argument's name; `const NAMESPACE`; a
        // trailing `,` in a group `use`.
        "<?php\nf(include: 1);\n",
        "<?php\nf(new: 1);\n",
        "<?php\nclass A { const NAMESPACE = 1; }\n",
        "<?php\nuse A\\{B,};\n",
    ] {
        assert!(
            parse(&mut parser, invalid).root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            php_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
    // An indented heredoc whose `/*` (past its first line) closes on its eleventh line: tree-sitter
    // reads it with an error.
    let long = format!(
        "<?php\n$x = <<<EOF\n  q\n  /*\n{}  */\n  r\n  EOF;\n",
        "  \n".repeat(8)
    );
    assert!(
        parse(&mut parser, &long).root_node().has_error(),
        "{long:?} reads without an error"
    );
    assert!(
        php_lean::encode(&grammar, &long, &|| false).is_err(),
        "accepted {long:?}"
    );
}

/// The product reads PHP with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-php.
#[test]
fn the_product_reads_php_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::php_lean;
    let grammar = Grammar::new(GrammarDialect::Php).unwrap();
    assert!(
        tree_parser(GrammarDialect::Php).is_some(),
        "PHP's parser is installed"
    );
    let source = "<?php\nfunction f(int $x): int { return $x + 1; } // one\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = php_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    // Refused (a syntax error): tree-sitter's tape, with its error.
    let invalid = "<?php\nfunction f( {\n";
    assert!(php_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// HTML, CSS and Markdown files are product languages (`languages/html.json`, `css.json`,
/// `markdown.json`): the registry selects each by its suffix, and its file is read by its grammar
/// (tree-sitter-html, tree-sitter-css, tree-sitter-md's block grammar) into a tape that persists and
/// restores.
#[test]
fn html_css_and_markdown_files_are_read_by_their_grammars() {
    let cases = [
        (
            "page.html",
            GrammarDialect::Html,
            "<!DOCTYPE html>\n<p class=\"a\">Hi &amp; bye<br></p>\n",
            "document",
            "element",
        ),
        (
            "site.css",
            GrammarDialect::Css,
            "a > b { color: red; }\n@media print { p { margin: 0 } }\n",
            "stylesheet",
            "rule_set",
        ),
        (
            "README.md",
            GrammarDialect::Markdown,
            "# Title\n\nSome *text*.\n\n```rust\nfn f() {}\n```\n",
            "document",
            "fenced_code_block",
        ),
    ];
    for (path, dialect, source, root, inner) in cases {
        let label = crate::code_rule_dsl::language::label_of_path(path);
        assert_eq!(
            label.and_then(crate::code_rule_dsl::language::dialect_by_label),
            Some(dialect),
            "{path}"
        );
        let grammar = Grammar::new(dialect).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false)
            .unwrap()
            .unwrap()
            .round_trip(&grammar)
            .unwrap();
        let doc = TapeDoc::restore(
            Arc::new(source.to_owned().into()),
            Arc::clone(&grammar),
            &image,
            &|| false,
        )
        .unwrap();
        let tree = AstGrep::doc(doc);
        assert_eq!(tree.root().kind(), root, "{path}");
        assert!(
            tree.root().dfs().any(|node| node.kind() == inner),
            "{path}: no {inner}"
        );
        assert!(
            !tree.root().dfs().any(|node| node.is_error()),
            "{path}: an error"
        );
    }
}

/// A Blade template's compiled program (its `compiled-view` document) is read by the lean PHP
/// parser (decision 0198), and its derived image is the one tree-sitter-php's tree gives, byte for
/// byte, its placeholders (`_`, `(0)`) left out the same. `RERAIL_BLADE_FILES` (a list of templates)
/// adds a corpus.
fn blade_programs_agree(source: &str) -> Option<bool> {
    use crate::code_rule_dsl::embed::{Parse, Program};
    let tree = crate::code_rule_dsl::blade::forte::tree::parse_tree(source.as_bytes()).ok()?;
    let view = crate::code_rule_dsl::blade::view::View::read(&tree, source, &|| false)
        .ok()?
        .view?;
    let lean = view.read_by_the_lean_parser();
    let host = Grammar::new(GrammarDialect::Blade).unwrap();
    let program = view.into_program();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Php.get_ts_language())
        .unwrap();
    let theirs = Program {
        dialect: program.dialect,
        parse: Parse::Tree(parser.parse(&program.text, None).unwrap()),
        text: program.text.clone(),
        segments: program.segments.clone(),
        host_errors: program.host_errors.clone(),
        at: program.at.clone(),
    };
    let ours =
        DerivedImage::encode(Via::CompiledView, program, 0, &host, source, &|| false).unwrap();
    let theirs =
        DerivedImage::encode(Via::CompiledView, theirs, 0, &host, source, &|| false).unwrap();
    let php = Grammar::new(GrammarDialect::Php).unwrap();
    assert!(
        ours.tape.to_bytes().unwrap() == theirs.tape.to_bytes().unwrap(),
        "the programs of {source:?} differ:\n{}\n{}",
        python_tape_plain(&ours.tape, &php),
        python_tape_plain(&theirs.tape, &php)
    );
    Some(lean)
}

#[test]
fn blade_programs_from_the_lean_parser_are_tree_sitters() {
    let templates = [
        "{{ $a }}\n",
        "<p>{{ $user->name }} {!! $html !!}</p>\n@if ($a > 1)\n  <b>{{ $a }}</b>\n@elseif ($b)\n  x\n@else\n  y\n@endif\n",
        "@foreach ($items as $k => $item)\n  {{ $loop->index }}: {{ $item['name'] ?? 'none' }}\n@endforeach\n@forelse ($xs as $x) {{ $x }} @empty none @endforelse\n",
        "@php\n  $color = match ($level) { 'error' => 'red', default => 'blue' };\n@endphp\n<x-alert :type=\"$color\" />\n@isset($title) {{ $title }} @endisset\n",
        "@section('content')\n  @include('partials.nav', ['active' => 'home'])\n  {{ __('Hello :name', ['name' => $name]) }}\n@endsection\n@props(['size' => 'md'])\n",
    ];
    for template in templates {
        assert_eq!(
            blade_programs_agree(template),
            Some(true),
            "{template:?}: the lean parser reads its program"
        );
    }
    let Ok(list) = std::env::var("RERAIL_BLADE_FILES").map(|l| std::fs::read_to_string(l).unwrap())
    else {
        return;
    };
    let (mut lean, mut all) = (0, 0);
    for path in list.lines().filter(|l| !l.is_empty()) {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(read) = blade_programs_agree(&source) {
            all += 1;
            lean += usize::from(read);
        }
    }
    eprintln!("{lean} of {all} programs read by the lean parser, every one as tree-sitter-php's");
}

/// What a PHP file's tape costs, by parser: the lean parser (`tape::php_lean`, the product's)
/// against tree-sitter-php, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read. Both read each file's
/// view (`parse_input`, made outside the measure).
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn php_parse_costs() {
    use crate::code_rule_dsl::tape::php_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Php;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| php_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let views: Vec<String> = files
        .iter()
        .map(|f| crate::code_rule_dsl::language::parse_input(gd, f).into_owned())
        .collect();
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            php_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    // Each step by the file's index, not its text.
    type IndexStep<'a> = (&'static str, Box<dyn Fn(usize) -> usize + 'a>);
    let steps: Vec<IndexStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|i| rerail_php_lean::tape(views[i].as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|i| {
                usize::from(
                    std::hint::black_box(php_lean::encode(&grammar, &files[i], &|| false)).is_ok(),
                )
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|i| {
                parser
                    .borrow_mut()
                    .parse(&views[i], None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|i| {
                let tree = parser.borrow_mut().parse(&views[i], None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(
                        tree.root_node(),
                        &grammar,
                        &files[i],
                        &|| false,
                    ))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            (0..files.len()).for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "PHP: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// What a C# file's tape costs, by parser: the lean parser (`tape::csharp_lean`, the product's)
/// against tree-sitter-c-sharp, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn csharp_parse_costs() {
    use crate::code_rule_dsl::tape::csharp_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::CSharp;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| csharp_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            csharp_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_csharp_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(csharp_lean::encode(&grammar, f, &|| false)).is_ok(),
                )
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "C#: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// HTML's tapes from the lean parser (the product's parser for HTML, decision 0199) are
/// tree-sitter-html's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-html-lean`'s `oracle`.
#[test]
fn html_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::html_lean;
    let sources = [
        "",
        "  \n",
        "<!DOCTYPE html>\n<html lang=en>\n<head><meta charset=\"utf-8\"><title>T</title>\n<link rel=stylesheet href=a.css>\n</head>\n<body class='x y' hidden>\n<!-- a comment -->\n<p>One &amp; two &#169; &#x1F600;<br>three<p>four\n<ul><li>a<li>b</ul>\n<img src=\"a.png\" alt=\"\"/>\n<my-element data-x=1></my-element>\n</body>\n</html>\n",
        "<script>if (a < b && c > d) { x = '</div>'; }</script>\n<style>p > a { color: red }</style>\n<script type=\"module\"></script>\n",
        "<table><tr><td>a<td>b<tr><td>c</table>\n<dl><dt>t<dd>d<dt>u</dl>\n<select><optgroup label=a><option>1<option>2</optgroup></select>\n",
        "<div><span>a</div>\n<p>x</span></p>\n",
        "<html><body><p>html and body open at the end</p>\n",
        "text alone &lt; &gt;\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Html).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Html.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = html_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for invalid in [
        "<div>\n",
        "<p a=\"x>\n",
        "a > b\n",
        "<script>x\n",
        "<!DOCTYPE>\n",
        "<p>open at the end\n",
        "</div>\n",
        "<%include file=\"a\"/>\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            html_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
    // The scanner serializes its open tags into tree-sitter's 1024-byte buffer: 127 custom tags of
    // six letters fit (4 + 127 × 8 bytes), the 128th comes back nameless and its end tag is an
    // error. Read to the limit, refused past it.
    let nested = |k: usize| format!("{}{}", "<abcdef>\n".repeat(k), "</abcdef>\n".repeat(k));
    let fits = nested(127);
    let ours = html_lean::encode(&grammar, &fits, &|| false)
        .unwrap_or_else(|why| panic!("refused 127 tags: {why:?}"))
        .unwrap();
    let tree = parser.parse(&fits, None).unwrap();
    assert!(!tree.root_node().has_error(), "127 tags read with an error");
    assert!(
        ours.to_bytes().unwrap()
            == TapeImage::encode(tree.root_node(), &grammar, &fits, &|| false)
                .unwrap()
                .primary
                .to_bytes()
                .unwrap(),
        "the tapes of 127 tags differ"
    );
    let past = nested(128);
    assert!(
        parser.parse(&past, None).unwrap().root_node().has_error(),
        "128 tags read without an error"
    );
    assert!(
        html_lean::encode(&grammar, &past, &|| false).is_err(),
        "accepted 128 tags"
    );
}

/// The product reads HTML with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-html.
#[test]
fn the_product_reads_html_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::html_lean;
    let grammar = Grammar::new(GrammarDialect::Html).unwrap();
    assert!(
        tree_parser(GrammarDialect::Html).is_some(),
        "HTML's parser is installed"
    );
    let source = "<p class=a>Hi <b>there</b></p>\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = html_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let invalid = "<div>\n";
    assert!(html_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a HTML file's tape costs, by parser: the lean parser (`tape::html_lean`, the product's)
/// against tree-sitter-html, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn html_parse_costs() {
    use crate::code_rule_dsl::tape::html_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Html;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| html_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            html_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_html_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(html_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "HTML: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// CSS's tapes from the lean parser (the product's parser for CSS, decision 0200) are
/// tree-sitter-css's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-css-lean`'s `oracle`.
#[test]
fn css_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::css_lean;
    let sources = [
        "",
        "  \n",
        "/* a comment */\n@charset \"utf-8\";\n@import url(\"a.css\") screen;\n:root { --x: 1px; }\n",
        "a > b.c:hover, #x .y ~ z + w { color: red !important; margin: 0 auto; }\n",
        "*, *::before { box-sizing: border-box; }\n.row > * { flex: 1 1 0%; }\n> div { a: b; }\n",
        "p { font: 72px/1 Test, 'Segoe UI', \"\", sans-serif; width: calc(100% - 2 * 3px); }\n",
        "@media (min-width: 768px) and (max-width: 1px) { .a { b: rgba(0, 0, 0, .5); } }\n",
        "@keyframes k { from { opacity: 0; } 50% { opacity: .5; } to { opacity: 1; } }\n",
        "li:nth-child(2n + 1):not(.a, .b)::after { content: '\\201C'; }\n",
        "a[href^=\"http\"], input[type=text] { b: c }\n.n { &:hover { d: e; } }\n",
        // An+B's `\s*` is the token's without a `+` after it; `of` is stored.
        ":nth-child(2n of a.b) {}\n:nth-child(odd of .a) {}\n:nth-child(2n ) {}\n:nth-child(2n + 1 of a) {}\n",
        // A pseudo-element's or a pseudo-class's argument: a `plain_value` where it is longer
        // than a selector's name, else a selector (`:not` takes selectors alone).
        "::slotted(cr-button:hover) {}\n::slotted(hr:not([hidden])) {}\n:foo(a.b) {}\n::slotted(button) {}\n:not(a.b) {}\n",
        // A comment where a string's token starts, when it ends past the content: in the string;
        // the closing quote then stored, as in an empty string.
        "a{content:\"/* \"}b{content:\" */\"}\n",
        "a{content:\"\\a/* \"}b{content:\"*/\"}\na{content:\"/* x */ y\"}\na{content:\"/* */\"}\na{content:\"\n\"}\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Css).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Css.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = css_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for invalid in [
        "a {\n",
        "a { b: c; \n",
        "a b {} }\n",
        "@keyframes k { 1.5% { a: b; } }\n",
        "a { b: 'c }\n",
        // In a block an at-keyword may start a postcss statement: `(min-width:` is its value.
        "a{@container (min-width: 1px){}}\n",
        "::slotted(button:not(:first-child, .a)) {}\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            css_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads CSS with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-css.
#[test]
fn the_product_reads_css_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::css_lean;
    let grammar = Grammar::new(GrammarDialect::Css).unwrap();
    assert!(
        tree_parser(GrammarDialect::Css).is_some(),
        "CSS's parser is installed"
    );
    let source = "a.b > c { d: e 1px; }\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = css_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let invalid = "a {\n";
    assert!(css_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a CSS file's tape costs, by parser: the lean parser (`tape::css_lean`, the product's)
/// against tree-sitter-css, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn css_parse_costs() {
    use crate::code_rule_dsl::tape::css_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Css;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| css_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            css_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_css_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(css_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "CSS: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// SQL's tapes from the lean parser (the product's parser for SQL, decision 0214) are
/// tree-sitter-sql's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-sql-lean`'s `oracle`.
#[test]
fn sql_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::sql_lean;
    let sources = [
        "",
        "  \n",
        "-- a comment\nSELECT a, b AS c FROM t WHERE a = 1 AND b <> 'x' ORDER BY a DESC LIMIT 10;\n",
        "select count(*), t.* from s.t as t join u on t.id = u.id left join v using (id) group by 1 having count(*) > 2;\n",
        "WITH x AS (SELECT 1) SELECT * FROM x UNION ALL SELECT 2;\n",
        "INSERT INTO t (a, b) VALUES (1, 'a'), (2, NULL);\nUPDATE t SET a = a + 1 WHERE b IS NOT NULL;\nDELETE FROM t WHERE a IN (1, 2);\n",
        "CREATE TABLE t (id serial PRIMARY KEY, name varchar(20) NOT NULL DEFAULT '', u int REFERENCES u (id));\n",
        "CREATE INDEX i ON t (a, b DESC);\nDROP TABLE IF EXISTS t;\nALTER TABLE t ADD COLUMN c text;\n",
        "SELECT CASE WHEN a BETWEEN 1 AND 2 THEN 'x' ELSE 'y' END, a::text, CAST(b AS int) FROM t /* c */;\n",
        "BEGIN;\nGRANT SELECT ON t TO r;\nCOMMIT;\n",
        "CREATE FUNCTION f(a int) RETURNS int AS $$ SELECT a $$ LANGUAGE sql;\n",
        // pg_dump's forms (a Rails `db/structure.sql`).
        "SET client_encoding = 'UTF8';\nSELECT pg_catalog.set_config('search_path', '', false);\n\n--\n-- Name: users; Type: TABLE\n--\n\nCREATE TABLE public.users (\n    id bigint NOT NULL,\n    email character varying DEFAULT ''::character varying NOT NULL,\n    name character varying(20),\n    created_at timestamp(6) without time zone NOT NULL\n);\n",
        "CREATE SEQUENCE public.users_id_seq\n    START WITH 1\n    INCREMENT BY 1\n    NO MINVALUE\n    NO MAXVALUE\n    CACHE 1;\nALTER SEQUENCE public.users_id_seq OWNED BY public.users.id;\nALTER TABLE ONLY public.users ALTER COLUMN id SET DEFAULT nextval('public.users_id_seq'::regclass);\n",
        "ALTER TABLE ONLY public.users\n    ADD CONSTRAINT users_pkey PRIMARY KEY (id);\nCREATE INDEX i ON public.users USING btree (lower((email)::text));\nCREATE TRIGGER t BEFORE UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION public.f();\nCREATE MATERIALIZED VIEW v AS SELECT 1 WITH NO DATA;\nREFRESH MATERIALIZED VIEW v;\nTRUNCATE TABLE a, b RESTART IDENTITY CASCADE;\n",
        // Window functions, filters, upserts, string casts.
        "SELECT row_number() OVER (PARTITION BY a, b ORDER BY c DESC) AS n, sum(x) OVER w FROM t WINDOW w AS (PARTITION BY z);\nSELECT sum(x) OVER (ORDER BY y ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW), count(*) FILTER (WHERE x > 1) FROM t;\n",
        "INSERT INTO t (a, b) VALUES (1, 2) ON CONFLICT (a) DO UPDATE SET b = excluded.b WHERE t.b <> excluded.b RETURNING *;\nINSERT INTO t (a) VALUES (1) ON CONFLICT ON CONSTRAINT t_pkey DO NOTHING;\n",
        "SELECT date '2020-01-01', timestamp with time zone '2020-01-01', interval '1 day' FROM t;\n",
        // Application SQL: parameters (`$1`, `?`, `:name`), lateral joins, `extract`, aggregates'
        // `distinct` and `order by`, `delete … using`.
        "SELECT id FROM users WHERE id = ? AND created_at BETWEEN :start AND :end AND status = ANY($1);\n",
        "SELECT u.id, l.at FROM users u LEFT JOIN LATERAL (SELECT at FROM o WHERE o.u = u.id LIMIT 1) last ON true CROSS JOIN LATERAL unnest(u.tags) AS t(tag), LATERAL (SELECT 1) x;\n",
        "SELECT extract(epoch FROM at), count(DISTINCT u), string_agg(n, ', ' ORDER BY n) FROM t;\n",
        "DELETE FROM orders USING users WHERE orders.user_id = users.id RETURNING orders.id;\n",
        // Migrations: `alter table`'s renames, constraints and their attributes.
        "ALTER TABLE users RENAME COLUMN name TO full_name;\nALTER TABLE users RENAME TO accounts;\nALTER TABLE o DROP CONSTRAINT IF EXISTS fk CASCADE, VALIDATE CONSTRAINT k;\nALTER TABLE o OWNER TO app;\nALTER TABLE o SET SCHEMA archive;\nALTER TABLE o ADD CONSTRAINT fk FOREIGN KEY (u) REFERENCES users (id) NOT VALID;\n",
        "CREATE TABLE m (u bigint, CONSTRAINT fk FOREIGN KEY (u) REFERENCES users (id) DEFERRABLE INITIALLY DEFERRED);\n",
        // `explain`, `is distinct from`, `at time zone`, `similar to`, subscripts, bit strings, row
        // locks, `lock table`.
        "EXPLAIN ANALYZE SELECT a[1], a[2:3], B'101' FROM t WHERE x IS DISTINCT FROM y AND ts AT TIME ZONE 'UTC' > now() AND n NOT SIMILAR TO 'a%' ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED;\nLOCK TABLE t IN SHARE ROW EXCLUSIVE MODE;\n",
        // Migrations: domains, enums, generated and identity columns, operator classes.
        "CREATE DOMAIN email AS text NOT NULL CHECK (VALUE ~* '@');\nALTER TYPE mood ADD VALUE IF NOT EXISTS 'meh' AFTER 'ok';\nDROP TYPE IF EXISTS mood CASCADE;\n",
        "CREATE TABLE p (id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, total numeric GENERATED ALWAYS AS (a * b) STORED);\nCREATE INDEX i ON docs USING gin (data jsonb_path_ops);\nCREATE INDEX j ON t (a DESC NULLS LAST);\n",
        // `position`, `substring` and `trim`'s forms; `default values`; `offset … rows fetch …`;
        // `grant … on all tables in schema`; `rollup`; `set (a, b) =`; `like` and `partition by`.
        "SELECT position('a' IN name), substring(name FROM 1 FOR 3), trim(BOTH ' ' FROM name), extract(epoch FROM at) FROM users OFFSET 10 ROWS FETCH NEXT 5 ROWS ONLY;\nINSERT INTO t DEFAULT VALUES;\nGRANT SELECT, INSERT ON ALL TABLES IN SCHEMA public TO app;\n",
        "SELECT a, sum(b) FROM t GROUP BY ROLLUP (a);\nUPDATE t SET (a, b) = (1, 2) WHERE id = 1;\nCREATE TABLE t2 (LIKE t INCLUDING ALL);\nCREATE TABLE logs (id bigint, at timestamptz) PARTITION BY RANGE (at);\n",
        // `default current_timestamp`: the keyword; a quoted column name: a `literal`; `trim(both
        // from x)`: no unit; `begin; …; end`: a `block`.
        "CREATE TABLE t (\"a\" timestamp DEFAULT CURRENT_TIMESTAMP NOT NULL, b int DEFAULT (current_timestamp));\nCREATE INDEX i ON t (\"a\" DESC, b);\n",
        "SELECT trim(BOTH FROM name) FROM t;\nBEGIN;\nSELECT 1;\nEND;\n",
        // Backtick names (MySQL, BigQuery): an `identifier` over its two backticks; ``` `` ``` a
        // backtick inside one. Where `op_other` takes more than the backtick, the operator.
        "SELECT `a`.`b` AS `c`, `d` `e` FROM `s`.`t` `x` WHERE `a` = 1;\nSELECT `a``b` FROM t;\n",
        "CREATE TABLE `t` (`id` int NOT NULL, `n` varchar(20));\nINSERT INTO `t` (`a`, `b`) VALUES (1, 2);\n",
        "SELECT a `| b FROM t;\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Sql).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Sql.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = sql_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    // `id=?` is one operator (`op_other`) to tree-sitter-sql, so a syntax error.
    // `@ b` is a `_tsql_parameter` (an identifier), `table` and `grouping` keywords where they
    // stand, `((select …) > 0)` no subquery, and an expression after `default current_timestamp`
    // none.
    for invalid in [
        "SELECT (1;\n",
        "SELECT a FROM t WHERE;\n",
        "CREATE TABLE t (a int;\n",
        "SELECT 'a;\n",
        "SELECT a FROM t WHERE id=?;\n",
        "SELECT a @ t.b AS c;\n",
        "SELECT a FROM t WHERE (table = 'x');\n",
        "SELECT a FROM t GROUP BY grouping, a;\n",
        "SELECT a FROM t WHERE ((SELECT 1) > 0 OR b);\n",
        "CREATE TABLE t (a int DEFAULT current_timestamp + 1);\n",
        // A backtick where an alias may follow is a name's, as tree-sitter's lexer takes it (a
        // string token wins its tie with `op_other`): not an operator between `a`, `b` and `c`.
        "SELECT a`b`c FROM t;\n",
        "SELECT \"a\"`b`\"c\" FROM t;\n",
        "SELECT `a FROM t;\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            sql_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads SQL with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-sql.
#[test]
fn the_product_reads_sql_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::sql_lean;
    let grammar = Grammar::new(GrammarDialect::Sql).unwrap();
    assert!(
        tree_parser(GrammarDialect::Sql).is_some(),
        "SQL's parser is installed"
    );
    let source = "SELECT a FROM t WHERE b = 1;\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = sql_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let invalid = "SELECT (1;\n";
    assert!(sql_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a SQL file's tape costs, by parser: the lean parser (`tape::sql_lean`, the product's)
/// against tree-sitter-sql, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn sql_parse_costs() {
    use crate::code_rule_dsl::tape::sql_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Sql;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| sql_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            sql_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_sql_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(sql_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "SQL: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Markdown's tapes from the lean parser (the product's parser for Markdown, decision 0237) are
/// tree-sitter-md's, byte for byte, on hand-written cases; a text tree-sitter reads with an error
/// the lean parser refuses (its file goes to tree-sitter). The corpus comparison is
/// `rerail-markdown-lean`'s `oracle`.
#[test]
fn markdown_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::markdown_lean;
    let sources = [
        "",
        "\n\n",
        "# Title\n\nSome *text*.\n\n```rust\nfn f() {}\n```\n",
        "Setext\n======\n\n- a\n- [x] b\n  - c\n\n1. d\n2) e\n",
        "> quote\n> more\n>\n> - item\n\n    indented\n    code\n\n***\n",
        "| a | b |\n|:--|--:|\n| c | `d` |\n\n<div>\nhtml\n</div>\n\n<!-- c -->\n",
        "[label]: https://x.y/z \"title\"\n[a-b]: <u>\n\n## Two ##\n",
        "---\ntitle: x\n---\n\n~~~ {.python}\ncode\n~~~\n",
        "[`a`]:\n    https://x/y?a&b=c&amp;d\n[b]: u\n\"t\" x\n\n```c&#38;d\n```\n",
        "> [a]:\n> <u> 'multi\n> line'\n",
        // A lone delimiter cell's trailing `|` is implied (left out), a second cell's is stored.
        "| a |\n| - |\n| b |\n\n| a | b |\n| - | - |\n",
        // Definitions after a blank line in a list item; a label led by a reference, as a
        // definition or indented.
        "* a\n\n  [b]: u\n  [c]: v\n\n[&amp; b]: u\n\n  [&amp; b](u)\n",
        // `<!…` (HTML block 4) ends with its container, not at `>`.
        "> <!X a>\n> b\n\nc\n\n<!DOCTYPE html>\n<html lang=\"en\">\n\npara\n",
    ];
    let grammar = Grammar::new(GrammarDialect::Markdown).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Markdown.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = markdown_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    // Also: an unindented label led by a character reference that is no definition, and an empty
    // delimiter cell (tree-sitter inserts a MISSING one).
    for invalid in [
        "a\n\n* ",
        "* <div>\n\n  y\n",
        "[\\<b>]\n",
        "[&amp; b](u)\n",
        "- [&#1; b](u)\n",
        "| a | b |\n| - |  |\n",
        "| a | b |\n|  | - |\n",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            markdown_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
}

/// The product reads Markdown with the lean parser: a file's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a file it refuses goes to tree-sitter-md.
#[test]
fn the_product_reads_markdown_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::markdown_lean;
    let grammar = Grammar::new(GrammarDialect::Markdown).unwrap();
    assert!(
        tree_parser(GrammarDialect::Markdown).is_some(),
        "Markdown's parser is installed"
    );
    let source = "# A\n\n- b\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = markdown_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let invalid = "* <div>\n\n  y\n";
    assert!(markdown_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Markdown file's tape costs, by parser: the lean parser (`tape::markdown_lean`, the product's)
/// against tree-sitter-md, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn markdown_parse_costs() {
    use crate::code_rule_dsl::tape::markdown_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Markdown;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| markdown_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            markdown_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_markdown_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(markdown_lean::encode(&grammar, f, &|| false)).is_ok(),
                )
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Markdown: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Vue's tapes from the lean parser (the product's parser for Vue, decision 0242) are
/// tree-sitter-vue's, byte for byte, on hand-written cases, and so are the programs they embed
/// (`FileImage::embedding`, built from either tape); a file tree-sitter reads with an error the
/// lean parser refuses (its file goes to tree-sitter). The corpus comparison is `rerail-vue-lean`'s
/// `oracle`.
#[test]
fn vue_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::vue_lean;
    let sources = [
        "",
        "  \n",
        "<template>\r\n  <div :a :b=\"c\" @d.e.f=\"g\" #h v-on:[i].j=\"k\" v-if='x'>\r\n    x {{ y }} z\r\n  </div>\r\n</template>\r\n",
        "\u{feff}<template><p>a<div>b</div><br>c<ul><li>1<li>2</ul></template>\n",
        "<template><ElTable.TableColumn :a=\"b\"/><x/><template v-slot:y=\"{ z }\"/></template>\n<script setup lang=\"ts\">\nif (a</b) {} // </scri\nconst y = 1\n</script>\n<style scoped>a{}</style>\n",
        "<!DOCTYPE html>\n<!-- c -->\n<template>\n  <a>  x <b>y</b>\n  z\r\n  </a>\n  <p>a < b { c } d}</p>\n  <q>{{}}{{\n}}{{ a }}}</q>\n  <i x=y z = \"w\" u='' v=\"\">&amp;</i>\n</template>\n",
        "<template><div></span></div></template>  ",
        "<script></script> <style>x</style>",
    ];
    let grammar = Grammar::new(GrammarDialect::Vue).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::Vue.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = vue_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false).unwrap();
        assert!(
            ours.to_bytes().unwrap() == theirs.primary.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs.primary, &grammar)
        );
        let ours = FileImage::embedding(ours, &grammar, source, &|| false).unwrap();
        assert_eq!(ours.derived.len(), theirs.derived.len(), "{source:?}");
        for (a, b) in ours.derived.iter().zip(&theirs.derived) {
            assert_eq!(a.text, b.text, "{source:?}");
            assert_eq!(
                a.tape.to_bytes().unwrap(),
                b.tape.to_bytes().unwrap(),
                "{source:?}"
            );
        }
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for invalid in [
        "<template>\n",
        "<div><br>",
        "foo",
        "<a>x\n  > y</a>",
        "<template>{{ a </template>",
        "</div>\n",
        "<script />",
    ] {
        assert!(
            parser.parse(invalid, None).unwrap().root_node().has_error(),
            "{invalid:?} reads without an error"
        );
        assert!(
            vue_lean::encode(&grammar, invalid, &|| false).is_err(),
            "accepted {invalid:?}"
        );
    }
    // Past the scanner's 1024-byte serialized state (here 4 + 1 + 128 × 8 bytes) the innermost tag
    // comes back nameless, and tree-sitter reads its end tag as an `erroneous_end_tag`: refused.
    let nested = |k: usize| {
        format!(
            "<template>\n{}{}</template>\n",
            "<abcdef>\n".repeat(k),
            "</abcdef>\n".repeat(k)
        )
    };
    assert!(
        vue_lean::encode(&grammar, &nested(127), &|| false).is_ok_and(|t| t.is_ok()),
        "refused 127 tags"
    );
    assert!(
        vue_lean::encode(&grammar, &nested(128), &|| false).is_err(),
        "accepted 128 tags"
    );
}

/// The product reads Vue with the lean parser: a file's documents through the engine's parse
/// (`TapeImage::parse`) are the lean parser's tape and the program built from it; a file it
/// refuses goes to tree-sitter-vue.
#[test]
fn the_product_reads_vue_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::vue_lean;
    let grammar = Grammar::new(GrammarDialect::Vue).unwrap();
    assert!(
        tree_parser(GrammarDialect::Vue).is_some(),
        "Vue's parser is installed"
    );
    let source = "<template><p :class=\"a\">{{ b }}</p></template>\n<script>\nexport default { data: () => ({ a: 1, b: 2 }) }\n</script>\n";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = vue_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    assert_eq!(product.derived.len(), 1, "the component's program");
    let invalid = "<template>\n";
    assert!(vue_lean::encode(&grammar, invalid, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, invalid, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What a Vue file's tape costs, by parser: the lean parser (`tape::vue_lean`, the product's)
/// against tree-sitter-vue, over `RERAIL_PY_FILES` (a list of paths): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read. The file's documents
/// (its tape and the program its view builds) are measured too, from either parser.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn vue_parse_costs() {
    let ecma_dialect = |d: GrammarDialect| match d {
        GrammarDialect::TypeScript => rerail_ecma_lean::Dialect::TypeScript,
        GrammarDialect::Tsx => rerail_ecma_lean::Dialect::Tsx,
        _ => rerail_ecma_lean::Dialect::JavaScript,
    };
    use crate::code_rule_dsl::tape::vue_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::Vue;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| vue_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            vue_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    // The embedded programs' texts, each read by the lean ECMAScript parser (tree-sitter where it
    // refuses one) and by tree-sitter alone.
    let programs: Vec<(GrammarDialect, String)> = files
        .iter()
        .filter_map(|f| {
            let tape = vue_lean::encode(&grammar, f, &|| false).unwrap().unwrap();
            let cache = tape.own_cache();
            let root = crate::code_rule_dsl::embed::host::Host::new(&tape, &grammar, cache.as_ref()).root()?;
            let view = crate::code_rule_dsl::vue::view::View::build(root, f, &|| false)?;
            Some((view.dialect(), view.text().to_owned()))
        })
        .collect();
    let read_lean = programs
        .iter()
        .filter(|(d, text)| {
            crate::code_rule_dsl::embed::Parse::read(*d, text, &|| false)
                .is_some_and(|p| p.is_lean())
        })
        .count();
    let program_bytes: usize = programs.iter().map(|(_, text)| text.len()).sum();
    let tape_of =
        |d: GrammarDialect, text: &str, parse: crate::code_rule_dsl::embed::Parse| match parse {
            crate::code_rule_dsl::embed::Parse::Tape(tape) => tape.node_count(),
            crate::code_rule_dsl::embed::Parse::Tree(tree) => {
                let grammar = Grammar::embedded(d).unwrap();
                TapeImage::encode_tree_image(tree.root_node(), &grammar, text, None, &|| false)
                    .map_or(0, |t| t.node_count())
            }
        };
    let program_nodes: usize = programs
        .iter()
        .map(|(d, text)| {
            tape_of(
                *d,
                text,
                crate::code_rule_dsl::embed::Parse::Tree(
                    crate::code_rule_dsl::embed::Parse::tree_sitter(*d, text).unwrap(),
                ),
            )
        })
        .sum();
    eprintln!(
        "{} programs, {read_lean} read by the lean parser, {program_bytes} bytes, {program_nodes} program nodes",
        programs.len()
    );
    for (d, text) in &programs {
        if let Err(why) = crate::code_rule_dsl::tape::ecma_lean::encode(
            ecma_dialect(*d),
            &Grammar::embedded(*d).unwrap(),
            text,
            &|| false,
        ) {
            let dir = std::env::var("RERAIL_VUE_REFUSED").unwrap_or_default();
            if !dir.is_empty() {
                std::fs::write(format!("{dir}/{}.txt", text.len()), text).unwrap();
            }
            eprintln!("refused, {:?}: {why:?}", d);
        }
    }
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| rerail_vue_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(std::hint::black_box(vue_lean::encode(&grammar, f, &|| false)).is_ok())
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "lean: the documents",
            Box::new(|f| {
                let tape = vue_lean::encode(&grammar, f, &|| false).unwrap().unwrap();
                usize::from(
                    std::hint::black_box(FileImage::embedding(tape, &grammar, f, &|| false))
                        .is_ok(),
                )
            }),
        ),
        (
            "tree-sitter host: the documents",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let programs_by = |lean: bool| {
        programs
            .iter()
            .map(|(d, text)| {
                let parse = if lean {
                    crate::code_rule_dsl::embed::Parse::read(*d, text, &|| false).unwrap()
                } else {
                    crate::code_rule_dsl::embed::Parse::Tree(
                        crate::code_rule_dsl::embed::Parse::tree_sitter(*d, text).unwrap(),
                    )
                };
                tape_of(*d, text, parse)
            })
            .sum::<usize>()
    };
    // Each step a pass: the files' steps over every file, the programs' over every program.
    let each_file = &files;
    let mut passes: Vec<CostPass<'_>> = steps
        .into_iter()
        .map(|(name, step)| {
            (
                name,
                Box::new(move || each_file.iter().map(|f| step(f)).sum::<usize>())
                    as Box<dyn Fn() -> usize>,
            )
        })
        .collect();
    passes.push(("lean: the programs' tapes", Box::new(|| programs_by(true))));
    passes.push((
        "tree-sitter: the programs' tapes",
        Box::new(|| programs_by(false)),
    ));
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); passes.len()];
    for _ in 0..3 {
        for (i, (_, pass)) in passes.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            std::hint::black_box(pass());
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Vue: {} files of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in passes.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a host node, {:7.1} cycles a host node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Inline texts' tapes from the lean parser (the product's parser for Markdown's inline grammar,
/// decision 0234) are tree-sitter-md-inline's, byte for byte, on hand-written cases: links of each
/// kind, images, emphasis by its delimiters' runs, code and LaTeX spans, HTML tags, autolinks,
/// references and line breaks. The corpus comparison is `rerail-markdown-inline-lean`'s `oracle`.
#[test]
fn markdown_inline_tapes_from_the_lean_parser_are_tree_sitters() {
    use crate::code_rule_dsl::tape::markdown_inline_lean;
    let sources = [
        "*a* _b_ and a*b",
        "**c** and ~~e~~",
        "`x` *a* _b_",
        "[a] [b][] [c][d] [e](f \"t\") [](g) [h](<i j> 'k') [l](\"m\")",
        "![a](b) ![c]",
        "![d][]",
        "[![f](g)](h)",
        "`a` ``b ` c`` $x$ and `<a b=c.d>`",
        "<a href=\"x.y\" b=c.d> </a> <br/> <x@y.z> <http://a.b>",
        "&amp; &#12; &#x1F; \\* a\\\nb  \nc",
        "[a *b] c* and *a [b* c]",
        // Emphasis after an image, where its opener needs no last token: kept.
        "![a](b)\n*c*",
        // Eight images: tree-sitter's stack keeps each reading.
        "![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)",
        // A link's `_` lexed for the text alone (no emphasis open): an opener, hidden.
        "[^_]",
    ];
    let grammar = Grammar::new(GrammarDialect::MarkdownInline).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&GrammarDialect::MarkdownInline.get_ts_language())
        .unwrap();
    for source in sources {
        let ours = markdown_inline_lean::encode(&grammar, source, &|| false)
            .unwrap_or_else(|why| panic!("refused {source:?}: {why:?}"))
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "tree-sitter's tree of {source:?} has an error"
        );
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false)
            .unwrap()
            .primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Refused: an empty text; an HTML comment (not read yet); an emphasis delimiter tree-sitter may
    // lex again after a reduction (`x __b c __d()`: its emphasis closes at `__d`); two trees of one
    // score (`![c] ![d][e]`: an image and a shortcut link, or one full-reference image); an opener
    // after an image that needs the last token's white space or punctuation (tree-sitter's stack
    // holds both of the image's readings and lexes again across them: `*(c.)*` is then text);
    // nine images (past `MAX_LINK_COUNT`, tree-sitter reads the first as `!` and a link); a link's
    // delimiter whose token depends on the versions' order (`_[^_]`: tree-sitter stores the `_`).
    for refused in [
        "",
        "<!-- c -->",
        "x __b c __d()",
        "![c] ![d][e]",
        "![a](b)\n*(c.)*",
        "![a]) .*/ b*",
        "![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)\n![a](b)",
        "_[^_]",
    ] {
        assert!(
            markdown_inline_lean::encode(&grammar, refused, &|| false).is_err(),
            "accepted {refused:?}"
        );
    }
}

/// The product reads inline texts with the lean parser: a text's image through the engine's parse
/// (`TapeImage::parse`) is the lean parser's; a text it refuses goes to tree-sitter-md-inline.
#[test]
fn the_product_reads_markdown_inline_with_the_lean_parser() {
    use crate::code_rule_dsl::tape::markdown_inline_lean;
    let grammar = Grammar::new(GrammarDialect::MarkdownInline).unwrap();
    assert!(
        tree_parser(GrammarDialect::MarkdownInline).is_some(),
        "the inline grammar's parser is installed"
    );
    let source = "Some *text* and [a link](b).";
    let product = TapeImage::parse(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    let lean = markdown_inline_lean::encode(&grammar, source, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        product.primary.to_bytes().unwrap(),
        lean.to_bytes().unwrap()
    );
    let refused = "a <!-- c --> b";
    assert!(markdown_inline_lean::encode(&grammar, refused, &|| false).is_err());
    assert!(
        TapeImage::parse(&grammar, refused, &|| false)
            .unwrap()
            .is_ok()
    );
}

/// What an inline text's tape costs, by parser: the lean parser (`tape::markdown_inline_lean`, the
/// product's) against tree-sitter-md-inline, over `RERAIL_PY_FILES` (a list of inline texts' paths,
/// as `rerail-markdown-lean`'s `inline_texts` writes them): instructions retired, cycles
/// and CPU time a node, the best of three passes. Only the files both read.
#[test]
#[ignore = "a measurement; needs RERAIL_PY_FILES"]
fn markdown_inline_parse_costs() {
    use crate::code_rule_dsl::tape::markdown_inline_lean;
    let list =
        std::fs::read_to_string(std::env::var("RERAIL_PY_FILES").expect("set RERAIL_PY_FILES"))
            .unwrap();
    let gd = GrammarDialect::MarkdownInline;
    let grammar = Grammar::new(gd).unwrap();
    let parser = std::cell::RefCell::new(tree_sitter::Parser::new());
    parser
        .borrow_mut()
        .set_language(&gd.get_ts_language())
        .unwrap();
    let mut files: Vec<String> = list
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .collect();
    let all = files.len();
    files.retain(|f| markdown_inline_lean::encode(&grammar, f, &|| false).is_ok_and(|t| t.is_ok()));
    let bytes: usize = files.iter().map(String::len).sum();
    let nodes: usize = files
        .iter()
        .map(|f| {
            markdown_inline_lean::encode(&grammar, f, &|| false)
                .unwrap()
                .unwrap()
                .node_count()
        })
        .sum();
    let retired = rerail_compiler::native_cpu_codegen::retired;
    let cpu = || rerail_prof::thread_cpu().as_secs_f64();
    let steps: Vec<CostStep<'_>> = vec![
        (
            "lean: the parser's tape",
            Box::new(|f| {
                rerail_markdown_inline_lean::tape(f.as_bytes()).map_or(0, |t| t.tape.len())
            }),
        ),
        (
            "lean: the tape",
            Box::new(|f| {
                usize::from(
                    std::hint::black_box(markdown_inline_lean::encode(&grammar, f, &|| false))
                        .is_ok(),
                )
            }),
        ),
        (
            "tree-sitter: parse",
            Box::new(|f| {
                parser
                    .borrow_mut()
                    .parse(f, None)
                    .unwrap()
                    .root_node()
                    .child_count() as usize
            }),
        ),
        (
            "tree-sitter: the tape",
            Box::new(|f| {
                let tree = parser.borrow_mut().parse(f, None).unwrap();
                usize::from(
                    std::hint::black_box(TapeImage::encode(tree.root_node(), &grammar, f, &|| {
                        false
                    }))
                    .is_ok(),
                )
            }),
        ),
    ];
    let mut best = vec![(f64::MAX, u64::MAX, u64::MAX); steps.len()];
    for _ in 0..3 {
        for (i, (_, step)) in steps.iter().enumerate() {
            let (i0, c0) = retired();
            let t0 = cpu();
            let mut n = 0;
            files.iter().for_each(|f| n += step(f));
            std::hint::black_box(n);
            let (i1, c1) = retired();
            let t = cpu() - t0;
            best[i] = (
                best[i].0.min(t),
                best[i].1.min(i1 - i0),
                best[i].2.min(c1 - c0),
            );
        }
    }
    eprintln!(
        "Markdown inline: {} texts of {all} the lean parser reads, {bytes} bytes, {nodes} nodes",
        files.len()
    );
    for ((name, _), (t, ins, cyc)) in steps.iter().zip(&best) {
        eprintln!(
            "{name:32} {:9.1} instructions a node, {:7.1} cycles a node, {:6.2} ns a byte, {:8.3} s",
            *ins as f64 / nodes as f64,
            *cyc as f64 / nodes as f64,
            t * 1e9 / bytes as f64,
            t
        );
    }
}

/// Decision 0293: every format a run reads (`RERAIL_TAPE`) answers as the restored records do: kind, field, span,
/// subtree, parent and children. The Python file is past two of 16z's fixed blocks, so its reads cross them.
#[test]
fn every_tape_format_reads_the_document_as_restored() {
    use super::formats::Format;
    use rerail_tape_format::f16::Z;
    use rerail_tape_format::tape::{Tape, children};
    let source: String = (0..400).map(|i| format!("x{i} = f(a, {i})\n")).collect();
    let (_, doc, _, _) = super::formats::as_run(Format::F16, || fixture(&source, GrammarDialect::Python));
    let n = doc.transducer_len();
    assert!(n > 2 * Z, "{n} nodes");
    for (format, name) in [(Format::F16, "16"), (Format::F16Z, "16z")] {
        // The tape a run of `format` makes, read against format 16's restored records.
        let (_, made, _, _) = super::formats::as_run(format, || fixture(&source, GrammarDialect::Python));
        let t = made.doc_tape().tape();
        assert_eq!(t.format(), name, "made in the run's format");
        rerail_tape_format::with_tape!(t, |t| {
            assert_eq!(t.len(), n);
            for i in 0..n {
                let node = i as u32;
                let e = t.table().entry(t.code(i));
                assert_eq!(
                    (e.kind, e.field),
                    (doc.transducer_kind(node), doc.transducer_field(node)),
                    "{name}: code of {i}"
                );
                let span = doc.transducer_span(node);
                assert_eq!(
                    (t.start(i), t.end(i)),
                    (span.start as u64, span.end as u64),
                    "{name}: span of {i}"
                );
                assert_eq!(
                    t.subtree_end(i),
                    doc.transducer_descendants(node).end as usize,
                    "{name}: subtree of {i}"
                );
                assert_eq!(
                    t.parent(i),
                    doc.transducer_parent(node).map(|p| p as usize),
                    "{name}: parent of {i}"
                );
                assert_eq!(
                    children(&t, i).collect::<Vec<_>>(),
                    doc.transducer_children(node)
                        .map(|c| c as usize)
                        .collect::<Vec<_>>(),
                    "{name}: children of {i}"
                );
            }
        });
    }
}

/// Decision 0353 step 4a (row 17): a parser writes its tape in its room where a device reads tapes (`placed::rooms`),
/// whichever parser it is (tree-sitter's, a Bitlex parser's, simdjson's): the image a parse gives is placed there, and
/// placing it (`place_stored`) is its home as it is, with the bytes it would be stored as.
#[test]
fn a_parse_writes_its_tape_in_its_room() {
    for (language, source) in [
        (GrammarDialect::Python, "def f(a):\n    return len(a)\n"),
        (GrammarDialect::Json, "{\"a\": [1, 2, {\"b\": null}]}\n"),
        (GrammarDialect::Go, "package p\n\nfunc f(a int) int { return a }\n"),
        (GrammarDialect::Rust, "fn f(a: u8) -> u8 { a }\n"),
    ] {
        let grammar = Arc::new(Grammar::new(language).unwrap());
        let image = TapeImage::parse(&grammar, source, &|| false).expect("parsed").expect("encoded").primary;
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        assert_eq!(image.placed(), super::placed::device().is_some(), "{language:?}: written in a slab's room where there is a device");
        let home = image.place_stored().unwrap();
        assert_eq!(&*home, &image.to_bytes().unwrap()[..], "{language:?}: the bytes as stored");
        if image.placed() {
            assert_eq!(home.as_ptr(), image.room().unwrap().bytes().as_ptr(), "{language:?}: placed where its parser wrote it");
        }
    }
}

/// Decision 0353 step 4a: a stored tape placed in its home (a slab's room, where the host has a device) and read
/// there answers every node question as the same tape read from a buffer of its own, in each format.
#[test]
fn a_placed_tape_reads_as_one_held_in_its_own_bytes() {
    use super::formats::Format;
    use rerail_tape_format::tape::{Tape, children};
    let source: String = (0..400).map(|i| format!("x{i} = f(a, {i})\n")).collect();
    let grammar = Arc::new(Grammar::new(GrammarDialect::Python).unwrap());
    for format in Format::ALL {
        // The parser's tape, as a run of `format` writes it.
        let (_, _, image, _) = super::formats::as_run(format, || fixture(&source, GrammarDialect::Python));
        assert_eq!(image.format(), format, "written in the run's format");
        let bytes = image.to_bytes().unwrap();
        let own = TapeImage::from_bytes(&bytes, &grammar).unwrap();
        let home = super::placed::place(bytes.len(), |out| {
            out.copy_from_slice(&bytes);
            Ok(())
        })
        .unwrap();
        let placed = TapeImage::from_home(home, &grammar).unwrap();
        assert_eq!(placed.format(), own.format(), "held as stored");
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        assert_eq!(placed.placed(), crate::code_rule_dsl::tape::placed::device().is_some(), "{format:?}: in a slab's room where there is a device");
        assert_eq!(placed.to_bytes().unwrap(), bytes, "{format:?}: the bytes as stored");
        let text = Arc::new(crate::code_rule_dsl::tape::Text::from(source.clone()));
        // Each read in a batch of its own: a 16z tape's blocks decoded into a cache made for it.
        let batch = |image: TapeImage| {
            let cache = image.own_cache().map(Arc::new);
            super::formats::DocTape::in_batch(Arc::new(image), Arc::clone(&text), cache)
        };
        let (own, placed) = (batch(own), batch(placed));
        let (a, b) = (own.tape(), placed.tape());
        rerail_tape_format::with_tape!(a, |a| {
            rerail_tape_format::with_tape!(b, |b| {
                assert_eq!(a.len(), b.len());
                for i in 0..a.len() {
                    let at = |what: &str| format!("{format:?}: {what} of {i}");
                    assert_eq!(a.table().entry(a.code(i)), b.table().entry(b.code(i)), "{}", at("code"));
                    assert_eq!((a.start(i), a.end(i)), (b.start(i), b.end(i)), "{}", at("span"));
                    assert_eq!(a.subtree_end(i), b.subtree_end(i), "{}", at("subtree"));
                    assert_eq!(a.parent(i), b.parent(i), "{}", at("parent"));
                    assert_eq!(children(&a, i).collect::<Vec<_>>(), children(&b, i).collect::<Vec<_>>(), "{}", at("children"));
                }
            });
        });
        let ((ta, va), (tb, vb)) = (own.in_place().unwrap(), placed.in_place().unwrap());
        // A 16z view names its tape by an id of its own (its blocks' keys in a cache): the rest is the same.
        let unnamed = |mut v: rerail_clif::view::TapeView| {
            if v.format == rerail_clif::view::F16Z_FORMAT {
                (v.side[2], v.side[3]) = (0, 0);
            }
            v
        };
        assert_eq!((ta, unnamed(va)), (tb, unnamed(vb)), "{format:?}: the reads' bytes and view");
    }
}

/// Decision 0293 §4: a file's documents made in each format and stored (as the store keeps and reads them) are held
/// as stored, and the documents restored from them answer every node question as format 16's do: a Python file of
/// several blocks, and an HTML host with a TSX guest.
#[test]
fn a_file_stored_in_each_format_reads_back_as_format_16() {
    use super::formats::Format;
    fn same(a: &TapeDoc, b: &TapeDoc, name: &str) {
        // Each read in a batch of its own: a 16z tape's blocks decoded into a cache made for it.
        let (a, b) = (&a.alone(), &b.alone());
        assert_eq!(a.transducer_len(), b.transducer_len(), "{name}: nodes");
        for node in 0..a.transducer_len() as u32 {
            assert_eq!(
                (
                    a.transducer_kind(node),
                    a.transducer_field(node),
                    a.transducer_flags(node),
                    a.transducer_span(node)
                ),
                (
                    b.transducer_kind(node),
                    b.transducer_field(node),
                    b.transducer_flags(node),
                    b.transducer_span(node)
                ),
                "{name}: node {node}"
            );
            assert_eq!(
                a.transducer_parent(node),
                b.transducer_parent(node),
                "{name}: parent of {node}"
            );
            assert_eq!(
                a.transducer_descendants(node),
                b.transducer_descendants(node),
                "{name}: subtree of {node}"
            );
            assert_eq!(
                a.transducer_children(node).collect::<Vec<_>>(),
                b.transducer_children(node).collect::<Vec<_>>(),
                "{name}: children of {node}"
            );
        }
        assert_eq!(
            a.transducer_supertypes().collect::<Vec<_>>(),
            b.transducer_supertypes().collect::<Vec<_>>(),
            "{name}: classes"
        );
    }
    let python: String = (0..400)
        .map(|i| {
            format!("class C{i}(B):\n    def f(self, x={i}):\n        return [y for y in x if y]\n")
        })
        .collect();
    let cases = [
        (GrammarDialect::Python, python),
        (
            GrammarDialect::TestHtml,
            "<script lang=\"tsx\">let x = <a />;</script>\n".to_owned(),
        ),
    ];
    for (lang, source) in cases {
        let grammar = Grammar::new(lang).unwrap();
        let image = super::formats::as_run(Format::F16, || TapeImage::parse(&grammar, &source, &|| false))
            .unwrap()
            .unwrap();
        let reference = TapeDoc::restore(
            Arc::new(source.clone().into()),
            Arc::clone(&grammar),
            &image,
            &|| false,
        )
        .unwrap();
        for format in Format::ALL {
            let stored = super::formats::as_run(format, || TapeImage::parse(&grammar, &source, &|| false)).unwrap().unwrap().round_trip(&grammar).unwrap();
            assert_eq!(stored.primary.format(), format, "{lang:?} in {}", format.name());
            for d in &stored.derived {
                assert_eq!(
                    d.tape.format(),
                    format,
                    "{lang:?}'s guest in {}",
                    format.name()
                );
            }
            let doc = TapeDoc::restore(
                Arc::new(source.clone().into()),
                Arc::clone(&grammar),
                &stored,
                &|| false,
            )
            .unwrap();
            same(&reference, &doc, &format!("{lang:?} in {}", format.name()));
            let (guests, read) = (reference.guests().unwrap(), doc.guests().unwrap());
            assert_eq!(guests.len(), read.len());
            assert_eq!(guests.len(), usize::from(lang == GrammarDialect::TestHtml));
            for (g, r) in guests.iter().zip(read.iter()) {
                same(
                    &g.doc,
                    &r.doc,
                    &format!("{lang:?}'s guest in {}", format.name()),
                );
            }
        }
    }
}

/// Decision 0293 (the user, 2026-11-16: "generalize the lean parsers so that they produce format 16 directly, and
/// ideally format 16 compressed directly"): each lean parser's writer, finished in a run of format 16 or 16z, writes
/// that format in one pass over its nodes (`formats::lean`). Its 16z tape answers every node question as its format
/// 16 tape does.
#[test]
fn the_lean_parsers_write_each_format_directly() {
    use super::formats::{Format, as_run};
        use crate::code_rule_dsl::transducer::packed::json_parser::simdjson as json_simdjson;
    use rerail_tape_format::tape::Tape;
    type Encode = fn(&Grammar, &str) -> Result<TapeImage, String>;
    fn lean<R: std::fmt::Debug>(
        r: Result<Result<TapeImage, String>, R>,
    ) -> Result<TapeImage, String> {
        r.map_err(|why| format!("the lean parser refused it: {why:?}"))?
    }
    let python: String = (0..300).map(|i| format!("class C{i}(B):\n    def f(self, x={i}):\n        return [y for y in x if y]\n")).collect();
    let js: String = (0..200).map(|i| format!("const x{i} = (a) => a?.b ?? {i};\nfunction f{i}(y) {{ return y + 1; }}\n")).collect();
    // Each language's file: a few hundred top-level items, so that the writer commits many times and its window
    // (decision 0434 §2) holds a small part of the tape. `{i}` is the item's number.
    let many = |item: &str| -> String { (0..200).map(|i| item.replace("{i}", &i.to_string())).collect() };
    #[allow(unused_mut)]
    let cases: Vec<(GrammarDialect, String, Encode)> = vec![
        (GrammarDialect::Python, python, |g, s| lean(super::python_lean::encode(g, s, &|| false))),
        (GrammarDialect::JavaScript, js, |g, s| lean(super::ecma_lean::encode(rerail_ecma_lean::Dialect::JavaScript, g, s, &|| false))),
        (GrammarDialect::TypeScript, many("interface A{i} { x?: number }\nconst x{i}: A{i} = {};\n"), |g, s| lean(super::ecma_lean::encode(rerail_ecma_lean::Dialect::TypeScript, g, s, &|| false))),
        (GrammarDialect::Tsx, many("const A{i} = () => <div title=\"x\">text {i}</div>;\n"), |g, s| lean(super::ecma_lean::encode(rerail_ecma_lean::Dialect::Tsx, g, s, &|| false))),
        (GrammarDialect::Rust, many("// item {i}\n#[inline]\nfn f{i}() { let x = Some({i}); if let Some(y) = x { println!(\"{y}\"); } }\nstruct S{i} { a: u8 }\n"), |g, s| lean(super::rust_lean::encode(g, s, &|| false))),
        (GrammarDialect::Php, format!("<?php\n{}", many("// f{i}\nfunction f{i}($x) { return $x + {i}; }\n$a{i} = \"v{$x}\";\n")), |g, s| lean(super::php_lean::encode(g, s, &|| false))),
        (GrammarDialect::Sql, many("-- q{i}\nSELECT a, count(*) FROM t{i} GROUP BY a;\nINSERT INTO t VALUES ({i});\n"), |g, s| lean(super::sql_lean::encode(g, s, &|| false))),
        (GrammarDialect::Vue, format!("{}<script>\nlet value = 1;\n</script>\n", many("<template><div>{{ value{i} }}</div></template>\n<!-- c{i} -->\n")), |g, s| lean(super::vue_lean::encode(g, s, &|| false))),
        (GrammarDialect::Css, many("/* r{i} */\na{i} { color: red; }\n.b > c { margin: 0 {i}px; }\n@media screen { d { x: y } }\n"), |g, s| lean(super::css_lean::encode(g, s, &|| false))),
        (GrammarDialect::CSharp, format!("using System;\n{}", many("// A{i}\nclass A{i} { void F() { var x = {i}; } }\n")), |g, s| lean(super::csharp_lean::encode(g, s, &|| false))),
        (GrammarDialect::Html, many("<div class=\"a\"><p>hi {i}<p>there</div>\n<!-- c{i} -->\ntext &amp; {i}\n"), |g, s| lean(super::html_lean::encode(g, s, &|| false))),
        (GrammarDialect::Java, format!("import a.b;\n{}", many("// A{i}\nclass A{i} { void f() { int x = {i}; } }\n")), |g, s| lean(super::java_lean::encode(g, s, &|| false))),
        (GrammarDialect::Markdown, many("# Title {i}\n\nSome *text* here.\n\nSetext {i}\n---\n\n- a\n- b\n\n    code {i}\n\n"), |g, s| lean(super::markdown_lean::encode(g, s, &|| false))),
        (GrammarDialect::MarkdownInline, many("Some *text* and `code` {i} [a](b) "), |g, s| lean(super::markdown_inline_lean::encode(g, s, &|| false))),
        (GrammarDialect::Rr, ".decl item(n:number)\nitem(1).\n.output item\n".into(), |g, s| TapeImage::encode_rr(g, s, &|| false)),
        // simdjson's kernel (decision 0434): strict JSON, and JSONC with its comments and a trailing comma.
        (GrammarDialect::Json, format!("[{}{{}}]", many("{\"k{i}\": [{i}, \"s\\n{i}\", true, null, {}, []], \"\": \"\"},\n")), |g, s| lean(json_simdjson::encode(g, s, &|| false))),
        (GrammarDialect::Json, format!("// head\n[{}1,]", many("/* {i} */ {\"k\": {i}, // c\n\"v\": {/* in */}},\n")), |g, s| lean(json_simdjson::encode(g, s, &|| false))),
    ];
    let mut direct = Vec::new();
    for (lang, source, encode) in cases {
        eprintln!("{lang:?}: {} bytes", source.len());
        let grammar = Grammar::new(lang).unwrap();
        let reference = as_run(Format::F16, || encode(&grammar, &source))
            .unwrap_or_else(|e| panic!("{lang:?}: {e}"));
        assert_eq!(reference.format(), Format::F16, "{lang:?}");
        {
            let format = Format::F16Z;
            let image = as_run(format, || encode(&grammar, &source)).unwrap();
            let at = format!("{lang:?} in {}", format.name());
            assert_eq!(image.format(), format, "{at}");
            direct.push(at.clone());
            assert_eq!(image.source_digest, reference.source_digest, "{at}");
            // 16z's pieces are in the order its blocks were finished, which follows the writer's commits: the same
            // blocks, read node by node below.
            // The tape read in a batch of its own: a 16z tape's blocks decoded into a cache made for it.
            let cache = image.own_cache();
            let (a, b) = (reference.tape(), image.tape_in(cache.as_ref()));
            rerail_tape_format::with_tape!(a, |a| rerail_tape_format::with_tape!(b, |b| {
                assert_eq!(a.len(), b.len(), "{at}");
                for i in 0..a.len() {
                    assert_eq!(
                        a.table().entry(a.code(i)),
                        b.table().entry(b.code(i)),
                        "{at}: code of {i}"
                    );
                    assert_eq!(
                        (a.start(i), a.end(i), a.subtree_end(i), a.parent(i)),
                        (b.start(i), b.end(i), b.subtree_end(i), b.parent(i)),
                        "{at}: node {i}"
                    );
                }
            }));
        }
    }
    assert!(
        direct.iter().any(|d| d.starts_with("Python")),
        "the tables cover Python: {direct:?}"
    );
    eprintln!("written directly: {direct:?}");
}

/// Decision 0434 §2 (step 3): tree-sitter's cursor walk, in a run of format 16 or 16z, writes that format by format
/// 16's ops as it walks (`formats::TreeOps`). The tape restored is the tree's: each node it keeps, in preorder, with
/// its kind and span, extras and error nodes among them.
#[test]
fn the_cursor_walk_writes_each_format_directly() {
    use super::formats::{Format, as_run};
    use ast_grep_language::LanguageExt;
    let python: String = (0..300).map(|i| format!("@d\nclass C{i}(B):\n    def f(self, x={i}):  # c\n        return [y for y in x if y]\n")).collect();
    let cases: Vec<(GrammarDialect, String)> = vec![
        (GrammarDialect::Python, python),
        (GrammarDialect::Python, "def f(:\n    pass\n".into()),
        (GrammarDialect::JavaScript, "const x = (a) => a?.b ?? 1; // c\nfunction f(y) { return <div>{y}</div>; }\n".into()),
        (GrammarDialect::TypeScript, "interface A { x?: number }\nconst x: A = {};\n".into()),
        (GrammarDialect::Rust, "fn main() { let x = Some(1); if let Some(y) = x { println!(\"{y}\"); } }\n".into()),
        (GrammarDialect::Go, "package main\nfunc main() { x := 1; _ = x }\n".into()),
        (GrammarDialect::Java, "class A { void f() { int x = 1; } }\n".into()),
        (GrammarDialect::Php, "<?php function f($x) { return $x + 1; }\n".into()),
        (GrammarDialect::Css, "a { color: red; }\n".into()),
        (GrammarDialect::CSharp, "class A { void F() { var x = 1; } }\n".into()),
        (GrammarDialect::Ruby, "def f(x)\n  x + 1\nend\n".into()),
    ];
    let mut direct = Vec::new();
    for (lang, source) in cases {
        let grammar = Grammar::new(lang).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.get_ts_language()).unwrap();
        let tree = parser.parse(&source, None).unwrap();
        // A grammar the store keeps no tapes of (Ruby's): none is made.
        if crate::code_rule_dsl::language::tape_tag(lang).is_none() {
            continue;
        }
        let encode = || TapeImage::encode_tree_image(tree.root_node(), &grammar, &source, None, &|| false);
        let native = AstGrep::doc(StrDoc::new(&source, lang));
        let expected = stored(&native);
        for format in [Format::F16, Format::F16Z] {
            let image = as_run(format, encode).unwrap_or_else(|e| panic!("{lang:?}: {e}"));
            let at = format!("{lang:?} in {}", format.name());
            assert_eq!(image.format(), format, "{at}: written in the run's format");
            let tape = AstGrep::doc(TapeDoc::restore(Arc::new(source.clone().into()), Arc::clone(&grammar), &image, &|| false).unwrap());
            let actual = tape.root().dfs().collect::<Vec<_>>();
            assert_eq!(expected.len(), actual.len(), "{at}: nodes");
            for (native, restored) in expected.iter().zip(&actual) {
                assert_eq!(
                    (restored.kind_id(), restored.kind(), restored.is_named(), restored.range()),
                    (native.kind_id(), native.kind(), native.is_named(), native.range()),
                    "{at}"
                );
            }
            direct.push(at);
        }
    }
    assert!(direct.iter().any(|d| d.starts_with("Python")), "the tables cover Python: {direct:?}");
    eprintln!("written directly: {direct:?}");
}

/// Decision 0434, R12: in a run of format 16 or 16z, every parser writes the run's format itself (the lean parsers
/// through their `Sink`, tree-sitter's cursor walk, Blade's Forte tree, the format strings' trees and the embedded
/// programs' documents through `TapeImage::walked`): each of a file's documents is in the run's format. JSON's simdjson
/// kernels give their walk's events to the ops, and tier 2's words call them (0434 step 3).
#[test]
fn no_intermediate_tape() {
    use super::formats::{Format, as_run};
    let cases: Vec<(GrammarDialect, &str)> = vec![
        (GrammarDialect::Python, "def f(x):\n    return '%s' % x  # c\n"),
        (GrammarDialect::JavaScript, "const x = (a) => a?.b ?? 1; // eslint-disable-line no-x\n"),
        (GrammarDialect::TypeScript, "interface A { x?: number }\nconst x: A = {};\n"),
        (GrammarDialect::Tsx, "const A = () => <div title=\"x\">text</div>;\n"),
        (GrammarDialect::Rust, "fn main() { println!(\"{}\", 1); }\n"),
        (GrammarDialect::Php, "<?php function f($x) { return sprintf('%d', $x); }\n"),
        (GrammarDialect::Blade, "<div>{{ $x }}</div>\n@if($y)\n<p>{!! $z !!}</p>\n@endif\n"),
        (GrammarDialect::Sql, "SELECT a FROM t;\n"),
        (GrammarDialect::Vue, "<template><div>{{ value }}</div></template>\n<script>\nlet value = 1;\n</script>\n"),
        (GrammarDialect::Rr, ".decl item(n:number)\nitem(1).\n"),
        (GrammarDialect::Json, "{\"a\": [1, \"x\\n\", true, null], \"b\": {}}\n"),
        (GrammarDialect::Json, "{\"a\": 1, // c\n}\n"),
    ];
    for format in [Format::F16, Format::F16Z] {
        for &(lang, source) in &cases {
            let grammar = Grammar::new(lang).unwrap();
            let at = format!("{lang:?} in {}", format.name());
            eprintln!("{at}");
            let image = as_run(format, || TapeImage::parse(&grammar, source, &|| false)).unwrap().unwrap();
            let docs = std::iter::once((&image.primary, lang)).chain(image.derived.iter().map(|d| (&d.tape, d.dialect())));
            for (tape, dialect) in docs {
                assert_eq!(tape.format(), format, "{at}: its {dialect:?} document");
            }
        }
    }
}

/// The reads written once in CLIF (decision 0310), JIT-compiled whole, read a document's tape where it is held as
/// the document reads it, in every format a run may hold: format 16's arrays, and a 16z tape's blocks as stored, a block decoded into the reader's slot where a read falls (0353 step
/// 4b). Every node's code (through the device's codes: kind,
/// field, flags), parent, next sibling, children, subtree and span; one document past a side-list part (1,024
/// nodes), with gaps and lengths past a byte.
#[test]
fn the_clif_reads_read_every_format_as_the_document_does() {
    use crate::code_rule_dsl::tape::formats::{Format, as_run};
    const NONE: u32 = u32::MAX;
    let reads = rerail_clif::jit::Reads::new();
    let long = format!("# {}\n", "x".repeat(300))
        + &(0..300)
            .map(|i| {
                format!(
                    "v{i} = [{i}, \"{}\", f(a, b=c)]  # {}\n",
                    "s".repeat(i % 40),
                    "c".repeat(260 * usize::from(i % 50 == 0))
                )
            })
            .collect::<String>();
    let sources = [
        (
            "value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n"
                .to_owned(),
            GrammarDialect::Python,
        ),
        (
            "const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n".to_owned(),
            GrammarDialect::JavaScript,
        ),
        (
            "def broken(:\n    pass\n".to_owned(),
            GrammarDialect::Python,
        ),
        (long, GrammarDialect::Python),
    ];
    for format in Format::ALL {
        for (source, lang) in &sources {
            let native = StrDoc::new(source, *lang);
            let grammar = Grammar::new(*lang).unwrap();
            let doc = as_run(format, || {
                let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
                TapeDoc::restore(Arc::new(source.clone().into()), grammar, &image, &|| false).unwrap()
            });
            assert_eq!(
                doc.doc_tape().image().format(),
                format,
                "held as {format:?}"
            );
            // A 16z tape read in a batch of its own, the CLIF reads and the document's: its blocks decoded into a
            // cache made for it.
            let doc = doc.alone();
            let (tape, view) = doc.doc_tape().in_place().unwrap();
            let view = &doc.doc_tape().cache().map_or(view, |c| c.view_for(&view, tape.as_ptr()));
            let codes = resident::doc_codes(&doc).unwrap();
            let none = |v: Option<u32>| v.unwrap_or(NONE);
            let n = doc.transducer_len() as u32;
            assert_eq!(view.nodes, n);
            for i in 0..n {
                let at = |what: &str| format!("{what} of {i} ({format:?}, {lang:?})");
                let code = codes[reads.one("tape_number", tape, view, i as usize) as usize];
                assert_eq!(
                    (code.kind, code.field, code.flags),
                    (
                        doc.transducer_kind(i),
                        doc.transducer_field(i),
                        doc.transducer_flags(i)
                    ),
                    "{}",
                    at("code")
                );
                let end = doc.transducer_descendants(i).end;
                assert_eq!(
                    reads.one("tape_end_of", tape, view, i as usize),
                    end,
                    "{}",
                    at("subtree")
                );
                let span = doc.transducer_span(i);
                assert_eq!(
                    reads.one("tape_start", tape, view, i as usize) as usize,
                    span.start,
                    "{}",
                    at("start")
                );
                assert_eq!(
                    reads.one("tape_end", tape, view, i as usize) as usize,
                    span.end,
                    "{}",
                    at("end")
                );
                assert_eq!(
                    reads.one("tape_parent", tape, view, i as usize),
                    none(doc.transducer_parent(i)),
                    "{}",
                    at("parent")
                );
                assert_eq!(
                    reads.one("tape_next", tape, view, i as usize),
                    none(doc.transducer_next_sibling(i, false)),
                    "{}",
                    at("next")
                );
                let kids = doc.transducer_children(i).collect::<Vec<_>>();
                assert_eq!(
                    reads.one("tape_child_count", tape, view, i as usize) as usize,
                    kids.len(),
                    "{}",
                    at("children")
                );
                assert_eq!(
                    reads.one("tape_first_child", tape, view, i as usize),
                    none(kids.first().copied()),
                    "{}",
                    at("first child")
                );
                for (k, &kid) in kids.iter().enumerate().take(8) {
                    assert_eq!(
                        reads.two("tape_child", tape, view, i as usize, k as u32),
                        kid,
                        "{}",
                        at("a child")
                    );
                }
            }
        }
    }
}

/// A grammar's encoder of a text, stopped where its callback says: its tape, or why it refused the text.
type Encode<E> = fn(&Grammar, &str, &dyn Fn() -> bool) -> Result<Result<TapeImage, String>, E>;

/// `dialect`'s tapes from its lean parser (`encode`) are tree-sitter's, byte for byte, on `sources`;
/// each of `invalid` (a text tree-sitter reads with an error) the lean parser refuses. And the product
/// reads the dialect with the lean parser: a file's image through the engine's parse is the lean
/// parser's; a file it refuses goes to tree-sitter.
fn lean_tapes_are_tree_sitters<E: std::fmt::Debug>(
    dialect: GrammarDialect,
    encode: Encode<E>,
    sources: &[&str],
    invalid: &[&str],
) {
    let grammar = Grammar::new(dialect).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&dialect.get_ts_language()).unwrap();
    for &source in sources {
        let ours = encode(&grammar, source, &|| false).unwrap_or_else(|why| panic!("refused {source:?}: {why:?}")).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error(), "tree-sitter's tree of {source:?} has an error");
        let theirs = TapeImage::encode(tree.root_node(), &grammar, source, &|| false).unwrap().primary;
        assert!(
            ours.to_bytes().unwrap() == theirs.to_bytes().unwrap(),
            "the tapes of {source:?} differ:\n{}\n{}",
            python_tape_plain(&ours, &grammar),
            python_tape_plain(&theirs, &grammar)
        );
    }
    // Any syntax error: refused, for tree-sitter, which recovers as it recovers.
    for &text in invalid {
        assert!(parser.parse(text, None).unwrap().root_node().has_error(), "{text:?} reads without an error");
        assert!(encode(&grammar, text, &|| false).is_err(), "accepted {text:?}");
    }
    // The product's route: the lean parser where it reads the file, tree-sitter where it refuses.
    assert!(tree_parser(dialect).is_some(), "{dialect:?}'s parser is installed");
    let source = sources.last().expect("a source");
    let product = TapeImage::parse(&grammar, source, &|| false).unwrap().unwrap();
    let lean = encode(&grammar, source, &|| false).unwrap().unwrap();
    assert_eq!(product.primary.to_bytes().unwrap(), lean.to_bytes().unwrap());
    let refused = invalid.first().expect("an invalid text");
    assert!(TapeImage::parse(&grammar, refused, &|| false).unwrap().is_ok());
}

/// Kotlin's tapes from the lean parser (the product's parser for Kotlin, decision 0326) are
/// tree-sitter-kotlin's; the corpus comparison is `rerail-kotlin-lean`'s `oracle`.
#[test]
fn kotlin_tapes_from_the_lean_parser_are_tree_sitters() {
    lean_tapes_are_tree_sitters(
        GrammarDialect::Kotlin,
        crate::code_rule_dsl::tape::kotlin_lean::encode,
        &[
            "",
            "// only a comment\n/* block */\nclass A\n",
            "package a.b\n\nimport c.d.*\n\nfun f(x: Int): Int = x + 1\n",
            "class A<T>(val x: T) : B(), C {\n    fun g() {\n        val y = listOf(1, 2).map { it * 2 }\n        println(\"$y ${x}\")\n    }\n}\n",
            "fun h(a: Any?) = when (a) {\n    is String -> a.length\n    null -> 0\n    else -> 1\n}\n",
            "object O {\n    const val N = 0x1F\n    var s: String? = null\n    fun k() = s?.length ?: -1\n}\n",
        ],
        &["class A {\n", "fun f( {\n", "val x = (1,\n"],
    );
}

/// Swift's tapes from the lean parser (the product's parser for Swift, decision 0326) are
/// tree-sitter-swift's; the corpus comparison is `rerail-swift-lean`'s `oracle`.
#[test]
fn swift_tapes_from_the_lean_parser_are_tree_sitters() {
    lean_tapes_are_tree_sitters(
        GrammarDialect::Swift,
        crate::code_rule_dsl::tape::swift_lean::encode,
        &[
            "",
            "// only a comment\nlet x = 1\n",
            "import Foundation\n\nstruct S<T>: P where T: Q {\n    var a: Int = 0\n    func f(_ x: Int) -> Int { return x + 1 }\n}\n",
            "let c = xs.map { $0 * 2 }.filter { x in x > 1 }\n",
            "enum E { case a, b(Int) }\nswitch e {\ncase .a: break\ncase .b(let n): print(\"\\(n)\")\n}\n",
            "class C {\n    init?() {\n        guard let y = z else { return nil }\n    }\n}\n",
        ],
        &["class A {\n", "func f( {\n", "let x = (1,\n"],
    );
}

/// Scala's tapes from the lean parser (the product's parser for Scala, decision 0326) are
/// tree-sitter-scala's; the corpus comparison is `rerail-scala-lean`'s `oracle`.
#[test]
fn scala_tapes_from_the_lean_parser_are_tree_sitters() {
    lean_tapes_are_tree_sitters(
        GrammarDialect::Scala,
        crate::code_rule_dsl::tape::scala_lean::encode,
        &[
            "",
            "// only a comment\nobject A\n",
            "package a.b\n\nimport c.d._\n\nclass C[T](x: T) extends D with E {\n  def f(y: Int): Int = y + 1\n}\n",
            "val xs = List(1, 2).map(_ * 2).filter { x => x > 1 }\n",
            "def g(a: Any) = a match {\n  case s: String => s.length\n  case _ => 0\n}\n",
            "trait T {\n  val s = s\"$x ${y + 1}\"\n  lazy val n = 0x1FL\n}\n",
            // A constructor annotation's `(`: id 40, named `arguments`, under its contextual alias `(` (decision 0631).
            "class A @Inject() (x: Int)\n",
        ],
        &["class A {\n", "def f( = 1\n", "val x = (1,\n"],
    );
}

/// A node reached without its parent (an anchor's, a binary search's: `TapeDoc::node`; a walk's, `dfs`) reads it from
/// the parents the document keeps once one is asked (`DocTape::parents`), not from a search of the tape a step: every
/// node's parent and climb are the tree's (the bytes kept are the format's).
/// PR #508's 765ed3f2e (t47).
#[test]
fn a_node_reached_without_its_parent_reads_the_kept_parents() {
    use ast_grep_core::source::SgNode as _;
    let source = format!(
        "class C:\n{}",
        "    def m(self, a):\n        return [a + 1 for b in a if b]\n".repeat(200)
    );
    const NONE: u32 = u32::MAX;
    let (_, tape, _, _) = fixture(&source, GrammarDialect::Python);
    // The tree's parents, from the subtrees open around each node: a pass that keeps none.
    let parents: Vec<u32> = tape
        .transducer_parents()
        .map(|parent| parent.unwrap_or(NONE))
        .collect();
    let id = |node: TapeNode<'_>| node.node_id() as u32;
    let walked: Vec<TapeNode<'_>> = tape.root_node().dfs().collect();
    assert_eq!(
        walked.iter().map(|&node| id(node)).collect::<Vec<_>>(),
        (0..parents.len() as u32).collect::<Vec<_>>()
    );
    for (index, &up) in parents.iter().enumerate() {
        let index = index as u32;
        for node in [tape.node(index), walked[index as usize]] {
            assert_eq!(node.parent().map_or(NONE, id), up, "parent of {index}");
            let mut climb = Vec::new();
            let mut at = up;
            while at != NONE {
                climb.push(at);
                at = parents[at as usize];
            }
            assert_eq!(
                node.ancestors(tape.root_node()).map(id).collect::<Vec<_>>(),
                climb,
                "ancestors of {index}"
            );
        }
        assert_eq!(
            tape.transducer_parent(index),
            (up != NONE).then_some(up),
            "transducer parent of {index}"
        );
    }
}


/// Decision 0466 R2 against tree-sitter: a `type` fragment's tape is tree-sitter-python's `type` node of the same text
/// written after `x: ` (positions less the prefix's bytes, its field gone), unions too, where the lean parser
/// refuses such a file and tree-sitter reads it.
#[test]
fn a_type_fragment_is_tree_sitters_written_annotation() {
    use crate::code_rule_dsl::tape::python_lean;
    use rerail_tape_format::tape::Tape as _;
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&GrammarDialect::Python.get_ts_language()).unwrap();
    // A tape's nodes from `root` on, in either format: depth, kind, field, flags, classes and span, the span less
    // `shift`, the root's field left out.
    // Each image is read in a batch of its own (a 16z tape's blocks decoded into a cache made for it, 0628 §6).
    let plain = |image: &TapeImage, root: usize, shift: u64| -> String {
        let cache = image.own_cache();
        let t = image.tape_in(cache.as_ref());
        let end = t.subtree_end(root);
        let mut open: Vec<usize> = Vec::new();
        let mut out = String::new();
        for i in root..end {
            while open.last().is_some_and(|&e| e <= i) {
                open.pop();
            }
            let e = t.table().entry(t.code(i));
            let field = if i == root || e.field == 0 { String::new() } else { format!(" {}:", grammar.field_name(e.field).unwrap_or("?")) };
            let classes: Vec<&str> = e.classes.iter().map(|&c| grammar.kind_name(c).unwrap_or("?")).collect();
            out.push_str(&format!("{}{}{field} {} {classes:?} [{}..{}]\n", "  ".repeat(open.len()), grammar.kind_name(e.kind).unwrap_or("?"), e.flags, t.start(i) - shift, t.end(i) - shift));
            open.push(t.subtree_end(i));
        }
        out
    };
    for text in ["int", "List[int]", "Dict[str, List[int]]", "a.b.C", "Optional['A']", "Callable[[int], str]", "int | None", "List[int] | None", "Optional[A] | B[C]", "Literal[1, 'a']", "*Ts", "x: y"] {
        let ours = python_lean::encode_from(&grammar, text, "type", &|| false).unwrap_or_else(|e| panic!("{text:?}: {e:?}")).unwrap();
        let src = format!("x: {text}\n");
        let tree = parser.parse(&src, None).unwrap();
        assert!(!tree.root_node().has_error(), "{src:?}");
        let theirs = TapeImage::encode(tree.root_node(), &grammar, &src, &|| false).unwrap().primary;
        let theirs_cache = theirs.own_cache();
        let t = theirs.tape_in(theirs_cache.as_ref());
        let at = (0..t.len()).find(|&i| grammar.kind_name(t.table().entry(t.code(i)).kind) == Some("type")).expect("the annotation's type");
        assert_eq!(plain(&ours, 0, 0), plain(&theirs, at, 3), "{text:?}");
        let ours_cache = ours.own_cache();
        let o = ours.tape_in(ours_cache.as_ref());
        assert_eq!(o.subtree_end(0), o.len(), "{text:?}: one root");
    }
}

/// Decision 0466 R4: every fragment the registry declares starts at a node kind its dialect's hand-written parser
/// parses from (`TreeParser::starts`): no fragment is left to tree-sitter, which has one start rule.
#[test]
fn every_fragment_starts_where_its_parser_starts() {
    for &(name, dialect, start, _) in crate::code_rule_dsl::language::fragments() {
        if !dialect.built() {
            continue;
        }
        assert!(super::starts(dialect).contains(&start), "fragment {name}: {dialect:?}'s hand-written parser does not start at `{start}` (it starts at {:?})", super::starts(dialect));
    }
}

/// Decision 0466 R1 through the router: a JSX fragment's tape is one `jsx_element` over its text, read by the lean
/// parser; a text it refuses is one `ERROR` over it less blanks (no tree made up); TypeScript starts at no element.
#[test]
fn a_fragment_is_parsed_from_its_start_by_its_parser() {
    use rerail_tape_format::tape::Tape as _;
    let js = Grammar::new(GrammarDialect::JavaScript).unwrap();
    let root = |image: &FileImage| {
        let cache = image.primary.own_cache();
        let t = image.primary.tape_in(cache.as_ref());
        assert_eq!(t.subtree_end(0), t.len(), "one root");
        let e = t.table().entry(t.code(0));
        (if e.flags & CODE_ERROR != 0 { "ERROR" } else { js.kind_name(e.kind).unwrap() }, t.start(0), t.end(0))
    };
    let image = TapeImage::parse_from(&js, " <a>{b}</a> ", "jsx_element", &|| false).unwrap().unwrap();
    assert_eq!(root(&image), ("jsx_element", 1, 11));
    let image = TapeImage::parse_from(&js, " <a>{b} ", "jsx_element", &|| false).unwrap().unwrap();
    assert_eq!(root(&image), ("ERROR", 1, 7));
    let ts = Grammar::new(GrammarDialect::TypeScript).unwrap();
    assert!(TapeImage::parse_from(&ts, "<a></a>", "jsx_element", &|| false).is_none());
    let py = Grammar::new(GrammarDialect::Python).unwrap();
    assert!(TapeImage::parse_from(&py, "int", "expression", &|| false).is_none(), "a start no parser lists");
}

/// Decision 0466 R1 for HTML: an element alone is one `element` over its text, with no `document`, `html` or `body`.
#[test]
fn an_html_fragment_is_one_element() {
    use rerail_tape_format::tape::Tape as _;
    let html = Grammar::new(GrammarDialect::Html).unwrap();
    let image = TapeImage::parse_from(&html, "\n<li>a <b>c</b></li>\n", "element", &|| false).unwrap().unwrap();
    let cache = image.primary.own_cache();
    let t = image.primary.tape_in(cache.as_ref());
    assert_eq!(t.subtree_end(0), t.len(), "one root");
    assert_eq!((html.kind_name(t.table().entry(t.code(0)).kind), t.start(0), t.end(0)), (Some("element"), 1, 20));
}

/// A file of a dialect that embeds others is read by its lean parser, and its programs are built over that tape
/// (`parse_own`): an HTML file's scripts and styles. Before, every lean parser left such a file to tree-sitter, so
/// declaring HTML's embeds (decision 0431 step 3a) sent every HTML file there.
#[test]
fn a_file_that_embeds_code_is_read_by_its_lean_parser() {
    let html = Grammar::new(GrammarDialect::Html).unwrap();
    let parser = tree_parser(GrammarDialect::Html).expect("HTML's parser");
    let source = "<p>a</p>\n<script>let x = 1;</script>\n<style>p { color: red }</style>\n";
    let image = parse_own(parser, &html, source, &|| false).expect("the lean parser reads it").unwrap();
    assert_eq!(image.derived.len(), 2, "a program per script and style");
    let routed = TapeImage::parse(&html, source, &|| false).unwrap().unwrap();
    assert_eq!(routed.derived.len(), 2);
    assert!(routed.primary.to_bytes().unwrap() == image.primary.to_bytes().unwrap(), "the router reads it the same way");
}


fn call_argument_programs(source: &str) -> Vec<(String, usize)> {
    let grammar = Grammar::new(GrammarDialect::Python).unwrap();
    let image = TapeImage::parse(&grammar, source, &|| false).unwrap().unwrap();
    let doc = TapeDoc::restore(std::sync::Arc::new(source.to_owned().into()), grammar, &image, &|| false).unwrap();
    crate::code_rule_dsl::embed::Embedder::build_in_doc(&crate::code_rule_dsl::embed::call_argument::CallArgument, &doc, &|| false).into_iter().map(|p| (p.text.clone(), p.segments.bounds(0..1).start)).collect()
}

/// Decision 0470 R2, R3: a string at a declared place is a program over its content, at its bytes; the callee is
/// matched as the file resolves it.
#[test]
fn call_argument_a_string_at_a_place_is_a_program_at_its_bytes() {
    let src = "from markupsafe import Markup as M\nimport markupsafe\nx = M(\"<b>a</b>\")\ny = markupsafe.Markup('<i>b</i>')\n";
    assert_eq!(call_argument_programs(src), [("<b>a</b>".to_owned(), 60), ("<i>b</i>".to_owned(), 94)]);
    let src = "from django.utils.html import format_html\nformat_html('<a href=\"{}\">{}</a>', '<p>not</p>')\n";
    assert_eq!(call_argument_programs(src).iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["<a href=\"{}\">{}</a>"], "argument 0 only");
    let src = "from django.utils.safestring import mark_safe\nmark_safe(\"<p>\" \"x</p>\")\n";
    assert_eq!(call_argument_programs(src).iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["<p>x</p>"], "a concatenation is one program");
}

/// R3: a name the file binds otherwise, or never imports, is not the place's callee.
#[test]
fn call_argument_a_callee_the_file_does_not_import_is_no_place() {
    for src in [
        "def Markup(s): return s\nMarkup('<b>a</b>')\n",
        "Markup('<b>a</b>')\n",
        "from other import Markup\nMarkup('<b>a</b>')\n",
        "from markupsafe import Markup\nMarkup = str\nMarkup('<b>a</b>')\n",
    ] {
        assert_eq!(call_argument_programs(src), [], "{src:?}");
    }
}

/// R4: only a plain string is a program.
#[test]
fn call_argument_only_a_plain_string_is_a_program() {
    for arg in ["f\"<b>{x}</b>\"", "b\"<b>x</b>\"", "name", "\"<b>\\n</b>\"", "\"<b>\" + x", "*args"] {
        let src = format!("from markupsafe import Markup\nMarkup({arg})\n");
        assert_eq!(call_argument_programs(&src), [], "{arg}");
    }
    assert_eq!(call_argument_programs("from markupsafe import Markup\nMarkup(r\"<b>\\d</b>\")\n").len(), 1, "a raw string's bytes are its value");
}
