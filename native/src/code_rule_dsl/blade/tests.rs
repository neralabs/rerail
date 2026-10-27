//! Blade's semantics through compiled code rules: what each predicate finds in a template's
//! embedded PHP, its compiled view (`ast.embedded: compiled-view`).

use crate::code_rule_dsl::test_support::*;
use serde_json::{Value as Json, json};

/// A rule over the template's compiled view.
fn run(rule: Json, source: &str) -> Vec<String> {
    run_in("blade", "view.blade.php", rule, source)
}

/// A rule over the template's own tree.
fn run_template(rule: Json, source: &str) -> Vec<String> {
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(
        clause_document(json!({
            "id": "blade",
            "forbid": { "ast": { "language": "blade", "rule": rule } },
            "violation": violation(),
        })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    matched_texts(
        &evaluate_file(&compiled, "blade", "view.blade.php", source),
        source,
    )
}

/// The same rule on a PHP file: what the embedded program's PHP answers.
fn run_php(rule: Json, source: &str) -> Vec<String> {
    run_in("php", "view.php", rule, source)
}

fn run_in(language: &str, path: &str, rule: Json, source: &str) -> Vec<String> {
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let mut ast = json!({ "language": language, "rule": rule });
    if language == "blade" {
        ast["embedded"] = json!("compiled-view");
    }
    let compiled = compile_document_with(
        clause_document(json!({
            "id": language,
            "forbid": { "ast": ast },
            "violation": violation(),
        })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    matched_texts(&evaluate_file(&compiled, language, path, source), source)
}

const TEMPLATE: &str = "\
@php($limit = 10)
@php
    $x = $limit;
@endphp
{{ $x }}
{{ $user->name }}
@foreach ($items as $item)
    {{ $loop->index }} {{ $item }}
@endforeach
{!! route('dashboard', ['id' => 3]) !!}
";

#[test]
fn blade_variables_follow_and_resolve_through_code_rules() {
    // The `$x` read follows to `$limit`, whose read follows to 10; neither definition is a read.
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "value": { "follow": true, "eq": 10 } }),
            TEMPLATE
        ),
        ["$limit", "$x"]
    );
    // `@foreach` assigns `$loop` where the directive stands.
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "resolves": { "binding": { "form": "assignment" } } }),
            TEMPLATE
        ),
        ["$limit", "$x", "$loop"]
    );
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "resolves": { "binding": { "value": { "kind": "integer" } } } }),
            TEMPLATE
        ),
        ["$limit"]
    );
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "resolves": { "binding": { "form": "for" } } }),
            TEMPLATE
        ),
        ["$item"]
    );
    // `$user` is view data, defined outside the template: neither undefined nor defined.
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "resolves": { "unresolved": true } }),
            TEMPLATE
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        run(
            json!({ "kind": "variable_name", "regex": "^\\$user$", "not": { "resolves": { "unresolved": true } } }),
            TEMPLATE
        ),
        Vec::<String>::new()
    );
}

/// An application's directive (`@custom(...)`) may compile to PHP that reads or assigns any
/// variable: no binding is known unused, and no value is known after it.
#[test]
fn application_directives_may_read_and_assign_every_variable() {
    let unused =
        json!({ "kind": "variable_name", "binds": { "form": "assignment", "used": false } });
    let follows = json!({ "kind": "variable_name", "value": { "follow": true, "eq": 1 } });
    let plain = "@php\n    $x = 1;\n    $y = 2;\n@endphp\n{{ $x }}\n";
    assert_eq!(run(unused.clone(), plain), ["$y"]);
    assert_eq!(run(follows.clone(), plain), ["$x"]);
    for custom in [
        "@php\n    $x = 1;\n    $y = 2;\n@endphp\n@custom($y)\n{{ $x }}\n",
        "@php\n    $x = 1;\n    $y = 2;\n@endphp\n@feature($y)\n@endfeature\n{{ $x }}\n",
    ] {
        assert_eq!(
            run(unused.clone(), custom),
            Vec::<String>::new(),
            "{custom}"
        );
        assert_eq!(
            run(follows.clone(), custom),
            Vec::<String>::new(),
            "{custom}"
        );
    }
}

