//! B5's checks: symbols, written by `intern` alone.

use super::*;
use crate::files::tests::W;
use crate::tape::{TapeView, View};
use crate::tests::{D, Fx};
use rerail_blocks::bistate::{ColumnIndex, Read, RowId};

/// Every symbol a leaf's `text` gives, by its text, over a document's views.
fn texts(fx: &Fx, doc: &D) -> Vec<(String, i64)> {
    fx.read(doc, |r, syms, names| {
        let v = TapeView::new(View::Text, 1, r, syms, names);
        (0..Read::end(&v).get())
            .map(RowId::new)
            .filter(|&id| Read::live(&v, id))
            .map(|id| (r.text(id.get() as usize).to_string(), Read::get(&v, id, ColumnIndex::new(2))))
            .collect()
    })
}

#[test]
fn the_same_text_gets_the_same_symbol_across_runs() {
    let mut fx = Fx::new();
    let a = fx.doc("p(x) :- e(x, \"s\").\n", "a.rerail");
    let first: std::collections::BTreeMap<String, i64> = texts(&fx, &a).into_iter().collect();
    let n = fx.atoms.count(&fx.dir);
    // A later run, another document naming the same texts and a new one.
    let b = fx.doc("e(y, z) :- p(y), q(z, \"s\").\n", "b.rerail");
    for (t, s) in texts(&fx, &b) {
        if let Some(&was) = first.get(&t) {
            assert_eq!(s, was, "{t:?}");
        }
        assert_eq!(fx.atoms.name(&fx.dir, s), t);
    }
    assert_eq!(fx.s("s"), fx.s("s"));
    assert!(fx.atoms.count(&fx.dir) > n, "the new texts (q, z) added");
    assert_eq!(fx.atoms.find(&fx.dir, "q").map(|q| q > i64::from(n)), Some(true), "a new id past the first run's");
}

/// An unchanged tape interns nothing: the intern answer is pure, kept by key ((tape, path, module), D4): its
/// symbols block is the one it had, `intern` does not run, the atoms are as they were; the same tape at another path
/// is interned (its key differs).
#[test]
fn an_unchanged_tape_interns_nothing() {
    let mut w = W::new("b5-same");
    let atoms = crate::sym::Atoms::new(&mut w.dir);
    let d = w.answer("bootstrap-l0.rerail").unwrap();
    let s = w.world.intern_answer(&w.ops, &mut w.dir, &mut w.blobs, atoms, &d).unwrap();
    let (runs, n) = (w.world.asked.interns, atoms.count(&w.dir));
    let again = w.answer("bootstrap-l0.rerail").unwrap();
    assert!(again.is_identity());
    assert_eq!(w.world.intern_answer(&w.ops, &mut w.dir, &mut w.blobs, atoms, &again).unwrap(), s);
    assert_eq!((w.world.asked.interns, atoms.count(&w.dir)), (runs, n));
    // The same tape answered at another path: another key, interned.
    let other = crate::files::Doc { path: "elsewhere.rerail".into(), module: "elsewhere".into(), ..again };
    assert_ne!(w.world.intern_answer(&w.ops, &mut w.dir, &mut w.blobs, atoms, &other).unwrap(), s);
    assert_eq!(w.world.asked.interns, runs + 1);
}

/// No view's read writes the atoms: every row of every view of l0 read, and the atoms as they were.
#[test]
fn no_views_read_writes_the_atoms() {
    let mut fx = Fx::new();
    let doc = fx.doc(crate::L0, "bootstrap-l0.rerail");
    let n = fx.atoms.count(&fx.dir);
    let mut cells = 0;
    fx.read(&doc, |r, syms, names| {
        for (view, _, arity) in View::ALL {
            let v = TapeView::new(view, 1, r, syms, names);
            for id in (0..Read::end(&v).get()).map(RowId::new).filter(|&id| Read::live(&v, id)) {
                for c in ColumnIndex::all(arity) {
                    Read::get(&v, id, c);
                    cells += 1;
                }
            }
        }
    });
    assert!(cells > 1000);
    assert_eq!(fx.atoms.count(&fx.dir), n);
}

