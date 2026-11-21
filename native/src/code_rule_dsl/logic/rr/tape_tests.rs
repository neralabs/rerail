//! `.rerail` tapes (decision 0061): the tree the parser builds, its errors, and its tape.

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

/// Decision 0061's example, on a named-only tape (RER-131): the tokens the grammar implies (`decl`, `(`, `,`, `:`,
/// `:-`, `!`, `)`, the clause's `.`) are left out (`docs/parser-generators/32-named-only-rr-tapes.md`). The
/// declaration's `.` is stored: its modifiers may hold a `.`.
#[test]
fn the_decisions_example_has_the_decisions_tree() {
    let src = "// Calls of eval.\n.decl bad(n: node)\nbad(c) :- kind(c, \"call\"), !safe(c).\n";
    let expected = r#"source_file
  comment // Calls of eval.
  declaration
    "."
    name: identifier bad
    columns: column_list
      column: column
        name: identifier n
        type: type
          name: identifier node
  clause
    head: head
      relation: identifier bad
      arguments: argument_list
        argument: variable c
    body: conjunction
      literal: atom
        relation: identifier kind
        arguments: argument_list
          argument: variable c
          argument: string "call"
      literal: negation
        operand: atom
          relation: identifier safe
          arguments: argument_list
            argument: variable c
"#;
    assert_eq!(render(src, &rerail_rr_syntax::engine_tape(src)), expected);
}

#[test]
fn terms_bodies_and_directives_read_as_the_decision_names_them() {
    let src = r#"#include "core.rerail"
.input a, b(IO=json, filename="x")
.type T <: symbol
.violation V(at: node) code {c} in "E[0-9]+" "message"
  fix safe rewrite "Use x"
  suppress noqa "E501", "E502" over {n}
.mode block
r(x, y) :- (a(x) ; b(x)), y = -1, z = -x, n ~ `eval($X)`, w = count : { c(_) }, s = join v by i sep "," : { d(v, i) }, q = @f(x) + 2 * 3.
"#;
    let tree = render(src, &rerail_rr_syntax::engine_tape(src));
    for line in [
        // Implied tokens are left out; `.input`, `safe`, `count`, `join` and the operators are stored.
        "include\n    path: string \"core.rerail\"",
        "io_directive\n    \".input\"\n    relation: io_relation\n      name: identifier a",
        "relation: io_relation\n      name: identifier b\n      parameters: raw_text IO=json, filename=\"x\"",
        "type_declaration\n    name: identifier T\n    definition: raw_text <: symbol",
        "code: code_column\n      column: identifier c\n      codes: string \"E[0-9]+\"",
        "fix: fix_line\n      \"fix\"\n      \"safe\"\n      relation: identifier rewrite\n      description: string \"Use x\"",
        "suppress: suppress_line\n      tool: identifier noqa\n      rule: string \"E501\"\n      rule: string \"E502\"\n      over: identifier n",
        "setting\n    \".\"\n    name: identifier mode\n    argument: identifier block",
        "literal: group\n        body: disjunction\n          alternative: conjunction",
        "right: number -1",
        "right: unary_expression\n          operand: variable x",
        "literal: pattern_match\n        subject: variable n\n        pattern: code_literal `eval($X)`",
        // An aggregate's `:` is stored (`by (k) : {` holds it beside the stored `)`).
        "right: aggregate\n          \":\"\n          body: conjunction",
        "right: aggregate\n          operator: \"join\"\n          value: variable v\n          \"by\"\n          key: variable i\n          \"sep\"\n          separator: string \",\"\n          \":\"",
        "right: binary_expression\n          left: functor_call\n            function: identifier f",
        "right: binary_expression\n            left: number 2\n            operator: \"*\"\n            right: number 3",
    ] {
        assert!(tree.contains(line), "{line}\nnot in\n{tree}");
    }
}

/// A bad statement is one ERROR leaf, and the statements after it are read.
#[test]
fn a_bad_statement_is_one_error_and_parsing_resumes() {
    let src = ".decl a(x: number)\na(1) :- b(.\n.decl c(x: number)\nc(2).\nd(,) .\ne(3).\n";
    let nodes = rerail_rr_syntax::engine_tape(src);
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
    let nodes = rerail_rr_syntax::engine_tape(src);
    let last = nodes.iter().rev().find(|n| n.parent == Some(0)).unwrap();
    assert!(last.is_error() && last.end as usize == src.len(), "{}", render(src, &nodes));
    assert_eq!(syntax_error(src).unwrap().1, "unterminated string at line 2");
}

