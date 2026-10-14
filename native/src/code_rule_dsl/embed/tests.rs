//! The embedding seam through tests' own embedder (`TestScript`): an HTML file whose scripts and
//! handlers make one TypeScript, TSX or JavaScript program, encoded by `TapeImage::encode`'s
//! embedder loop, persisted, restored, and read as its region by scope and usage.

use super::test_script::html;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::regions;
use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
use crate::code_rule_dsl::scope::{Form, ScopeTable};
use crate::code_rule_dsl::tape::{EmbeddedProgram, Grammar, TapeDoc, TapeImage};
use ast_grep_core::AstGrep;
use std::sync::Arc;

/// The tape holds the program as the root's last child, its kinds numbered as the host numbers
/// its grammar's and each node over the host bytes it stands for, and records the dialect the
/// embedder built it in: JavaScript, TSX and TypeScript programs share their root's kind code, so
/// the code does not tell their region's dialect.
#[test]
fn a_region_is_read_in_the_dialect_its_embedder_built_it_in() {
    let host = GrammarDialect::Html;
    let typescript = host
        .kinds()
        .embedding(GrammarDialect::TypeScript)
        .expect("HTML embeds TypeScript");
    let program = typescript.kind_code(typescript.grammar.id_for_node_kind("program", true));
    for (source, dialect, kind, text) in [
        (
            "<p>a</p>\n<script>let x = 1;</script>\n",
            GrammarDialect::JavaScript,
            "lexical_declaration",
            "let x = 1;",
        ),
        (
            "<script lang=\"tsx\">let x = <a />;</script>\n",
            GrammarDialect::Tsx,
            "jsx_self_closing_element",
            "<a />",
        ),
        (
            "<script lang=\"ts\">let x: number = 1;</script>\n",
            GrammarDialect::TypeScript,
            "type_annotation",
            ": number",
        ),
    ] {
        let doc = html(source);
        let root = doc.root();
        let read = regions(&root, host)
            .map(|region| (region.dialect, region.root.kind_id(), region.root.node_id()))
            .collect::<Vec<_>>();
        let last = root.children().last().expect("the program").node_id();
        assert_eq!(read, [(dialect, program, last)], "{source:?}");
        let tape: &TapeDoc = root.get_doc();
        assert_eq!(
            tape.programs(),
            [EmbeddedProgram {
                root: last as u32,
                dialect,
                apart: true,
            }],
            "{source:?}"
        );
        let node = root
            .dfs()
            .find(|node| node.kind() == kind)
            .unwrap_or_else(|| panic!("{source:?}: no {kind}"));
        assert_eq!(node.text(), text, "{source:?}");
    }
    let doc = html("<p>no script</p>\n");
    assert_eq!(regions(&doc.root(), host).count(), 0);
    assert!(doc.root().get_doc().programs().is_empty());
}

/// Blade's compiled view is its tape's one program: PHP, read through the template's errors.
#[test]
fn a_blade_tape_records_its_compiled_view() {
    let source = "{{ $a }}\n";
    let grammar = Grammar::new(GrammarDialect::Blade).expect("grammar");
    let image = TapeImage::encode_template(&grammar, source, &|| false).expect("encodes");
    let doc = AstGrep::doc(
        TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
            .expect("restores"),
    );
    let root = doc.root();
    let program = root.children().last().expect("the view");
    assert_eq!(program.kind(), "program");
    assert_eq!(
        root.get_doc().programs(),
        [EmbeddedProgram {
            root: program.node_id() as u32,
            dialect: GrammarDialect::Php,
            apart: false,
        }]
    );
    let read = regions(&root, GrammarDialect::Blade)
        .map(|region| (region.dialect, region.root.node_id()))
        .collect::<Vec<_>>();
    assert_eq!(read, [(GrammarDialect::Php, program.node_id())]);
}

/// Scope reads the program region with its own grammar's kinds and fields as the host numbers
/// them, and usage pairs each of its references and names with its node though the program reads
/// a handler after the script that stands after it in the file.
#[test]
fn scope_and_usage_read_an_embedded_program() {
    let source = "<button onclick=\"go()\"></button>\n<script>function go() {}</script>\n";
    let doc = html(source);
    let table = ScopeTable::build(doc.root(), GrammarDialect::Html, usize::MAX, &|| false)
        .expect("table builds");
    let start = source.find("go()").expect("the handler");
    let reference = table
        .reference(&(start..start + 2))
        .expect("the handler reads go");
    assert_eq!(
        table
            .resolve(reference, "go")
            .map(|binding| binding.site.form),
        Some(Form::Definition)
    );
    let usage = Usage::build(&table, &doc.root(), usize::MAX, &|| false).expect("usage");
    let scope = table
        .scopes
        .iter()
        .position(|scope| scope.bindings.contains_key("go"))
        .expect("go is bound");
    let go = BindingId {
        scope: scope as u32,
        name: "go",
        index: 0,
    };
    assert_eq!(usage.facts(go).used, Some(true));

    // A TSX region's JSX reads its bindings: its kinds are TSX's, which TypeScript has not.
    let source = "<script lang=\"tsx\">const A = 1; let e = <A />;</script>\n";
    let doc = html(source);
    let table = ScopeTable::build(doc.root(), GrammarDialect::Html, usize::MAX, &|| false)
        .expect("table builds");
    let start = source.rfind('A').expect("the element");
    let reference = table
        .reference(&(start..start + 1))
        .expect("the element reads A");
    assert_eq!(
        table
            .resolve(reference, "A")
            .map(|binding| binding.site.form),
        Some(Form::Assignment)
    );
}

/// A token a program writes where the host spells none (a handler's `;`) is rewritten
/// (`embed::rewritten`); a token it copies from the host is not.
#[test]
fn a_token_the_program_writes_is_rewritten() {
    let doc = html("<button onclick=\"go()\"></button>\n<script>go();</script>\n");
    let root = doc.root();
    let rewritten = root
        .dfs()
        .filter(|node| node.kind() == "expression_statement")
        .map(|statement| (statement.text().into_owned(), super::rewritten(&statement)))
        .collect::<Vec<_>>();
    assert_eq!(
        rewritten,
        [("go();".to_owned(), false), ("go()".to_owned(), true)]
    );
}
