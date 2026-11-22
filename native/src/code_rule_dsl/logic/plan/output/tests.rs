use super::*;

/// Relation `name` of the stack in `bytes`, each value as shown (`Relation::shown`): sorted, each once.
fn shown_in(bytes: &[u8], name: &str) -> Vec<Vec<String>> {
    Outputs::of(bytes).unwrap().get(name).unwrap_or_else(|| panic!("no relation {name}")).shown()
}

/// Expected rows from values as written.
fn rows(v: &[&[&str]]) -> Vec<Vec<String>> {
    v.iter().map(|r| r.iter().map(|s| (*s).to_owned()).collect()).collect()
}

/// A stack's bytes: the images given, one after another.
fn stacked(images: &[&[u8]]) -> Vec<u8> {
    images.concat()
}

#[test]
fn an_image_holds_each_relations_rows_and_types_as_written() {
    let mut b = Builder::new(flags::OWN, false);
    let (x, y) = (i64::from(b.text("x")), i64::from(b.text("y")));
    // Unsorted, with a row twice: held sorted, once.
    b.rows("p", &[col::TEXT, col::NODE, col::INT], vec![y, 5 << 32 | 3, -7, x, 1 << 32, 2, y, 5 << 32 | 3, -7]);
    b.rows("empty", &[col::INT], Vec::new());
    b.rows("none", &[], vec![]);
    let many: Vec<i64> = (0..10_000).rev().flat_map(|i| [i, i * 3]).collect();
    b.rows("many", &[col::INT, col::INT], many);
    let bytes = b.finish();
    assert_eq!(bytes.len() % 16, 0);
    let stack = Image::stack(&bytes).unwrap();
    assert_eq!(stack.len(), 1);
    let image = &stack[0];
    assert!(!image.is_overlay());
    assert_eq!(image.names().collect::<Vec<_>>(), ["p", "empty", "none", "many"]);
    let p = image.rel("p").unwrap();
    assert_eq!((p.width, p.rows), (3, 2));
    assert_eq!(p.types(), [col::TEXT, col::NODE, col::INT]);
    assert_eq!(p.row(0), vec![x, 1 << 32, 2]);
    assert_eq!(p.row(1), vec![y, 5 << 32 | 3, -7]);
    assert_eq!(image.string(p.col(0)[1] as u32), "y");
    // Read typed: a text, a node (of no document the image names: a guest's), an int.
    assert_eq!(p.value(0, 0), Value::Text("x"));
    assert_eq!(p.value(0, 1), Value::Node(Node::Guest { doc: 1, local: 0 }));
    assert_eq!(p.value(1, 2), Value::Int(-7));
    assert_eq!(image.rel("empty").unwrap().rows, 0);
    assert_eq!(image.rel("none").unwrap().rows, 0);
    let many = image.rel("many").unwrap();
    assert_eq!(many.rows, 10_000);
    assert!(many.col(0).windows(2).all(|w| w[0] < w[1]), "sorted");
    assert!((0..many.rows).all(|i| many.col(1)[i] == many.col(0)[i] * 3));
    assert!(image.rel("q").is_none());
}

