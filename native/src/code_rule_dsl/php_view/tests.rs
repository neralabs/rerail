use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::{ast_grep, parse_input};
use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
use ast_grep_language::LanguageExt;
use serde_json::{Value, json};

/// `text` as the view blanks inline text: each byte a space, blanks and line ends kept.
fn blanked(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b' ' | b'\t' | b'\n' | b'\r' => byte as char,
            _ => ' ',
        })
        .collect()
}

/// `source`'s view is `expected`: owned, as long as `source`, with its line ends in place.
fn assert_view(source: &str, expected: &str) {
    let view = view(source);
    assert!(matches!(view, Cow::Owned(_)), "{source:?}");
    assert_eq!(view, expected, "{source:?}");
    assert_eq!(view.len(), source.len(), "{source:?}");
    for (at, (&from, &to)) in source.as_bytes().iter().zip(view.as_bytes()).enumerate() {
        if matches!(from, b'\n' | b'\r') || matches!(to, b'\n' | b'\r') {
            assert_eq!(from, to, "{source:?} at {at}");
        }
    }
}

const INLINE: &str =
    "<?php\nprint(1);\n?>\n<script>\nx = eval('document.getElementById (option)');\n</script>\n";
const INLINE_REAL_EVAL: &str =
    "<?php\neval('$a = 1;');\n?>\n<p>hi</p>\n<?php eval('$b = 2;'); ?>\n";
const IN_HTML: &str = "<html><?php foo(); ?></html>\n";
const ALTERNATIVE: &str = "<?php if ($a): ?>\n<p>x</p>\n<?php endif; ?>\n";
const SHORT_ECHO: &str = "<p><?= $x ?></p>\n";

#[test]
fn pure_php_is_its_own_view() {
    for source in [
        "",
        "<?php\nfoo();\n",
        "<?php\nfoo();\n?>\n",
        "<?php\nfoo();\n?>",
        "<?php\nfoo();\n?>\n\t \r\n",
        "<?php $s = '?>';\n",
        "<?php\n// ?>\n",
        "<?php\n$s = <<<EOT\n<?php ?>\nEOT;\n",
    ] {
        assert!(matches!(view(source), Cow::Borrowed(_)), "{source:?}");
    }
    assert!(matches!(
        parse_input(GrammarDialect::Python, "x = '?>'\n<p>"),
        Cow::Borrowed(_)
    ));
}

#[test]
fn inline_text_after_a_final_close_tag_is_blanked() {
    assert_view(
        INLINE,
        &format!(
            "<?php\nprint(1);\n?>{}",
            blanked("\n<script>\nx = eval('document.getElementById (option)');\n</script>\n")
        ),
    );
}

#[test]
fn inline_text_around_php_blocks_is_blanked() {
    assert_view(
        IN_HTML,
        &format!(
            "{}<?php foo(); ?>{}",
            blanked("<html>"),
            blanked("</html>\n")
        ),
    );
    // `<?=` before a blank is the `echo` PHP compiles it to, over the same bytes.
    assert_view(
        SHORT_ECHO,
        &format!("{}echo$x ?>{}", blanked("<p>"), blanked("</p>\n")),
    );
    assert_view(
        "<p><?= $a, $b ?></p>\n",
        &format!("{}echo$a, $b ?>{}", blanked("<p>"), blanked("</p>\n")),
    );
    // Before a name, a number or `\`, `echo` would read as one word with it: the tag is blanked.
    for expression in ["htmlspecialchars($t)", "TRUE", "new Foo()", "\\f()", "1"] {
        assert_view(
            &format!("<p><?= {expression} ?></p>\n"),
            &format!("{}{expression} ?>{}", blanked("<p><?= "), blanked("</p>\n")),
        );
    }
    assert_view(
        INLINE_REAL_EVAL,
        &format!(
            "<?php\neval('$a = 1;');\n{} eval('$b = 2;'); ?>\n",
            blanked("?>\n<p>hi</p>\n<?php")
        ),
    );
}

