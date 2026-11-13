use super::*;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use ast_grep_core::AstGrep;
use ast_grep_core::matcher::Pattern;
use ast_grep_core::tree_sitter::StrDoc;

#[test]
fn tape_v8_persists_abstract_tree_sitter_supertypes() {
    let source = "value = left + right\n";
    let (_, tape, image, grammar) = fixture(source, GrammarDialect::Python);
    let bytes = image.to_bytes().unwrap();
    assert_eq!(&bytes[..8], b"TMPTAP9N");
    assert_eq!(TAPE_FORMAT_VERSION, 11);

    let memberships = tape.transducer_supertypes().collect::<Vec<_>>();
    assert!(!memberships.is_empty());
    assert!(memberships.iter().any(|&(node, kind)| {
        tape.transducer_has_supertype(node, kind) && grammar.kind_name(kind) == Some("expression")
    }));

    let persisted = TapeImage::from_bytes(&bytes, &grammar).unwrap();
    let restored =
        TapeDoc::restore(Arc::new(source.to_owned()), Arc::clone(&grammar), &persisted, &|| false).unwrap();
    assert_eq!(
        restored.transducer_supertypes().collect::<Vec<_>>(),
        memberships
    );

    // Memberships live in the codes: corrupt ones are rejected, in memory and in bytes.
    let source = Arc::new(source.to_owned());
    let reject = |candidate: &TapeImage| {
        assert!(TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), candidate, &|| false).is_err());
        assert!(TapeImage::from_bytes(&candidate.to_bytes().unwrap(), &grammar).map_or(true, |read| {
            TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &read, &|| false).is_err()
        }));
    };
    let member = image.codes.iter().position(|code| code.class_count > 0).expect("a code with memberships");
    let concrete = (0..grammar.kinds.kind_count() as u16).find(|&kind| !grammar.kinds.kind_is_supertype(kind)).unwrap();
    let mut broken = image.clone();
    broken.codes[member].classes[0] = concrete;
    reject(&broken);
    let mut broken = image.clone();
    broken.codes[member].classes[0] = u16::MAX;
    reject(&broken);
    let mut broken = image.clone();
    let class = broken.codes[member].classes[0];
    let count = usize::from(broken.codes[member].class_count);
    broken.codes[member].classes[count] = class;
    broken.codes[member].class_count += 1;
    reject(&broken);
    let mut broken = image.clone();
    broken.codes[member].class_count = MAX_NODE_SUPERTYPES as u8 + 1;
    assert!(TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &broken, &|| false).is_err());
    let mut broken = image.clone();
    broken.codes[member].classes[MAX_NODE_SUPERTYPES - 1] = class;
    assert!(TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &broken, &|| false).is_err());
    // A code table whose stated length disagrees with its codes.
    let mut corrupt = bytes.clone();
    let stated = u64::from_le_bytes(corrupt[72..80].try_into().unwrap());
    corrupt[72..80].copy_from_slice(&(stated + 2).to_le_bytes());
    assert!(TapeImage::from_bytes(&corrupt, &grammar).is_err());
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
        assert_eq!(tape.transducer_text(node), tape.transducer_text_str(node).as_bytes());
        let leaf = tape.transducer_children(node).next().is_none();
        let name = grammar.kind_name(tape.transducer_kind(node));
        if name == Some("identifier") {
            assert!(tape.transducer_is_named_leaf(node));
        }
        if !leaf {
            assert!(!tape.transducer_is_named_leaf(node));
        }
    }
    let keys: Vec<_> = calls.iter().map(|&call| tape.transducer_structural_key(call)).collect();
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
    let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false)
        .unwrap()
        .primary;
    let tape = TapeDoc::restore(
        Arc::new(source.to_owned()),
        Arc::clone(&grammar),
        &image,
        &|| false,
    )
    .unwrap();
    (native, tape, image, grammar)
}

