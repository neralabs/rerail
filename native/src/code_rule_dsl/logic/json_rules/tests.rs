//! The JSON translator's tests (decisions 0175, 0182): the generated pack is the translator's output on main's 537
//! rules, and the Python semantics the translator keeps.

use super::{patterns, pyre, souffle, textglob, value};
use std::collections::BTreeSet;

fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("..")
}

/// Every JSON rule of main's pinned pack translates to the pack's `<id>.rerail`, byte for byte: the generated pack is
/// what the engine reads from the JSON, so a translator change regenerates it (`site/regen_main_pack.sh`, or
/// `RERAIL_WRITE_PACK=1`, which writes each file that differs).
#[test]
fn json_rules_pack_is_current() {
    let pack = repo().join("docs/rule-syntax/packs/python-main");
    let mut jsons: Vec<_> = std::fs::read_dir(pack.join("source/rules")).expect("the pinned rules").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    jsons.sort();
    assert_eq!(jsons.len(), 537, "main's pack 0.1.8 has 537 Python rules");
    let translator = super::Translator::new();
    let mut differ = Vec::new();
    for json in &jsons {
        let id = json.file_stem().expect("a stem").to_string_lossy().into_owned();
        let golden = pack.join(format!("{id}.rerail"));
        let want = std::fs::read_to_string(&golden).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
        match translator.translate_file(json) {
            Ok(got) if got == want => {}
            Ok(got) if std::env::var_os("RERAIL_WRITE_PACK").is_some() => std::fs::write(&golden, got).unwrap_or_else(|e| panic!("{}: {e}", golden.display())),
            Ok(_) => differ.push(format!("{id}: differs from {}", golden.display())),
            Err(e) => differ.push(format!("{id}: {e}")),
        }
    }
    assert!(differ.is_empty(), "{} of {} rules differ:\n{}", differ.len(), jsons.len(), differ.join("\n"));
}

/// A guest capture the rules read by text (FAST003's route `{K}`) is passed to its helpers by its text (decisions
/// 0652, 0677). A helper that reads K only by text takes the text (`text(K, t)` is `t = K`, its "any node" binder
/// goes); a helper and the clause that bind K join `text(K, …)` where they pass it, inside an aggregate too.
#[test]
fn json_rules_a_capture_read_by_text_is_passed_by_its_text() {
    let mut rules: Vec<(String, Vec<String>)> = vec![
        ("eq(n, K)".into(), vec!["node(_, K, _, _, _)".into(), "kind(n, \"identifier\")".into(), "text(K, t1), text(n, t1)".into()]),
        ("has(n, K)".into(), vec!["child(n, c, _)".into(), "eq(c, K)".into()]),
        ("FAST(n, K)".into(), vec!["guest_name(n, c)".into(), "K = c".into(), "count : { child(n, p, _), !has(p, K) } = 0".into()]),
    ];
    let mut refs: Vec<(&mut String, &mut Vec<String>)> = rules.iter_mut().map(|(h, l)| (h, l)).collect();
    let keyed = super::key_captures_by_text(&BTreeSet::from(["K".to_owned()]), &mut refs);
    assert_eq!(keyed, [("eq".to_owned(), 1), ("has".to_owned(), 1)]);
    assert_eq!(rules[0], ("eq(n, K)".to_owned(), vec!["kind(n, \"identifier\")".to_owned(), "t1 = K, text(n, t1)".to_owned()]));
    assert_eq!(rules[1].1, ["child(n, c, _)", "eq(c, K)"], "has passes K, now its text, on");
    assert_eq!(rules[2].1[2], "count : { child(n, p, _), text(K, K_text1), !has(p, K_text1) } = 0");
}

