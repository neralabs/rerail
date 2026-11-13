    use super::{NATIVE_TAGS, View};
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::vue::template::Directive;
    use ast_grep_language::LanguageExt;
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use tree_sitter::{Node, Parser, Tree};

    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&GrammarDialect::Vue.get_ts_language())
            .expect("Vue grammar");
        parser.parse(source, None).expect("Vue tree")
    }

    /// Every named node under `node`, in document order.
    fn named<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
        out.push(node);
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            named(child, out);
        }
    }

    /// Checks that each copied segment's program text is the file's text it stands for.
    fn check_copies(view: &View, source: &str) -> Result<(), String> {
        for (program, anchor) in view.segments.copied_stretches() {
            let program = &view.text[program];
            let file = &source[anchor];
            if program != file {
                return Err(format!("copied {program:?} stands for {file:?}"));
            }
        }
        Ok(())
    }

    /// The `v-for` and directive spans the view must give statements: each `v-for` element's
    /// range, and each directive's range that reads its value as one expression statement.
    fn template_spans(
        node: Node<'_>,
        source: &str,
        loops: &mut Vec<Range<usize>>,
        directives: &mut Vec<Range<usize>>,
    ) {
        if matches!(node.kind(), "element" | "template_element") {
            let mut cursor = node.walk();
            let start = node
                .named_children(&mut cursor)
                .find(|child| matches!(child.kind(), "start_tag" | "self_closing_tag"));
            let read: Vec<Directive<'_>> = start
                .map(|start| {
                    let mut cursor = start.walk();
                    start
                        .named_children(&mut cursor)
                        .filter_map(|attribute| Directive::read(attribute, source))
                        .collect()
                })
                .unwrap_or_default();
            if read.iter().any(|directive| directive.name == "pre") {
                return;
            }
            for directive in &read {
                let has_value = directive
                    .value
                    .clone()
                    .is_some_and(|value| !source[value].trim().is_empty());
                match directive.name {
                    "for" if has_value => loops.push(node.byte_range()),
                    "for" | "slot" | "slot-scope" | "scope" | "if" | "else-if" | "else" => {}
                    _ if has_value => directives.push(directive.node.byte_range()),
                    _ => {}
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            template_spans(child, source, loops, directives);
        }
    }

    /// The view of `source`, after checking what every view keeps: a program without syntax
    /// errors, copies that match the file, identifiers whose span is their text, a `v-for`
    /// statement over each `v-for` element and a statement over each expression directive.
    fn view(source: &str) -> View {
        checked(source, true)
    }

    /// `view`, checking identifiers only where `identifiers` (the view writes a camelized
    /// same-name shorthand's identifier itself).
    fn checked(source: &str, identifiers: bool) -> View {
        let tree = parse(source);
        assert!(!tree.root_node().has_error(), "Vue tree of {source:?}");
        let view = View::build(tree.root_node(), source).expect("a program");
        let text = view.text();
        assert!(!view.root().has_error(), "syntax error in {text:?}");
        check_copies(&view, source).expect("copies");
        let mut nodes = Vec::new();
        named(view.root(), &mut nodes);
        let mut statements = Vec::new();
        let mut loops = Vec::new();
        for node in nodes {
            // The only placeholders are empty alias slots' parameters, which the tape leaves out.
            if view.segments().omitted(node.byte_range()) {
                assert_eq!(&text[node.byte_range()], "_", "placeholder in {text:?}");
                assert!(
                    view.span(node, source).is_empty(),
                    "placeholder in {text:?}"
                );
                continue;
            }
            if identifiers && node.kind().ends_with("identifier") {
                assert_eq!(
                    &source[view.span(node, source)],
                    &text[node.byte_range()],
                    "identifier in {text:?}"
                );
            }
            if node.kind() != "expression_statement" {
                continue;
            }
            statements.push(view.span(node, source));
            let mut cursor = node.walk();
            let kids: Vec<_> = node.named_children(&mut cursor).collect();
            if let [sequence] = kids.as_slice()
                && sequence.kind() == "sequence_expression"
            {
                let mut cursor = sequence.walk();
                let parts: Vec<_> = sequence
                    .named_children(&mut cursor)
                    .map(|part| part.kind())
                    .collect();
                assert_eq!(
                    parts,
                    ["parenthesized_expression", "arrow_function"],
                    "{text:?}"
                );
                loops.push(view.span(node, source));
            }
        }
        let mut expected_loops = Vec::new();
        let mut directives = Vec::new();
        template_spans(
            tree.root_node(),
            source,
            &mut expected_loops,
            &mut directives,
        );
        assert_eq!(loops, expected_loops, "v-for statements of {text:?}");
        for directive in directives {
            assert!(
                statements.contains(&directive),
                "no statement over {:?} in {text:?}",
                &source[directive]
            );
        }
        view
    }

    #[test]
    fn native_tags_are_sorted() {
        assert!(NATIVE_TAGS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    /// The spans of the program's nodes of `kind`, in document order.
    fn spans(view: &View, source: &str, kind: &str) -> Vec<Range<usize>> {
        let mut nodes = Vec::new();
        named(view.root(), &mut nodes);
        nodes
            .into_iter()
            .filter(|node| node.kind() == kind)
            .map(|node| view.span(node, source))
            .collect()
    }

    /// The byte range of the first occurrence of `text` in `source`.
    fn find(source: &str, text: &str) -> Range<usize> {
        let start = source.find(text).expect("text in source");
        start..start + text.len()
    }

    #[test]
    fn interpolation() {
        let source = "<template><p>{{ message }}</p></template>";
        let view = view(source);
        assert_eq!(view.text(), "{ (message); }");
        assert_eq!(view.dialect(), GrammarDialect::JavaScript);
        assert_eq!(
            spans(&view, source, "statement_block"),
            vec![0..source.len()]
        );
        assert_eq!(
            spans(&view, source, "expression_statement"),
            [find(source, "{{ message }}")]
        );
    }

    #[test]
    fn bindings_and_handlers() {
        let source =
            r#"<template><input :a="x" @click="go" @input="n++" v-on="{ a: b }"></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (x); (go); (() => { n++ }); ({ a: b }); }");
        assert_eq!(
            spans(&view, source, "arrow_function"),
            [find(source, r#"@input="n++""#)]
        );
    }

    #[test]
    fn handler_object_and_statements() {
        // `v-on` without an argument is an object of handlers, never statements; with one, a
        // call is statements and a function or member is the handler itself. `$event` is no
        // parameter of an inline handler's arrow.
        let source = r#"<template><a v-on="go()" @x="go($event)" @y="(e) => go(e)" @z="a.b" @w="list.map(f => f)"></a></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ (go()); (() => { go($event) }); ((e) => go(e)); (a.b); (() => { list.map(f => f) }); }"
        );
    }

    #[test]
    fn same_name_shorthand() {
        // tree-sitter-vue reads `:a :b` without values as one attribute, so they are apart here.
        let source = r#"<template><a :href class="c" :aria-label class :class></a></template>"#;
        let view = checked(source, false);
        assert_eq!(view.text(), "{ (href); (ariaLabel); }");
        let synthetic = spans(&view, source, "identifier")[1].clone();
        assert_eq!(&source[synthetic], "aria-label");
    }

    #[test]
    fn character_references() {
        let source =
            r#"<template><p v-if="a &gt; 0 &amp;&amp; b">{{ c &lt; d &#38; e }}</p></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ if (a > 0 && b) { (c < d & e); } }");
    }

    /// An unquoted value that starts or ends with a multibyte character is no quoted one.
    #[test]
    fn multibyte_unquoted_value() {
        let source = "<template><div :title=名前></div><p :a=x名></p></template>";
        assert_eq!(view(source).text(), "{ (名前); (x名); }");
    }

    #[test]
    fn if_chain_across_a_comment() {
        let source = "<template>\n  <p v-if=\"a\">1</p>\n  <!-- c -->\n  <p v-else-if=\"b\">2</p>\n  <p v-else>3</p>\n  <p v-else>4</p>\n</template>";
        let view = view(source);
        assert_eq!(view.text(), "{ if (a) { } else if (b) { } else { } }");
        let first = find(source, "<p v-if=\"a\">1</p>");
        let second = find(source, "<p v-else-if=\"b\">2</p>");
        let last = find(source, "<p v-else>3</p>");
        // Each statement stands for its branches' elements and the white space between them, not
        // the comment between two, and reports where it starts.
        let mut statements = Vec::new();
        named(view.root(), &mut statements);
        let ifs = statements
            .into_iter()
            .filter(|node| node.kind() == "if_statement")
            .map(|node| view.ranges(node, source))
            .collect::<Vec<_>>();
        let chain = second.start..last.end;
        assert_eq!(ifs.len(), 2);
        assert_eq!(ifs[0], [first.clone(), chain.clone()]);
        assert_eq!(ifs[1].as_slice(), std::slice::from_ref(&chain));
        assert_eq!(
            spans(&view, source, "if_statement"),
            [first, second.start..last.end]
        );
        assert_eq!(
            spans(&view, source, "parenthesized_expression"),
            [
                find(source, r#"v-if="a""#),
                find(source, r#"v-else-if="b""#)
            ]
        );
        assert_eq!(
            spans(&view, source, "statement_block")[1..],
            [
                find(source, "<p v-if=\"a\">1</p>"),
                find(source, "<p v-else-if=\"b\">2</p>"),
                last,
            ]
        );
    }

    #[test]
    fn text_ends_an_if_chain() {
        let source =
            r#"<template><p v-if="a"></p> text <p v-else-if="b"></p><p v-else></p></template>"#;
        assert_eq!(view(source).text(), "{ if (a) { } (b); }");
    }

    #[test]
    fn condition_less_if_reads_true() {
        let source = r#"<template><p v-if></p><p v-else-if=" "></p></template>"#;
        assert_eq!(view(source).text(), "{ if (true) { } else if (true) { } }");
    }

    #[test]
    fn for_aliases() {
        let source = r#"<template><li v-for="item in items">{{ item }}</li><li v-for="(item, index) of items" :key="index">{{ item }}</li><li v-for="{ id, name } in rows">{{ id }}{{ name }}</li><li v-for="value, key in object"></li></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ (items), (item) => { (item); }; (items), (item, index) => { (index); (item); }; \
             (rows), ({ id, name }) => { (id); (name); }; (object), (value, key) => { }; }"
        );
        // The parentheses stand for no bytes (the opening one written at the element's start):
        // the arrow holds the element whole, its parameters stand for the aliases they copy.
        let element = find(
            source,
            r#"<li v-for="(item, index) of items" :key="index">{{ item }}</li>"#,
        );
        assert_eq!(spans(&view, source, "arrow_function")[1], element);
        assert_eq!(
            spans(&view, source, "formal_parameters")[1],
            find(source, "item, index")
        );
        assert_eq!(
            spans(&view, source, "formal_parameters")[3],
            find(source, "value, key")
        );
        assert!(
            view.copied()
                .any(|copy| copy == find(source, "item, index"))
        );
    }

    /// An empty slot of a parenthesized alias gets a placeholder parameter, so the program
    /// parses; the tape leaves the placeholder out, and the scope reads the file's aliases alone,
    /// at their spans.
    #[test]
    fn for_alias_empty_slots() {
        use crate::code_rule_dsl::scope::ScopeTable;
        use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
        use ast_grep_core::AstGrep;
        use std::sync::Arc;
        for (alias, program) in [
            ("(,a,b)", "(_,a,b)"),
            ("(a,,b)", "(a,_,b)"),
            ("(a,b,,)", "(a,b,_,)"),
            ("(,a)", "(_,a)"),
        ] {
            let source =
                format!(r#"<template><li v-for="{alias} in list">{{{{ a }}}}</li></template>"#);
            let built = view(&source);
            assert_eq!(
                built.text(),
                format!("{{ (list), {program} => {{ (a); }}; }}"),
                "{alias}"
            );
            assert!(!built.root().has_error(), "{alias}");
            let grammar = Grammar::new(GrammarDialect::Vue).expect("grammar");
            let tree = parse(&source);
            let image =
                TapeImage::encode(tree.root_node(), &grammar, &source, &|| false).expect("encodes");
            let host = TapeDoc::restore(Arc::new(source.clone()), grammar, &image, &|| false)
                .expect("restores");
            let program = host.guests().expect("owned")[0].doc.clone();
            let dialect = *ast_grep_core::Doc::get_lang(&program);
            let doc = AstGrep::doc(program.clone());
            let root = doc.root();
            assert!(
                root.dfs().all(|node| &*node.text() != "_"),
                "{alias}: the placeholder is left out"
            );
            let table =
                ScopeTable::build(root, dialect, usize::MAX, &|| false).expect("scope table");
            let mut names = table
                .scopes
                .iter()
                .flat_map(|scope| scope.bindings.iter())
                .flat_map(|(name, bindings)| {
                    bindings.iter().filter_map(|binding| {
                        let span = binding.name?;
                        Some((
                            name.to_string(),
                            crate::code_rule_dsl::origin::InFile::tape(&program)
                                .range(span.start as usize..span.end as usize),
                        ))
                    })
                })
                .collect::<Vec<_>>();
            names.sort_by_key(|(_, span)| span.start);
            let at = |name: &str| {
                let start = source.find(alias).unwrap() + alias.find(name).unwrap();
                (name.to_owned(), start..start + 1)
            };
            let expected = ["a", "b"]
                .into_iter()
                .filter(|name| alias.contains(name))
                .map(at)
                .collect::<Vec<_>>();
            assert_eq!(names, expected, "{alias}");
        }
    }

    /// `scope` is Vue 2's slot scope on a `<template>` only; on `<th>` it is HTML's attribute.
    #[test]
    fn scope_is_a_slot_scope_on_template_only() {
        let source = r#"<template><th scope="col">{{ x }}</th></template>"#;
        assert_eq!(view(source).text(), "{ (x); }");
        let source = r#"<template><R><template scope="p">{{ p }}</template></R></template>"#;
        assert!(
            view(source).text().contains("(p) => {"),
            "{}",
            view(source).text()
        );
    }

    #[test]
    fn if_wraps_for_on_one_element() {
        let source =
            r#"<template><li v-for="item in items" v-if="show">{{ item }}</li></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ if (show) { (items), (item) => { (item); }; } }"
        );
        let element = find(source, "<li v-for");
        assert_eq!(
            spans(&view, source, "if_statement"),
            vec![element.start..source.len() - "</template>".len()]
        );
    }

    #[test]
    fn for_in_a_chain_branch() {
        let source = r#"<template><Row v-if="ok" v-for="item in items" :item>{{ item }}</Row><p v-else>{{ none }}</p></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ if (ok) { (items), (item) => { Row; (item); (item); }; } else { (none); } }"
        );
    }

    #[test]
    fn pre_in_a_chain() {
        let source = r#"<template><p v-if="a"></p><p v-else v-pre>{{ b }}</p><p v-else-if="c"></p></template>"#;
        assert_eq!(view(source).text(), "{ if (a) { } (c); }");
    }

    #[test]
    fn component_slot() {
        let source = r#"<template><Comp v-slot="{ item }">{{ item }}</Comp></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ Comp; ({ item }) => { (item); }; }");
        let element = find(source, r#"<Comp v-slot="{ item }">{{ item }}</Comp>"#);
        assert_eq!(
            spans(&view, source, "arrow_function"),
            std::slice::from_ref(&element)
        );
        assert_eq!(
            spans(&view, source, "expression_statement"),
            [find(source, "Comp"), element, find(source, "{{ item }}")]
        );
    }

    #[test]
    fn template_slot() {
        let source = r#"<template><Table><template #row="r">{{ r }}</template></Table></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ Table; (r) => { (r); }; }");
        assert_eq!(
            spans(&view, source, "arrow_function"),
            [find(source, r#"<template #row="r">{{ r }}</template>"#)]
        );
    }

    #[test]
    fn dynamic_argument() {
        let source = r#"<template><a :[k]="v"></a><Comp><template #[name]="p">{{ p }}</template></Comp></template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (k); (v); Comp; (name); (p) => { (p); }; }");
        let statements = spans(&view, source, "expression_statement");
        assert_eq!(statements[0], find(source, "[k]"));
        assert_eq!(statements[1], find(source, r#":[k]="v""#));
        assert_eq!(statements[3], find(source, "[name]"));
    }

    #[test]
    fn dotted_component_tag() {
        // tree-sitter-vue reads `<ElTable.TableColumn>` as the tag name `ElTable` and a `.`
        // shorthand `.TableColumn`; vue-eslint-parser reads the one tag `ElTable.TableColumn`,
        // a component the view reads as a member, and no prop bind or same-name shorthand.
        let source = r#"<template><ElTable.TableColumn label="a" :b="c"/><a.b.c/><Foo.bar-baz/><Foo .prop="v"/></template>"#;
        let view = view(source);
        assert_eq!(
            view.text(),
            "{ ElTable.TableColumn; (c); a.b.c; Foo; (v); }"
        );
        assert_eq!(
            spans(&view, source, "member_expression"),
            [
                find(source, "ElTable.TableColumn"),
                find(source, "a.b.c"),
                find(source, "a.b")
            ]
        );
        assert!(
            view.copied()
                .any(|copy| copy == find(source, "ElTable.TableColumn"))
        );
    }

    #[test]
    fn pre_subtree() {
        let source = r#"<template><div v-pre :a="x">{{ y }}<p :b="z"></p></div>{{ w }}</template>"#;
        let view = view(source);
        assert_eq!(view.text(), "{ (w); }");
        let pre = find(source, r#"<div v-pre :a="x">{{ y }}<p :b="z"></p></div>"#);
        assert!(
            view.copied()
                .all(|copy| copy.start >= pre.end || copy.end <= pre.start)
        );
    }

    #[test]
    fn template_first_with_two_scripts() {
        let source = "<template><p>{{ a + b }}</p></template>\n<script>\nexport const a = 1\n</script>\n<script setup lang=\"ts\">\nconst b: number = 2\n</script>\n<style>p { color: red }</style>\n";
        let view = view(source);
        assert_eq!(view.dialect(), GrammarDialect::TypeScript);
        assert_eq!(
            view.text(),
            "{ (a + b); }\n;\nexport const a = 1\n\n;\n;\nconst b: number = 2\n\n;"
        );
        // The closing `;` ends the last statement where it has none, at its last byte.
        assert_eq!(
            spans(&view, source, "empty_statement"),
            [
                find(source, "<script>"),
                find(source, "<script setup lang=\"ts\">")
            ]
        );
        assert_eq!(
            spans(&view, source, "lexical_declaration"),
            [
                find(source, "const a = 1"),
                find(source, "const b: number = 2")
            ]
        );
    }

    #[test]
    fn script_dialects() {
        let coffee = "<template><p>{{ a }}</p></template><script lang=\"coffee\">a = 1</script>";
        assert!(View::build(parse(coffee).root_node(), coffee).is_none());
        let template_only = "<template><p>{{ a }}</p></template>";
        assert_eq!(view(template_only).dialect(), GrammarDialect::JavaScript);
        let style_only = "<style>p {}</style>";
        assert!(View::build(parse(style_only).root_node(), style_only).is_none());
    }

    /// Builds the view of `source`, whose template error recovery leaves a dynamic argument of
    /// one `[` byte (its `]` missing): the argument is empty, so the view has none.
    fn dynamic_argument_without_value(source: &str) {
        let tree = parse(source);
        assert!(tree.root_node().has_error(), "Vue tree of {source:?}");
        let view = View::build(tree.root_node(), source);
        assert!(
            view.is_none_or(|view| !view.text().contains('[')),
            "{source:?}"
        );
    }

    #[test]
    fn dynamic_argument_without_value_before_tag_end() {
        dynamic_argument_without_value("<template><a :[></a></template>");
    }

    #[test]
    fn dynamic_argument_without_value_before_space() {
        dynamic_argument_without_value("<template><a :[ ></a></template>");
    }

    /// The `.vue` files under `directory`, recursively.
    fn vue_files(directory: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(directory)
            .unwrap_or_else(|error| panic!("{}: {error}", directory.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                vue_files(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "vue") {
                out.push(path);
            }
        }
    }

    /// Every Vue fixture: copies match the file, and the program parses wherever the template
    /// does, but for a few files whose template expressions are no ECMAScript (listed).
    #[test]
    fn corpus() {
        let fixtures = Path::new(crate::code_rule_dsl::fixtures::fixtures_dir!());
        let mut files = Vec::new();
        vue_files(&fixtures.join("vue/language/view/fixtures"), &mut files);
        vue_files(
            &crate::code_rule_dsl::test_support::pack_test_data(
                "packs/vue/upstream/eslint-plugin-vue/fixtures",
            ),
            &mut files,
        );
        files.sort();
        let mut built = 0;
        let mut broken = Vec::new();
        for path in &files {
            let source = std::fs::read_to_string(path).expect("fixture");
            let tree = parse(&source);
            let Some(view) = View::build(tree.root_node(), &source) else {
                continue;
            };
            built += 1;
            if let Err(error) = check_copies(&view, &source) {
                panic!("{}: {error}", path.display());
            }
            if view.root().has_error() && !tree.root_node().has_error() {
                broken.push(
                    path.strip_prefix(fixtures)
                        .unwrap_or(path)
                        .display()
                        .to_string(),
                );
            }
        }
        eprintln!(
            "{} files, {built} programs, {} with errors only in the program:\n{}",
            files.len(),
            broken.len(),
            broken.join("\n")
        );
        assert!(files.len() > 700, "{} files", files.len());
        assert!(broken.len() * 50 <= files.len(), "{broken:#?}");
    }

    /// A node of the program stands for the file bytes of its pieces, never a range widened over
    /// bytes it does not stand for: a written token's anchor holds the copied text beside it, and
    /// the program's root, over a script and the template after it, stands for both apart, not
    /// the closing tag between them.
    #[test]
    fn nodes_stand_for_their_pieces_and_nothing_between() {
        let source =
            r#"<template><li v-for="item in items" :title="item">{{ item }}</li></template>"#;
        let built = view(source);
        let mut nodes = Vec::new();
        named(built.root(), &mut nodes);
        let ranges_of = |text: &str| {
            let node = nodes
                .iter()
                .find(|node| &built.text()[node.byte_range()] == text)
                .unwrap_or_else(|| panic!("no {text:?}"));
            built.ranges(*node, source)
        };
        // `(item)` is written over the attribute, its `item` copied inside it.
        assert_eq!(ranges_of("(item);"), [find(source, r#":title="item""#)]);
        assert_eq!(
            ranges_of("(items)"),
            [find(
                source,
                r#"<li v-for="item in items" :title="item">{{ item }}</li>"#
            )]
        );
        let source = "<script>\nlet a = 1\n</script>\n<template><p>{{ a }}</p></template>\n";
        let view = view(source);
        let script = find(source, "<script>\nlet a = 1\n");
        let template = find(source, "<template><p>{{ a }}</p></template>");
        assert_eq!(view.ranges(view.root(), source), [script.clone(), template]);
        assert_eq!(view.span(view.root(), source), script);
    }