#[test]
fn blade_calls_read_their_arguments_and_values() {
    assert_eq!(
        run(
            json!({ "kind": "function_call_expression",
                    "arguments": { "signature": ["name", "parameters"], "bind": { "R": "name" } },
                    "has": { "stopBy": "end", "kind": "integer", "value": { "eq": 3 } } }),
            TEMPLATE
        ),
        ["route('dashboard', ['id' => 3]) R='dashboard'"]
    );
}

const BOUNDARIES: &str = "\
<script src=\"{{ asset('app.js') }}\" nonce=\"{{ $nonce }}\">
    const escaped = @@json(exec('escaped'));
    const read = @json(system('read'));
</script>
@guarded(exec('custom'))
    {{ exec('body') }}
@endguarded
@if ($ok /* ) */ && exec('condition'))
@endif
<?php $x = 1; // trailing; ?>{{ exec('echo') }}
";

/// The calls a code rule finds are the PHP Blade compiles: a `<script>` tag's attributes, a directive
/// in its text but not an escaped one (`@@json(...)`), the body of a pair Blade does not compile
/// but not its arguments, a condition past a `)` in a comment, and an echo after a `<?php` block
/// whose line comment `?>` ends.
#[test]
fn blade_calls_are_the_php_blade_compiles() {
    assert_eq!(
        run(
            json!({ "kind": "function_call_expression", "regex": "^(asset|exec|system)\\(" }),
            BOUNDARIES
        ),
        [
            "asset('app.js')",
            "system('read')",
            "exec('body')",
            "exec('condition')",
            "exec('echo')"
        ]
    );
}

/// A pattern of a clause over the compiled view is PHP's and matches the view; one of a clause over
/// the template is Blade's and matches the template.
#[test]
fn blade_php_patterns_match_the_embedded_program() {
    let source =
        "{{ sprintf('%s', $a) }}\n@php($x = sprintf('%d', 1, 2))\n<p>sprintf($a)</p>\n@csrf\n";
    assert_eq!(
        run(json!({ "pattern": "sprintf($F, $$$A)" }), source),
        [
            "sprintf('%s', $a) A=$a F='%s'",
            "sprintf('%d', 1, 2) F='%d'"
        ]
    );
    assert_eq!(
        run(
            json!({ "pattern": { "context": "class A { function f() { return $X; } }", "selector": "return_statement" } }),
            "@php\nclass A { function f() { return 1; } }\n@endphp\n"
        ),
        ["return 1; X=1"]
    );
    assert_eq!(
        run_template(json!({ "pattern": "@csrf" }), source),
        ["@csrf"]
    );
    assert!(run_template(json!({ "pattern": "sprintf($F, $$$A)" }), source).is_empty());
}