#[test]
fn a_mid_file_close_tag_terminates_only_an_open_statement() {
    // After `:` (alternative syntax) and `;`, `?>` is blanked.
    assert_view(
        ALTERNATIVE,
        &format!(
            "<?php if ($a): {} endif; ?>\n",
            blanked("?>\n<p>x</p>\n<?php")
        ),
    );
    // A later `<?=` is `echo`; the statement before it is ended.
    assert_view(
        "<p><?php $a = 1 ?></p><?= $a ?>\n",
        &format!(
            "{}<?php $a = 1 ; {}echo$a ?>\n",
            blanked("<p>"),
            blanked("</p>")
        ),
    );
    // Without a blank after it, `<?=` is blanked: its expression is a statement.
    assert_view(
        "<p><?php $a = 1 ?></p><?=$a?>\n",
        &format!(
            "{}<?php $a = 1 ; {}$a?>\n",
            blanked("<p>"),
            blanked("</p><?=")
        ),
    );
    // No code before `?>`: nothing to end.
    assert_view(
        "<p><?php ?></p><?php foo(); ?></p>\n",
        &format!(
            "{}<?php {} foo(); ?>{}",
            blanked("<p>"),
            blanked("?></p><?php"),
            blanked("</p>\n")
        ),
    );
    // A block holding only inline text keeps a statement; a switch body before `case` does not.
    assert_view(
        "<?php if ($a) { ?>\n<p>a</p>\n<?php } ?>\n",
        &format!(
            "<?php if ($a) {{ ; {} }} ?>\n",
            blanked("\n<p>a</p>\n<?php")
        ),
    );
    assert_view(
        "<?php switch ($x) { ?>\n<?php case 1: ?>\n<p>a</p>\n<?php } ?>\n",
        &format!(
            "<?php switch ($x) {{ {} case 1: {} }} ?>\n",
            blanked("?>\n<?php"),
            blanked("?>\n<p>a</p>\n<?php")
        ),
    );
    // A `}` may end an expression (a closure, a `match`, a dynamic member), which the `?>` ends.
    for code in [
        "function f() {}",
        "$x = function() {}",
        "$x = match ($a) { 1 => 2 }",
        "echo $o->{$field}",
    ] {
        assert_view(
            &format!("<?php {code} ?>html<?php eval(1);\n"),
            &format!("<?php {code} ; {} eval(1);\n", blanked("html<?php")),
        );
    }
    // Unless what follows continues the statement the brace closes.
    for next in [
        "else",
        "elseif ($b)",
        "catch (E $e)",
        "finally",
        "while ($a);",
    ] {
        assert_view(
            &format!("<?php if ($a) {{ f(); }} ?>\n<?php {next} {{}}\n"),
            &format!(
                "<?php if ($a) {{ f(); }} {} {next} {{}}\n",
                blanked("?>\n<?php")
            ),
        );
    }
}

#[test]
fn a_close_tag_in_a_string_or_block_comment_stays_php() {
    assert_view(
        "<?php $s = '?>'; $t = \"?>\"; $u = 'it\\'s ?>'; /* ?> */ ?>\n<p>x</p>\n",
        &format!(
            "<?php $s = '?>'; $t = \"?>\"; $u = 'it\\'s ?>'; /* ?> */ ?>{}",
            blanked("\n<p>x</p>\n")
        ),
    );
    // A string an interpolation nests is part of the outer string, `?>` and all.
    for literal in [
        "\"{$obj->{\"?>\"}}\"",
        "\"{$a[\"?>\"]}\"",
        "\"${a[\"?>\"]}\"",
        "`{$a[\"?>\"]}`",
    ] {
        assert_view(
            &format!("<?php $s = {literal}; eval(1); ?>\n<p>x</p>\n"),
            &format!(
                "<?php $s = {literal}; eval(1); ?>{}",
                blanked("\n<p>x</p>\n")
            ),
        );
    }
}

