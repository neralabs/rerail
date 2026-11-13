use super::version::{Edit, Made, Version};
use super::{Extents, Grouped, Image, Parts, R, Strings, decode, encode};
use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Lists, Native, Program, Rule, Term, Value};
use std::sync::Arc;

fn same(a: &Program, b: &Program) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

fn atom(rel: &str, args: Vec<Term>) -> Atom {
    Atom { rel: rel.into(), args: args.into(), neg: false, gated: false }
}

fn v(x: &str) -> Term {
    Term::Var(x.to_owned().into())
}

/// Every part of a program, as `program_text`'s own test has them: names, the least number, strings that need
/// escapes, each literal and aggregate, gated negations, a relation with no rows, and the `.about` parts.
fn every_part() -> Program {
    let mut prog = Program::default();
    prog.decls.insert("r".into(), [("c".into(), "int".into()), ("d".into(), "sym".into())].into());
    prog.decls.insert("nearest".into(), [("x".into(), "int".into())].into());
    prog.inputs = vec!["a".into(), "b".into()];
    prog.outputs = vec!["r".into()];
    prog.facts.insert("k".into(), vec![[Value::Int(i64::MIN), Value::Str("q\"\\\n\u{1}é".into())].into(), [Value::Int(-3), Value::Str(String::new().into())].into()]);
    prog.facts.insert("empty".into(), vec![]);
    let mut gated = atom("done_x", vec![v("x")]);
    gated.neg = true;
    gated.gated = true;
    let mut neg = atom("b", vec![Term::Wild, Term::Top]);
    neg.neg = true;
    prog.rules = vec![
        Rule { head: atom("r", vec![v("x"), Term::Call("cat".into(), [v("top"), Term::Str("s".into())].into())]), body: vec![
            Literal::Atom(atom("a", vec![v("x"), v("count_"), v("min")])),
            Literal::Atom(neg),
            Literal::Atom(gated),
            Literal::Cmp { l: v("x"), op: "<=", r: Term::Int(i64::MIN) },
            Literal::Cmp { l: Term::Call("rx".into(), [v("x"), Term::Str("^a$".into())].into()), op: "=", r: Term::Int(1 << 40) },
        ].into() },
        Rule { head: atom("nearest", vec![v("n")]), body: vec![
            Literal::Agg { var: "n".into(), op: AggOp::Count, over: None, atom: atom("a", vec![v("x"), Term::Wild]) },
            Literal::Agg { var: "m".into(), op: AggOp::Min, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "s".into(), op: AggOp::Sum, over: None, atom: atom("a", vec![v("y")]) },
            Literal::Agg { var: "j".into(), op: AggOp::Join { by: vec!["k".into(), "sep".into()], sep: ", ".into() }, over: Some("y".into()), atom: atom("ast", vec![v("y"), v("k"), v("sep")]) },
            Literal::Agg { var: "j2".into(), op: AggOp::Join { by: vec![], sep: "".into() }, over: Some("y".into()), atom: atom("a", vec![v("y")]) },
            Literal::Nearest { n: "x".into(), a: "p".into(), kindset: "Scope".into() },
            Literal::Ast { document: "d".into(), node: "n".into(), matcher: "{\"kind\": \"call\"}".into(), captures: vec![("NAME".into(), "v".into())] },
        ].into() },
        Rule { head: atom("unit", vec![]), body: vec![].into() },
    ];
    prog.natives.insert("reach_x".into(), Native::Reach { seed: "s".into(), edge: "e".into() });
    prog.kindsets.insert("Scope".into(), vec!["module".into(), "class".into()]);
    prog.demand = vec!["*".into()];
    prog.settings = vec![("mode".into(), "block".into()), ("input".into(), "r(IO=json)".into())];
    prog.nullary.insert("unit".into());
    prog
}

fn pack_files() -> Vec<std::path::PathBuf> {
    let packs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs");
    let mut files: Vec<_> = std::fs::read_dir(&packs).unwrap().flatten().filter(|d| d.path().is_dir()).flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten().map(|f| f.path())).filter(|p| p.extension().is_some_and(|e| e == "rr")).collect();
    files.sort();
    files
}

/// Every seventh pack program, as the text's test samples them.
fn pack_programs() -> Vec<(std::path::PathBuf, Program)> {
    pack_files().into_iter().step_by(7).filter_map(|p| Some((p.clone(), crate::code_rule_dsl::logic::rr::parse_file(&p).ok()?))).collect()
}

