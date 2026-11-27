//! Node paths' tables and codes (decision 0722).

use crate::code_rule_dsl::language::{Family, GrammarDialect, base, tape_tag};

/// Every name a family's node path tables hold is a kind or field of one of its grammars: a table written for another
/// version of a grammar fails here, not as paths made longer by escapes.
#[test]
fn every_name_in_a_familys_path_tables_is_one_of_its_grammars() {
    let mut families: Vec<(Family, Vec<GrammarDialect>)> = Vec::new();
    for &d in GrammarDialect::all() {
        if tape_tag(d).is_none() {
            continue;
        }
        match families.iter_mut().find(|(f, _)| *f == Family::of(d)) {
            Some((_, ds)) => ds.push(d),
            None => families.push((Family::of(d), vec![d])),
        }
    }
    let mut wrong = Vec::new();
    for (family, dialects) in &families {
        let tables = base(*family).path_tables();
        let kind = |k: &str| dialects.iter().any(|d| d.kinds().kind_id(k) != 0);
        let field = |f: &str| f.is_empty() || dialects.iter().any(|d| d.kinds().field_id(f).is_some());
        for p in tables.parents {
            if !kind(p.kind) {
                wrong.push(format!("{family:?}: parent {}", p.kind));
            }
            for s in p.slots {
                if !field(s.field) {
                    wrong.push(format!("{family:?}: {}'s field {}", p.kind, s.field));
                }
                if !s.grouped && !s.kinds.is_empty() {
                    wrong.push(format!("{family:?}: {}.{} lists kinds but is not kind-grouped", p.kind, s.field));
                }
                wrong.extend(s.kinds.iter().filter(|k| !kind(k)).map(|k| format!("{family:?}: {}.{}'s kind {k}", p.kind, s.field)));
            }
        }
        wrong.extend(tables.lists.iter().filter(|k| !kind(k)).map(|k| format!("{family:?}: list {k}")));
        wrong.extend(tables.labels.iter().filter(|(n, _)| !n.strip_prefix('.').map_or_else(|| kind(n), field)).map(|(n, _)| format!("{family:?}: label of {n}")));
    }
    assert!(wrong.is_empty(), "names no grammar of the family has:\n{}", wrong.join("\n"));
}

use super::{Atoms, DocPaths, WORD_STEPS, is_path_child};
use crate::code_rule_dsl::facts::core::{self, Built, Facts};
use crate::code_rule_dsl::facts::vtab::syntax::{Dialect, Projected, Shape};
use crate::code_rule_dsl::model::FileInput;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A test's interning.
#[derive(Default)]
struct Interned(Mutex<(Vec<String>, HashMap<String, u32>)>);

impl Atoms for Interned {
    fn atom(&self, text: &str) -> u32 {
        let mut t = self.0.lock().unwrap();
        if let Some(&a) = t.1.get(text) {
            return a;
        }
        let a = t.0.len() as u32;
        t.0.push(text.to_owned());
        t.1.insert(text.to_owned(), a);
        a
    }
    fn text(&self, atom: u32) -> Option<String> {
        self.0.lock().unwrap().0.get(atom as usize).cloned()
    }
}

fn facts(label: &str, path: &str, source: &str) -> Arc<Facts> {
    let file = FileInput::new(path.to_owned(), label.to_owned(), Arc::new(source.to_owned().into()), None, None, "core").unwrap();
    let Ok(Built::Facts(facts)) = core::build(&file) else { panic!("{label}: no facts") };
    Arc::new(facts)
}

