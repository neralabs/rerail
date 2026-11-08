//! `.rr` tapes (decision 0061): the tree the parser builds, its errors, and its tape.

use super::tape::{FIELD_NAMES, KIND_NAMES, Node};
use super::*;
use crate::code_rule_dsl::language::GrammarDialect;

/// The tree as decision 0061 draws it: one line per node, indented by depth, `field: kind`, a leaf's text after it,
/// anonymous tokens quoted.
fn render(src: &str, nodes: &[Node]) -> String {
    let mut depth = vec![0usize; nodes.len()];
    let mut out = String::new();
    for (i, node) in nodes.iter().enumerate() {
        depth[i] = node.parent.map_or(0, |p| depth[p as usize] + 1);
        let leaf = nodes.get(i + 1).is_none_or(|next| next.parent != Some(i as u32));
        let text = &src[node.start as usize..node.end as usize];
        let kind = if node.is_error() { "ERROR" } else { KIND_NAMES[usize::from(node.kind)] };
        let field = if node.field == 0 { String::new() } else { format!("{}: ", FIELD_NAMES[usize::from(node.field)]) };
        let line = if node.kind >= tape::FIRST_ANONYMOUS && !node.is_error() {
            format!("{field}{kind:?}")
        } else if leaf {
            format!("{field}{kind} {text}")
        } else {
            format!("{field}{kind}")
        };
        out.push_str(&format!("{}{line}\n", "  ".repeat(depth[i])));
    }
    out
}

#[test]
fn the_decisions_example_has_the_decisions_tree() {
    let src = "// Calls of eval.\n.decl bad(n: node)\nbad(c) :- kind(c, \"call\"), !safe(c).\n";
    let expected = r#"source_file
  comment // Calls of eval.
  declaration
    ".decl"
    name: identifier bad
    columns: column_list
      "("
      column: column
        name: identifier n
        ":"
        type: type
          name: identifier node
      ")"
  clause
    head: head
      relation: identifier bad
      arguments: argument_list
        "("
        argument: variable c
        ")"
    ":-"
    body: conjunction
      literal: atom
        relation: identifier kind
        arguments: argument_list
          "("
          argument: variable c
          ","
          argument: string "call"
          ")"
      ","
      literal: negation
        "!"
        operand: atom
          relation: identifier safe
          arguments: argument_list
            "("
            argument: variable c
            ")"
    "."
"#;
    assert_eq!(render(src, &tape(src)), expected);
}

#[test]
fn terms_bodies_and_directives_read_as_the_decision_names_them() {
    let src = r#"#include "core.rr"
.input a, b(IO=json, filename="x")
.type T <: symbol
.violation V(at: node) code {c} in "E[0-9]+" "message"
  fix safe rewrite "Use x"
  suppress noqa "E501", "E502" over {n}
.mode block
r(x, y) :- (a(x) ; b(x)), y = -1, z = -x, n ~ `eval($X)`, w = count : { c(_) }, s = join v by i sep "," : { d(v, i) }, q = @f(x) + 2 * 3.
"#;
    let tree = render(src, &tape(src));
    for line in [
        "include\n    \"#include\"\n    path: string \"core.rr\"",
        "relation: io_relation\n      name: identifier b\n      \"(\"\n      parameters: raw_text IO=json, filename=\"x\"\n      \")\"",
        "type_declaration\n    \".type\"\n    name: identifier T\n    definition: raw_text <: symbol",
        "code: code_column\n      \"code\"\n      \"{\"\n      column: identifier c\n      \"}\"\n      \"in\"\n      codes: string \"E[0-9]+\"",
        "fix: fix_line\n      \"fix\"\n      \"safe\"\n      relation: identifier rewrite\n      description: string \"Use x\"",
        "suppress: suppress_line\n      \"suppress\"\n      tool: identifier noqa\n      rule: string \"E501\"\n      \",\"\n      rule: string \"E502\"\n      \"over\"\n      \"{\"\n      over: identifier n",
        "setting\n    \".\"\n    name: identifier mode\n    argument: identifier block",
        "literal: group\n        \"(\"\n        body: disjunction\n          alternative: conjunction",
        "right: number -1",
        "right: unary_expression\n          operator: \"-\"\n          operand: variable x",
        "literal: pattern_match\n        subject: variable n\n        \"~\"\n        pattern: code_literal `eval($X)`",
        "right: aggregate\n          operator: \"count\"\n          \":\"\n          \"{\"\n          body: conjunction",
        "right: aggregate\n          operator: \"join\"\n          value: variable v\n          \"by\"\n          key: variable i\n          \"sep\"\n          separator: string \",\"",
        "right: binary_expression\n          left: functor_call\n            \"@\"\n            function: identifier f",
        "right: binary_expression\n            left: number 2\n            operator: \"*\"\n            right: number 3",
    ] {
        assert!(tree.contains(line), "{line}\nnot in\n{tree}");
    }
}