/// Every part of a program reads back from its image as written.
#[test]
fn a_program_reads_back_from_its_image() {
    let prog = every_part();
    let bytes = encode(&prog).unwrap();
    same(&decode(&bytes, &Lists::default()).unwrap(), &prog);
    // And as its text reads it.
    let text = crate::code_rule_dsl::logic::program_text::print(&prog).unwrap();
    same(&decode(&bytes, &Lists::default()).unwrap(), &crate::code_rule_dsl::logic::program_text::read(&text).unwrap());
}

/// Every pack program, as read from its file, reads back from its image.
#[test]
fn pack_programs_read_back_from_their_images() {
    let lists = Lists::default();
    let mut n = 0;
    for (path, prog) in pack_programs() {
        let Ok(bytes) = encode(&prog) else { continue };
        let back = decode(&bytes, &lists).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(format!("{back:?}"), format!("{prog:?}"), "{}", path.display());
        n += 1;
    }
    assert!(n > 50, "{n}");
}

/// What the image has no form for is refused, as the text refuses it.
#[test]
fn the_image_refuses_what_it_cannot_keep() {
    let mut prog = every_part();
    prog.rules[0].body.push(Literal::Cmp { l: v("x"), op: "~", r: Term::Int(1) });
    assert!(encode(&prog).is_err());
}

/// A damaged image is refused, never read wrong: bad magic, a short image, a region past the end, a node whose size
/// overruns its rule, a slot past its rule's variables.
#[test]
fn a_damaged_image_is_refused() {
    let bytes = encode(&every_part()).unwrap();
    let lists = Lists::default();
    let mut b = bytes.clone();
    b[0] = b'X';
    assert!(decode(&b, &lists).is_err());
    assert!(decode(&bytes[..bytes.len() / 2], &lists).is_err());
    let mut b = bytes.clone();
    let at = 16 + 24 * R::Val as usize + 16;
    b[at..at + 8].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    assert!(decode(&b, &lists).is_err());
    let image = Image::open(&bytes).unwrap();
    let size_at = image.region(R::Size).0 + 4;
    let mut b = bytes.clone();
    b[size_at..size_at + 4].copy_from_slice(&1000u32.to_le_bytes());
    assert!(decode(&b, &lists).is_err());
    let val_at = image.region(R::Val).0 + 4 * 2;
    let mut b = bytes.clone();
    b[val_at..val_at + 4].copy_from_slice(&999u32.to_le_bytes());
    assert!(decode(&b, &lists).is_err());
}

/// The string table finds each of 200,000 texts and no other, and grows one bucket at a time: no insert rebuilds it.
#[test]
fn the_string_table_grows_one_bucket_at_a_time() {
    let mut s = Strings::default();
    let mut buckets = s.table().buckets.len();
    for i in 0..200_000u32 {
        assert_eq!(s.intern(&format!("name_{i}")), i);
        let now = s.table().buckets.len();
        assert!(now == buckets || now == buckets + 1, "{buckets} → {now}");
        buckets = now;
    }
    for i in (0..200_000u32).step_by(7) {
        assert_eq!(s.find(&format!("name_{i}")), Some(i));
        assert_eq!(s.intern(&format!("name_{i}")), i);
    }
    assert_eq!(s.find("name_200000"), None);
    assert_eq!(s.find(""), None);
    assert!(s.table().count as usize == s.len() && buckets > 30_000, "{buckets}");
}

/// A region of extents never moves what it holds: an item's address is the same after it grows thirty-fold.
#[test]
fn extents_keep_their_items_in_place() {
    let mut e: Extents<u32> = Extents::new(4);
    e.extend(&(0..100).collect::<Vec<_>>());
    let early = e.address(37);
    for i in 100..3000u32 {
        e.push(i);
    }
    e.extend(&(3000..10_000).collect::<Vec<_>>());
    assert_eq!(e.address(37), early);
    assert!((0..10_000).all(|i| e.get(i) == i as u32));
    e.set(5000, 7);
    assert_eq!(e.get(5000), 7);
    assert_eq!(e.extents().map(<[u32]>::len).sum::<usize>(), 10_000);
    assert!(e.extent_count() <= 10, "{}", e.extent_count());
}

/// A grouped index's appended values follow its base, in order, and freeze to the CSR a build would make.
#[test]
fn a_grouped_index_appends_to_its_tails() {
    let mut g = Grouped::of(vec![0, 2, 2, 5], vec![10, 11, 20, 21, 22]);
    for i in 0..40 {
        g.push(1, 100 + i);
    }
    g.push(0, 12);
    g.push(4, 40);
    assert_eq!(g.get(0).collect::<Vec<_>>(), vec![10, 11, 12]);
    assert_eq!(g.get(1).collect::<Vec<_>>(), (100..140).collect::<Vec<_>>());
    assert_eq!(g.get(2).collect::<Vec<_>>(), vec![20, 21, 22]);
    assert_eq!(g.get(3).count(), 0);
    assert_eq!(g.tail_blocks(), 5);
    let (starts, values) = g.freeze(5);
    assert_eq!(starts, vec![0, 3, 43, 46, 46, 47]);
    assert_eq!(values[3..43].to_vec(), (100..140).collect::<Vec<_>>());
}