const FORMATS: &str = "\
{{ sprintf('%s and %s', $a) }}
@php($x = sprintf('%d', 1, 2))
{{ vsprintf('%s', [$a, $b]) }}
@php($n = sscanf($line, '%d %[a-z]', $first))
{{ sprintf(\"%s $a\", $b) }}
";

/// `formatArgs` and the `printfFormat`/`scanfFormat` grammars read the format calls and strings of
/// the PHP a template embeds as PHP's.
#[test]
fn blade_format_calls_read_their_formats() {
    let printf = |side: &str, rule: Json| {
        let mut test = json!({ "style": "printf", "string": "F", "args": "A" });
        test[side] = rule;
        json!({ "any": [
            { "pattern": "sprintf($F, $$$A)", "formatArgs": test.clone() },
            { "pattern": "vsprintf($F, $A)", "formatArgs": test },
        ] })
    };
    // The second `%s` has no value; `vsprintf`'s `$b` fills no field; `2` fills none.
    assert_eq!(
        run(printf("missing", json!({ "kind": "placeholder" })), FORMATS),
        ["sprintf('%s and %s', $a) A=$a F='%s and %s'"]
    );
    assert_eq!(
        run(
            printf(
                "unused",
                json!({ "any": [{ "kind": "argument" }, { "kind": "array_element_initializer" }] })
            ),
            FORMATS
        ),
        [
            "sprintf('%d', 1, 2) F='%d'",
            "vsprintf('%s', [$a, $b]) A=[$a, $b] F='%s'"
        ]
    );
    // `sscanf` assigns its second field to no variable.
    assert_eq!(
        run(
            json!({ "pattern": "sscanf($S, $F, $$$A)", "formatArgs": { "style": "scanf", "string": "F", "args": "A", "missing": { "kind": "placeholder" } } }),
            FORMATS
        ),
        ["sscanf($line, '%d %[a-z]', $first) A=$first F='%d %[a-z]' S=$line"]
    );
    // `phpDoc` parses the embedded PHP's doc comments, not Blade's `{{-- --}}` comments (`comment`
    // names Blade's kind in a Blade rule; `parse` finds the PHP comments itself).
    let docs = "{{-- @var int $x --}}\n@php\n/** @var int $count */\n$count = 1;\n@endphp\n";
    assert_eq!(
        run(
            json!({ "parse": { "grammar": "phpDoc", "has": { "kind": "var_tag_value", "stopBy": "end" } } }),
            docs
        ),
        ["/** @var int $count */"]
    );
    // The interpolated string's format is unknown; PHP refuses `%[` in a printf format.
    let strings = json!([{ "kind": "string" }, { "kind": "encapsed_string" }]);
    assert_eq!(
        run(
            json!({ "any": strings, "parse": { "grammar": "printfFormat", "has": { "kind": "placeholder", "capture": "P" } } }),
            FORMATS
        ),
        ["'%s and %s' P=%s", "'%d' P=%d", "'%s' P=%s"]
    );
    assert_eq!(
        run(
            json!({ "any": strings, "parse": { "grammar": "scanfFormat", "has": { "kind": "placeholder", "regex": "^%\\[" } } }),
            FORMATS
        ),
        ["'%d %[a-z]'"]
    );
}

/// `compare`'s text modes read the embedded PHP as they read a PHP file; a node Blade wrote a
/// token into (`@unless(c)`'s `!(`) compares as unknown.
#[test]
fn blade_compare_modes_read_the_embedded_php() {
    let compare = |text: &str| json!({ "pattern": "$L === $R", "compare": { "left": "L", "op": "eq", "right": "R", "text": text } });
    let conditions = [
        "$user->name === $user['name']",
        "$user->name === $user->name",
        "PHP_EOL === 'PHP_EOL'",
        "[1, 'a' => 2] === array(1, \"a\" => 2)",
        "$o->{'x'} === $o->x",
    ];
    let blade: String = conditions
        .iter()
        .map(|condition| format!("@if({condition}) @endif\n"))
        .collect();
    let php: String = std::iter::once("<?php\n".to_owned())
        .chain(
            conditions
                .iter()
                .map(|condition| format!("if ({condition}) {{}}\n")),
        )
        .collect();
    let line = |index: usize| {
        let (left, right) = conditions[index].split_once(" === ").expect("a comparison");
        format!("{} L={left} R={right}", conditions[index])
    };
    for (text, expected) in [
        ("key", vec![line(0), line(1), line(2), line(4)]),
        ("member", vec![line(0), line(1), line(2), line(4)]),
        ("reference", vec![line(1), line(4)]),
        ("name", vec![line(2)]),
        ("structure", vec![line(1), line(3)]),
        ("structure-hash", vec![line(1), line(3)]),
    ] {
        let found = run(compare(text), &blade);
        assert_eq!(found, expected, "{text}");
        assert_eq!(found, run_php(compare(text), &php), "{text} against PHP");
    }
    // The `@unless` condition is `!($a)` with a `!(` the template does not spell: neither equal
    // nor not equal to the `!($a)` the template writes.
    let pair = |op: &str| {
        json!({
            "kind": "program",
            "has": { "stopBy": "end", "kind": "assignment_expression", "has": { "field": "right", "pattern": "$L" } },
            "all": [{ "has": { "stopBy": "end", "pattern": "$R", "kind": "unary_op_expression", "inside": { "kind": "if_statement", "stopBy": "end" } } }],
            "compare": { "left": "L", "op": op, "right": "R", "text": "structure" },
        })
    };
    let unless = "@php($b = !($a))\n@unless($a) @endunless\n";
    assert!(run(pair("eq"), unless).is_empty());
    assert!(run(pair("ne"), unless).is_empty());
    let spelled = "@php($b = !($a))\n@if(!($a)) @endif\n";
    assert_eq!(run(pair("eq"), spelled).len(), 1);
}

/// `arguments` reads named arguments by keyword in `syntactic` mode, as in PHP.
#[test]
fn blade_syntactic_arguments_find_named_arguments() {
    assert_eq!(
        run(
            json!({ "kind": "function_call_expression",
                    "arguments": { "mode": "syntactic", "signature": ["name", "parameters"], "bind": { "P": "parameters" } } }),
            "{{ route(parameters: ['id' => 3], name: 'home') }}\n"
        ),
        ["route(parameters: ['id' => 3], name: 'home') P=['id' => 3]"]
    );
}

const BINDINGS: &str = "\
@use('App\\Support\\Str')
@php
    const LIMIT = 5;
    $count = LIMIT;
    $count = 2;
    [$first, $second] = [1, 2];
    $twice = fn ($n) => $n * $count;
@endphp
{{ Str::limit($title, LIMIT) }} {{ $count }} {{ $first }}
";

const BINDINGS_PHP: &str = "<?php
use App\\Support\\Str;
const LIMIT = 5;
$count = LIMIT;
$count = 2;
[$first, $second] = [1, 2];
$twice = fn ($n) => $n * $count;
echo Str::limit($title, LIMIT); echo $count; echo $first;
";

/// The binding keys and flags PHP reads from its flow read Blade's embedded program as they read
/// the same PHP in a file.
#[test]
fn blade_binding_flags_read_the_flow() {
    let binds = |filter: Json| json!({ "kind": "variable_name", "binds": filter });
    let binding =
        |binding: Json| json!({ "kind": "variable_name", "resolves": { "binding": binding } });
    for (rule, expected) in [
        // The first `$count` is overwritten before the template ends.
        (binds(json!({ "latest": false })), vec!["$count"]),
        (binds(json!({ "outer": true })), vec![]),
        (
            binds(json!({ "redefines": { "form": "assignment" } })),
            vec!["$count"],
        ),
        (
            binds(
                json!({ "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } } }),
            ),
            vec!["$count", "$count", "$n"],
        ),
        // The arrow function's and the echo's `$count` read `$count = 2`.
        (
            binding(json!({ "value": { "kind": "integer" } })),
            vec!["$count", "$count", "$first"],
        ),
        (
            binding(json!({ "destructuredValue": false, "value": { "kind": "integer" } })),
            vec!["$count", "$count"],
        ),
        (binding(json!({ "only": true })), vec!["$n", "$first"]),
        // `LIMIT` follows to its declaration in the template.
        (
            json!({ "kind": "name", "value": { "follow": true, "eq": 5 } }),
            vec!["LIMIT", "LIMIT"],
        ),
        // `@use` imports the class, as a `use` clause names it.
        (
            json!({ "resolves": { "imported": ["App\\Support\\Str"] } }),
            vec!["App\\Support\\Str"],
        ),
    ] {
        let found = run(rule.clone(), BINDINGS);
        assert_eq!(found, expected, "{rule}");
        assert_eq!(
            found,
            run_php(rule.clone(), BINDINGS_PHP),
            "{rule} against PHP"
        );
    }
}

/// What the PHP a template embeds genuinely lacks is refused as PHP's; semantics over the
/// template's own tree are refused as the compiled view's.
#[test]
fn blade_refusals_say_what_its_php_lacks() {
    let refusal_in = |ast: Json| {
        let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
        match compile_document_with(
            clause_document(json!({
                "id": "blade",
                "forbid": { "ast": ast },
                "violation": violation(),
            })),
            &capabilities,
        ) {
            Ok(_) => panic!("compiled"),
            Err(error) => error.reason,
        }
    };
    let refusal = |rule: Json| {
        refusal_in(json!({ "language": "blade", "embedded": "compiled-view", "rule": rule }))
    };
    for rule in [
        json!({ "kind": "directive", "resolves": { "unresolved": true } }),
        json!({ "kind": "echo", "has": { "kind": "directive", "value": { "eq": 1 } } }),
    ] {
        let reason = refusal_in(json!({ "language": "blade", "rule": rule }));
        assert!(
            reason.contains("reads blade's embedded program, not the file's own tree"),
            "{reason}"
        );
    }
    let format_args = |style: &str| json!({ "pattern": "f($S, $$$A)", "formatArgs": { "style": style, "string": "S", "args": "A", "unused": { "kind": "argument" } } });
    for (rule, message) in [
        (
            json!({ "kind": "function_call_expression", "arguments": { "mode": "estree", "signature": ["a"], "bind": { "A": "a" } } }),
            "arguments/mode: `estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
        ),
        (
            json!({ "pattern": "$L == $R", "compare": { "left": "L", "op": "eq", "right": "R", "text": "structure-parts" } }),
            "compare/text: `structure-parts` compares Python's implicitly concatenated string parts; PHP has no implicit string concatenation",
        ),
        (
            json!({ "kind": "variable_name", "resolves": { "binding": { "stopAtGlobal": false } } }),
            "binding/stopAtGlobal: a php read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so php has no lookup through it",
        ),
        (
            json!({ "kind": "variable_name", "binds": { "exported": true } }),
            "binds/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none",
        ),
        (
            format_args("percent"),
            "formatArgs/style: `percent` reads Python's `%` formats; PHP's printf-family calls are style `printf`",
        ),
        (
            json!({ "kind": "string", "parse": { "grammar": "percentFormat", "regex": "." } }),
            "parse/grammar: `percentFormat` parses Python literals; PHP's printf formats are `printfFormat`",
        ),
    ] {
        let reason = refusal(rule);
        assert!(reason.ends_with(message), "{reason}");
    }
}

/// `calls` follows a template's calls into the workspace's PHP definitions: a function, a
/// constructor through `new`, through a chain; `$this` in a template is no class's: unknown.
#[test]
fn blade_calls_reach_workspace_definitions() {
    let capabilities: Json = serde_json::from_str(CAPABILITIES).expect("capabilities");
    let compiled = compile_document_with(
        clause_document(json!({
            "id": "blade-calls",
            "forbid": { "ast": { "language": "blade", "embedded": "compiled-view", "rule": {
                "calls": { "has": { "stopBy": "end", "kind": "function_call_expression", "regex": "^exec\\(" } }
            } } },
            "violation": violation(),
        })),
        &capabilities,
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let template = "{{ \\App\\run_it() }}\n{{ \\App\\safe() }}\n@php($r = new \\App\\Runner())\n{{ $this->run() }}\n";
    let files = [
        (
            "app/helpers.php",
            "php",
            "<?php\nnamespace App;\nfunction run_it() { inner(); }\nfunction inner() { exec('ls'); }\nfunction safe() { return 1; }\nclass Runner { function __construct() { exec('ls'); } }\n",
        ),
        ("resources/views/page.blade.php", "blade", template),
    ];
    let snapshot = json!({
        "facts": [], "format": "rerail.repository-snapshot/v1", "revision": 0,
        "files": files.iter().map(|(path, language, source)| json!({ "language": language, "path": path, "source": source })).collect::<Vec<_>>(),
    });
    let output: Json = serde_json::from_str(
        &compiled
            .evaluate(snapshot.to_string(), "report".to_owned())
            .expect("evaluation succeeds"),
    )
    .expect("JSON");
    let found: Vec<&str> = output["violations"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|violation| violation["path"] == "resources/views/page.blade.php")
        .map(|violation| {
            let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
            let end = violation["span"]["endByte"].as_u64().expect("end") as usize;
            &template[start..end]
        })
        .collect();
    assert_eq!(
        found,
        ["\\App\\run_it()", "new \\App\\Runner()"],
        "{output}"
    );
}

/// Blade reads a `(` as a directive's arguments only after spaces or tabs (`[ \t]*` in
/// `compileStatements`); Forte's lexer also skips line breaks. A `(` on the next line is the
/// template's: the echo in it is PHP, the call in its text is not, and Forte's errors in the
/// arguments it read there (`(it's required)`: an unclosed string) are not the template's, so
/// `resolves`, which decides only in a template without syntax errors, decides.
#[test]
fn a_paren_on_the_next_line_is_not_arguments() {
    let rule = json!({
        "kind": "function_call_expression",
        "has": { "field": "function", "resolves": { "import": ["sprintf"] } }
    });
    for (source, expected) in [
        (
            "@foreach($items as $item)\n{{ $item }}\n@endforeach\n({{ sprintf('%d %d', 1) }} total)",
            &["sprintf('%d %d', 1)"][..],
        ),
        (
            "@if($a)\nx\n@else\n({{ sprintf('%d %d', 1) }})\n@endif",
            &["sprintf('%d %d', 1)"],
        ),
        (
            "@csrf\n(it's required)\n<p>{{ sprintf('%d %d', 1) }}</p>",
            &["sprintf('%d %d', 1)"],
        ),
        (
            "@csrf\n(it's required)\n<p>{{ sprintf('%d %d', 1) }}</p>\n",
            &["sprintf('%d %d', 1)"],
        ),
        ("@auth\n  (sprintf('%d %d', 1))\n@endauth", &[]),
        ("@csrf\n(sprintf('%d %d', 1))", &[]),
        ("@if \t(sprintf('%d', 1))\n@endif", &["sprintf('%d', 1)"]),
    ] {
        assert_eq!(run(rule.clone(), source), expected, "{source:?}");
    }
}

/// Lines (1-based) a shipped Blade pack rule reports on `source`, which must evaluate without a
/// failure.
fn pack_rule_lines(document: &str, source: &str) -> Vec<u64> {
    let compiled = crate::code_rule_dsl::CodeRuleDslRuntime::compile(
        document.to_owned(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let result = evaluate_file(&compiled, "blade", "resources/views/view.blade.php", source);
    assert_eq!(result["failures"], json!([]), "{result}");
    result["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .map(|violation| violation["span"]["startLine"].as_u64().expect("line") + 1)
        .collect()
}

/// The templates tree-sitter-blade read as one error region or as text
///: a directive inside an element, in a start tag or an attribute value, and an echo
/// in `<script>`. Forte reads each as Blade.
#[test]
fn directives_in_elements_tags_and_scripts_are_reported() {
    let debug = include_str!("../../../../packs/blade/rules/blade-parser-debug-directive.json");
    let dump = include_str!("../../../../packs/blade/rules/tlint-no-dump-directives.json");
    let ray = include_str!("../../../../packs/blade/rules/tlint-no-ray-directive.json");
    for (source, line) in [
        ("<div>\n@dump($x)\n</div>\n", 2),
        (
            "<div>\n    @if ($d)\n        @dump($f)\n    @endif\n</div>\n",
            3,
        ),
        ("@if($a)\n<div>\n@dump($x)\n</div>\n@endif\n", 3),
        ("<div @if($a) class=\"x\" @endif>\n@dump($x)\n</div>\n", 2),
        ("<p>y</p>\n@dump($x)\n", 2),
        ("@if($w)\n<br>\n@dump($x)\n@endif\n", 3),
    ] {
        assert_eq!(pack_rule_lines(debug, source), [line], "{source:?}");
        assert_eq!(pack_rule_lines(dump, source), [line], "{source:?}");
    }
    let ray_source = "<div>\n    @if ($d)\n        @ray($f)\n    @endif\n</div>\n";
    assert_eq!(pack_rule_lines(ray, ray_source), [3]);
    let spacing = "<input @if($a == 1) checked @endif>\n<div class=\"x @if($a) y @endif\">z</div>\n@if($b)\nx\n@endif\n<script>\nvar n = '{{$prefix}}';\n</script>\n<p>{{$prefix}}</p>\n";
    let space_after = include_str!(
        "../../../../packs/blade/candidates/rules/tlint-space-after-blade-directives.json"
    );
    let render_content = include_str!(
        "../../../../packs/blade/candidates/rules/tlint-spaces-around-blade-render-content.json"
    );
    assert_eq!(pack_rule_lines(space_after, spacing), [1, 2, 3]);
    assert_eq!(pack_rule_lines(render_content, spacing), [7, 9]);
}