#[test]
fn a_close_tag_ends_a_line_comment() {
    assert_view(
        "<?php foo(); // c ?>\n<p>x</p>\n<?php bar();\n",
        &format!(
            "<?php foo(); /* */{} bar();\n",
            blanked("?>\n<p>x</p>\n<?php")
        ),
    );
    assert_view(
        "<?php foo(); # c ?>\n<p>x</p>\n<?php bar();\n",
        &format!(
            "<?php foo(); /**/{} bar();\n",
            blanked("?>\n<p>x</p>\n<?php")
        ),
    );
    // The statement's `;` follows the closed comment.
    assert_view(
        "<?php foo() // c ?>\n<p>x</p>\n<?php bar();\n",
        &format!(
            "<?php foo() /* */; {} bar();\n",
            blanked("\n<p>x</p>\n<?php")
        ),
    );
    assert_view(
        "<?php foo()// comment ?>\n<?php bar();\n",
        &format!(
            "<?php foo()/*{}*/; {} bar();\n",
            " ".repeat("// comment ".len() - 4),
            blanked("\n<?php")
        ),
    );
    // A comment too short for `/**/` is blanked.
    assert_view(
        "<?php foo() # ?>\n<?php bar();\n",
        &format!("<?php foo()   ; {} bar();\n", blanked("\n<?php")),
    );
    // A final `?>` ends the comment in the grammar too.
    assert_view(
        "<?php foo() // c ?>\n<p>x</p>\n",
        &format!("<?php foo() // c ?>{}", blanked("\n<p>x</p>\n")),
    );
    // `#[` is an attribute: its string's `?>` is no tag.
    assert_view(
        "<?php #[A('?>')] function f() {} ?>\n<p>x</p>\n<?php f();\n",
        &format!(
            "<?php #[A('?>')] function f() {{}} ; {} f();\n",
            blanked("\n<p>x</p>\n<?php")
        ),
    );
}

#[test]
fn a_close_tag_in_a_heredoc_or_nowdoc_stays_php() {
    for opening in ["<<<EOT", "<<<'EOT'", "<<<\"EOT\"", "<<< EOT"] {
        let source = format!("<?php $s = {opening}\n?> <p>\nEOT;\n?>\n<p>x</p>\n");
        assert_view(
            &source,
            &format!(
                "<?php $s = {opening}\n?> <p>\nEOT;\n?>{}",
                blanked("\n<p>x</p>\n")
            ),
        );
    }
    // PHP 7.3's indented closing label, followed by any byte that cannot continue it.
    assert_view(
        "<?php\n    f(<<<EOT\n    ?>\n    EOTX\n    EOT) ?>\n<p>x</p>\n<?php g();\n",
        &format!(
            "<?php\n    f(<<<EOT\n    ?>\n    EOTX\n    EOT) ; {} g();\n",
            blanked("\n<p>x</p>\n<?php")
        ),
    );
}

#[test]
fn line_ends_and_multi_byte_text_keep_their_places() {
    assert_view(
        "<?php foo(); ?>\r\n<p>x</p>\r\n<?php bar(); ?>\r\n",
        &format!(
            "<?php foo(); {} bar(); ?>\r\n",
            blanked("?>\r\n<p>x</p>\r\n<?php")
        ),
    );
    assert_view(
        "<p>héllo ✓</p>\r<?php $s = 'é'; ?>\n<p>✓</p>\n",
        &format!(
            "{}<?php $s = 'é'; ?>{}",
            blanked("<p>héllo ✓</p>\r"),
            blanked("\n<p>✓</p>\n")
        ),
    );
}

#[test]
fn a_file_without_an_open_tag_is_all_inline_text() {
    let source = "<html>\n<?xml version=\"1.0\"?>\n<?phpx f(); ?>\n</html>\n";
    assert_view(source, &blanked(source));
}

#[test]
fn a_short_open_tag_opens_php() {
    // PHP's compiled default (`short_open_tag`) and tree-sitter-php's `php_tag` read `<?` before a
    // blank as an open tag.
    assert_view(
        "<? eval($a); ?>\n<p>x</p>\n<?\n$b();\n",
        &format!("<? eval($a); {}\n$b();\n", blanked("?>\n<p>x</p>\n<?")),
    );
}

#[test]
fn the_data_after_halt_compiler_is_no_php() {
    assert_view(
        "<?php f();\n__halt_compiler();\neval('x'); ?>\n<p>",
        &format!(
            "<?php f();\n__halt_compiler();{}",
            blanked("\neval('x'); ?>\n<p>")
        ),
    );
    // A `?>` ends the call, as it ends any statement.
    assert_view(
        "<?php __HALT_COMPILER() ?>data eval('x');\n",
        &format!("<?php __HALT_COMPILER() ; {}", blanked("data eval('x');\n")),
    );
    // Not the directive, which PHP reads only as a statement: a longer name, a variable call, a
    // method or static method, a call inside an expression. Each file is pure PHP: its own view.
    for source in [
        "<?php __halt_compilers(); eval('1');\n",
        "<?php $o->x__halt_compiler(); eval('1');\n",
        "<?php $__halt_compiler = static fn() => 0; $__halt_compiler(); eval('1');\n",
        "<?php $o->__halt_compiler(); eval('1');\n",
        "<?php $o?->__halt_compiler(); eval('1');\n",
        "<?php Foo::__halt_compiler(); eval('1');\n",
        "<?php f(__halt_compiler()); eval('1');\n",
    ] {
        assert!(matches!(view(source), Cow::Borrowed(_)), "{source:?}");
    }
}

