    use crate::code_rule_dsl::CompiledCodeRuleDsl;
    use crate::code_rule_dsl::test_support::*;
    use napi::Result;
    use serde_json::{Value, json};

    const SOURCE: &str = "<?php
function f(bool $c, array $items) {
    global $config;
    $config = 1;
    $limit = 10;
    $limit = 20;
    [$a, $b] = [1, 2];
    echo $a, $b, $limit, $config;
    if ($c) {
        global $maybe;
    }
    $maybe = 1;
    $once = 5;
    echo $once;
    foreach ($items as $item) {
        $last = $item;
    }
    echo $last ?? null;
    $fn = fn() => $once + $item;
}
";

    fn compile(rule: Value) -> Result<CompiledCodeRuleDsl> {
        compile_document_with(
            clause_document(json!({
                "id": "php-bindings",
                "forbid": { "ast": { "language": "php", "rule": rule } },
                "violation": violation(),
            })),
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
    }

    /// The lines of the variables a rule matches in `SOURCE`, with their text.
    fn run(rule: Value) -> Vec<String> {
        let compiled = compile(rule).unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "f.php", SOURCE);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
                let end = violation["span"]["endByte"].as_u64().expect("end") as usize;
                let line = SOURCE[..start].matches('\n').count() + 1;
                format!("{line}:{}", &SOURCE[start..end])
            })
            .collect()
    }

    fn binds(filter: Value) -> Vec<String> {
        run(json!({ "kind": "variable_name", "binds": filter }))
    }

    fn binding(binding: Value) -> Vec<String> {
        run(json!({ "kind": "variable_name", "resolves": { "binding": binding } }))
    }

    #[test]
    fn outer_bindings_write_a_global_variable_on_every_path() {
        // `$maybe = 1` writes the global only where `$c` held: neither outer nor not.
        assert_eq!(
            binds(json!({ "outer": true })),
            ["3:$config", "4:$config", "10:$maybe"]
        );
        assert!(!binds(json!({ "outer": false })).contains(&"12:$maybe".to_owned()));
        assert!(binds(json!({ "outer": false })).contains(&"5:$limit".to_owned()));
    }

    #[test]
    fn latest_bindings_reach_their_scope_end() {
        assert_eq!(
            binds(json!({ "latest": false })),
            ["3:$config", "5:$limit", "10:$maybe"]
        );
    }

    #[test]
    fn redefines_reads_the_bindings_a_binding_overwrites() {
        // `$limit = 20` overwrites `$limit = 10`, which nothing read. `$config = 1` overwrites
        // the global's binding, whose use another function may make: unknown.
        assert_eq!(
            binds(json!({ "redefines": { "used": false } })),
            ["6:$limit"]
        );
        assert_eq!(
            binds(json!({ "redefines": { "form": "global" } })),
            ["4:$config", "12:$maybe"]
        );
        // The loop's `$last` overwrites itself on a second iteration.
        assert_eq!(
            binds(
                json!({ "form": "assignment", "redefines": { "sameScope": true, "every": true, "form": "assignment" } })
            ),
            ["6:$limit", "16:$last"]
        );
    }

    #[test]
    fn occurrences_are_every_node_naming_the_variable() {
        // The arrow function reads `$once` and `$item` of `f`, which it copies.
        assert_eq!(
            binds(
                json!({ "form": ["assignment", "for"], "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } } })
            ),
            ["13:$once", "15:$item"]
        );
        // The read in the arrow function is defined on some paths only: its binding is unknown.
        assert_eq!(
            binding(
                json!({ "occurrence": { "inside": { "kind": "arrow_function", "stopBy": "end" } }, "form": "for" })
            ),
            ["16:$item"]
        );
    }

    #[test]
    fn only_and_destructured_values_read_the_binding() {
        assert_eq!(
            binding(json!({ "only": true, "scope": "function" })),
            [
                "8:$a",
                "8:$b",
                "9:$c",
                "14:$once",
                "15:$items",
                "16:$item",
                "19:$once"
            ]
        );
        assert_eq!(
            binding(json!({ "value": { "kind": "integer" } })),
            ["8:$a", "8:$b", "8:$limit", "14:$once", "19:$once"]
        );
        assert_eq!(
            binding(json!({ "destructuredValue": false, "value": { "kind": "integer" } })),
            ["8:$limit", "14:$once", "19:$once"]
        );
        assert_eq!(
            run(json!({ "kind": "variable_name", "value": { "follow": true, "eq": 2 } })),
            ["8:$b"]
        );
        // PHP's reads stop at `global $x` whether asked or not.
        assert_eq!(
            binding(json!({ "stopAtGlobal": true, "form": "assignment", "outer": true })),
            ["8:$config"]
        );
    }

    /// What `resolves.this` would ask has PHP's answer in `unresolved`: `$this` outside an object
    /// context is undefined.
    #[test]
    fn this_outside_an_object_is_unresolved() {
        let source = "<?php\nfunction g() { return $this; }\nclass A {\n    static function s() { return $this; }\n    function m() { return $this; }\n}\n";
        let compiled =
            compile(json!({ "kind": "variable_name", "resolves": { "unresolved": true } }))
                .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "php", "g.php", source);
        let lines: Vec<_> = result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let start = violation["span"]["startByte"].as_u64().expect("start") as usize;
                source[..start].matches('\n').count() + 1
            })
            .collect();
        assert_eq!(lines, [2, 4]);
    }

    #[test]
    fn what_php_has_no_counterpart_for_is_refused_with_why() {
        let at = "/clauses/0/forbid/ast/rule";
        for (rule, message) in [
            (
                json!({ "kind": "variable_name", "binds": { "exported": false } }),
                format!(
                    "{at}/binds/exported: `exported` reads Python's re-exports and TypeScript's exports; a PHP file exports no variables (an including file shares its global scope, which no declaration exports); php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "builtin": true } }),
                format!(
                    "{at}/binds/builtin: `builtin` reads ESLint's configured globals; PHP's superglobals are no file's bindings (an assignment to `$_GET` writes the superglobal); php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "directives": "eslint" } }),
                format!(
                    "{at}/binds/directives: `directives` reads ESLint's /* exported */ comments; PHP exports no variables; php has none"
                ),
            ),
            (
                json!({ "kind": "variable_name", "binds": { "redefines": { "sameImport": true } } }),
                format!(
                    "{at}/binds/redefines/sameImport: qualified import names are Python's; a PHP variable is never bound by an import"
                ),
            ),
            (
                json!({ "kind": "variable_name", "resolves": { "write": "last" } }),
                format!(
                    "{at}/resolves/write: ESLint reports an unused variable once, at its last write; php's usage oracle, Psalm, reports each unused assignment, which `binds: {{ used: false }}` matches"
                ),
            ),
            (
                json!({ "kind": "variable_name", "resolves": { "binding": { "stopAtGlobal": false } } }),
                format!(
                    "{at}/resolves/binding/stopAtGlobal: a php read after `global $x` reads that statement's binding; which top-level binding the global variable then holds depends on when the function runs, so php has no lookup through it"
                ),
            ),
        ] {
            assert_eq!(rejection(compile(rule)), message);
        }
    }