#[test]
fn json_rules_policy_json_is_recognised() {
    assert!(super::is_policy_json(r#"{"format": "rerail.policy/v1", "id": "x"}"#));
    assert!(!super::is_policy_json(r#"{"format": "other"}"#));
    assert!(!super::is_policy_json(".policy \"x\""));
}

#[test]
fn json_rules_sub_matches_python_on_empty_matches() {
    // re.sub(r"x*", "-", "abxd") == "-a-b--d-"
    assert_eq!(pyre::sub_str(&pyre::re("x*"), "abxd", "-"), "-a-b--d-");
    assert_eq!(pyre::split(&pyre::re("(,)"), "a,b"), vec![Some("a".into()), Some(",".into()), Some("b".into())]);
}

#[test]
fn json_rules_whole_and_check_rule() {
    use souffle::{check_rule, whole};
    let px = patterns::Patterns::new();
    assert_eq!(whole(&px, "^foo$"), "foo");
    assert_eq!(whole(&px, "foo"), ".*foo.*");
    assert_eq!(whole(&px, "^(a|b)"), "(a|b).*");
    assert_eq!(whole(&px, "a|^b"), ".*(?:a|^b).*");
    let none = BTreeSet::new();
    assert_eq!(check_rule(&px, "r(n) :- kind(n, \"x\").", &none, &[]).unwrap(), Vec::<String>::new());
    assert_eq!(check_rule(&px, "r(n, X) :- kind(n, \"x\").", &none, &[]).unwrap(), vec!["X".to_owned()]);
    assert_eq!(check_rule(&px, "r(n) :- !kind(n, \"x\").", &none, &[]).unwrap(), vec!["n".to_owned()]);
}

#[test]
fn json_rules_float_repr_is_pythons() {
    for (f, want) in [(1.0, "1.0"), (0.5, "0.5"), (1e16, "1e+16"), (1.5e-5, "1.5e-05"), (0.0001, "0.0001"), (123.456, "123.456"), (-2.0, "-2.0"), (1e15, "1000000000000000.0")] {
        assert_eq!(value::float_repr(f), want);
    }
}

#[test]
fn json_rules_objects_keep_key_order() {
    let v = value::parse(r#"{"b": 1, "a": [true, null, 2.5, "xé"], "b": 3}"#).unwrap();
    assert_eq!(value::compact(&v), r#"{"b": 3, "a": [true, null, 2.5, "xé"]}"#);
    assert_eq!(value::dumps(&v, true, ", ", ": ", false), r#"{"b": 3, "a": [true, null, 2.5, "x\u00e9"]}"#);
}

#[test]
fn json_rules_text_tests_are_textglobs() {
    let cases = [
        ("hasattr", Some(r#"glob("hasattr", s)"#)),
        (".*_async", Some(r#"suffix("_async", s)"#)),
        ("Sql.*", Some(r#"prefix("Sql", s)"#)),
        (".*cache.*", Some(r#"contains("cache", s)"#)),
        ("(?:compile|lower)", Some(r#"glob("{compile,lower}", s)"#)),
        ("(?i).*encoding.*", Some(r#"icontains("encoding", s)"#)),
        ("_.*(?:branch_tip|revision_tip).*", Some(r#"glob("_**{branch_tip,revision_tip}**", s)"#)),
        ("l\\d{4}_.*", Some(r#"glob("l[0-9][0-9][0-9][0-9]_**", s)"#)),
        ("[A-Z].*", Some(r#"glob("[A-Z]**", s)"#)),
        ("a\\.b", Some(r#"glob("a.b", s)"#)),
        ("(?:api/)?impl/.*", Some(r#"glob("{api/,}impl/**", s)"#)),
        ("_\\w*cache\\w*", None),
        ("[^_].*", None),
        ("(?<x>a)", None),
        (".+", None),
        ("a?b", None),
        ("(?m)^x", None),
        ("\\bSELECT\\b", None),
    ];
    for (rx, want) in cases {
        assert_eq!(textglob::text_test(rx, "s").unwrap().as_deref(), want, "{rx}");
    }
}

/// ast-grep's `any` takes its first alternative that matches: a util's catch-all alternative (`regex: "^"`), which
/// leaves MV unset, holds only where the earlier one, which binds it, does not. Without the guard each identifier
/// also had an unset-MV row, which no-shadow's "is a type" test read (typescript-eslint's valid `type x` in a block).
#[test]
fn json_rules_any_catch_all_holds_only_where_earlier_fails() {
    let json = r#"{"format":"rerail.policy/v1","id":"mini","revision":1,"mode":"steer","description":"mini","scope":{"include":["**/*"],"exclude":[]},
"clauses":[{"id":"shared","define":{"language":"typescript","utils":{
 "mk":{"has":{"stopBy":"end","kind":"identifier","pattern":"$N","any":[{"not":{"kind":"type_identifier"},"pattern":"$MV"},{"regex":"^"}]}}}}},
{"id":"c","forbid":{"ast":{"language":"typescript","rule":{"kind":"identifier","pattern":"$N","inside":{"stopBy":"end","kind":"program","matches":"mk"},
 "not":{"compare":{"left":"MV","op":"eq","right":"MV","text":"node"}}}},"message":"x"}}]}"#;
    let rr = super::Translator::new().translate_text(json, "mini").expect("translates");
    let guarded = "unset(MV), count : { !kind(c1, \"type_identifier\"),node(_, c1, _, _, _), MV_E0 = c1 } = 0";
    assert!(rr.contains(guarded), "the catch-all alternative is guarded:\n{rr}");
}

/// A JSON tool-call rule (a conversation rule on a coding agent's tool calls) translates to a `.scope tool_calls`
/// policy (`docs/rule-syntax/tool-calls.md`) that finds what the JSON rule finds on its pack's examples, sampled as pack
/// validation samples a `tool:*` rule's (`conformance/pack-validation.ts`: a Bash call, its `command` the text): the
/// caught command of `rerail-pack-data-through-publish` is a finding at its call, its passes command none. The
/// translation had no `.violation` before tool calls were an input of the language, and `run-policies` refused it.
#[test]
fn json_tool_call_rule_finds_its_examples() {
    let id = "rerail-pack-data-through-publish";
    let pack: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(repo().join("packs/rerail/pack.json")).unwrap()).unwrap();
    let examples = &pack["rules"].as_array().unwrap().iter().find(|r| r["id"] == id).expect("the rule in its pack")["examples"];
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join(format!("{id}.rerail"));
    let text = super::Translator::new().translate_file(&repo().join(format!("packs/rerail/rules/{id}.json"))).unwrap();
    std::fs::write(&policy, &text).unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let calls = ["caught", "passes"].map(|side| serde_json::json!({"id": side, "tool": "Bash", "input": {"command": examples[side]["text"]}}));
    let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &dir.path().join("out"), &serde_json::json!({ "toolCalls": calls })).unwrap();
    let found: Vec<_> = report["diagnostics"].as_array().unwrap().iter().map(|d| (d["code"].clone(), d["path"].clone(), d["line"].clone(), d["at"].clone())).collect();
    assert_eq!(found, [(serde_json::json!(id), serde_json::json!(""), serde_json::json!(0), serde_json::json!("caught"))], "{text}");
    assert!(text.contains(".scope tool_calls") && text.contains(&format!(".violation rerail_pack_data_through_publish(at: symbol) code \"{id}\"")), "{text}");
    // A form no shipped tool-call rule uses is named, not dropped.
    let eq = r#"{"id": "x", "mode": "steer", "match": {"field": "input.command", "eq": "ls"}, "sample": {"source": "tool:*", "boundary": "tool-call"}, "action": {"content": "no"}}"#;
    let refused = super::Translator::new().translate_text(eq, "x").unwrap_err();
    assert!(refused.contains("not translated"), "{refused}");
}

/// A value's `type` is the node's own (`value_type_of(n, t)`), not its text's (decision 0677): `value_type(v, t)` is
/// keyed by the text, which `"12"` and `12` share, so `value(n, x), value_type(x, "int")` held at the string too,
/// from the int's row, and joined every file's types in the cross-file part. A value followed through a binding
/// (`follow`) is no node's own, and keeps its text's type.
#[test]
fn json_rules_value_type_is_the_nodes_own() {
    let rule = |value: &str| {
        format!(
            r#"{{"format":"rerail.policy/v1","id":"int-literal","revision":1,"mode":"steer","description":"x","scope":{{"include":["**/*"],"exclude":[]}},
"clauses":[{{"id":"c","forbid":{{"ast":{{"language":"python","rule":{{"any":[{{"kind":"integer"}},{{"kind":"string"}}],"value":{value}}}}}}},"message":"an int"}}]}}"#
        )
    };
    let own = super::Translator::new().translate_text(&rule(r#"{"type":"int"}"#), "int-literal").expect("translates");
    assert!(own.contains("value_type_of(") && !own.contains("value_type("), "{own}");
    let followed = super::Translator::new().translate_text(&rule(r#"{"follow":true,"type":"int"}"#), "int-literal").expect("translates");
    assert!(followed.contains("static_value(") && followed.contains("value_type("), "{followed}");
    let dir = tempfile::tempdir().unwrap();
    let policy = dir.path().join("int-literal.rerail");
    std::fs::write(&policy, &own).unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.py"), "a = \"12\"\nb = 12\n").unwrap();
    let lines = |text: &str| {
        std::fs::write(&policy, text).unwrap();
        let report = crate::code_rule_dsl::tools::run_policies(std::slice::from_ref(&policy), &root, &dir.path().join("out"), &serde_json::Value::Null).unwrap();
        report["diagnostics"].as_array().unwrap().iter().map(|d| d["line"].as_i64().unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(lines(&own), [2], "the int alone, not the string of the same text:\n{own}");
    // The text's type, as it was translated: the string's text is the int's, so it is found too.
    let at = own.find("value_type_of(").unwrap() + "value_type_of(".len();
    let node = own[at..].split(',').next().unwrap();
    let read = own.find(&format!("value({node}, ")).unwrap() + format!("value({node}, ").len();
    let var = own[read..].split(')').next().unwrap();
    let by_text = own.replace(&format!("value_type_of({node}, "), &format!("value_type({var}, "));
    assert_eq!(lines(&by_text), [1, 2], "{by_text}");
}
