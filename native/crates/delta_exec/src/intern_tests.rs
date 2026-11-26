//! B5's checks: symbols, written by `intern` alone.

use super::*;
use crate::files::Files;
use crate::tape::{TapeView, View};
use crate::tests::{D, Fx};
use rerail_blocks::bistate::{ColumnIndex, Read, RowId};

/// Every symbol a leaf's `text` gives, by its text, over a document's views.
fn texts(fx: &Fx, doc: &D) -> Vec<(String, i64)> {
    fx.read(doc, |r, syms, names| {
        let v = TapeView::new(View::Text, 1, r, syms, names);
        (0..Read::end(&v).get() as u32)
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

/// An unchanged tape interns nothing: its symbols block is the one it had, `intern` does not run, the atoms are as
/// they were.
#[test]
fn an_unchanged_tape_interns_nothing() {
    let dir_path = std::env::temp_dir().join(format!("rerail-b5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    std::fs::write(dir_path.join("bootstrap-l0.rerail"), crate::L0).unwrap();
    let mut files = Files::new(dir_path.clone(), dir_path.join("observer.db"));
    let mut fx = Fx::new();
    let mut interned = Interned::default();
    let d = files.answer("bootstrap-l0.rerail", &fx.reg, &mut fx.blobs, &mut fx.dir).unwrap();
    let s = interned.of(&fx.reg, &fx.ops, &mut fx.dir, &mut fx.blobs, fx.atoms, &d).unwrap();
    let (runs, atoms) = (interned.runs, fx.atoms.count(&fx.dir));
    let again = files.answer("bootstrap-l0.rerail", &fx.reg, &mut fx.blobs, &mut fx.dir).unwrap();
    assert!(again.is_identity());
    assert_eq!(interned.of(&fx.reg, &fx.ops, &mut fx.dir, &mut fx.blobs, fx.atoms, &again).unwrap(), s);
    assert_eq!((interned.runs, fx.atoms.count(&fx.dir)), (runs, atoms));
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
            for id in (0..Read::end(&v).get() as u32).map(RowId::new).filter(|&id| Read::live(&v, id)) {
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
