//! The embedding seam through tests' own embedder (`TestScript`): an HTML file whose scripts and
//! handlers make one TypeScript, TSX or JavaScript program, encoded by `TapeImage::encode`'s
//! embedder loop, persisted, restored as a document of its own, and read by scope and usage.

use super::test_script::html;
use crate::code_rule_dsl::language::{GrammarDialect, Via};
use crate::code_rule_dsl::origin::InFile;
use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
use crate::code_rule_dsl::scope::{Form, ScopeTable};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use ast_grep_core::{AstGrep, Doc};
use std::sync::Arc;

/// The one program a file of `doc` embeds, as a document.
fn program(doc: &AstGrep<TapeDoc>) -> AstGrep<TapeDoc> {
    let guests = doc
        .root()
        .get_doc()
        .guests()
        .expect("the host owns its guests");
    let [guest] = guests.as_slice() else {
        panic!("one program, not {}", guests.len());
    };
    AstGrep::doc(guest.doc.clone())
}

/// The program is a document in the dialect its embedder built it in, its nodes spanning its own
/// text; each stands for the file bytes it was copied from. A file without a script embeds none.
#[test]
fn a_program_is_a_document_in_the_dialect_its_embedder_built_it_in() {
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
        let program = program(&doc);
        let guest = program.root().get_doc().clone();
        assert_eq!(*guest.get_lang(), dialect, "{source:?}");
        assert_eq!(guest.via(), Some(Via::TestScript));
        assert!(
            guest
                .host()
                .is_some_and(|host| host.same(doc.root().get_doc()))
        );
        let node = program
            .root()
            .dfs()
            .find(|node| node.kind() == kind)
            .unwrap_or_else(|| panic!("{source:?}: no {kind}"));
        assert_eq!(node.text(), text, "{source:?}");
        let at = source.find(text).expect("the file holds the text");
        assert_eq!(InFile::tape(&guest).node(&node), at..at + text.len());
        // The host's own tree holds none of the program's nodes.
        assert!(
            doc.root().dfs().all(|node| node.kind() != kind),
            "{source:?}"
        );
    }
    let doc = html("<p>no script</p>\n");
    assert!(
        doc.root()
            .get_doc()
            .guests()
            .is_some_and(|guests| guests.is_empty())
    );
}

/// Blade's compiled view is a PHP document read through the template's syntax errors, which the
/// template's own tree holds as ERROR leaves.
#[test]
fn a_blade_tape_holds_its_compiled_view_as_a_php_document() {
    let source = "{{ $a }}\n{{-- open\n";
    let grammar = Grammar::new(GrammarDialect::Blade).expect("grammar");
    let image = TapeImage::encode_template(&grammar, source, &|| false).expect("encodes");
    let host = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
        .expect("restores");
    let errors = host.syntax_error_ranges();
    assert!(!errors.is_empty());
    let guests = host.guests().expect("owned");
    let [guest] = guests.as_slice() else {
        panic!("one view");
    };
    assert_eq!(guest.via, Via::CompiledView);
    assert_eq!(*guest.doc.get_lang(), GrammarDialect::Php);
    assert_eq!(guest.doc.host_errors().collect::<Vec<_>>(), errors);
    let view = AstGrep::doc(guest.doc.clone());
    assert_eq!(view.root().kind(), "program");
    let variable = view
        .root()
        .dfs()
        .find(|node| node.kind() == "variable_name")
        .expect("the echo's variable");
    assert_eq!(InFile::tape(&guest.doc).text(&variable), "$a");
}

/// Scope reads the program with its own grammar's kinds and fields, and usage pairs each of its
/// references and names with its node though the program reads a handler after the script that
/// stands after it in the file.
#[test]
fn scope_and_usage_read_an_embedded_program() {
    let source = "<button onclick=\"go()\"></button>\n<script>function go() {}</script>\n";
    let doc = program(&html(source));
    let text = doc.root().get_doc().get_source().clone();
    let table = ScopeTable::build(doc.root(), GrammarDialect::JavaScript, usize::MAX, &|| {
        false
    })
    .expect("table builds");
    let start = text.rfind("go()").expect("the handler");
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

    // A TSX program's JSX reads its bindings.
    let source = "<script lang=\"tsx\">const A = 1; let e = <A />;</script>\n";
    let doc = program(&html(source));
    let text = doc.root().get_doc().get_source().clone();
    let table = ScopeTable::build(doc.root(), GrammarDialect::Tsx, usize::MAX, &|| false)
        .expect("table builds");
    let start = text.rfind('A').expect("the element");
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
    let doc = program(&html(
        "<button onclick=\"go()\"></button>\n<script>go();</script>\n",
    ));
    let rewritten = doc
        .root()
        .dfs()
        .filter(|node| node.kind() == "expression_statement")
        .map(|statement| {
            (
                InFile::tape(doc.root().get_doc())
                    .text(&statement)
                    .into_owned(),
                super::rewritten(&statement),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rewritten,
        [("go();".to_owned(), false), ("go()".to_owned(), true)]
    );
}
