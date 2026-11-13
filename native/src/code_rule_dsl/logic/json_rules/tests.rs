//! The JSON translator's tests (decision 0175): parity with the Python translator on main's 537 rules, and the
//! Python semantics the port keeps.

use super::{pyre, souffle, textglob, value};
use std::collections::BTreeSet;
use std::path::Path;

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

/// Every JSON rule of main's pinned pack translates to the text the Python translator wrote, byte for byte: the
/// pack's `<id>.rr`, or for a rule with a hand-written override, the generated text kept beside it.
#[test]
fn json_rules_parity_with_python() {
    let pack = repo().join("docs/rule-syntax/packs/python-main");
    let mut jsons: Vec<_> = std::fs::read_dir(pack.join("source/rules")).expect("the pinned rules").flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    jsons.sort();
    assert_eq!(jsons.len(), 537, "main's pack 0.1.8 has 537 Python rules");
    let translator = super::Translator::new();
    let mut differ = Vec::new();
    for json in &jsons {
        let id = json.file_stem().expect("a stem").to_string_lossy().into_owned();
        let kept = pack.join("generated-before-override").join(format!("{id}.rr"));
        let golden = if kept.exists() { kept } else { pack.join(format!("{id}.rr")) };
        let want = std::fs::read_to_string(&golden).unwrap_or_else(|e| panic!("{}: {e}", golden.display()));
        match translator.translate_file(json) {
            Ok(got) if got == want => {}
            Ok(_) => differ.push(format!("{id}: differs from {}", golden.display())),
            Err(e) => differ.push(format!("{id}: {e}")),
        }
    }
    assert!(differ.is_empty(), "{} of {} rules differ:\n{}", differ.len(), jsons.len(), differ.join("\n"));
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
    assert_eq!(whole("^foo$"), "foo");
    assert_eq!(whole("foo"), ".*foo.*");
    assert_eq!(whole("^(a|b)"), "(a|b).*");
    assert_eq!(whole("a|^b"), ".*(?:a|^b).*");
    let none = BTreeSet::new();
    assert_eq!(check_rule("r(n) :- kind(n, \"x\").", &none, &[]).unwrap(), Vec::<String>::new());
    assert_eq!(check_rule("r(n, X) :- kind(n, \"x\").", &none, &[]).unwrap(), vec!["X".to_owned()]);
    assert_eq!(check_rule("r(n) :- !kind(n, \"x\").", &none, &[]).unwrap(), vec!["n".to_owned()]);
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