/// Every `.rerail` file in the repository tapes. Its tape has an ERROR exactly when running it fails to parse, every
/// node's span holds its children's, and the leaves and comments cover every byte but whitespace and the tokens the
/// grammar implies, once (a named-only tape, RER-131).
#[test]
fn every_rule_file_tapes_and_its_leaves_cover_it() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax");
    let mut files = Vec::new();
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rerail") {
                files.push(path);
            }
        }
    }
    assert!(files.len() > 100, "{} files", files.len());
    let mut refused = 0;
    for path in &files {
        let src = std::fs::read_to_string(path).unwrap();
        let nodes = rerail_rr_syntax::engine_tape(&src);
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
        // A run of uncovered bytes is an implied token (`.decl` as `.` and `decl`).
        const IMPLIED: &[&str] = &[
            "(", ")", ",", ":", ":-", "=>", ".", ";", "!", "@", "~", "-", "{", "}", "<", ">", "->", "#include", "decl", "edit",
            "functor", "type", "lattice", "pragma", "violation", "policy", "code", "tag", "in", "fix", "suppress", "over",
            "options", "count", "test", "fixtures", "uses", "end", "at",
        ];
        let mut at = 0;
        while at < src.len() {
            let byte = src.as_bytes()[at];
            assert!(covered[at] <= 1, "{}: byte {at} ({:?}) covered {} times", path.display(), byte as char, covered[at]);
            if covered[at] == 1 || byte.is_ascii_whitespace() {
                at += 1;
                continue;
            }
            let run = src[at..].bytes().take_while(|&b| !b.is_ascii_whitespace()).count();
            let run = (0..run).take_while(|&k| covered[at + k] == 0).count();
            let text = &src[at..at + run];
            let mut rest = text;
            while !rest.is_empty() {
                let Some(token) = IMPLIED.iter().filter(|t| rest.starts_with(**t)).max_by_key(|t| t.len()) else {
                    panic!("{}: byte {at}: {text:?} is no implied token", path.display());
                };
                rest = &rest[token.len()..];
            }
            at += run;
        }
    }
    assert!(refused < files.len() / 10, "{refused} of {} files do not parse", files.len());
}

/// A `.rerail` file's tape encodes through the tape format, its kinds and fields read back by the `rr` catalog.
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

/// A corpus run with `.language rr` reads the repository's `.rerail` files as documents: here the pack rule that flags
/// an input read from outside the repository (`packs/rr/rr-input-outside-repository.rerail`).
#[test]
fn a_rule_about_rules_runs_over_rule_files() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-on-rr");
    std::fs::create_dir_all(root.join("rules")).unwrap();
    std::fs::write(root.join("rules/a.rerail"), ".decl f(x: symbol)\n.input f(IO=file, filename=\"/etc/passwd\")\n").unwrap();
    std::fs::write(root.join("rules/b.rerail"), ".decl g(x: symbol)\n.input g(IO=file, filename=\"g.csv\")\n").unwrap();
    std::fs::write(root.join("rules/c.rerail"), ".decl h(x: symbol)\n.input h(IO=file, filename=\"../up.csv\")\n").unwrap();
    std::fs::write(root.join("x.py"), "print(1)\n").unwrap();
    let rule = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/rr/rr-input-outside-repository.rerail");
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
    assert!(found[0].0.ends_with("a.rerail") && found[1].0.ends_with("c.rerail"), "{found:?}");
}

/// The text facts of a `.rerail` tape: `name` of identifiers and variables, `str` and `value` of strings, numbers and
/// backtick literals, `op` of anonymous tokens, `comment`, and `syntax_error` with the message running it gives.
#[test]
fn rule_files_have_text_facts() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-facts");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rerail"), "// note\n.decl r(x: number)\nr(n) :- s(\"a\\nb\", -2, 0.5), n ~ `f($X)`, n <= 3.\n").unwrap();
    std::fs::write(root.join("bad.rerail"), ".decl q(x: number)\nq(1) :- .\n").unwrap();
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
    let path = root.with_extension("rule.rerail");
    std::fs::write(&path, rule).unwrap();
    let program = print_version(&parse_file_version(&path).unwrap()).unwrap();
    let out = root.join("out");
    crate::code_rule_dsl::logic::plan::corpus::run(&program, &root, None, &out, "program", crate::code_rule_dsl::typing::file_local::Binding::Cpu, 1 << 18).unwrap();
    let rows = |rel: &str| crate::code_rule_dsl::logic::plan::output::shown(&out, rel).unwrap_or_default();
    let (names, strs, values, ops, comments, errors) = (rows("names"), rows("strs"), rows("values"), rows("ops"), rows("comments"), rows("errors"));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&path);
    // `bad.rerail` is withheld: a file that does not parse gives its `syntax_error` row only.
    for name in ["r", "x", "number", "n", "s"] {
        assert!(names.iter().any(|row| row == &[name]), "name {name} in {names:?}");
    }
    assert!(strs.iter().any(|row| row[0] == "string" && (row[1] == "a\\nb" || row[1] == "\"a\\nb\"" || row[1].starts_with('a'))), "{strs:?}");
    assert!(strs.iter().any(|row| row == &["code_literal", "f($X)"]), "{strs:?}");
    assert!(values.iter().any(|row| row == &["number", "-2"]) && values.iter().any(|row| row == &["float", "0.5"]), "{values:?}");
    assert_eq!(ops, [["comparison", "<="]]);
    assert_eq!(comments, [["note"]]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0][0], "2", "{errors:?}");
}