/// `intern` names only identifiers and strings: a document's new atoms are its identifiers and variables, its strings'
/// values, its path and its module; not a number's text, a string's quoted text, or an operator (the language's
/// name for its token, found). The language's names are interned once, by the first run.
#[test]
fn intern_names_only_identifiers_and_strings() {
    let mut fx = Fx::new();
    fx.doc("e(1, 2).\n", "first.rerail");
    let first = fx.atoms.count(&fx.dir);
    let doc = fx.doc("r1(xa, \"s q\") :- e1(xa, 12345), xa != 67890.\n", "second.rerail");
    let added: std::collections::BTreeSet<String> = (first + 1..=fx.atoms.count(&fx.dir)).map(|id| fx.atoms.name(&fx.dir, i64::from(id))).collect();
    let want: std::collections::BTreeSet<String> = ["second.rerail", "second", "r1", "xa", "s q", "e1"].map(String::from).into();
    assert_eq!(added, want);
    // The operator's symbol is the language's `!=`, and the view reads it.
    let ne = fx.s("!=");
    assert!(texts(&fx, &doc).iter().any(|(t, s)| t == "!=" && *s == ne));
    assert!(texts(&fx, &doc).iter().all(|(t, _)| !t.starts_with(['"', '1', '6'])), "no literal's text");
}

/// `intern` is *0686: Operations* §10's two phases, a hash build and then a scan: at every tiling of the hash phase
/// a document's symbols and the atoms are the same, and its new names take ids in node order.
#[test]
fn interns_hash_then_scan_alike_at_every_tiling_new_ids_in_node_order() {
    let src = "zeta(alpha, \"mid\") :- beta(alpha, gamma), !zeta(gamma, \"mid\"), alpha != gamma.\n";
    let run = |tiling: Tiling| {
        let mut fx = Fx::new();
        fx.doc("e(1, 2).\n", "first.rerail");
        let ids = crate::files::doc_of(&fx.ops, &mut fx.dir, &mut fx.blobs, src).unwrap();
        let (s, t) = (fx.place(&ids.source), fx.place(&ids.tape));
        let syms = intern_at(&fx.ops, &mut fx.dir, &mut fx.blobs, fx.atoms, s, t, "two.rerail", "two", tiling).unwrap();
        let names: Vec<String> = (1..=fx.atoms.count(&fx.dir)).map(|id| fx.atoms.name(&fx.dir, i64::from(id))).collect();
        (syms, names)
    };
    let whole = run(Tiling::WHOLE);
    for tile in [1, 2, 7] {
        for seed in [0, 3] {
            assert_eq!(run(Tiling { tile, seed }), whole, "tiles of {tile}, seed {seed}");
        }
    }
    let at = |n: &str| whole.1.iter().position(|x| x == n).unwrap();
    // The path and module first, then the document's new names as its nodes give them.
    let order = ["two.rerail", "two", "zeta", "alpha", "mid", "beta", "gamma"];
    assert!(order.windows(2).all(|w| at(w[0]) < at(w[1])), "{:?}", &whole.1[whole.1.len() - order.len()..]);
    assert_eq!(whole.1.len() - at("two.rerail"), order.len(), "each new name once");
}

/// A string's value is interned unescaped, read as its body streams (no value gathered): two spellings of one value
/// (`\n` and `\u{0A}`) are one symbol, its name the value itself; a body with no escape is interned as written.
#[test]
fn a_strings_value_is_interned_unescaped_where_its_body_lies() {
    let mut fx = Fx::new();
    let a = fx.doc("p(\"a\\nb\").\nq(\"a\\u{0A}b\").\nr(\"tab\\tx\\\\\").\ns(\"plain\").\n", "a.rerail");
    let values: Vec<(String, i64)> = fx.read(&a, |r, syms, names| {
        let v = TapeView::new(View::Value, 1, r, syms, names);
        (0..Read::end(&v).get())
            .map(RowId::new)
            .filter(|&id| Read::live(&v, id))
            .map(|id| (r.text(id.get() as usize).to_string(), Read::get(&v, id, ColumnIndex::new(2))))
            .collect()
    });
    let sym = |text: &str| values.iter().find(|v| v.0 == text).unwrap_or_else(|| panic!("{text}: {values:?}")).1;
    assert_eq!(sym("\"a\\nb\""), sym("\"a\\u{0A}b\""), "one value, two spellings");
    assert_eq!(fx.atoms.name(&fx.dir, sym("\"a\\nb\"")), "a\nb");
    assert_eq!(fx.atoms.name(&fx.dir, sym("\"tab\\tx\\\\\"")), "tab\tx\\");
    assert_eq!(fx.atoms.name(&fx.dir, sym("\"plain\"")), "plain");
    assert_eq!(fx.atoms.find(&fx.dir, "a\\nb"), None, "the escaped spelling is no value");
}