/// The minimal templates of NER-1808 and of its review, which the grammar read with errors.
const TEMPLATES: &[&str] = &[
    INLINE,
    INLINE_REAL_EVAL,
    IN_HTML,
    ALTERNATIVE,
    SHORT_ECHO,
    "<p><?= $a, $b ?></p>\n",
    "<? eval($a); ?>\n<p>x</p>\n",
    "<?php $s = \"{$obj->{\"?>\"}}\"; eval(1); ?>\n<p>x</p>\n",
    "<?php $x = function() {} ?>html<?php eval(1);\n",
    "<?php $x = match ($a) { 1 => 2 } ?>html<?php eval(1);\n",
    "<?php echo $o->{$field} ?>html<?php eval(1);\n",
    "<?php if ($a) { ?>\n<p>a</p>\n<?php } else { ?>\n<p>b</p>\n<?php } ?>\n",
    "<p><?= htmlspecialchars($t) ?></p>\n",
    "<p><?= new Foo() ?></p>\n",
    "<p><?= eval('i') ?></p>\n",
];

#[test]
fn templates_parse_clean_and_read_the_source() {
    for &source in TEMPLATES {
        assert!(
            GrammarDialect::Php
                .ast_grep(source)
                .root()
                .get_doc()
                .tree
                .root_node()
                .has_error(),
            "{source:?} parsed clean without its view"
        );
        let root = ast_grep(GrammarDialect::Php, source);
        assert!(
            !root.root().get_doc().tree.root_node().has_error(),
            "{source:?}"
        );
        // The program starts at its first token, as for any file.
        assert_eq!(root.root().text(), &source[root.root().range()]);
    }
}

/// The shipped `phpcs-squiz-php-eval`: every `eval` construct.
fn squiz_php_eval() -> crate::code_rule_dsl::CompiledCodeRuleDsl {
    crate::code_rule_dsl::CodeRuleDslRuntime::compile(
        include_str!("../../../../packs/php/rules/phpcs-squiz-php-eval.json").to_owned(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason))
}

fn violation_lines(result: &Value, source: &str) -> Vec<usize> {
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            source[..start].matches('\n').count() + 1
        })
        .collect()
}

#[test]
fn inline_text_is_no_php_code_to_a_rule() {
    let compiled = squiz_php_eval();
    // JavaScript's `eval` after `?>` is inline text, and the file reads clean.
    assert_eq!(
        violation_lines(
            &evaluate_file(&compiled, "php", "inline.php", INLINE),
            INLINE
        ),
        Vec::<usize>::new()
    );
    // PHP's `eval` in each block is still found.
    assert_eq!(
        violation_lines(
            &evaluate_file(&compiled, "php", "inline-real-eval.php", INLINE_REAL_EVAL),
            INLINE_REAL_EVAL
        ),
        [2, 5]
    );
    // The review's templates: `eval` after a nested interpolation's `?>`, and after a closure, a
    // `match` and a dynamic member whose `}` the `?>` ends, is found, and each reads clean.
    for source in &TEMPLATES[7..11] {
        assert_eq!(
            violation_lines(
                &evaluate_file(&compiled, "php", "review.php", source),
                source
            ),
            [1],
            "{source:?}"
        );
    }
    // `eval` right after `<?=` is found too.
    let source = "<p><?= eval('i') ?></p>\n";
    assert_eq!(
        violation_lines(&evaluate_file(&compiled, "php", "echo.php", source), source),
        [1]
    );
    // Code after `__halt_compiler();` is data: its `eval` is no finding.
    let source = "<?php f();\n__halt_compiler();\neval('x');\n";
    assert_eq!(
        violation_lines(&evaluate_file(&compiled, "php", "halt.php", source), source),
        Vec::<usize>::new()
    );
    // A variable or method named `__halt_compiler` stops nothing: the `eval` after it is found.
    for source in [
        "<?php $__halt_compiler = static fn() => 0; $__halt_compiler(); eval('1');\n",
        "<?php $o->__halt_compiler(); eval('1');\n",
    ] {
        assert_eq!(
            violation_lines(&evaluate_file(&compiled, "php", "call.php", source), source),
            [1],
            "{source:?}"
        );
    }
}