/// O105: a Python policy and an `rr` policy over one repository (`tools::run_policies`). Each reads its own
/// language's documents through the one store, and the findings of both come back together.
#[test]
fn python_and_rr_policies_run_over_one_repository() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("rr-and-py");
    std::fs::create_dir_all(root.join("rules")).unwrap();
    std::fs::write(root.join("rules/a.rerail"), ".decl f(x: symbol)\n.input f(IO=file, filename=\"/etc/passwd\")\n").unwrap();
    std::fs::write(root.join("x.py"), "eval(\"1\")\nprint(2)\n").unwrap();
    let python = root.with_extension("py-policy.rerail");
    std::fs::write(
        &python,
        ".language python\n.violation eval_call(at: node) code \"PY1\" \"eval\"\neval_call(c) :- node(_, c, \"call\", _, _), field(c, \"function\", f), text(f, \"eval\").\n",
    )
    .unwrap();
    let rr = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/rr/rr-input-outside-repository.rerail");
    let report = crate::code_rule_dsl::tools::run_policies(&[python.clone(), rr], &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_file(&python);
    let files: Vec<String> = report["diagnostics"].as_array().unwrap().iter().map(|d| d["file"].as_str().or(d["path"].as_str()).unwrap_or("").to_owned()).collect();
    assert_eq!(files.len(), 2, "{report:#}");
    assert!(files[0].ends_with("x.py") && files[1].ends_with("a.rerail"), "{files:?}");
}