/// Each path child's word, by tape node, after checking that it decodes back to it.
fn round_trip(label: &str, path: &str, source: &str, atoms: &Interned) -> (Arc<Facts>, HashMap<u32, u64>, usize) {
    let facts = facts(label, path, source);
    let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
    let p = Projected::new(1, Arc::clone(&facts), Some(&dialect), Shape::Core, false, None);
    let paths = DocPaths::new(&p, &dialect, path).expect("ids by definition");
    let f = p.f();
    let mut words = HashMap::new();
    let mut anchored = 0;
    for x in 0..f.len() as u32 {
        // Only the root and nodes on path children all the way up: the paths' tree.
        let mut up = x;
        let mut reached = true;
        while let Some(q) = f.parent(up) {
            if !is_path_child(f, up) {
                reached = false;
                break;
            }
            up = q;
        }
        if !reached {
            continue;
        }
        let word = paths.encode(&p, &dialect, atoms, x).unwrap_or_else(|| panic!("{label}: no path for node {x} ({})", dialect.name(f, x)));
        assert!((word as u32).leading_zeros() >= 32 - WORD_STEPS - 1, "{label}: a word's steps");
        anchored += usize::from(atoms.text((word >> 32) as u32).unwrap().contains("\t@"));
        assert_eq!(paths.decode(&p, &dialect, atoms, word), Some(x), "{label}: node {x} ({}) from its path", dialect.name(f, x));
        words.insert(x, word);
    }
    let mut distinct: Vec<u64> = words.values().copied().collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), words.len(), "{label}: a word a node");
    (facts, words, anchored)
}

/// Every node the paths reach, in each language, decodes from its path to itself, and no two share a word; a deep
/// tree and a long list are placed under anchors and still fit their words.
#[test]
fn a_node_path_decodes_to_its_node_in_every_language() {
    // 300 parentheses are one only-child run: its steps fit a word (gamma(300)), no anchor.
    let deep = format!("x = {}1{}\n", "(".repeat(300), ")".repeat(300));
    // 300 sums each nested in the right of the one above: each level its own steps, so anchors.
    let sums = format!("x = {}1{}\n", "(1 + ".repeat(300), ")".repeat(300));
    let long = format!("y = [{}]\n", (0..70_000).map(|i| i.to_string()).collect::<Vec<_>>().join(", "));
    let cases: &[(&str, &str, &str)] = &[
        ("python", "pkg/a.py", "import os\nclass C:\n    x = 1\n    def m(self, a, b=2):\n        if a:\n            return b\n        elif b:\n            pass\n        else:\n            raise E()\n        # note\n        return [i for i in a]\n"),
        ("python", "pkg/deep.py", &deep),
        ("python", "pkg/sums.py", &sums),
        ("python", "pkg/long.py", &long),
        ("typescript", "src/a.ts", "export function A(x: number): string { if (x) { return `${x}`; } return ''; }\nexport class Z { f() { return 1; } }\n"),
        ("rust", "src/a.rs", "impl S { fn a(&self) -> u32 { match self.x { 1 => 2, _ => 3 } } }\nmod m { pub fn g() -> u32 { 1 } }\n"),
        ("php", "a.php", "<?php\nfunction f($a) { if ($a) { return 1; } else { return 2; } }\nclass K { public function m() { echo 'x'; } }\n"),
        ("c", "a.c", "int f(int a) { if (a) { return 1; } return 0; }\nstruct S { int x; };\n"),
        ("go", "a.go", "package main\nfunc f(a int) int { if a > 0 { return 1 }; return 0 }\n"),
        ("java", "A.java", "class A { int f(int a) { if (a > 0) { return 1; } return 0; } }\n"),
        ("sql", "a.sql", "SELECT a, b FROM t WHERE a = 1;\nCREATE TABLE u (x INT);\n"),
        ("json", "a.json", "{\"a\": [1, 2, {\"b\": null}], \"c\": \"d\"}\n"),
    ];
    let atoms = Interned::default();
    for &(label, path, source) in cases {
        let (_, words, anchored) = round_trip(label, path, source, &atoms);
        assert!(words.len() > 5, "{label}: {} paths", words.len());
        if path.contains("sums") || path.contains("long") {
            assert!(anchored > 0, "{label} {path}: anchors placed");
        } else {
            assert_eq!(anchored, 0, "{label} {path}: no anchor where every path fits its word");
        }
    }
}