#[test]
fn known_field_edges_match_native_first_child_without_larger_records() {
    assert_eq!(std::mem::size_of::<Rec>(), 40);
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
        for (left, right) in native.root().dfs().zip(restored.root().dfs()) {
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
    // `grammars/tree-sitter-typescript/PATCHES.md`) renumbers them; a tape format (7: one
    // document per tape; 9: v9np's columns, decision 0025; 10: memberships in the codes; 11: columns at 16 bytes,
    // decision 0032) changes
    // the bytes.
    assert_eq!(
        digest,
        "bc05feccadd8b7779a4c39a2e056f2cd821fdadf580cbf4e1ffcd4d2e60d526c"
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
    let grammar = Grammar::new(GrammarDialect::Html).unwrap();
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
    // The file's tape is its own tree alone: the header and its tables, nothing after them.
    let primary = image.primary.to_bytes().unwrap();
    let aliases = image
        .primary
        .kind_aliases
        .iter()
        .map(|(name, _)| 5 + name.len())
        .sum::<usize>();
    let tables = &image.primary;
    assert_eq!(
        primary.len(),
        HEADER_BYTES
            + padded(tables.code.len())
            + padded(tables.size.len())
            + padded(tables.base.len())
            + padded(tables.off.len())
            + padded(tables.tail.len())
            + tables.code_table_bytes()
            + aliases
    );
    assert_eq!(&primary[..8], TAPE_MAGIC);
    let read = image.round_trip(&grammar).unwrap();
    assert_eq!(read.primary.to_bytes().unwrap(), primary);
    let tape = document.tape.to_bytes().unwrap();
    assert_eq!(read.derived[0].tape.to_bytes().unwrap(), tape);
    let restored = TapeDoc::restore(
        Arc::new(source.to_owned()),
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

    let mut header = bytes.clone();
    header[9] = 1;
    assert!(TapeImage::from_bytes(&header, &grammar).is_err());

    let mut noncanonical = image.clone();
    noncanonical.codes[0].flags = 0x40;
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
    // Header count slots (v9np): codes at 64, code bytes at 72, aliases at 80, alias bytes at 88.
    for (offset, count) in [
        (80, u64::MAX >> 1),
        (80, 1 << 20),
        (80, 1 << 16 | 1),
        (64, u64::MAX >> 1),
        (64, 1 << 20),
        (72, u64::MAX >> 1),
        (88, u64::MAX >> 1),
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
    broken.code.pop();
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.tail.pop();
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.size[0..2].copy_from_slice(&0u16.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.code[0..2].copy_from_slice(&u16::MAX.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.off[0] = 0xff;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.widths[0] = 3;
    reject(&broken, Arc::clone(&grammar));
    reject(&image, Grammar::new(GrammarDialect::TypeScript).unwrap());
    let mut broken = image.clone();
    broken.source_len += 1;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.base[0] ^= 0x80;
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.tail.extend_from_slice(&[0, 0]);
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.size[0..2].copy_from_slice(&SENTINEL.to_le_bytes());
    reject(&broken, Arc::clone(&grammar));
    let mut broken = image.clone();
    broken.codes[0].alias = 7;
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
    broken.size[2..4].copy_from_slice(&2u16.to_le_bytes());
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
        assert_eq!(!image.error_nodes().is_empty(), errors, "{source}");
        assert_eq!(!image.missing_nodes().is_empty(), missing, "{source}");
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
        // A node's flags are its code's (`TapeImage::codes`).
        let code = |image: &TapeImage, node: u32| usize::from(image.code_at(node as usize));
        if let Some(&index) = image.error_nodes().first() {
            let mut broken = image.clone();
            broken.codes[code(&image, index)].flags &= !CODE_ERROR;
            reject(&broken);
            let mut broken = image.clone();
            broken.codes[code(&image, index)].flags |= CODE_MISSING;
            reject(&broken);
        }
        if let Some(&index) = image.missing_nodes().first() {
            let mut broken = image.clone();
            broken.codes[code(&image, 0)].flags |= CODE_MISSING;
            reject(&broken);
            let mut broken = image.clone();
            broken.codes[code(&image, index)].flags |= CODE_ERROR;
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
        image.primary.node_count()
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
        assert_eq!(image.base.len(), expected_count.div_ceil(16) * 4);
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
        // v9np's span columns are fixed-width per document: every length here fits 2 bytes.
        let _ = expected_width;
        assert_eq!((image.off_width(), image.tail_width()), (2, 2), "comment length {length}");
        let mut saw_comment = false;
        for (index, (a, b)) in native_nodes.iter().zip(tape_nodes.iter()).enumerate() {
            assert_eq!(
                (a.kind_id(), a.range(), a.text(), a.start_pos(), a.end_pos()),
                (b.kind_id(), b.range(), b.text(), b.start_pos(), b.end_pos()),
                "comment length {length} node {index}"
            );
            assert_eq!(image.start_at(index) as usize, a.range().start, "comment length {length} node {index}");
            if a.is_extra() {
                assert_eq!(image.tail_at(index) as usize, a.range().len(), "comment length {length}");
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
    assert_eq!(image.size_width(), 4);
    assert_eq!(
        matches(tape, "foo($A)", GrammarDialect::Python),
        matches(native, "foo($A)", GrammarDialect::Python)
    );
    let mut broken = image.clone();
    broken.size[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
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
        // The root's size is the node count, 4 bytes wide past 65,535 nodes; no parent column.
        assert_eq!(image.size_at(0) as usize, expected_count, "root size at {expected_count}");
        assert_eq!(image.size_width(), if expected_count > 65_535 { 4 } else { 2 }, "size width at {expected_count}");
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
            missing_parent.widths[0] = 2;
            assert!(
                TapeDoc::restore(
                    Arc::new(source.clone()),
                    Arc::clone(&grammar),
                    &missing_parent,
                    &|| false
                )
                .is_err(),
                "size column narrower than its width"
            );
            let mut missing_size = image.clone();
            missing_size.size[0..4].copy_from_slice(&65_535_u32.to_le_bytes());
            assert!(
                TapeDoc::restore(Arc::new(source), grammar, &missing_size, &|| false).is_err(),
                "root size short of the node count"
            );
        }
    }
}

#[test]
fn restore_rejects_noncanonical_side_tables_and_checkpoint_boundaries() {
    let source = format!("{}#é\n", "x\n".repeat(8));
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    assert_eq!(image.node_count(), 18);
    assert_eq!(image.extra_nodes().len(), 1);
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
    broken.base.pop();
    reject("truncated block base", &broken);
    let mut broken = image.clone();
    broken.base[4] ^= 0x40;
    reject("second block base", &broken);
    let mut broken = image.clone();
    broken.off[2 * 17] ^= 0x40;
    reject("second block offset", &broken);
    let mut broken = image.clone();
    broken.codes.push(broken.codes[0]);
    broken.code[0..2].copy_from_slice(&(broken.codes.len() as u16).to_le_bytes());
    reject("out-of-range code", &broken);
    let mut broken = image.clone();
    broken.widths[1] = 4;
    reject("offset column narrower than its width", &broken);
    let mut broken = image.clone();
    broken.widths[2] |= TAIL_FROM_START;
    reject("tail measured from the wrong node", &broken);
    let extra = image.extra_nodes()[0] as usize;
    let mut broken = image.clone();
    broken.codes[usize::from(image.code_at(extra))].flags |= CODE_ERROR;
    reject("extra node flagged an error of a visible kind", &broken);
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
    broken.off[0..2].copy_from_slice(&u16::MAX.to_le_bytes());
    reject("start beyond the source", &broken);
    let mut broken = image.clone();
    broken.tail[0..2].copy_from_slice(&u16::MAX.to_le_bytes());
    reject("root ends beyond the source", &broken);
    // A two-byte leaf at 0 (`é`) whose end moves inside the codepoint while the root and its
    // parent stay valid.
    let leaf = (1..image.node_count())
        .find(|&index| image.start_at(index) == 0 && image.size_at(index) == 1 && image.tail_at(index) == 2)
        .expect("a UTF-8 leaf at the start");
    let mut broken = image.clone();
    let at = leaf * image.tail_width();
    broken.tail[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
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
    // The root ends at its last descendant's start plus its tail: shorten it to 2.
    let last = image.size_at(0) as usize - 1;
    let tail = 2 - image.start_at(last).min(2);
    broken.tail[0..2].copy_from_slice(&(tail as u16).to_le_bytes());
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
        let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores");
        assert_eq!(
            tape.syntax_error_ranges(),
            Vec::<Range<usize>>::new(),
            "{source:?}"
        );
    }
}

/// The production image's stored bytes against v8's as stored (the harness's v7 encoding, which
/// is v8 without supertypes, plus v8's 6 bytes per supertype membership) on a corpus: `RERAIL_TAPE_SIZE='<lang> <dir>'`.
#[test]
#[ignore = "measurement; RERAIL_TAPE_SIZE='<lang> <dir>', --ignored --nocapture"]
fn stored_tape_bytes_against_v8() {
    let spec = std::env::var("RERAIL_TAPE_SIZE").expect("RERAIL_TAPE_SIZE='<lang> <dir>'");
    let (lang, dir) = spec.split_once(' ').expect("<lang> <dir>");
    let (docs, _, _) = super::v9np::load(lang, &[dir.to_owned()], usize::MAX);
    let (mut nodes, mut v8, mut v9, mut table, mut small_v8, mut small_v9, mut memberships) = (0, 0, 0, 0, 0, 0, 0);
    for doc in &docs {
        let dialect = crate::code_rule_dsl::language::dialect_by_label(crate::code_rule_dsl::language::label_of_path(doc.path.to_str().unwrap()).unwrap()).unwrap();
        let grammar = Grammar::new(dialect).unwrap();
        let native = StrDoc::new(&doc.source, dialect);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, &doc.source, &|| false).unwrap().primary;
        let old = super::v9np::V7::encode(&doc.nodes).unwrap().to_bytes().len() + 6 * image.membership_count();
        // v8's bytes without its supertype table; v9np's with its memberships (in the codes).
        let new = image.to_bytes().unwrap().len();
        nodes += image.node_count();
        v8 += old;
        v9 += new;
        table += image.code_table_bytes();
        memberships += image.membership_count();
        if image.node_count() < 2_000 {
            small_v8 += old;
            small_v9 += new;
        }
    }
    println!(
        "{} files, {nodes} nodes: v8 {:.3} B/node, v9np {:.3} B/node ({:.3}x), of which the code table {:.3} B/node; files under 2,000 nodes: {:.3}x; supertype memberships {memberships} ({:.3} per node; v8 stored them at {:.3} B/node)",
        docs.len(),
        v8 as f64 / nodes as f64,
        v9 as f64 / nodes as f64,
        v9 as f64 / v8 as f64,
        table as f64 / nodes as f64,
        small_v9 as f64 / small_v8.max(1) as f64,
        memberships as f64 / nodes as f64,
        6.0 * memberships as f64 / nodes as f64,
    );
}

/// Whether a node's stored supertype memberships are a lookup: by its kind alone (the grammar's
/// supertype map, transitively), or by (parent kind, field, kind) learned from the corpus.
/// `RERAIL_TAPE_SIZE='<lang> <dir>'`.
#[test]
#[ignore = "measurement; RERAIL_TAPE_SIZE='<lang> <dir>', --ignored --nocapture"]
fn supertype_memberships_against_lookups() {
    use std::collections::{BTreeSet, HashMap};
    let spec = std::env::var("RERAIL_TAPE_SIZE").expect("RERAIL_TAPE_SIZE='<lang> <dir>'");
    let (lang, dir) = spec.split_once(' ').expect("<lang> <dir>");
    let (docs, _, _) = super::v9np::load(lang, &[dir.to_owned()], usize::MAX);
    let (mut nodes, mut by_kind_equal, mut by_kind_superset) = (0_usize, 0_usize, 0_usize);
    let mut by_context: HashMap<(u16, u16, u16), HashMap<Vec<u16>, usize>> = HashMap::new();
    let mut by_kind_alone: HashMap<u16, HashMap<Vec<u16>, usize>> = HashMap::new();
    for doc in &docs {
        let dialect = crate::code_rule_dsl::language::dialect_by_label(crate::code_rule_dsl::language::label_of_path(doc.path.to_str().unwrap()).unwrap()).unwrap();
        let grammar = Grammar::new(dialect).unwrap();
        let language = grammar.language();
        // Each kind's supertypes in the grammar's map, closed over supertypes that are subtypes.
        let mut map: HashMap<u16, BTreeSet<u16>> = HashMap::new();
        for &supertype in language.supertypes() {
            let mut stack = vec![supertype];
            let mut seen = BTreeSet::new();
            while let Some(at) = stack.pop() {
                for &sub in language.subtypes_for_supertype(at) {
                    if seen.insert(sub) {
                        map.entry(sub).or_default().insert(supertype);
                        stack.push(sub);
                    }
                }
            }
        }
        let native = StrDoc::new(&doc.source, dialect);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, &doc.source, &|| false).unwrap().primary;
        let tape = TapeDoc::restore(Arc::new(doc.source.clone()), Arc::clone(&grammar), &image, &|| false).unwrap();
        let mut stored: HashMap<u32, Vec<u16>> = HashMap::new();
        for (node, kind) in tape.transducer_supertypes() {
            stored.entry(node).or_default().push(kind);
        }
        for node in 0..tape.transducer_len() as u32 {
            let kind = tape.transducer_kind(node);
            let mut set = stored.remove(&node).unwrap_or_default();
            set.sort_unstable();
            let lookup = map.get(&kind).map(|set| set.iter().copied().collect::<Vec<_>>()).unwrap_or_default();
            nodes += 1;
            by_kind_equal += usize::from(lookup == set);
            by_kind_superset += usize::from(set.iter().all(|s| lookup.contains(s)));
            let parent = tape.transducer_parent(node).map_or(u16::MAX, |p| tape.transducer_kind(p));
            *by_context.entry((parent, tape.transducer_field(node), kind)).or_default().entry(set.clone()).or_default() += 1;
            *by_kind_alone.entry(kind).or_default().entry(set).or_default() += 1;
        }
    }
    // A key decides its set where one set holds all its nodes; otherwise its minority nodes miss.
    fn misses<K>(table: &HashMap<K, HashMap<Vec<u16>, usize>>) -> usize {
        table.values().map(|sets| sets.values().sum::<usize>() - sets.values().max().unwrap()).sum()
    }
    println!(
        "{} files, {nodes} nodes: grammar map by kind equals the stored set for {:.4}%, contains it for {:.4}%; \
         learned by kind misses {} nodes ({} kinds), by (parent kind, field, kind) misses {} nodes ({} keys)",
        docs.len(),
        100.0 * by_kind_equal as f64 / nodes as f64,
        100.0 * by_kind_superset as f64 / nodes as f64,
        misses(&by_kind_alone),
        by_kind_alone.len(),
        misses(&by_context),
        by_context.len(),
    );
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
                if !matches!(name.to_str(), Some("node_modules" | ".git" | "target" | ".venv" | "venv" | "__pycache__" | "dist" | "site-packages")) {
                    stack.push(path);
                }
            } else if super::v9np::language(lang, &path).is_some() {
                files.push(path);
            }
        }
    }
    files.sort();
    let (mut nodes, mut memberships, mut documents) = (0_usize, 0_usize, 0_usize);
    let mut checked_detects = false;
    for path in &files {
        let Ok(source) = std::fs::read_to_string(path) else { continue };
        let dialect = crate::code_rule_dsl::language::dialect_by_label(crate::code_rule_dsl::language::label_of_path(path.to_str().unwrap()).unwrap()).unwrap();
        let grammar = Grammar::new(dialect).unwrap();
        let native = StrDoc::new(&source, dialect);
        let Some(encoded) = TapeImage::encode(native.tree.root_node(), &grammar, &source, &|| false).ok() else { continue };
        let image = TapeImage::from_bytes(&encoded.primary.to_bytes().unwrap(), &grammar).unwrap();
        let tape = TapeDoc::restore(Arc::new(source.clone()), Arc::clone(&grammar), &image, &|| false).unwrap();
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
        assert_eq!(tape.transducer_len(), native_nodes.len(), "{path}: node count");
        assert_eq!(tape.transducer_supertypes().collect::<Vec<_>>(), expected, "{path}: memberships");
        for (index, ((node, field), parent)) in native_nodes.iter().zip(&parent_of).enumerate() {
            let at = index as u32;
            let kind = if node.is_error() { ERROR_SYMBOL } else { node.kind_id() };
            assert_eq!(
                (tape.transducer_kind(at), tape.transducer_field(at), tape.transducer_span(at), tape.transducer_parent(at)),
                (kind, *field, node.byte_range(), parent.map(|p| p as u32)),
                "{path}: node {index}"
            );
            let flags = tape.node(at);
            assert_eq!(
                (flags.is_named(), flags.is_extra(), flags.is_error(), flags.is_missing()),
                (node.is_named(), node.is_extra(), node.is_error(), node.is_missing()),
                "{path}: node {index} flags"
            );
        }
        // The comparison is not vacuous: one code's memberships dropped makes it fail.
        if !checked_detects {
            if let Some(member) = image.codes.iter().position(|code| code.class_count > 0) {
                let mut altered = image.clone();
                let code = &mut altered.codes[member];
                code.class_count -= 1;
                code.classes[usize::from(code.class_count)] = 0;
                let restored = TapeDoc::restore(Arc::new(source.clone()), Arc::clone(&grammar), &altered, &|| false).unwrap();
                assert_ne!(restored.transducer_supertypes().collect::<Vec<_>>(), expected, "{path}: altered memberships go unnoticed");
                checked_detects = true;
            }
        }
        nodes += native_nodes.len();
        memberships += expected.len();
        documents += 1;
    }
    assert!(documents > 0 && checked_detects);
    println!("{documents} documents, {nodes} nodes, {memberships} memberships: all equal the cursor's");
}

#[test]
fn direct_facts_reject_corruption_through_the_shared_admission_contract() {
    use crate::code_rule_dsl::facts::core;
    let source = Arc::new("é = foo(1)\n".to_owned());
    let (_, _, image, grammar) = fixture(&source, GrammarDialect::Python);
    let lines = Arc::new(LineTable::new(&source, Terminators::of(GrammarDialect::Python)));
    let reject = |image: &TapeImage| {
        assert!(TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), image, &|| false).is_err());
        assert!(core::from_image(Arc::clone(&source), &grammar, image, Arc::clone(&lines)).is_err());
    };
    let mut corrupt = image.clone(); corrupt.source_digest[0] ^= 1; reject(&corrupt);
    let mut corrupt = image.clone(); corrupt.code.pop(); reject(&corrupt);
    let mut corrupt = image.clone(); corrupt.size[0..2].copy_from_slice(&0u16.to_le_bytes()); reject(&corrupt);
    let mut corrupt = image.clone(); corrupt.codes[0].alias = SENTINEL - 1; reject(&corrupt);
    let mut corrupt = image.clone(); corrupt.codes[0].class_count = MAX_NODE_SUPERTYPES as u8 + 1; reject(&corrupt);
    let mut corrupt = image.clone(); corrupt.tail[0..2].copy_from_slice(&u16::MAX.to_le_bytes()); reject(&corrupt);
    let leaf = (1..image.node_count()).find(|&i| image.start_at(i) == 0 && image.size_at(i) == 1 && image.tail_at(i) == 2).unwrap();
    let mut corrupt = image.clone(); corrupt.tail[leaf*2..leaf*2+2].copy_from_slice(&1u16.to_le_bytes()); reject(&corrupt);
    // A malformed nested size whose byte span still fits cannot escape its parent's subtree.
    let (_, _, nested, ng) = fixture("f(g(x))\n", GrammarDialect::Python);
    let mut corrupt = nested.clone();
    corrupt.size[2..4].copy_from_slice(&2u16.to_le_bytes());
    assert!(core::from_image(Arc::new("f(g(x))\n".into()), &ng, &corrupt, Arc::new(LineTable::new("f(g(x))\n", Terminators::Newlines))).is_err());
}

/// The lean `.rr` parser writes the engine's tape (decision 0140): its bytes are the engine's
/// image of every `.rr` file in the repository, and of a prefix of each (an unfinished file).
/// The language tag it writes is the registry's.
#[test]
fn lean_rr_tape_is_the_engines_image() {
    assert_eq!(crate::code_rule_dsl::language::tape_tag(GrammarDialect::Rr), Some(rerail_tape_format::RR_TAG));
    assert_eq!(super::wire::CHECKPOINT_STRIDE, rerail_tape_format::CHECKPOINT_STRIDE);
    let grammar = Grammar::new(GrammarDialect::Rr).expect("grammar");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (mut files, mut work) = (Vec::new(), vec![root]);
    while let Some(dir) = work.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let skip = p.file_name().is_some_and(|n| [".git", "target", "node_modules"].contains(&n.to_string_lossy().as_ref()));
            if p.is_dir() && !skip {
                work.push(p);
            } else if p.extension().is_some_and(|x| x == "rr") {
                files.extend(std::fs::read_to_string(&p).ok().map(|t| (p.display().to_string(), t)));
            }
        }
    }
    assert!(files.len() > 1000, "{} files", files.len());
    // And tapes whose columns need 4 bytes, which the lean parser parses again at them.
    let long = "x".repeat(70_000);
    files.push(("a 70,000-byte string".into(), format!("r(\"{long}\").\n")));
    files.push(("a 70,000-byte comment".into(), format!("r(1).\n/* {long} */\nr(2).\nr(3).\n")));
    files.push(("20,000 facts".into(), "r(1).\n".repeat(20_000)));
    for (path, text) in &files {
        let mut half = text.len() / 2;
        while !text.is_char_boundary(half) {
            half -= 1;
        }
        for src in [text.as_str(), &text[..half]] {
            let engine = TapeImage::encode_rr(&grammar, src, &|| false).expect("encoded").to_bytes().expect("bytes");
            let lean = rerail_rr_syntax::lean::tape(src);
            if lean != engine {
                let k = lean.iter().zip(&engine).position(|(a, b)| a != b).unwrap_or(lean.len().min(engine.len()));
                panic!("{path} ({} bytes): tapes of {} / {} bytes differ from byte {k}: {}", src.len(), lean.len(), engine.len(), rerail_tape_format::place(&engine, k));
            }
        }
    }
}