/// O-batch-setup (decision 0168): a batch's policies of one language run as one pack (decisions 0051, 0089): their
/// libraries read, lowered, hidden and evaluated once, not once a policy. Each policy's findings are its own, in
/// its own `out/<k>`, and the same as its run alone; a policy of another language runs in its own group.
#[test]
fn a_batchs_policies_of_one_language_run_as_one_pack() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("batch-pack");
    std::fs::create_dir_all(root.join("rules")).unwrap();
    std::fs::write(root.join("rules/a.rerail"), ".decl f(x: symbol)\n.input f(IO=file, filename=\"/etc/passwd\")\n").unwrap();
    std::fs::write(root.join("x.py"), "eval(\"1\")\nexec(\"2\")\nprint(3)\n").unwrap();
    let policy = |name: &str, callee: &str| {
        let path = root.with_extension(format!("{name}.rerail"));
        std::fs::write(
            &path,
            format!(
                ".policy \"t:{name}\" revision 1\n.language python\n#include \"python/calls.rerail\"\n\
                 .violation {name}_call(at: node) code \"{name}\" \"a call of {callee}\"\n\
                 {name}_call(c) :- node(_, c, \"call\", _, _), field(c, \"function\", f), text(f, \"{callee}\").\n"
            ),
        )
        .unwrap();
        path
    };
    let (a, b) = (policy("ev", "eval"), policy("ex", "exec"));
    // A policy with neither `.policy` nor `.language` reads the tree's grammar (Python here), so it joins the pack.
    let c = root.with_extension("pr.rerail");
    std::fs::write(
        &c,
        "#include \"python/calls.rerail\"\n.violation pr_call(at: node) code \"pr\" \"a call of print\"\n\
         pr_call(c) :- node(_, c, \"call\", _, _), field(c, \"function\", f), text(f, \"print\").\n",
    )
    .unwrap();
    let rr = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/rr/rr-input-outside-repository.rerail");
    let paths = [a.clone(), rr.clone(), b.clone(), c.clone()];
    let batch = crate::code_rule_dsl::tools::run_policies(&paths, &root, &root.join("out"), &serde_json::Value::Null).unwrap();
    let codes = |report: &serde_json::Value| -> Vec<String> {
        report["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap_or("").to_owned()).collect()
    };
    let reports: Vec<serde_json::Value> = paths
        .iter()
        .enumerate()
        .map(|(k, p)| crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(p), &root, &root.join(format!("alone{k}")), &serde_json::Value::Null).unwrap())
        .collect();
    let alone: Vec<Vec<String>> = reports.iter().map(codes).collect();
    // Each finding written in the member's own directory, as the CLI reports it (`tools::diagnostics`).
    let written = |k: usize| -> Vec<String> { crate::code_rule_dsl::tools::diagnostics(&root.join("out").join(k.to_string())).unwrap_or_default().iter().map(|d| d.to_string()).collect() };
    let (w0, w1, w2) = (written(0), written(1), written(2));
    let _ = std::fs::remove_dir_all(&root);
    for p in [&a, &b, &c] {
        let _ = std::fs::remove_file(p);
    }
    // The findings, in the order of the policies, each the policy's own run's.
    assert_eq!(codes(&batch), alone.concat(), "{batch:#}");
    assert_eq!(alone[0], ["ev"]);
    assert_eq!(alone[2], ["ex"]);
    assert_eq!(alone[3], ["pr"]);
    // The three policies of the Python tree were one run; the rr policy its own.
    let together: Vec<u64> = batch["runs"].as_array().unwrap().iter().map(|r| r["together"].as_u64().unwrap_or(0)).collect();
    // Under `RERAIL_BATCH_PACK=off` each runs alone, and this fails.
    assert_eq!(together, [3, 1, 3, 3], "{batch:#}");
    // Each policy's metadata, as its run alone gives it, less the `.global library` settings the reader writes for
    // each file a policy includes (decision 0091): the engine's, not the policy's.
    let own = |m: &serde_json::Value| -> serde_json::Value {
        let mut m = m.clone();
        if let Some(settings) = m["settings"].as_array_mut() {
            settings.retain(|s| s[0] != "global" || !s[1].as_str().is_some_and(|t| t.starts_with("library ")));
        }
        m
    };
    for (k, report) in reports.iter().enumerate() {
        assert_eq!(own(&batch["runs"][k]["metadata"]), own(&report["runs"][0]["metadata"]), "policy {k}");
    }
    // Each policy's findings in its own directory.
    let any = |w: &[String], s: &str| w.iter().any(|j| j.contains(s));
    assert!(any(&w0, "\"ev\"") && !any(&w0, "\"ex\""), "{w0:?}");
    assert!(any(&w2, "\"ex\"") && !any(&w2, "\"ev\""), "{w2:?}");
    assert!(any(&w1, "rules/a.rerail"), "{w1:?}");
}

