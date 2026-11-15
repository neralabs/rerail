use super::*;

/// The `.csv` writer's rendering of a run's lines (corpus.rs, "write the outputs", before decision 0239), as the
/// oracle: each node column's value shown by its document, then the lines sorted, each once.
fn writer_lines(lines: &[String], nodes: &[usize], paths: &HashMap<i64, String>, first: &HashMap<i64, i64>, places: &HashMap<i64, String>, own: bool, by_definition: bool) -> Vec<String> {
    let mut decoded: Vec<String> = lines
        .iter()
        .map(|row| {
            row.split('\t')
                .enumerate()
                .map(|(i, v)| match v.parse::<i64>() {
                    Ok(v) if nodes.contains(&i) && v < -1 => places[&v].clone(),
                    Ok(v) if nodes.contains(&i) && v >= 0 && !paths.contains_key(&(v >> 32)) && !first.contains_key(&(v >> 32)) => format!("guest{}:{}", v >> 32, v & 0xffff_ffff),
                    Ok(v) if nodes.contains(&i) && (own || by_definition) && paths.contains_key(&(v >> 32)) => format!("{}:{}", paths[&(v >> 32)], shown_local(v & 0xffff_ffff, by_definition)),
                    Ok(v) if nodes.contains(&i) && !own && first.contains_key(&(v >> 32)) => (first[&(v >> 32)] + (v & 0xffff_ffff)).to_string(),
                    _ => v.to_owned(),
                })
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    decoded.sort();
    decoded.dedup();
    decoded
}

/// An image made from `lines` as the corpus run makes it: each document and place its node columns name registered.
fn image_of(lines: &[String], nodes: &[usize], ints: &[usize], paths: &HashMap<i64, String>, first: &HashMap<i64, i64>, places: &HashMap<i64, String>, f: u64) -> Vec<u8> {
    let mut b = Builder::new(f, false);
    // A node column's value as the writer showed it, one value at a time.
    let shown = |c: usize, v: &str| {
        if nodes.contains(&c) { writer_lines(&[v.to_owned()], &[0], paths, first, places, f & flags::OWN != 0, f & flags::DEFINITION_IDS != 0).pop().unwrap_or_default() } else { v.to_owned() }
    };
    for v in lines.iter().flat_map(|l| l.split('\t').enumerate().filter(|(c, _)| nodes.contains(c)).map(|(_, v)| v)).filter_map(|v| v.parse::<i64>().ok()) {
        if v < -1 {
            b.place(v, &places[&v]);
        } else if v >= 0 {
            let d = v >> 32;
            if paths.contains_key(&d) || first.contains_key(&d) {
                b.doc(d, &Doc { path: paths.get(&d).cloned(), first: first.get(&d).copied() });
            }
        }
    }
    b.lines("r", nodes, ints, lines, &shown);
    b.finish()
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
    assert_eq!(image.rel("empty").unwrap().rows, 0);
    assert_eq!(image.rel("none").unwrap().rows, 0);
    let many = image.rel("many").unwrap();
    assert_eq!(many.rows, 10_000);
    assert!(many.col(0).windows(2).all(|w| w[0] < w[1]), "sorted");
    assert!((0..many.rows).all(|i| many.col(1)[i] == many.col(0)[i] * 3));
    assert!(image.rel("q").is_none());
}

#[test]
fn an_images_lines_are_the_csv_writers() {
    let paths: HashMap<i64, String> = [(1, "a.py".to_owned()), (2, "pkg/b.py".to_owned())].into();
    let first: HashMap<i64, i64> = [(1, 100), (3, 5_000)].into();
    let places: HashMap<i64, String> = [(-2 - (7i64 << 32 | 4), "@place(\"c.py\", 4)".to_owned())].into();
    let place = -2 - (7i64 << 32 | 4);
    let lines: Vec<String> = [
        format!("{}\tcall\t3", 1i64 << 32 | 9),
        format!("{}\tname\t007", 2i64 << 32 | (5 << 26 | 3)),
        format!("{}\tx\t-4", 3i64 << 32 | 2),
        format!("{}\t{{\"json\": [1, 2]}}\t8", 9i64 << 32 | 1),
        format!("-1\t12\t8"),
        format!("{place}\tz\t0"),
        format!("{}\tcall\t3", 1i64 << 32 | 9),
    ]
    .into();
    for f in [0, flags::OWN, flags::DEFINITION_IDS, flags::OWN | flags::DEFINITION_IDS] {
        let want = writer_lines(&lines, &[0], &paths, &first, &places, f & flags::OWN != 0, f & flags::DEFINITION_IDS != 0);
        let stack = Image::stack(&image_of(&lines, &[0], &[2], &paths, &first, &places, f)).unwrap();
        let r = find(&stack, "r").unwrap();
        assert_eq!(r.lines(), want, "flags {f}");
        // "007" is no integer as written: the column is text, and shows as it was.
        assert_eq!(r.types(), [col::NODE, col::TEXT, col::TEXT]);
    }
    // A node column holding a value that is no integer: text, shown as the writer showed each value.
    let odd: Vec<String> = vec![format!("{}\t1", 1i64 << 32 | 2), "None\t2".into()];
    let want = writer_lines(&odd, &[0], &paths, &first, &places, true, true);
    let stack = Image::stack(&image_of(&odd, &[0], &[1], &paths, &first, &places, flags::OWN | flags::DEFINITION_IDS)).unwrap();
    let r = find(&stack, "r").unwrap();
    assert_eq!(r.types(), [col::TEXT, col::INT]);
    assert_eq!(r.lines(), want);
    // Rows of differing widths, and a relation of no columns.
    let ragged: Vec<String> = vec!["a\tb".into(), "c".into()];
    let stack = Image::stack(&image_of(&ragged, &[], &[], &paths, &first, &places, 0)).unwrap();
    assert_eq!(find(&stack, "r").unwrap().lines(), writer_lines(&ragged, &[], &paths, &first, &places, false, false));
    let blank: Vec<String> = vec![String::new(), String::new()];
    let stack = Image::stack(&image_of(&blank, &[], &[], &paths, &first, &places, 0)).unwrap();
    assert_eq!(find(&stack, "r").unwrap().lines(), vec![String::new()]);
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
    assert_eq!(find(&stack, "p").unwrap().lines(), ["b", "c"]);
    assert_eq!(find(&stack, "q").unwrap().lines(), ["1", "2"]);
    // A base written again starts the stack over.
    let mut again = Builder::new(0, false);
    again.rows("q", &[col::INT], vec![3]);
    write(&dir, &again.finish()).unwrap();
    let stack = read(&dir).unwrap();
    assert_eq!(stack.len(), 1);
    assert!(find(&stack, "p").is_none());
    assert!(read(&dir.join("nowhere")).unwrap().is_empty());
}

/// A stack folded into one base (`Builder::take`, each relation from the image that holds it) shows every relation
/// as the stack did: its strings, documents and places copied with it.
#[test]
fn a_folded_stack_shows_what_the_stack_showed() {
    let place = -2 - (3i64 << 32 | 8);
    let mut base = Builder::new(flags::OWN | flags::DEFINITION_IDS, false);
    base.doc(1, &Doc { path: Some("a.py".into()), first: None });
    base.place(place, "@place(\"b.py\", 8)");
    let t = i64::from(base.text("int"));
    base.rows("type_of", &[col::NODE, col::TEXT], vec![1 << 32 | (2 << crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS | 5), t, place, t, 9 << 32 | 1, t]);
    base.rows("gone", &[col::INT], vec![4]);
    let mut over = Builder::new(flags::OWN | flags::DEFINITION_IDS, true);
    let s = i64::from(over.text("str"));
    over.rows("gone", &[col::TEXT], vec![s]);
    let mut bytes = base.finish();
    bytes.extend(over.finish());
    let stack = Image::stack(&bytes).unwrap();
    let mut fold = Builder::new(flags::OWN | flags::DEFINITION_IDS, false);
    for name in ["type_of", "gone"] {
        fold.take(name, find(&stack, name).unwrap());
    }
    let folded = Image::stack(&fold.finish()).unwrap();
    assert_eq!(folded.len(), 1);
    for name in ["type_of", "gone"] {
        assert_eq!(find(&folded, name).unwrap().lines(), find(&stack, name).unwrap().lines(), "{name}");
    }
    assert_eq!(find(&folded, "type_of").unwrap().lines(), ["@place(\"b.py\", 8)\tint", "a.py:2.5\tint", "guest9:1\tint"]);
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

/// A run's rows (`Rows`) are its text outputs as they were (`lines`: each value as it is, a node as its number), and an
/// image made from them (`Builder::put`) shows what the writer showed of those lines.
#[test]
fn rows_are_the_runs_lines_and_put_shows_what_the_writer_showed() {
    let paths: HashMap<i64, String> = [(1, "a.py".to_owned()), (2, "pkg/b.py".to_owned())].into();
    let first: HashMap<i64, i64> = [(1, 100), (3, 5_000)].into();
    let place = -2 - (7i64 << 32 | 4);
    let places: HashMap<i64, String> = [(place, "@place(\"c.py\", 4)".to_owned())].into();
    // As `decoded_outputs` hands them: a node column, a text column, an int column; a text in the int column.
    let mut a = Rows::new(vec![col::NODE, col::TEXT, col::INT]);
    a.push(&[Cell::Int(1 << 32 | 9), Cell::Text("call"), Cell::Int(3)]);
    a.push(&[Cell::Int(3 << 32 | 2), Cell::Text("x\ty"), Cell::Int(-4)]);
    a.push(&[Cell::Int(place), Cell::Text("z"), Cell::Int(0)]);
    a.push(&[Cell::Int(1 << 32 | 9), Cell::Text("call"), Cell::Int(3)]);
    let mut b = Rows::new(vec![col::NODE, col::TEXT, col::INT]);
    b.push(&[Cell::Int(2 << 32 | 1), Cell::Text("name"), Cell::Text("007")]);
    b.push(&[Cell::Int(-1), Cell::Text("12"), Cell::Int(8)]);
    assert_eq!(b.types(), [col::NODE, col::TEXT, col::TEXT], "a text makes the int column text");
    let want_a = vec![format!("{}\tcall\t3", 1i64 << 32 | 9), format!("{}\tx\ty\t-4", 3i64 << 32 | 2), format!("{place}\tz\t0")];
    let mut sorted = want_a.clone();
    sorted.sort();
    assert_eq!(a.lines(), sorted);
    // Lines read back as rows are the same rows.
    assert_eq!(Rows::from_lines(&a.lines(), &[0]).lines(), a.lines());
    let mut all = a.clone();
    all.append(&b);
    let lines = all.lines();
    assert_eq!(lines.len(), 5);
    for f in [0, flags::OWN, flags::DEFINITION_IDS, flags::OWN | flags::DEFINITION_IDS] {
        let (own, by_definition) = (f & flags::OWN != 0, f & flags::DEFINITION_IDS != 0);
        let mut image = Builder::new(f, false);
        for v in [1i64 << 32 | 9, 3 << 32 | 2, 2 << 32 | 1] {
            let d = v >> 32;
            if paths.contains_key(&d) || first.contains_key(&d) {
                image.doc(d, &Doc { path: paths.get(&d).cloned(), first: first.get(&d).copied() });
            }
        }
        image.place(place, &places[&place]);
        let shown = |_: usize, v: &str| writer_lines(&[v.to_owned()], &[0], &paths, &first, &places, own, by_definition).pop().unwrap_or_default();
        image.put("r", &[&a, &b], &[0], &shown);
        let stack = Image::stack(&image.finish()).unwrap();
        // A value holding a tab (`x\ty`) is one value here; the writer split it, but showed only the node column.
        let r = find(&stack, "r").unwrap();
        assert_eq!(r.types(), [col::NODE, col::TEXT, col::TEXT], "flags {f}");
        assert_eq!(r.lines(), writer_lines(&lines, &[0], &paths, &first, &places, own, by_definition), "flags {f}");
    }
}

/// A node column holding a text, rows of differing widths, no columns, and the edits a run makes of its rows.
#[test]
fn rows_of_every_shape() {
    // `None` in a node column: the column is text, each value shown by the writer.
    let mut r = Rows::from_lines(&["4294967298\t1".into(), "None\t2".into()], &[0]);
    assert_eq!(r.types(), [col::TEXT, col::INT]);
    assert_eq!(r.lines(), ["4294967298\t1", "None\t2"]);
    let mut image = Builder::new(flags::OWN, false);
    image.put("r", &[&r], &[0], &|_, v| if v == "None" { v.to_owned() } else { format!("shown {v}") });
    assert_eq!(find(&Image::stack(&image.finish()).unwrap(), "r").unwrap().lines(), ["None\t2", "shown 4294967298\t1"]);
    // A row of another width: every row held as its line.
    r.push(&[Cell::Text("odd")]);
    assert!(r.ragged());
    assert_eq!(r.lines(), ["4294967298\t1", "None\t2", "odd"]);
    let mut image = Builder::new(0, false);
    image.put("r", &[&r], &[], &|_, v| v.to_owned());
    assert_eq!(find(&Image::stack(&image.finish()).unwrap(), "r").unwrap().lines(), ["4294967298\t1", "None\t2", "odd"]);
    // No columns: one row, shown as an empty line.
    let mut bare = Rows::new(Vec::new());
    bare.push(&[]);
    bare.push(&[]);
    assert_eq!((bare.len(), bare.lines()), (2, vec![String::new()]));
    let mut image = Builder::new(0, false);
    image.put("b", &[&bare], &[], &|_, v| v.to_owned());
    image.put("e", &[&Rows::new(Vec::new())], &[], &|_, v| v.to_owned());
    let stack = Image::stack(&image.finish()).unwrap();
    assert_eq!(find(&stack, "b").unwrap().lines(), [""]);
    assert!(find(&stack, "e").unwrap().lines().is_empty());
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
    assert_eq!(s.lines(), ["0\t0", "2\t20", "4\t40"]);
    assert_eq!(s.without_first().lines(), ["0", "20", "40"]);
}
