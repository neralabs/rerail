use super::*;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};

/// A file as facts read it, with no request around it.
struct Text(String, Arc<String>);

impl SourceFile for Text {
    fn language(&self) -> &str {
        &self.0
    }
    fn source(&self) -> &Arc<String> {
        &self.1
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn facts(path: &str, language: &str, source: &str) -> Facts {
    let file = Text(language.to_owned(), Arc::new(source.to_owned()));
    match build(&file).unwrap() {
        Built::Facts(facts) => facts,
        Built::Unparsed => panic!("{path} did not parse"),
    }
}

fn atom(language: &str, kind: &str) -> u32 {
    let lang: GrammarDialect = dialect_by_label(language).unwrap();
    kind_atom(tape_tag(lang).unwrap(), lang.kinds().kind_id(kind))
}

#[test]
fn columns_are_a_preorder_tree_with_children_and_kind_postings() {
    let f = facts("a.py", "python", "value = left + right\nprint(value)\n");
    assert_eq!(f.parent_or_none(0 as u32), NONE);
    for id in 1..f.len() as u32 {
        let parent = f.parent(id).unwrap();
        assert!(parent < id, "a parent precedes its children in preorder");
        assert!(f.children(parent).contains(&id));
        assert!(id < f.subtree_end(parent));
        assert!(f.start(parent) <= f.start(id) && f.end(id) <= f.end(parent));
    }
    let identifiers = f.of_kind(atom("python", "identifier"));
    assert_eq!(
        identifiers.iter().map(|&id| f.text(id)).collect::<Vec<_>>(),
        ["value", "left", "right", "print", "value"]
    );
    assert_eq!(f.start_line(*identifiers.last().unwrap()), 2);
    assert!(f.of_kind(atom("python", "binary_operator")).iter().all(|&id| f.named(id)));
    assert!(!f.syntax_errors());
    assert!(f.depth() >= 4);
}

#[test]
fn abstract_classes_post_the_nodes_the_tape_records() {
    let f = facts("a.py", "python", "value = left + right\n");
    let expressions = f.of_class(atom("python", "expression"));
    let binary = f.of_kind(atom("python", "binary_operator"));
    assert_eq!(binary.len(), 1);
    assert!(expressions.contains(&binary[0]));
    // A class is not a concrete kind: no node has it as its own kind.
    assert!(f.of_kind(atom("python", "expression")).is_empty());
}

#[test]
fn every_dialect_reads_its_own_grammar_and_kinds_never_collide() {
    let python = facts("a.py", "python", "f(x)\n");
    let typescript = facts("a.ts", "typescript", "f(x);\n");
    assert_eq!(typescript.of_kind(atom("typescript", "call_expression")).len(), 1);
    assert_eq!(python.of_kind(atom("python", "call")).len(), 1);
    let (p, t) = (python.kind(0) / KIND_STRIDE, typescript.kind(0) / KIND_STRIDE);
    assert_ne!(p, t);
    assert!((0..python.len() as u32).all(|id| python.kind(id) / KIND_STRIDE == p));
}

#[test]
fn the_digest_is_of_the_content_not_the_path() {
    let a = digest(&facts("a.py", "python", "x = 1\n"));
    assert_eq!(a, digest(&facts("b.py", "python", "x = 1\n")));
    assert_ne!(a, digest(&facts("a.py", "python", "x  = 1\n")));
    // Same kinds and spans, other text: a reader of `text` sees the difference, so the digest does.
    assert_ne!(a, digest(&facts("a.py", "python", "y = 2\n")));
}

/// The facts read the same tree in every format (decision 0293, step 4), whether held by a restored document or by
/// an image alone: every node's answers, the kind and class scans, and the lines.
#[test]
fn every_format_reads_the_same_facts_off_the_tape() {
    use crate::code_rule_dsl::{lines::{LineTable, Terminators}, tape::{Grammar, TapeDoc, TapeImage, formats::Format}};
    for (label, source) in [
        ("python", "é = f(1)\r\nclass A:\r def f(self):\n  return 'x'\n"),
        ("python", "def broken(:\n    pass\n"),
        ("python", ""),
        ("javascript", "const x = (a) => a?.b ?? `x${a}`; // c\n"),
        ("typescript", "interface A { x?: number }; const x: A = {};\n"),
        ("rust", "fn main() { let x = Some(1); if let Some(y) = x { println!(\"{y}\"); } }"),
        ("php", "<?php function f($x) { return $x + 1; }\n"),
        ("sql", "SELECT a, count(*) FROM t GROUP BY a;"),
        ("vue", "<template><div>{{ value }}</div></template><script>let value = 1;</script>"),
        ("blade", "@if($ok)<div>{{ $value }}</div>@endif"),
        ("tsx", "const A = () => <div title=\"x\">text</div>;"),
        ("rr", ".decl item(n:number)\nitem(1).\n.output item\n"),
    ] {
        let lang = dialect_by_label(label).unwrap();
        let grammar = Grammar::new(lang).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false).unwrap().unwrap();
        let source = Arc::new(source.to_owned());
        let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
        let doc = TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &image.primary, &|| false).unwrap();
        let old = from_tape(&doc, Arc::clone(&lines)).unwrap();
        let same = from_image(Arc::clone(&source), &grammar, image.primary.clone(), Arc::clone(&lines)).unwrap();
        assert_eq!(digest(&old), digest(&same), "{label}");
        let (kinds, classes) = old.histogram();
        for format in Format::ALL {
            let new = from_image(Arc::clone(&source), &grammar, image.primary.clone().in_format(format, source.as_bytes()), Arc::clone(&lines)).unwrap();
            let at = format!("{label} in {}: {source}", format.name());
            assert_eq!(old.len(), new.len(), "{at}");
            for id in 0..new.len() as u32 {
                assert_eq!((old.kind(id), old.parent(id), old.span(id), old.subtree_end(id), old.flags(id), old.field(id)),
                    (new.kind(id), new.parent(id), new.span(id), new.subtree_end(id), new.flags(id), new.field(id)), "{at}: node {id}");
                assert_eq!((old.start(id), old.end(id)), new.span(id), "{at}: node {id}");
                assert_eq!(old.children(id), new.children(id), "{at}: node {id}");
                assert_eq!(new.start_line(id), lines.line(new.start(id) as usize) as u32 + 1);
            }
            assert_eq!((kinds.clone(), classes.clone()), new.histogram(), "{at}");
            for &(kind, n) in &kinds {
                let nodes = new.of_kind(kind);
                assert_eq!(nodes.len() as u32, n, "{at}");
                assert_eq!(nodes, old.of_kind(kind), "{at}");
                assert!(nodes.iter().all(|&id| new.kind(id) == kind));
            }
            for &(class, _) in &classes {
                assert_eq!(new.of_class(class), old.of_class(class), "{at}");
            }
            assert_eq!(old.syntax_errors(), new.syntax_errors(), "{at}");
            assert_eq!(old.depth(), new.depth(), "{at}");
        }
    }
}