/// A policy's findings are its run alone's whatever else shares its pack (RUF060 beside C417). `anns` reads
/// `list_like` from a string annotation, so hiding copies it for the annotation, and folding the copy widened
/// `list_like` to take the annotations in (`hiding::fold_own`). `cmps` reads it through `empty_literal` from the file:
/// beside `anns` it found the annotation's `1 in []` as well, which it does not alone. It reads it as hiding left it.
#[test]
fn a_policy_beside_one_that_reads_annotations_finds_what_it_finds_alone() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("corun-annotations");
    let (src, rules) = (root.join("src"), root.join("rules"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(src.join("x.py"), "a = 1 in []\nx: \"1 in []\" = 1\n").unwrap();
    // Both include the same library, which asks for the annotations' trees: they ask the same guests, one pack.
    let policy = |name: &str, body: &str| {
        let path = rules.join(format!("{name}.rerail"));
        std::fs::write(&path, format!(".policy \"t:{name}\" revision 1\n.language python\n#include \"python/ruff.rerail\"\n{body}\n")).unwrap();
        path
    };
    let cmps = policy("cmps", ".violation Cmp(at: node) code \"cmp\" \"in an empty list\"\nCmp(c) :- cmp(c, \"in\", _, r), empty_literal(r, _).");
    let anns = policy("anns", ".violation Ann(at: node) code \"ann\" \"a list in an annotation\"\nAnn(e) :- annotation(e), list_like(e, _).");
    let found = |report: &serde_json::Value, policy: &str| -> Vec<(u64, String)> {
        let rows = report["diagnostics"].as_array().unwrap().iter().filter(|d| d["policy"] == policy);
        let mut found: Vec<(u64, String)> = rows.map(|d| (d["line"].as_u64().unwrap_or(0), d["code"].as_str().unwrap_or("").to_owned())).collect();
        found.sort();
        found
    };
    let run = |paths: &[PathBuf], out: &str| crate::code_rule_dsl::tools::run_policies(paths, &src, &root.join(out), &serde_json::Value::Null).unwrap();
    let (alone, beside) = (run(std::slice::from_ref(&cmps), "alone"), run(&[cmps, anns], "beside"));
    let _ = std::fs::remove_dir_all(&root);
    let together: Vec<u64> = beside["runs"].as_array().unwrap().iter().map(|r| r["together"].as_u64().unwrap_or(0)).collect();
    assert_eq!(together, [2, 2], "one pack: {beside:#}");
    assert_eq!(found(&beside, "t:anns"), [(2, "ann".to_owned())], "the annotation's list is read: {beside:#}");
    assert_eq!(found(&alone, "t:cmps"), [(1, "cmp".to_owned())], "{alone:#}");
    assert_eq!(found(&beside, "t:cmps"), found(&alone, "t:cmps"), "{beside:#}");
}

/// A batch's policies of a language of several dialects (`.language typescript`) run as one pack a dialect, as a
/// policy alone runs once a dialect (`tools::run_program`): each policy's findings are its run alone's, from every
/// dialect, and a policy whose scope gives other dialects runs in a pack of its own. Before, such policies ran alone.
#[test]
fn a_batchs_policies_of_several_dialects_run_as_one_pack_a_dialect() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrdialectpack");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ts"), "let a = 1;\nf(a);\n").unwrap();
    std::fs::write(dir.join("b.js"), "\nlet b = 2;\ng(b);\n").unwrap();
    std::fs::write(dir.join("c.tsx"), "\n\nlet c = <p/>;\nh(c);\n").unwrap();
    let policy = |name: &str, scope: &str, body: &str| {
        let path = dir.join(format!("{name}.rerail"));
        std::fs::write(&path, format!(".policy \"t:{name}\" revision 1\n.language typescript\n.scope include \"{scope}\"\n{body}\n")).unwrap();
        path
    };
    let lets = policy("lets", "**/*", ".violation Let(at: node, name: symbol) code \"let\" \"{name}\"\nLet(d, x) :- kind(d, \"variable_declarator\"), field(d, \"name\", n), text(n, x).");
    let calls = policy("calls", "**/*", ".violation Call(at: node, name: symbol) code \"call\" \"{name}\"\nCall(c, x) :- kind(c, \"call_expression\"), field(c, \"function\", f), text(f, x).");
    let js = policy("js", "**/*.js", ".violation Js(at: node, name: symbol) code \"js\" \"{name}\"\nJs(d, x) :- kind(d, \"variable_declarator\"), field(d, \"name\", n), text(n, x).");
    let paths = [lets.clone(), js.clone(), calls.clone()];
    let batch = crate::code_rule_dsl::tools::run_policies(&paths, &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let found = |report: &serde_json::Value| -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["code"].as_str().unwrap_or("").to_owned(), d["message"].as_str().unwrap_or("").to_owned()))
            .collect();
        found.sort();
        found
    };
    let alone: Vec<serde_json::Value> = paths
        .iter()
        .enumerate()
        .map(|(k, p)| crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(p), &dir, &dir.join(format!("alone{k}")), &serde_json::Value::Null).unwrap())
        .collect();
    let together: Vec<u64> = batch["runs"].as_array().unwrap().iter().map(|r| r["together"].as_u64().unwrap_or(0)).collect();
    // `calls`' findings in its own directory, every dialect's, as the CLI reports them (`tools::diagnostics`).
    let written: Vec<String> = crate::code_rule_dsl::tools::diagnostics(&dir.join("out/2")).unwrap_or_default().iter().map(|d| d.to_string()).collect();
    let _ = std::fs::remove_dir_all(&dir);
    let mut each: Vec<(String, String)> = alone.iter().flat_map(found).collect();
    each.sort();
    assert_eq!(found(&batch), each, "{batch:#}");
    let e = |c: &str, m: &str| (c.to_owned(), m.to_owned());
    assert_eq!(found(&alone[0]), [e("let", "a"), e("let", "b"), e("let", "c")]);
    assert_eq!(found(&alone[1]), [e("js", "b")]);
    assert_eq!(found(&alone[2]), [e("call", "f"), e("call", "g"), e("call", "h")]);
    // `lets` and `calls` read the three dialects, one pack each; `js` reads JavaScript alone.
    assert_eq!(together, [2, 1, 2], "{batch:#}");
    assert_eq!(written.len(), 3, "{written:?}");
    assert!(written.iter().all(|w| w.contains("\"call\"")), "{written:?}");
}