/// A large pack program, for the versions' tests: the largest pack file that reads and has an image.
fn big() -> Program {
    let mut files = pack_files();
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map_or(0, |m| m.len())));
    files.iter().find_map(|p| crate::code_rule_dsl::logic::rr::parse_file(p).ok().filter(|p| encode(p).is_ok() && p.rules.len() > 40 && p.decls.len() > 2)).expect("a large pack program")
}

fn base(p: &Program) -> Arc<Version> {
    Version::open(&encode(p).unwrap()).unwrap()
}

/// A version's rules by head are the program's.
fn heads_agree(v: &Version, p: &Program, lists: &Lists) {
    let stack = v.stack();
    for rel in p.rules.iter().map(|r| r.head.rel.as_str()).collect::<std::collections::BTreeSet<_>>() {
        let mut got = Vec::new();
        let mut readers: Vec<_> = stack.iter().map(|im| super::Reader::new(im, lists)).collect();
        let mut at = Vec::new();
        v.by_head(rel, &mut |i, k, _| at.push((i, k)));
        for (i, k) in at {
            got.push(format!("{:?}", readers[i].rule(k as usize).unwrap()));
        }
        let want: Vec<String> = p.rules.iter().filter(|r| r.head.rel.as_str() == rel).map(|r| format!("{r:?}")).collect();
        assert_eq!(got, want, "{rel}");
    }
}

/// A view keeps a base's rules in ranges; an overlay adds, removes and replaces parts; three stacked are the program
/// they say, by rule and by head, and their digests are the digest of that program made whole.
#[test]
fn views_and_overlays_are_the_programs_they_say() {
    let lists = Lists::default();
    let p0 = big();
    let v0 = base(&p0);
    // A view: the first and last thirds.
    let n = p0.rules.len();
    let removed: Vec<u32> = (n as u32 / 3..2 * n as u32 / 3).collect();
    let (v1, made) = Version::edit(&v0, Edit { removed: removed.clone(), ..Edit::default() }, &lists).unwrap();
    assert_eq!(made, Made::View);
    let mut p1 = p0.clone();
    p1.rules = p0.rules.iter().enumerate().filter(|(k, _)| !removed.contains(&(*k as u32))).map(|(_, r)| r.clone()).collect();
    same(&v1.program(&lists).unwrap(), &p1);
    heads_agree(&v1, &p1, &lists);
    // An overlay on the view: its delta is the second image of a stack whose first is the view's base.
    let mut p1b = p1.clone();
    p1b.rules.remove(0);
    p1b.rules.push(Rule { head: atom("on_the_view", vec![v("x")]), body: vec![Literal::Atom(atom(&p0.rules[0].head.rel, vec![v("x")]))].into() });
    let mut about = crate::code_rule_dsl::logic::program_facts::Facts::new();
    crate::code_rule_dsl::logic::program_facts::write_about(&mut about, "a", &p1b);
    let (v1b, made) = Version::edit(&v1, v1.diff(&p1b, &about, &lists).unwrap(), &lists).unwrap();
    assert_eq!((made, v1b.depth(), v1b.stack().len()), (Made::Overlay, 1, 2));
    same(&v1b.program(&lists).unwrap(), &p1b);
    heads_agree(&v1b, &p1b, &lists);
    assert_eq!(v1b.digest(&lists).unwrap(), base(&p1b).digest(&lists).unwrap());
    // An overlay on the base: two rules out, one in, a declaration changed, one gone, a fact relation added, the
    // inputs changed.
    let new_rule = Rule { head: atom("made_here", vec![v("x")]), body: vec![Literal::Atom(atom(&p0.rules[0].head.rel, vec![v("x")]))].into() };
    let mut p2 = p0.clone();
    p2.rules.remove(5);
    p2.rules.remove(1);
    p2.rules.push(new_rule.clone());
    let (d_changed, d_gone) = { let mut k = p0.decls.keys(); (k.next().unwrap().clone(), k.next().unwrap().clone()) };
    p2.decls.insert(d_changed.clone(), [("only".into(), "int".into())].into());
    p2.decls.remove(&d_gone);
    p2.decls.insert("made_here".into(), [("x".into(), "int".into())].into());
    p2.facts.insert("made_facts".into(), vec![[Value::Int(7)].into()]);
    p2.inputs.push("one_more".into());
    let mut about = crate::code_rule_dsl::logic::program_facts::Facts::new();
    crate::code_rule_dsl::logic::program_facts::write_about(&mut about, "a", &p2);
    let edit = v0.diff(&p2, &about, &lists).unwrap();
    assert_eq!((edit.removed.len(), edit.added.len(), edit.parts.decls.len(), edit.parts.decls_gone.len()), (2, 1, 2, 1));
    let (v2, made) = Version::edit(&v0, edit, &lists).unwrap();
    assert_eq!(made, Made::Overlay);
    same(&v2.program(&lists).unwrap(), &p2);
    heads_agree(&v2, &p2, &lists);
    assert_eq!(v2.digest(&lists).unwrap(), base(&p2).digest(&lists).unwrap());
    // Two more overlays on it: a stack three deep.
    let mut p3 = p2.clone();
    p3.rules.truncate(p3.rules.len() - 3);
    p3.facts.remove("made_facts");
    p3.outputs.push("made_here".into());
    let mut about = crate::code_rule_dsl::logic::program_facts::Facts::new();
    crate::code_rule_dsl::logic::program_facts::write_about(&mut about, "a", &p3);
    let (v3, made) = Version::edit(&v2, v2.diff(&p3, &about, &lists).unwrap(), &lists).unwrap();
    assert_eq!(made, Made::Overlay);
    let mut p4 = p3.clone();
    p4.rules.insert(0, new_rule.clone());
    let mut about = crate::code_rule_dsl::logic::program_facts::Facts::new();
    crate::code_rule_dsl::logic::program_facts::write_about(&mut about, "a", &p4);
    let (v4, _) = Version::edit(&v3, v3.diff(&p4, &about, &lists).unwrap(), &lists).unwrap();
    assert_eq!(v4.depth(), 3);
    // The added rule goes last in an overlay: its order is the version's, and the program is the same up to it.
    let mut p4_order = p3.clone();
    p4_order.rules.push(new_rule);
    same(&v4.program(&lists).unwrap(), &p4_order);
    heads_agree(&v4, &p4_order, &lists);
    assert_eq!(v4.digest(&lists).unwrap(), base(&p4_order).digest(&lists).unwrap());
    // The base is unchanged by all of it.
    same(&v0.program(&lists).unwrap(), &p0);
}