/// A node column's values read as the documents and places the image names: by path and local part (`OWN`, or by
/// definition), by T's numbering (a document's first node plus the local part), a guest's, `unset`, a place.
#[test]
fn an_images_nodes_read_by_their_documents() {
    use crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS;
    let place = -2 - (7i64 << 32 | 4);
    let by_def = 5i64 << INDEX_BITS | 3;
    let values = [1i64 << 32 | 9, 2 << 32 | by_def, 3 << 32 | 2, 9 << 32 | 1, -1, place];
    let image = |f: u64| {
        let mut b = Builder::new(f, false);
        b.doc(1, &Doc { path: Some("a.py".into()), first: Some(100) });
        b.doc(2, &Doc { path: Some("pkg/b.py".into()), first: None });
        b.doc(3, &Doc { path: None, first: Some(5_000) });
        b.place(place, "c.py", 4);
        let call = i64::from(b.text("call"));
        b.rows("r", &[col::NODE, col::TEXT], values.iter().flat_map(|&v| [v, call]).collect());
        b.finish()
    };
    let shown = |f: u64| -> Vec<String> {
        let mut v: Vec<String> = shown_in(&image(f), "r").into_iter().map(|r| r[0].clone()).collect();
        v.sort();
        v
    };
    let sorted = |v: &[String]| {
        let mut v = v.to_vec();
        v.sort();
        v
    };
    let (two, three) = ((2i64 << 32 | by_def).to_string(), (3i64 << 32 | 2).to_string());
    let common = |mut v: Vec<String>| {
        v.extend(["guest9:1".to_owned(), "-1".to_owned(), "@place(\"c.py\", 4)".to_owned()]);
        sorted(&v)
    };
    // T's numbering: a document's first node plus the local part; a document of no first node, its value.
    assert_eq!(shown(0), common(vec!["109".into(), two.clone(), "5002".into()]));
    // Its own documents: by path and local part; a document of no path, its value.
    assert_eq!(shown(flags::OWN), common(vec!["a.py:9".into(), format!("pkg/b.py:{by_def}"), three.clone()]));
    // By definition: `slot.index`.
    assert_eq!(shown(flags::DEFINITION_IDS), common(vec!["a.py:0.9".into(), "pkg/b.py:5.3".into(), "5002".into()]));
    assert_eq!(shown(flags::OWN | flags::DEFINITION_IDS), common(vec!["a.py:0.9".into(), "pkg/b.py:5.3".into(), three]));
    // Typed, each is its node.
    let bytes = image(flags::OWN);
    let outputs = Outputs::of(&bytes).unwrap();
    let nodes: Vec<Node<'_>> = outputs.rows("r").iter().filter_map(|r| r.get(0).node()).collect();
    assert!(nodes.contains(&Node::Place { path: "c.py", line: 4 }));
    assert!(nodes.contains(&Node::Guest { doc: 9, local: 1 }));
    assert!(nodes.contains(&Node::Other(-1)));
    assert!(nodes.iter().any(|n| n.at() == Some(("a.py", 9))));
    assert!(outputs.rows("r").iter().any(|r| r.get(0).path() == Some("c.py")));
    assert_eq!(outputs.rows("r").iter().filter(|r| r.get(1).text() == Some("call")).count(), values.len());
}

/// A place is found by its value among many (the places region is sorted and searched).
#[test]
fn an_images_places_are_searched_by_value() {
    let mut b = Builder::new(0, false);
    let places: Vec<i64> = (0..500).map(|i| -2 - ((i % 7) << 32 | i)).collect();
    for (i, &v) in places.iter().enumerate().rev() {
        b.place(v, &format!("p{}.py", i % 7), i as i64);
    }
    b.rows("r", &[col::NODE], places.clone());
    let image = Image::one(&b.finish()).unwrap();
    for (i, &v) in places.iter().enumerate() {
        assert_eq!(image.place(v), Some((format!("p{}.py", i % 7).as_str(), i as i64)));
    }
    assert_eq!(image.place(-2 - (99 << 32 | 1)), None);
    assert_eq!(image.node(-2 - (99 << 32 | 1)), Node::Other(-2 - (99 << 32 | 1)));
}