/// A policy that reads `within(n, a)` finds in a pack, in either order, what it finds alone. A pack's programs are
/// lowered in their libraries' scope (`rr::Text::lower_in`), which took the program's declaration of `within` out as
/// the library's; the library part, whose rules read no `within`, declared none, and the run panicked (`no declaration
/// of within`) where each policy alone ran.
#[test]
fn a_policy_that_reads_within_finds_in_a_pack_in_either_order_what_it_finds_alone() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrwithinpack");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.js"), "const f = (y) => y + z;\ndebugger;\nw;\n").unwrap();
    let policy = |name: &str, body: &str| {
        let path = dir.join(format!("{name}.rerail"));
        std::fs::write(&path, format!(".policy \"t:{name}\" revision 1\n.language javascript\n{body}\n")).unwrap();
        path
    };
    let under = policy("under", ".violation Under(at: node, name: symbol) code \"under\" \"{name}\"\nUnder(n, x) :- kind(n, \"identifier\"), within(n, a), kind(a, \"arrow_function\"), text(n, x).");
    let stops = policy("stops", ".violation Stop(at: node) code \"stop\" \"stop\"\nStop(n) :- kind(n, \"debugger_statement\").");
    let found = |report: &serde_json::Value| -> Vec<(String, String, u64)> {
        let mut found: Vec<(String, String, u64)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (d["code"].as_str().unwrap_or("").to_owned(), d["message"].as_str().unwrap_or("").to_owned(), d["line"].as_u64().unwrap_or(0)))
            .collect();
        found.sort();
        found
    };
    let run = |paths: &[std::path::PathBuf], out: &str| crate::code_rule_dsl::tools::run_policies(paths, &dir, &dir.join(out), &serde_json::Value::Null).unwrap_or_else(|e| panic!("{out}: {e}"));
    let mut alone: Vec<_> = [&under, &stops].into_iter().enumerate().flat_map(|(k, p)| found(&run(std::slice::from_ref(p), &format!("alone{k}")))).collect();
    alone.sort();
    let first = run(&[under.clone(), stops.clone()], "first");
    let second = run(&[stops.clone(), under.clone()], "second");
    let _ = std::fs::remove_dir_all(&dir);
    let e = |c: &str, m: &str, l: u64| (c.to_owned(), m.to_owned(), l);
    assert_eq!(alone, [e("stop", "stop", 2), e("under", "y", 1), e("under", "y", 1), e("under", "z", 1)]);
    assert_eq!(found(&first), alone, "{first:#}");
    assert_eq!(found(&second), alone, "{second:#}");
    let together = |report: &serde_json::Value| -> Vec<u64> { report["runs"].as_array().unwrap().iter().map(|r| r["together"].as_u64().unwrap_or(0)).collect() };
    assert_eq!(together(&first), [2, 2], "one pack: {first:#}");
    assert_eq!(together(&second), [2, 2], "one pack: {second:#}");
}