/// Decision 0434 §4, step 6: a parsed tape's home holds its presence after it; the store keeps the two as one, and a
/// tape read back from the store answers each block's kinds as the parse's did, by the same read where it lies
/// (`tape_has_kind`). Format 16 and 16z alike: the stored frame is the tape's bytes as they are, and its presence is
/// read there unloaded.
#[test]
fn a_stored_tape_holds_the_presence_its_writer_made() {
    use crate::code_rule_dsl::build_ids::grammar_version;
    use crate::code_rule_dsl::tape::{self, PRESENCE_BLOCK};
    use crate::code_rule_dsl::tape::formats::{Format, this_thread_stores};
    use crate::code_rule_dsl::facts::core::build_cached_with_tape;
    for format in [Format::F16, Format::F16Z] {
        this_thread_stores(format);
        let at = format.name();
        let dir = tempfile::tempdir().unwrap();
        let cache = crate::content_store::ContentStore::open(&dir.path().join("p.sqlite")).unwrap();
        // Past two blocks of nodes: the class is in the first alone.
        let file = Text("python".into(), Arc::new(format!("class A:\n    def f(self):\n        return g(1)\n{}", "x = 1\n".repeat(500))));
        let (_, _, parsed) = build_cached_with_tape(&file, &cache, None, true).unwrap();
        let parsed = parsed.expect("the tape");
        assert!(parsed.has_presence(), "{at}: a parse lays its presence in the tape's home");
        cache.written().unwrap();
        let tag = i64::from(tape_tag(dialect_by_label("python").unwrap()).unwrap());
        let (gv, fv) = (grammar_version(), i64::from(tape::formats::stored_version()));
        let source = cache.source_by_hash(crate::content_store::HashKey::sha256(file.source().as_bytes())).unwrap().expect("the source, stored");
        let frame = cache.tape_frames_for(&[source], tag, gv, fv).unwrap().remove(0).expect("the tape, stored");
        let bytes = frame.in_place().unwrap_or_else(|| panic!("{at}: the frame is the tape's bytes as they are"));
        let stored = tape::stored_presence(bytes, tag as u8).unwrap_or_else(|| panic!("{at}: the presence, stored"));
        let (_, doc) = build_in_wave_with_tape(&file, &cache, None, Some(&frame), false, true).unwrap();
        let doc = doc.expect("the tape, read from the store");
        assert!(doc.has_presence(), "{at}: a stored tape's home holds its presence");
        let blocks = parsed.transducer_len().div_ceil(PRESENCE_BLOCK);
        assert!(blocks >= 2, "{blocks} blocks");
        let mut lacking = false;
        for kind in 0..400u16 {
            assert_eq!(doc.may_have_kind(kind), parsed.may_have_kind(kind), "{at}: kind {kind}");
            assert_eq!(stored.has_kind(u32::MAX, kind), parsed.may_have_kind(kind), "{at}: kind {kind}, unloaded");
            for b in 0..blocks {
                assert_eq!(doc.block_may_have_kind(b, kind), parsed.block_may_have_kind(b, kind), "{at}: block {b}, kind {kind}");
            }
            lacking |= parsed.may_have_kind(kind) && !parsed.block_may_have_kind(blocks - 1, kind);
        }
        assert!(lacking, "{at}: a kind of the document the last block lacks");
    }
}