#[test]
fn an_overlay_replaces_the_relations_it_holds_and_the_rest_read_through() {
    let dir = crate::code_rule_dsl::fixtures::temp();
    std::fs::create_dir_all(&dir).unwrap();
    let mut base = Builder::new(0, false);
    let a = i64::from(base.text("a"));
    base.rows("p", &[col::TEXT], vec![a]);
    base.rows("q", &[col::INT], vec![1, 2]);
    write(&dir, &base.finish()).unwrap();
    let mut over = Builder::new(0, true);
    let (b, c) = (i64::from(over.text("b")), i64::from(over.text("c")));
    over.rows("p", &[col::TEXT], vec![c, b]);
    write(&dir, &over.finish()).unwrap();
    let stack = read(&dir).unwrap();
    assert_eq!(stack.len(), 2);
    assert!(stack[1].is_overlay());
    assert_eq!(shown(&dir, "p").unwrap(), [["b"], ["c"]]);
    assert_eq!(shown(&dir, "q").unwrap(), [["1"], ["2"]]);
    // A base written again starts the stack over.
    let mut again = Builder::new(0, false);
    again.rows("q", &[col::INT], vec![3]);
    write(&dir, &again.finish()).unwrap();
    let stack = read(&dir).unwrap();
    assert_eq!(stack.len(), 1);
    assert!(find(&stack, "p").is_none());
    assert!(read(&dir.join("nowhere")).unwrap().is_empty());
    // An overlay over no stack is refused, and leaves nothing.
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(write(&empty, &Builder::new(0, true).finish()).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert!(!empty.join(FILE).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file is replaced whole: a map of the old one reads it as it was, the new one is read as written, and no file
/// is left beside it.
#[test]
fn a_write_replaces_the_file_whole_and_a_map_keeps_the_old() {
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("replace");
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = Builder::new(0, false);
    a.rows("n", &[col::INT], vec![1, 2, 3]);
    write(&dir, &a.finish()).unwrap();
    let old = Image::mapped(&dir.join(FILE)).unwrap();
    let mut b = Builder::new(0, false);
    b.rows("n", &[col::INT], vec![7]);
    replace(&dir, &[&b.finish()]).unwrap();
    assert_eq!(find(&old, "n").unwrap().len(), 3, "the old map reads the old file");
    assert_eq!(shown(&dir, "n").unwrap(), [["7"]]);
    let left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, [FILE]);
    // A mapped stack starts 16-byte aligned.
    let mapped = Image::mapped(&dir.join(FILE)).unwrap();
    assert_eq!(mapped[0].bytes().as_ptr() as usize % 16, 0);
    assert_eq!(Image::mapped(&dir.join("none")).map(|_| ()).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The file holds each image as the root of a frozen block (decision 0266's shared header): blocks one after
/// another, each read where it is. A block cut short, not frozen, or an image without its block is refused.
#[test]
fn the_file_holds_each_image_as_a_frozen_block() {
    use rerail_blocks::block::{frozen_at, kind};
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("blocks");
    std::fs::create_dir_all(&dir).unwrap();
    let mut base = Builder::new(0, false);
    base.rows("n", &[col::INT], vec![1, 2]);
    let base = base.finish();
    write(&dir, &base).unwrap();
    let mut over = Builder::new(0, true);
    over.rows("n", &[col::INT], vec![5]);
    write(&dir, &over.finish()).unwrap();
    let file = std::fs::read(dir.join(FILE)).unwrap();
    let first = frozen_at(&file, kind::OUTPUTS).unwrap();
    assert_eq!(&file[first.root..first.root + base.len()], &base[..], "the image is the block's root, as made");
    let second = frozen_at(&file[first.length..], kind::OUTPUTS).unwrap();
    assert_eq!(first.length + second.length, file.len(), "two blocks, nothing else");
    assert_eq!(shown(&dir, "n").unwrap(), [["5"]]);
    let refused = |bytes: &[u8]| {
        // The file is written read-only (decision 0574): replaced, as a write replaces it whole.
        let _ = std::fs::remove_file(dir.join(FILE));
        std::fs::write(dir.join(FILE), bytes).unwrap();
        read(&dir).is_err()
    };
    assert!(refused(&file[..file.len() - 64]), "a block written in part");
    let mut open = file.clone();
    let state = std::mem::offset_of!(rerail_blocks::Header, state);
    open[first.length + state..first.length + state + 4].copy_from_slice(&rerail_blocks::block::OPEN.to_ne_bytes());
    assert!(refused(&open), "a block not frozen");
    assert!(refused(&base), "an image without its block");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A stack folded into one base (`Builder::take`, each relation from the image that holds it) reads every relation
/// as the stack did: its strings, documents and places copied with it.
#[test]
fn a_folded_stack_shows_what_the_stack_showed() {
    let place = -2 - (3i64 << 32 | 8);
    let mut base = Builder::new(flags::OWN | flags::DEFINITION_IDS, false);
    base.doc(1, &Doc { path: Some("a.py".into()), first: None });
    base.place(place, "b.py", 8);
    let t = i64::from(base.text("int"));
    base.rows("type_of", &[col::NODE, col::TEXT], vec![1 << 32 | (2 << crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS | 5), t, place, t, 9 << 32 | 1, t]);
    base.rows("gone", &[col::INT], vec![4]);
    let mut over = Builder::new(flags::OWN | flags::DEFINITION_IDS, true);
    let s = i64::from(over.text("str"));
    over.rows("gone", &[col::TEXT], vec![s]);
    let bytes = stacked(&[&base.finish(), &over.finish()]);
    let stack = Image::stack(&bytes).unwrap();
    let mut fold = Builder::new(flags::OWN | flags::DEFINITION_IDS, false);
    for name in ["type_of", "gone"] {
        fold.take(name, &find(&stack, name).unwrap());
    }
    let folded = fold.finish();
    assert_eq!(Image::stack(&folded).unwrap().len(), 1);
    for name in ["type_of", "gone"] {
        assert_eq!(shown_in(&folded, name), shown_in(&bytes, name), "{name}");
    }
    assert_eq!(shown_in(&folded, "type_of"), rows(&[&["@place(\"b.py\", 8)", "int"], &["a.py:2.5", "int"], &["guest9:1", "int"]]));
    assert_eq!(shown_in(&folded, "gone"), [["str"]]);
}

#[test]
fn a_damaged_image_is_refused() {
    let mut b = Builder::new(0, false);
    let x = i64::from(b.text("x"));
    b.rows("p", &[col::TEXT], vec![x]);
    let good = b.finish();
    assert!(Image::stack(&good).is_ok());
    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(Image::stack(&magic).is_err());
    // The cols region's offset moved past the end.
    let mut bounds = good.clone();
    let at = 16 + 24 * R::Cols as usize + 16;
    bounds[at..at + 8].copy_from_slice(&(good.len() as u64 + 16).to_le_bytes());
    assert!(Image::stack(&bounds).is_err());
    // A text value naming no string.
    let mut b = Builder::new(0, false);
    b.rows("p", &[col::TEXT], vec![9]);
    assert!(Image::stack(&b.finish()).is_err());
    // A stack that starts with an overlay.
    let o = Builder::new(0, true).finish();
    assert!(Image::stack(&o).is_err());
}

/// A run's rows (`Rows`), as `decoded_outputs` hands them, put in an image (`Builder::put`) as they are: the parts'
/// texts held once, a text column taking another part's integers as shown, nodes read by their documents.
#[test]
fn rows_put_in_an_image_read_as_the_run_made_them() {
    let place = -2 - (7i64 << 32 | 4);
    let mut a = Rows::new(vec![col::NODE, col::TEXT, col::INT]);
    a.push(&[Cell::Int(1 << 32 | 9), Cell::Text("call"), Cell::Int(3)]);
    a.push(&[Cell::Int(3 << 32 | 2), Cell::Text("x\ty"), Cell::Int(-4)]);
    a.push(&[Cell::Int(place), Cell::Text("z"), Cell::Int(0)]);
    a.push(&[Cell::Int(1 << 32 | 9), Cell::Text("call"), Cell::Int(3)]);
    assert_eq!(a.len(), 4, "as handed: not yet each once");
    // Another part holds its third column as text, and its second takes an integer (held as it is shown).
    let mut b = Rows::new(vec![col::NODE, col::TEXT, col::TEXT]);
    b.push(&[Cell::Int(2 << 32 | 1), Cell::Text("name"), Cell::Text("007")]);
    b.push(&[Cell::Int(-1), Cell::Int(12), Cell::Text("8")]);
    assert_eq!(b.cell(1, 1), Cell::Text("12"));
    let mut all = a.clone();
    all.append(&a);
    assert_eq!(all.len(), 8, "parts of one type append as they are");
    for f in [0, flags::OWN] {
        let mut image = Builder::new(f, false);
        image.doc(1, &Doc { path: Some("a.py".into()), first: Some(100) });
        image.doc(2, &Doc { path: Some("pkg/b.py".into()), first: None });
        image.place(place, "c.py", 4);
        image.put("r", &[&a, &b], &[0]);
        let bytes = image.finish();
        let outputs = Outputs::of(&bytes).unwrap();
        let r = outputs.get("r").unwrap();
        assert_eq!(r.types(), [col::NODE, col::TEXT, col::TEXT], "flags {f}");
        assert_eq!(r.len(), 5, "flags {f}: each row once");
        // A value holding a tab is one value.
        assert!(r.rows().any(|row| row.get(1).text() == Some("x\ty") && row.get(2).text() == Some("-4")));
        let a_py = if f & flags::OWN != 0 { "a.py:9" } else { "109" };
        let guest = "guest3:2".to_owned();
        let b_py = if f & flags::OWN != 0 { "pkg/b.py:1".to_owned() } else { (2i64 << 32 | 1).to_string() };
        let mut want = rows(&[&[a_py, "call", "3"], &[&guest, "x\ty", "-4"], &["@place(\"c.py\", 4)", "z", "0"], &[&b_py, "name", "007"], &["-1", "12", "8"]]);
        want.sort();
        assert_eq!(r.shown(), want, "flags {f}");
    }
}

/// NULL (SQL's) in an int or node column: held as `NULL`, read as `Value::Null`, shown `None`; in a text column,
/// shown `None` too.
#[test]
fn a_null_reads_as_none() {
    let mut r = Rows::new(vec![col::NODE, col::INT, col::TEXT]);
    r.push(&[Cell::Int(NULL), Cell::Int(NULL), Cell::Int(NULL)]);
    r.push(&[Cell::Int(1 << 32 | 2), Cell::Int(5), Cell::Text("t")]);
    assert_eq!(r.cell(0, 0), Cell::Int(NULL));
    assert_eq!(r.cell(0, 2), Cell::Text("None"));
    assert_eq!(r.shown(), rows(&[&[&(1i64 << 32 | 2).to_string(), "5", "t"], &["None", "None", "None"]]));
    let mut image = Builder::new(flags::OWN, false);
    image.doc(1, &Doc { path: Some("a.py".into()), first: None });
    image.put("r", &[&r], &[0]);
    let bytes = image.finish();
    let outputs = Outputs::of(&bytes).unwrap();
    let row = outputs.rows("r").into_iter().find(|row| row.get(1) == Value::Null).expect("the row of nulls");
    assert_eq!(row.get(0), Value::Null);
    assert_eq!((row.get(0).int(), row.get(0).node(), row.get(0).path()), (None, None, None));
    assert_eq!(row.get(2), Value::Text("None"));
    assert_eq!(outputs.get("r").unwrap().shown(), rows(&[&["None", "None", "None"], &["a.py:2", "5", "t"]]));
    // A NULL is no place: no place is looked for.
    let image = Image::one(&bytes).unwrap();
    assert_eq!(image.place(NULL), None);
}

#[test]
#[should_panic(expected = "a text")]
fn a_text_in_an_int_column_is_a_fault() {
    let mut r = Rows::new(vec![col::INT]);
    r.push(&[Cell::Text("3")]);
}

#[test]
#[should_panic(expected = "a row of 1 values in rows of 2")]
fn a_row_of_another_width_is_a_fault() {
    let mut r = Rows::new(vec![col::INT, col::INT]);
    r.push(&[Cell::Int(1), Cell::Int(2)]);
    r.push(&[Cell::Int(3)]);
}

/// Rows from their parts (a memo entry's): as they were, a text column's values indexes of its texts; refused where
/// the parts do not make rows.
#[test]
fn rows_of_their_parts() {
    let mut r = Rows::new(vec![col::TEXT, col::NODE, col::INT]);
    r.push(&[Cell::Text("a"), Cell::Int(1 << 32 | 3), Cell::Int(-2)]);
    r.push(&[Cell::Text("b"), Cell::Int(NULL), Cell::Int(7)]);
    r.push(&[Cell::Text("a"), Cell::Int(2 << 32), Cell::Int(0)]);
    let vals: Vec<i64> = (0..r.len()).flat_map(|i| r.row(i).to_vec()).collect();
    let texts: Vec<String> = r.texts().iter().map(|t| t.to_string()).collect();
    assert_eq!(texts, ["a", "b"], "each text once");
    let back = Rows::of_parts(r.types().to_vec(), vals.clone(), texts.clone(), r.len()).unwrap();
    assert_eq!(back, r);
    // A text index out of its texts.
    let mut bad = vals.clone();
    bad[0] = 2;
    assert!(Rows::of_parts(r.types().to_vec(), bad, texts.clone(), r.len()).is_err());
    let mut bad = vals.clone();
    bad[3] = -1;
    assert!(Rows::of_parts(r.types().to_vec(), bad, texts.clone(), r.len()).is_err());
    // Values not of the rows' count.
    assert!(Rows::of_parts(r.types().to_vec(), vals[..8].to_vec(), texts.clone(), r.len()).is_err());
    assert!(Rows::of_parts(Vec::new(), vec![1], Vec::new(), 1).is_err());
    // Rows of no columns: their count.
    let bare = Rows::of_parts(Vec::new(), Vec::new(), Vec::new(), 2).unwrap();
    assert_eq!(bare.len(), 2);
    // Equal rows are of equal types and values in order.
    let mut other = Rows::new(vec![col::TEXT, col::NODE, col::INT]);
    other.append(&r);
    assert_eq!(other, r);
    other.retain(|i, _| i > 0);
    assert_ne!(other, r);
}

/// Rows of no columns, untyped rows, and the edits a run makes of its rows.
#[test]
fn rows_of_every_shape() {
    // No columns: rows counted, one in the image.
    let mut bare = Rows::new(Vec::new());
    bare.push(&[]);
    bare.push(&[]);
    assert_eq!(bare.len(), 2);
    let mut image = Builder::new(0, false);
    image.put("b", &[&bare], &[]);
    image.put("e", &[&Rows::new(Vec::new())], &[]);
    let bytes = image.finish();
    let outputs = Outputs::of(&bytes).unwrap();
    assert_eq!(outputs.get("b").unwrap().len(), 1);
    assert_eq!(outputs.get("b").unwrap().shown(), [Vec::<String>::new()]);
    assert!(outputs.get("e").unwrap().is_empty());
    // Untyped rows take their first row's shape.
    let mut u = Rows::default();
    u.push(&[Cell::Int(5), Cell::Text("t")]);
    assert_eq!(u.types(), [col::INT, col::TEXT]);
    // `retain` and `without_first`.
    let mut s = Rows::new(vec![col::INT, col::INT]);
    for i in 0..6 {
        s.push(&[Cell::Int(i), Cell::Int(i * 10)]);
    }
    s.retain(|_, row| row[0] % 2 == 0);
    assert_eq!(s.shown(), rows(&[&["0", "0"], &["2", "20"], &["4", "40"]]));
    assert_eq!(s.without_first().shown(), [["0"], ["20"], ["40"]]);
    assert_eq!(s.without_first().types(), [col::INT]);
}

#[test]
fn a_directorys_outputs_are_read_from_its_stack() {
    let dir = std::env::temp_dir().join(format!("rerail-output-readers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // No stack: every relation absent.
    assert_eq!(shown(&dir, "a").unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert!(Outputs::open(&dir).unwrap().names().is_empty());
    let mut base = Builder::new(0, false);
    let (x, y) = (i64::from(base.text("x")), i64::from(base.text("y")));
    base.rows("a", &[col::TEXT, col::INT], vec![y, 2, x, 1]);
    base.rows("e", &[col::INT], Vec::new());
    write(&dir, &base.finish()).unwrap();
    assert_eq!(shown(&dir, "a").unwrap(), rows(&[&["x", "1"], &["y", "2"]]));
    assert_eq!(shown(&dir, "e").unwrap(), Vec::<Vec<String>>::new());
    assert_eq!(shown(&dir, "b").unwrap_err().kind(), std::io::ErrorKind::NotFound);
    // Over the stack, an overlay: the rest read through.
    let mut over = Builder::new(0, true);
    let t = i64::from(over.text("v\twith\ttabs"));
    over.rows("b", &[col::TEXT], vec![t]);
    write(&dir, &over.finish()).unwrap();
    assert_eq!(read(&dir).unwrap().len(), 2);
    let outputs = Outputs::open(&dir).unwrap();
    assert_eq!(outputs.names(), ["a", "b", "e"]);
    assert_eq!(outputs.images().len(), 2);
    // Each row's values, typed.
    let mut a: Vec<(&str, i64)> = outputs.rows("a").iter().map(|r| (r.get(0).text().unwrap(), r.get(1).int().unwrap())).collect();
    a.sort();
    assert_eq!(a, [("x", 1), ("y", 2)]);
    assert_eq!(outputs.rows("b")[0].get(0), Value::Text("v\twith\ttabs"));
    assert!(outputs.rows("nowhere").is_empty());
    drop(outputs);
    // A base of some of them; `.csv` files of all (a renderer for scripts, never read back here).
    let base = base_of(&dir, ["b", "none"].into_iter()).unwrap();
    let one = Image::one(&base).unwrap();
    assert!(!one.is_overlay());
    assert_eq!(one.names().collect::<Vec<_>>(), ["b"]);
    let to = dir.join("csv");
    assert_eq!(to_csv(&dir, &to).unwrap(), 3);
    assert!(to.join("a.csv").exists() && to.join("b.csv").exists() && to.join("e.csv").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_overlay_of_rows_gone_and_added_reads_as_the_relation_they_make() {
    // Relation r of rows (text, int), q of one text.
    let image = |r: &[(&str, i64)], q: &[&str]| {
        let mut b = Builder::new(0, false);
        let vals: Vec<i64> = r.iter().flat_map(|&(t, n)| [i64::from(b.text(t)), n]).collect();
        b.rows("r", &[col::TEXT, col::INT], vals);
        let q: Vec<i64> = q.iter().map(|t| i64::from(b.text(t))).collect();
        if !q.is_empty() {
            b.rows("q", &[col::TEXT], q);
        }
        b.finish()
    };
    let base = image(&[("a", 1), ("b", 2), ("c", 3)], &["x"]);
    // The run's new rows, an image of their own: "b" gone, "d" added.
    let made = Image::one(&image(&[("d", 4), ("c", 3), ("a", 1)], &[])).unwrap();
    let (old, new) = (Image::one(&base).unwrap().rel("r").unwrap().hashes(), made.rel("r").unwrap().hashes());
    // The same rows hash the same in both images, their strings held at other indexes.
    assert_eq!(old.iter().filter(|h| new.binary_search(h).is_ok()).count(), 2);
    let r = made.rel("r").unwrap();
    let added: Vec<usize> = (0..r.rows).filter(|&i| old.binary_search(&r.hash(i)).is_err()).collect();
    let gone: Vec<u128> = old.iter().copied().filter(|h| new.binary_search(h).is_err()).collect();
    assert_eq!((added.len(), gone.len()), (1, 1));
    let mut overlay = Builder::new(0, true);
    overlay.added("r", r, &added);
    overlay.gone("r", &gone);
    let mut bytes = stacked(&[&base, &overlay.finish()]);
    let stack = Image::stack(&bytes).unwrap();
    let view = find(&stack, "r").unwrap();
    assert_eq!(shown_in(&bytes, "r"), rows(&[&["a", "1"], &["c", "3"], &["d", "4"]]));
    assert_eq!(view.hashes(), new);
    assert_eq!(shown_in(&bytes, "q"), [["x"]]);
    // A second overlay takes away a row the first added, and one of the base's.
    let gone2: Vec<u128> = view.each().filter(|(r, i)| r.value(*i, 0) != Value::Text("c")).map(|(r, i)| r.hash(i)).collect();
    let mut overlay = Builder::new(0, true);
    overlay.gone("r", &gone2);
    bytes.extend(overlay.finish());
    assert_eq!(shown_in(&bytes, "r"), [["c", "3"]]);
    // Folded, it reads the same.
    let stack = Image::stack(&bytes).unwrap();
    let mut fold = Builder::new(0, false);
    for name in ["q", "r"] {
        fold.take(name, &find(&stack, name).unwrap());
    }
    let folded = fold.finish();
    assert_eq!(Image::stack(&folded).unwrap().len(), 1);
    assert_eq!(shown_in(&folded, "r"), [["c", "3"]]);
    // An entry of rows gone or added is refused in a base.
    let mut bad = Builder::new(0, false);
    bad.gone("r", &gone);
    assert!(Image::one(&bad.finish()).is_err());
}

#[test]
fn a_nodes_hash_is_its_documents_not_its_key() {
    // One document under two keys, in two images: its nodes hash the same; another document's differ.
    let mut a = Builder::new(flags::OWN, false);
    a.doc(3, &Doc { path: Some("m.py".into()), first: None });
    a.rows("n", &[col::NODE], vec![3 << 32 | 7]);
    let mut b = Builder::new(flags::OWN, false);
    b.doc(5, &Doc { path: Some("m.py".into()), first: None });
    b.doc(6, &Doc { path: Some("o.py".into()), first: None });
    b.rows("n", &[col::NODE], vec![5 << 32 | 7, 6 << 32 | 7]);
    let (a_bytes, b_bytes) = (a.finish(), b.finish());
    let (a, b) = (Image::one(&a_bytes).unwrap(), Image::one(&b_bytes).unwrap());
    let (ra, rb) = (a.rel("n").unwrap(), b.rel("n").unwrap());
    assert_eq!(ra.hash(0), rb.hash(0));
    assert_ne!(rb.hash(0), rb.hash(1));
    // Every row's hash at once is each row's.
    assert_eq!(rb.each_hash(), [rb.hash(0), rb.hash(1)]);
    let mut t = Builder::new(0, false);
    let (x, y) = (i64::from(t.text("a")), i64::from(t.text("b")));
    t.rows("t", &[col::TEXT, col::INT], vec![x, 1, y, 2, x, 3]);
    let t = Image::one(&t.finish()).unwrap();
    let rt = t.rel("t").unwrap();
    assert_eq!(rt.each_hash(), (0..3).map(|i| rt.hash(i)).collect::<Vec<_>>());
    // A row's hash through the reader is its relation's.
    let outputs = Outputs::of(&a_bytes).unwrap();
    assert_eq!(outputs.rows("n")[0].hash(), ra.hash(0));
    assert_eq!(outputs.get("n").unwrap().shown(), [["m.py:7"]]);
}