/// A pack's programs make one set of guests (decision 0055), and a rule that reaches a guest reads every one the run
/// made: `ruff-classes.rerail`'s `ann_input` asks for the annotation in `Literal["dict({})"]`, and C408, which reads
/// string annotations, found a `dict({})` call there once a policy that only includes that file ran beside it. A
/// policy that asks for other guests runs in its own pack (`rr::guest_asks`): C408 finds what it finds alone, and a
/// policy that asks the same as C408 still shares its pack.
#[test]
fn a_policy_reads_in_a_pack_only_the_guests_it_asks_for_alone() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("rrguestpack");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("c.py"),
        "from typing import List, Literal, cast\n\na: \"dict({})\" = 1\nb: \"dict(\" \"{})\" = 1\nc: List[\"dict({1: 2})\"] = []\nd: Literal[\"dict({})\"] = 1\ne = cast(\"(dict)({ })\", None)\n",
    )
    .unwrap();
    let c408 = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/ruff-c408-unnecessary-collection-call.rerail");
    let text = std::fs::read_to_string(&c408).unwrap();
    let twin = dir.join("twin.rerail");
    std::fs::write(&twin, text.replace("\"ruff-c408-unnecessary-collection-call\"", "\"t:c408-twin\"")).unwrap();
    let classes = dir.join("classes.rerail");
    std::fs::write(
        &classes,
        ".policy \"t:classes\" revision 1\n.language python\n.scope include \"**/*\"\n#include \"python/ruff-classes.rerail\"\n\
         .violation W(at: node) code \"W\" \"w\"\nW(n) :- kind(n, \"zzz_none\").\n",
    )
    .unwrap();
    let paths = [c408.clone(), classes.clone(), twin.clone()];
    let batch = crate::code_rule_dsl::tools::run_policies(&paths, &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let alone = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&c408), &dir, &dir.join("alone"), &serde_json::Value::Null).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let lines = |report: &serde_json::Value, policy: &str| -> Vec<u64> {
        let mut lines: Vec<u64> = report["diagnostics"].as_array().unwrap().iter().filter(|d| d["policy"] == policy).map(|d| d["line"].as_u64().unwrap_or(0)).collect();
        lines.sort_unstable();
        lines
    };
    let own = lines(&alone, "ruff-c408-unnecessary-collection-call");
    assert!(!own.is_empty() && !own.contains(&6), "C408 alone finds the string annotations, not the `Literal`'s text: {alone:#}");
    assert_eq!(lines(&batch, "ruff-c408-unnecessary-collection-call"), own, "{batch:#}");
    assert_eq!(lines(&batch, "t:c408-twin"), own, "{batch:#}");
    // C408 and its twin ask the same and run as one pack; the policy that includes `ruff-classes.rerail` runs alone.
    let together: Vec<u64> = batch["runs"].as_array().unwrap().iter().map(|r| r["together"].as_u64().unwrap_or(0)).collect();
    assert_eq!(together, [2, 1, 2], "{batch:#}");
}

/// O-pha000-kept-part: a reader whose body is only a negation of a demanded relation (`PHA000 :- !allow_status(
/// "plan_helpers", "valid")`) demands nothing, so the relation is built whole: computed only for its other readers'
/// keys, it held no row for this one, and the constant finding was reported though the status is valid. The run
/// from nothing kept (no K4, no kept part: the session tests' oracle, which demands) and the default run agree.
#[test]
fn a_reader_that_only_negates_a_demanded_relation_reads_it_whole() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("negation-only-reader");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("x.py"), "x = 1\n").unwrap();
    let policy = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/data-layer/plan-helpers-allowlist.rerail");
    let codes = |out: &str| -> Vec<String> {
        let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &root.join(out), &serde_json::Value::Null).unwrap();
        report["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap_or("").to_owned()).collect()
    };
    let kept = codes("kept");
    // The oracle's switches, held from this run's start to its end (no other test's run reads them meanwhile).
    let switched = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[("RERAIL_CROSS_K4", "off"), ("RERAIL_KEPT_PART", "off")]);
    let fresh = codes("fresh");
    drop(switched);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(fresh, kept, "the run from nothing kept and the default run");
    assert!(!kept.iter().any(|c| c == "PHA000"), "the allowlist's status is valid: {kept:?}");
}

/// O-store-single-program: a policy whose kept inputs cannot be taken (`public-api-triples.rerail`: an export's rows kept
/// by another column than the memo attributes them by) is refused the kept route before its first wave, and every
/// run over one store gives the first one's findings. In one process this held before too: the fault was a later
/// process's, whose snapshot holds no memo entry of a path looked up before the first wave, so a hit read for its
/// outputs alone and decoded whole at the waves' end was "gone" (or, strict, a read inside the waves). The CLI's
/// batches over one store, strict, show that one; this test keeps the route's refusal from changing the findings.
#[test]
fn a_policy_refused_its_kept_inputs_runs_again_over_its_store() {
    let root = crate::code_rule_dsl::fixtures::temp().with_extension("kept-route-refused");
    std::fs::create_dir_all(root.join("_internal")).unwrap();
    std::fs::write(root.join("_internal/__init__.py"), "\"\"\"Internal helpers.\"\"\"\n\ndef get(o, name):\n    return getattr(o, name)\n").unwrap();
    let policy = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/data-layer/public-api-triples.rerail");
    let run = |out: &str| -> Vec<String> {
        let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &root.join(out), &serde_json::Value::Null).unwrap_or_else(|e| panic!("run {out}: {e}"));
        let mut codes: Vec<String> = report["diagnostics"].as_array().unwrap().iter().map(|d| d.to_string()).collect();
        codes.sort();
        codes
    };
    let first = run("1");
    let (second, third) = (run("2"), run("3"));
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(second, first, "the second run over the store");
    assert_eq!(third, first, "the third run over the store");
}