/// An edit inside one function leaves every path of another function as it was: its granule's key and its steps.
#[test]
fn an_edit_in_one_function_leaves_another_functions_paths() {
    let before = "def f(a):\n    x = a\n    return x\n\ndef g(b):\n    if b:\n        return b(1, 2)\n    return 0\n";
    let after = "def f(a):\n    print(a)\n    x = a + 1\n    return x\n\ndef h():\n    pass\n\ndef g(b):\n    if b:\n        return b(1, 2)\n    return 0\n";
    let atoms = Interned::default();
    let (fa, wa, _) = round_trip("python", "m.py", before, &atoms);
    let (fb, wb, _) = round_trip("python", "m.py", after, &atoms);
    let in_g = |facts: &Facts, words: &HashMap<u32, u64>| {
        let g_at = facts.source.find("def g").unwrap() as u32;
        facts.alone(|f| {
            let mut out: Vec<(u64, String)> = words.iter().filter(|&(&x, _)| f.span(x).0 >= g_at).map(|(&x, &w)| (w, f.text(x).to_owned())).collect();
            out.sort();
            out
        })
    };
    let (ga, gb) = (in_g(&fa, &wa), in_g(&fb, &wb));
    assert!(ga.len() > 8, "g's paths: {}", ga.len());
    assert_eq!(ga, gb, "g's paths after an edit to f and a function added");
}

/// A small file's paths fit their words with no anchor: its steps are coded by its language's tables, none escaped.
#[test]
fn a_small_files_paths_need_no_anchor() {
    let source = "def f(x):\n    return g(x, 1)\n\nclass C:\n    def m(self):\n        return g(2)\n";
    let atoms = Interned::default();
    let (facts, words, anchored) = round_trip("python", "a.py", source, &atoms);
    if anchored > 0 {
        let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
        let p = Projected::new(1, Arc::clone(&facts), Some(&dialect), Shape::Core, false, None);
        let paths = DocPaths::new(&p, &dialect, "a.py").unwrap();
        let f = p.f();
        let mut lines = Vec::new();
        for (&x, &w) in &words {
            if let Some(q) = f.parent(x) {
                let bits = paths.steps(f, &dialect, q).to(x).map(|b| b.len());
                lines.push(format!("{} > {}: step {bits:?}, base {:?}", dialect.name(f, q), dialect.name(f, x), atoms.text((w >> 32) as u32)));
            }
        }
        lines.sort();
        panic!("{anchored} paths under anchors:\n{}", lines.join("\n"));
    }
}

/// A path is spelled (decision 0722 §4) by its file, its units (a granule's name; `#k` only where siblings of its
/// parent share it) and its steps (a field's label, else a kind's; `#k` only where siblings are spelled alike).
#[test]
fn a_path_is_spelled_by_its_units_and_labelled_steps() {
    let source = "class C:\n    def m(self):\n        if a:\n            return 1\n        return 2\n\ndef f():\n    pass\n\ndef f():\n    pass\n";
    let facts = facts("python", "pkg/mod.py", source);
    let dialect = Dialect::new(core::facts_tag(&facts)).unwrap();
    let p = Projected::new(1, Arc::clone(&facts), Some(&dialect), Shape::Core, false, None);
    let paths = DocPaths::new(&p, &dialect, "pkg/mod.py").unwrap();
    let f = p.f();
    let mut spelled: Vec<String> = (0..f.len() as u32).filter(|&x| f.named(x)).filter_map(|x| paths.spell(&p, &dialect, x)).collect();
    spelled.sort();
    for want in [
        "pkg/mod.py::C.m",
        "pkg/mod.py::C.m/body.if",
        "pkg/mod.py::C.m/body.if.cond",
        "pkg/mod.py::C.m/body.if.then.return",
        "pkg/mod.py::C.m/body.return",
        "pkg/mod.py::C.m/params",
        "pkg/mod.py::f#0",
        "pkg/mod.py::f#1",
    ] {
        assert!(spelled.iter().any(|s| s == want), "{want} among {spelled:#?}");
    }
    assert!(!spelled.iter().any(|s| s.contains("::C#") || s.contains("m#")), "one C and one m: no ordinal {spelled:#?}");
}