/// An edit that reaches half its parent is folded into a new base, which is the program.
#[test]
fn an_edit_of_half_its_parent_folds() {
    let lists = Lists::default();
    let p0 = big();
    let v0 = base(&p0);
    let mut p1 = p0.clone();
    p1.rules.truncate(p0.rules.len() / 3);
    p1.inputs.push("x".into());
    let mut about = crate::code_rule_dsl::logic::program_facts::Facts::new();
    crate::code_rule_dsl::logic::program_facts::write_about(&mut about, "a", &p1);
    let (v1, made) = Version::edit(&v0, v0.diff(&p1, &about, &lists).unwrap(), &lists).unwrap();
    assert_eq!(made, Made::Folded);
    assert_eq!(v1.depth(), 0);
    same(&v1.program(&lists).unwrap(), &p1);
}

/// An overlay's delta is the size of its edit, not of its parent: one rule added to a program ten times larger
/// (its rules, nine times over) makes a delta no larger.
#[test]
fn a_delta_is_the_size_of_its_edit() {
    let lists = Lists::default();
    let small = big();
    let mut large = small.clone();
    for k in 0..9 {
        large.rules.extend(small.rules.iter().map(|r| {
            let mut r = r.clone();
            r.head.rel = format!("{}_{k}", r.head.rel).into();
            r
        }));
    }
    let rule = Rule { head: atom("one", vec![v("x")]), body: vec![Literal::Atom(atom("two", vec![v("x")]))].into() };
    let mut sizes = Vec::new();
    for p in [&small, &large] {
        let parent = base(p);
        let (v, made) = Version::edit(&parent, Edit { added: vec![&rule], ..Edit::default() }, &lists).unwrap();
        assert_eq!(made, Made::Overlay);
        let Version::Overlay { delta, .. } = &*v else { unreachable!() };
        sizes.push((encode(p).unwrap().len(), delta.bytes().len()));
    }
    assert!(sizes[1].0 > 5 * sizes[0].0, "{sizes:?}");
    assert_eq!(sizes[0].1, sizes[1].1, "{sizes:?}");
    assert!(sizes[1].1 < 2048, "{sizes:?}");
    let _ = Parts::default();
}