/// Decision 0062: the statements lowering reads come off the tape. For every `.rerail` file in the repository they are
/// the parser's, statement for statement: the same kind, name and line, a block's statements too, and each clause
/// one the clause reads take (`clause::Reads::check_clause`), with the parser's heads. A file that does not parse
/// gives the parser's error. (The tape is no longer made into the parser's form to compare them whole: the lowering
/// reads the tape itself, and the print-back tests pin what it lowers.)
#[test]
fn statements_from_the_tape_are_the_parsers() {
    use super::clause::Reads;
    use super::from_tape::{RrFile, S, Stmt};
    use super::syntax::{Item, Statement};
    fn same(path: &Path, parsed: &[Statement], taped: &[Stmt]) {
        assert_eq!(parsed.len(), taped.len(), "{}", path.display());
        for ((item, line), st) in parsed.iter().zip(taped) {
            let at = format!("{}:{line}", path.display());
            assert_eq!(*line, st.line(), "{at}");
            let what = st.read().unwrap_or_else(|e| panic!("{at}: {e:?}"));
            match (item, what) {
                (Item::Include(p), S::Include(t)) => assert_eq!(*p, t, "{at}"),
                (Item::Decl { name, .. }, S::Decl(t)) | (Item::Edit { name }, S::Edit(t)) | (Item::Functor { name }, S::Functor(t)) | (Item::Type { name }, S::Type(t)) | (Item::Lattice { name, .. }, S::Lattice(t)) | (Item::Violation { name, .. }, S::Violation(t)) => assert_eq!(name, t, "{at}"),
                (Item::Io { output, .. }, S::Io { output: t }) => assert_eq!(*output, t, "{at}"),
                (Item::Pragma(_), S::Pragma) | (Item::Policy(_), S::Policy) => {}
                (Item::Setting { key, text }, S::Setting { key: k, text: t }) => assert_eq!((key.as_str(), text.as_str()), (k, t), "{at}"),
                (Item::Clause { heads, .. }, S::Clause) => {
                    st.file().check_clause(st.node()).unwrap_or_else(|e| panic!("{at}: {e:?}"));
                    assert_eq!(heads.iter().map(|h| h.rel.as_str()).collect::<Vec<_>>(), st.heads(), "{at}");
                }
                (Item::Test { items, .. } | Item::Fixtures { items, .. }, S::Test | S::Fixtures) => {
                    let (t, n) = st.tape();
                    let (_, _, _, body) = t.block(n).unwrap();
                    same(path, items, &body.into_iter().map(|c| Stmt::at(st.file(), c)).collect::<Vec<_>>());
                }
                (item, what) => panic!("{at}: parser {item:?}, tape {what:?}"),
            }
        }
    }
    // The rule library and packs, and the engine's own programs (typing's, and the pinned golden ones).
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut pending = vec![manifest.join("../docs/rule-syntax"), manifest.join("src/code_rule_dsl/typing/programs"), manifest.join("src/code_rule_dsl/logic/plan/golden/programs")];
    let mut checked = 0;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if !path.extension().is_some_and(|e| e == "rerail") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            let parsed = syntax::Parser::new(&src).and_then(|mut p| p.statements());
            match (parsed, RrFile::read(src.as_str())) {
                (Ok(parsed), Ok(file)) => same(&path, &parsed, &file.statements()),
                (Err(p), Err(t)) => assert_eq!(format!("{p:?}"), format!("{t:?}"), "{}", path.display()),
                (p, t) => panic!("{}: parser {:?}, tape {:?}", path.display(), p.map(|s| s.len()), t.map(|f| f.statements().len())),
            }
            checked += 1;
        }
    }
    assert!(checked > 100);
}

/// RUF013 finds an implicit Optional, and not an explicit one. PR #508's adfb49fac (t47): with Tim's ports of
/// decision 0201 §5 and §6, ruff-ruf013-implicit-optional found none of its 33 keys on the ruff fixtures, so he
/// reverted them; the hub's own 0201 must keep its findings.
#[test]
fn ruff_ruf013_finds_an_implicit_optional() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("ruf013");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.py"),
        "from typing import Optional\n\ndef f(x: int = None):\n    pass\n\ndef g(x: Optional[int] = None):\n    pass\n",
    )
    .unwrap();
    let pack = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs/python/ruff-ruf013-implicit-optional.rerail");
    let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&pack), &dir, &dir.join("out"), &serde_json::Value::Null).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let lines: Vec<u64> = report["diagnostics"].as_array().unwrap().iter().map(|d| d["line"].as_u64().unwrap_or(0)).collect();
    assert_eq!(lines, [3], "{report:#}");
}