/// A bad statement is one ERROR leaf, and the statements after it are read.
#[test]
fn a_bad_statement_is_one_error_and_parsing_resumes() {
    let src = ".decl a(x: number)\na(1) :- b(.\n.decl c(x: number)\nc(2).\nd(,) .\ne(3).\n";
    let nodes = tape(src);
    let errors: Vec<&str> = nodes.iter().filter(|n| n.is_error()).map(|n| &src[n.start as usize..n.end as usize]).collect();
    assert_eq!(errors, ["a(1) :- b(.", "d(,) ."]);
    let statements: Vec<&str> = nodes.iter().filter(|n| n.parent == Some(0)).map(|n| KIND_NAMES.get(usize::from(n.kind)).copied().unwrap_or("ERROR")).collect();
    assert_eq!(statements, ["declaration", "ERROR", "declaration", "clause", "ERROR", "clause"]);
    // The file's error is the one running it gives.
    let (line, message) = syntax_error(src).expect("an error");
    assert_eq!(line, 2);
    assert!(message.starts_with("line 2: expected a term"), "{message}");
    // An unterminated string runs to the end of the file.
    let src = "a(1).\nb(\"x).\nc(2).\n";
    let nodes = tape(src);
    let last = nodes.iter().rev().find(|n| n.parent == Some(0)).unwrap();
    assert!(last.is_error() && last.end as usize == src.len(), "{}", render(src, &nodes));
    assert_eq!(syntax_error(src).unwrap().1, "unterminated string at line 2");
}

/// Every `.rr` file in the repository tapes. Its tape has an ERROR exactly when running it fails to parse, every
/// node's span holds its children's, and the leaves and comments cover every byte but whitespace, once.
#[test]
fn every_rule_file_tapes_and_its_leaves_cover_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax");
    let mut files = Vec::new();
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rr") {
                files.push(path);
            }
        }
    }
    assert!(files.len() > 100, "{} files", files.len());
    let mut refused = 0;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap();
        let nodes = tape(&src);
        let parses = syntax::Parser::new(&src).and_then(|mut p| p.statements()).is_ok();
        let errors = nodes.iter().any(Node::is_error);
        assert_eq!(errors, !parses, "{}", path.display());
        refused += usize::from(!parses);
        let mut covered = vec![0u8; src.len()];
        for (i, node) in nodes.iter().enumerate() {
            if let Some(p) = node.parent {
                let parent = &nodes[p as usize];
                assert!(parent.start <= node.start && node.end <= parent.end, "{}: node {i} outside its parent", path.display());
            }
            let leaf = nodes.get(i + 1).is_none_or(|next| next.parent != Some(i as u32));
            if leaf && i != 0 {
                for b in &mut covered[node.start as usize..node.end as usize] {
                    *b += 1;
                }
            }
        }
        for (at, (&count, byte)) in covered.iter().zip(src.bytes()).enumerate() {
            let want = u8::from(!byte.is_ascii_whitespace());
            assert!(count == want || (count == 1 && byte.is_ascii_whitespace()), "{}: byte {at} ({:?}) covered {count} times", path.display(), byte as char);
        }
    }
    assert!(refused < files.len() / 10, "{refused} of {} files do not parse", files.len());
}

/// A `.rr` file's tape encodes through the tape format, its kinds and fields read back by the `rr` catalog.
#[test]
fn an_rr_tape_encodes_and_reads_its_kinds_back() {
    let kinds = GrammarDialect::Rr.kinds();
    assert_eq!(kinds.kind_id("clause"), tape::Kind::Clause as u16);
    assert_eq!(kinds.kind_name(tape::Kind::Turnstile as u16), Some(":-"));
    assert!(kinds.kind_is_named(tape::Kind::Comment as u16) && !kinds.kind_is_named(tape::FIRST_ANONYMOUS));
    assert_eq!(kinds.field_id("relation"), Some(tape::Field::Relation as u16));
    let grammar = crate::code_rule_dsl::tape::Grammar::new(GrammarDialect::Rr).expect("grammar");
    let src = "// c\n.decl a(x: number)\na(1).\nb(.\n";
    let image = crate::code_rule_dsl::tape::TapeImage::parse(&grammar, src, &|| false).expect("parsed").expect("encoded");
    let bytes = image.primary.to_bytes().expect("bytes");
    assert!(!bytes.is_empty());
}

/// A corpus run with `.language rr` reads the repository's `.rr` files as documents: here the pack rule that flags
/// an input read from outside the repository (`packs/rr/rr-input-outside-repository.rr`).
#[test]
fn a_rule_about_rules_runs_over_rule_files() {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-on-rr");
    std::fs::create_dir_all(root.join("rules")).unwrap();
    std::fs::write(root.join("rules/a.rr"), ".decl f(x: symbol)\n.input f(IO=file, filename=\"/etc/passwd\")\n").unwrap();
    std::fs::write(root.join("rules/b.rr"), ".decl g(x: symbol)\n.input g(IO=file, filename=\"g.csv\")\n").unwrap();
    std::fs::write(root.join("rules/c.rr"), ".decl h(x: symbol)\n.input h(IO=file, filename=\"../up.csv\")\n").unwrap();
    std::fs::write(root.join("x.py"), "print(1)\n").unwrap();
    let rule = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/rr/rr-input-outside-repository.rr");
    let report = crate::code_rule_dsl::tools::run_rr(&rule, &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let mut found: Vec<(String, i64)> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["file"].as_str().or(d["path"].as_str()).unwrap_or("").to_owned(), d["line"].as_i64().or(d["start"]["line"].as_i64()).unwrap_or(-1)))
        .collect();
    found.sort();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(found.len(), 2, "{report:#}");
    assert!(found[0].0.ends_with("a.rr") && found[1].0.ends_with("c.rr"), "{found:?}");
}

/// The text facts of a `.rr` tape: `name` of identifiers and variables, `str` and `value` of strings, numbers and
/// backtick literals, `op` of anonymous tokens, `comment`, and `syntax_error` with the message running it gives.
#[test]
fn rule_files_have_text_facts() {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-facts");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rr"), "// note\n.decl r(x: number)\nr(n) :- s(\"a\\nb\", -2, 0.5), n ~ `f($X)`, n <= 3.\n").unwrap();
    std::fs::write(root.join("bad.rr"), ".decl q(x: number)\nq(1) :- .\n").unwrap();
    let rule = r#".language rr
.decl names(s: symbol)
names(s) :- node(_, n, k, _, _), name(n, s), k != "string".
.decl strs(k: symbol, v: symbol)
strs(k, v) :- node(_, n, k, _, _), str(n, v).
.decl values(k: symbol, v: symbol)
values(k, v) :- node(_, n, k, _, _), value(n, v).
.decl ops(k: symbol, o: symbol)
ops(k, o) :- node(_, n, k, _, _), op(n, o), k = "comparison".
.decl comments(s: symbol)
comments(s) :- node(_, n, "comment", _, _), comment(n, s).
.decl errors(l: number, m: symbol)
errors(l, m) :- syntax_error(_, l, m).
.output names, strs, values, ops, comments, errors
"#;
    let path = root.with_extension("rule.rr");
    std::fs::write(&path, rule).unwrap();
    let program = print(&parse_file(&path).unwrap()).unwrap();
    let out = root.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = |rel: &str| {
        let mut rows: Vec<String> = std::fs::read_to_string(out.join(format!("{rel}.csv"))).unwrap_or_default().lines().map(str::to_owned).collect();
        rows.sort();
        rows
    };
    let (names, strs, values, ops, comments, errors) = (rows("names"), rows("strs"), rows("values"), rows("ops"), rows("comments"), rows("errors"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&path);
    // `bad.rr` is withheld: a file that does not parse gives its `syntax_error` row only.
    for name in ["r", "x", "number", "n", "s"] {
        assert!(names.iter().any(|row| row == name), "name {name} in {names:?}");
    }
    assert!(strs.iter().any(|row| row == "string\ta\\nb" || row == "string\t\"a\\nb\"" || row.starts_with("string\ta")), "{strs:?}");
    assert!(strs.iter().any(|row| row == "code_literal\tf($X)"), "{strs:?}");
    assert!(values.iter().any(|row| row == "number\t-2") && values.iter().any(|row| row == "float\t0.5"), "{values:?}");
    assert_eq!(ops, ["comparison\t<="]);
    assert_eq!(comments, ["note"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("2\t"), "{errors:?}");
}

/// O105: a Python policy and an `rr` policy over one repository (`tools::run_policies`). Each reads its own
/// language's documents through the one store, and the findings of both come back together.
#[test]
fn python_and_rr_policies_run_over_one_repository() {
    let root = crate::code_rule_dsl::prefilter::tests::temp().with_extension("rr-and-py");
    std::fs::create_dir_all(root.join("rules")).unwrap();
    std::fs::write(root.join("rules/a.rr"), ".decl f(x: symbol)\n.input f(IO=file, filename=\"/etc/passwd\")\n").unwrap();
    std::fs::write(root.join("x.py"), "eval(\"1\")\nprint(2)\n").unwrap();
    let python = root.with_extension("py-policy.rr");
    std::fs::write(
        &python,
        ".language python\n.violation eval_call(at: node) code \"PY1\" \"eval\"\neval_call(c) :- node(_, c, \"call\", _, _), field(c, \"function\", f), text(f, \"eval\").\n",
    )
    .unwrap();
    let rr = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/rr/rr-input-outside-repository.rr");
    let report = crate::code_rule_dsl::tools::run_policies(&[python.clone(), rr], &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&python);
    let files: Vec<String> = report["diagnostics"].as_array().unwrap().iter().map(|d| d["file"].as_str().or(d["path"].as_str()).unwrap_or("").to_owned()).collect();
    assert_eq!(files.len(), 2, "{report:#}");
    assert!(files[0].ends_with("x.py") && files[1].ends_with("a.rr"), "{files:?}");
}

/// Decision 0062: the statements lowering reads come off the tape. For every `.rr` file in the repository they are
/// the parser's own, statement for statement, and a file that does not parse gives the parser's error.
#[test]
fn statements_from_the_tape_are_the_parsers() {
    // The rule library and packs, and the engine's own programs (typing's, and the pinned golden ones).
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pending = vec![manifest.join("../docs/rule-syntax"), manifest.join("src/code_rule_dsl/typing/programs"), manifest.join("src/code_rule_dsl/logic/plan/golden/programs")];
    let mut checked = 0;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !path.extension().is_some_and(|e| e == "rr") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            let parsed = syntax::Parser::new(&src).and_then(|mut p| p.statements());
            let taped = from_tape::statements(&src, &tape(&src));
            match (parsed, taped) {
                (Ok(parsed), Ok(taped)) => {
                    assert_eq!(parsed.len(), taped.len(), "{}", path.display());
                    for (p, t) in parsed.iter().zip(&taped) {
                        assert_eq!(p, t, "{}", path.display());
                    }
                }
                (Err(p), Err(t)) => assert_eq!(format!("{p:?}"), format!("{t:?}"), "{}", path.display()),
                (p, t) => panic!("{}: parser {:?}, tape {:?}", path.display(), p.map(|s| s.len()), t.map(|s| s.len())),
            }
            checked += 1;
        }
    }
    assert!(checked > 100);
}
