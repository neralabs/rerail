use super::version::{Made, Version};
use super::{AtomId, Builder, CmpOp, Extents, Grouped, Image, Lit, R, RelId, RuleId, Strings, T, TermRef, Tree, decode, encode};
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

/// Every pack program, as read from its file, reads back from its image; and its handles say what its tree says:
/// each rule's head, its literals, its variables' names, rules by head and the atoms that read each relation.
#[test]
fn pack_programs_read_back_from_their_images() {
    let lists = Lists::default();
    let mut n = 0;
    for (path, prog) in pack_programs() {
        let Ok(bytes) = encode(&prog) else { continue };
        let back = decode(&bytes, &lists).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(format!("{back:?}"), format!("{prog:?}"), "{}", path.display());
        handles_agree(&Version::open(&bytes).unwrap(), &prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
}

/// A version's handles agree with `p`, its program as the tree: rule by rule, by head, and by reader.
fn handles_agree(v: &Version, p: &Program) {
    let rules: Vec<_> = v.rules().collect();
    assert_eq!(rules.len(), p.rules.len());
    for (r, t) in rules.iter().zip(&p.rules) {
        assert_eq!(v.rel_name(r.head().rel()), t.head.rel.as_str());
        assert_eq!(r.head().arity(), t.head.args.len());
        assert_eq!(r.body().count(), t.body.len());
        for (l, lt) in r.body().zip(t.body.iter()) {
            match (l.view(), lt) {
                (Lit::Atom(a), Literal::Atom(at)) => {
                    assert_eq!((v.rel_name(a.rel()), a.neg(), a.gated(), a.arity()), (at.rel.as_str(), at.neg, at.gated, at.args.len()));
                    for (x, xt) in a.args().zip(at.args.iter()) {
                        if let (TermRef::Var(s), Term::Var(name)) = (x, xt) {
                            assert_eq!(v.text(r.var(s)), name.as_str());
                        }
                    }
                }
                (Lit::Cmp { op, .. }, Literal::Cmp { op: ot, .. }) => assert_eq!(op.as_str(), *ot),
                (Lit::Agg { var, atom: a, .. }, Literal::Agg { var: vt, atom: at, .. }) => assert_eq!((v.text(r.var(var)), v.rel_name(a.rel())), (vt.as_str(), at.rel.as_str())),
                (Lit::Nearest { kindset, .. }, Literal::Nearest { kindset: kt, .. }) => assert_eq!(v.text(kindset), kt),
                (Lit::Ast { matcher, .. }, Literal::Ast { matcher: mt, .. }) => assert_eq!(v.text(matcher), mt),
                _ => panic!("literal kinds differ"),
            }
        }
    }
    // By head, and by reader, each rule as `rules` gives it.
    let order: std::collections::HashMap<RuleId, usize> = rules.iter().enumerate().map(|(k, r)| (r.id(), k)).collect();
    for rel in p.rules.iter().map(|r| r.head.rel.as_str()).chain(p.rules.iter().flat_map(|r| r.body.iter()).filter_map(|l| match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.as_str()),
        _ => None,
    })) {
        let r = v.rel(rel).unwrap_or_else(|| panic!("relation {rel}"));
        let got: Vec<usize> = v.heads(r).map(|x| order[&x.id()]).collect();
        let want: Vec<usize> = p.rules.iter().enumerate().filter(|(_, x)| x.head.rel.as_str() == rel).map(|(k, _)| k).collect();
        assert_eq!(got, want, "heads of {rel}");
        let got: Vec<usize> = v.readers(r).map(|(x, a)| {
            assert_eq!(a.rel(), r);
            order[&x.id()]
        }).collect();
        let want: Vec<usize> = p.rules.iter().enumerate().flat_map(|(k, x)| x.body.iter().filter(move |l| matches!(l, Literal::Atom(a) | Literal::Agg { atom: a, .. } if a.rel.as_str() == rel)).map(move |_| k)).collect();
        assert_eq!(got, want, "readers of {rel}");
    }
}

/// What the image has no form for is refused, as the text refuses it.
#[test]
fn the_image_refuses_what_it_cannot_keep() {
    let mut prog = every_part();
    prog.rules[0].body.push(Literal::Cmp { l: v("x"), op: "~", r: Term::Int(1) });
    assert!(encode(&prog).is_err());
}

/// A damaged image is refused at open, never read wrong: bad magic, a short image, a region past the end, a node
/// whose size overruns its rule, a slot past its rule's variables, a node of no kind.
#[test]
fn a_damaged_image_is_refused() {
    let bytes = encode(&every_part()).unwrap();
    let mut b = bytes.clone();
    b[0] = b'X';
    assert!(Image::open(&b).is_err());
    assert!(Image::open(&bytes[..bytes.len() / 2]).is_err());
    let mut b = bytes.clone();
    let at = 16 + 24 * R::Val as usize + 16;
    b[at..at + 8].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
    assert!(Image::open(&b).is_err());
    let image = Image::open(&bytes).unwrap();
    for (r, at, value) in [(R::Size, 1, 1000u32), (R::Val, 2, 999), (R::Size, 0, 3)] {
        let mut b = bytes.clone();
        let o = image.region(r).0 + 4 * at;
        b[o..o + 4].copy_from_slice(&value.to_le_bytes());
        assert!(Image::open(&b).is_err(), "{r:?} {at}");
    }
    let mut b = bytes.clone();
    b[image.region(R::Kind).0 + 3] = 0x1f;
    assert!(Image::open(&b).is_err());
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

/// A region of extents never moves what it holds: an item's address is the same after it grows thirty-fold, and
/// after what follows it is cut.
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
    e.truncate(40);
    assert_eq!((e.len(), e.address(37)), (40, early));
    e.extend(&(40..5000).collect::<Vec<_>>());
    assert!((0..5000).all(|i| e.get(i) == i as u32));
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

/// `to` made from `from` by `diff`: the version, and what form it took.
fn edit(from: &Arc<Version>, to: &Program) -> (Arc<Version>, Made) {
    from.diff(to).unwrap().finish().unwrap()
}

/// A view keeps a base's rules in ranges; an overlay adds, removes and replaces parts; three stacked are the program
/// they say, by rule, by head and by reader, and their digests are the digest of that program made whole.
#[test]
fn views_and_overlays_are_the_programs_they_say() {
    let lists = Lists::default();
    let p0 = big();
    let v0 = base(&p0);
    // A view: the first and last thirds.
    let n = p0.rules.len();
    let removed: Vec<u32> = (n as u32 / 3..2 * n as u32 / 3).collect();
    let mut b = v0.builder();
    for &g in &removed {
        b.remove(RuleId(g));
    }
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::View);
    let mut p1 = p0.clone();
    p1.rules = p0.rules.iter().enumerate().filter(|(k, _)| !removed.contains(&(*k as u32))).map(|(_, r)| r.clone()).collect();
    same(&v1.program(&lists).unwrap(), &p1);
    handles_agree(&v1, &p1);
    assert_eq!(v1.digest(), base(&p1).digest());
    // A view of the view: one more rule out.
    let mut b = v1.builder();
    b.remove(RuleId(0));
    let (v1v, made) = b.finish().unwrap();
    assert_eq!(made, Made::View);
    let mut p1v = p1.clone();
    p1v.rules.remove(0);
    same(&v1v.program(&lists).unwrap(), &p1v);
    assert_eq!(v1v.digest(), base(&p1v).digest());
    // An overlay on the view: its delta over the view's base.
    let mut p1b = p1.clone();
    p1b.rules.remove(0);
    p1b.rules.push(Rule { head: atom("on_the_view", vec![v("x")]), body: vec![Literal::Atom(atom(&p0.rules[0].head.rel, vec![v("x")]))].into() });
    let (v1b, made) = edit(&v1, &p1b);
    assert_eq!((made, v1b.depth()), (Made::Overlay, 1));
    same(&v1b.program(&lists).unwrap(), &p1b);
    handles_agree(&v1b, &p1b);
    assert_eq!(v1b.digest(), base(&p1b).digest());
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
    let b = v0.diff(&p2).unwrap();
    assert_eq!(b.size(), 2 + 1 + 2 + 1 + 1);
    let (v2, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    same(&v2.program(&lists).unwrap(), &p2);
    handles_agree(&v2, &p2);
    assert_eq!(v2.digest(), base(&p2).digest());
    // Two more overlays on it: a stack three deep.
    let mut p3 = p2.clone();
    p3.rules.truncate(p3.rules.len() - 3);
    p3.facts.remove("made_facts");
    p3.outputs.push("made_here".into());
    let (v3, made) = edit(&v2, &p3);
    assert_eq!(made, Made::Overlay);
    let mut p4 = p3.clone();
    p4.rules.insert(0, new_rule.clone());
    let (v4, _) = edit(&v3, &p4);
    assert_eq!(v4.depth(), 3);
    // The added rule goes last in an overlay: its order is the version's, and the program is the same up to it.
    let mut p4_order = p3.clone();
    p4_order.rules.push(new_rule);
    same(&v4.program(&lists).unwrap(), &p4_order);
    handles_agree(&v4, &p4_order);
    assert_eq!(v4.digest(), base(&p4_order).digest());
    // The base is unchanged by all of it.
    same(&v0.program(&lists).unwrap(), &p0);
}

/// Ids are the stack's: a delta's atoms, relations and rules follow its parent's, a parent's atom it makes a
/// relation keeps its id, every id of the parent means the same in the overlay, and a fold keeps them all but the
/// rules'.
#[test]
fn ids_are_the_same_through_a_stack_and_a_fold() {
    let mut p0 = every_part();
    // Enough rules that two more are an overlay, not a fold.
    p0.rules.extend((0..8).map(|k| Rule { head: atom(&format!("pad_{k}"), vec![v("x")]), body: vec![Literal::Atom(atom("a", vec![v("x")]))].into() }));
    let v0 = base(&p0);
    let mut b = v0.builder();
    // "^a$" is a string constant of the base (no relation: a native's seed `s` is one); here it becomes a relation.
    let s = v0.find("^a$").unwrap();
    assert_eq!(v0.rel_of(s), None);
    assert!(v0.rel("s").is_some(), "a relation an about row names has an id");
    b.rule().head("^a$", &[T::Var("x")]).atom("a", &[T::Var("x"), T::Wild, T::Wild]).finish().unwrap();
    b.rule().head("brand_new", &[T::Str("never seen")]).finish().unwrap();
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    assert_eq!(v1.find("^a$"), Some(s));
    let rs = v1.rel_of(s).unwrap();
    assert_eq!(rs.0, v0.rel_end());
    let new = v1.rel("brand_new").unwrap();
    assert_eq!(new.0, v0.rel_end() + 1);
    assert!(v1.find("never seen").unwrap().0 >= v0.atom_end());
    for a in 0..v0.atom_end() {
        assert_eq!(v1.text(AtomId(a)), v0.text(AtomId(a)));
    }
    for r in 0..v0.rel_end() {
        assert_eq!(v1.rel_name(RelId(r)), v0.rel_name(RelId(r)));
    }
    let ids: Vec<RuleId> = v1.rules().map(|r| r.id()).collect();
    assert_eq!(ids, (0..v0.rule_end() + 2).map(RuleId).collect::<Vec<_>>());
    // Folded: the same atoms and relations, by the same ids; the same program and digest.
    let f = v1.fold().unwrap();
    assert_eq!((f.depth(), f.digest()), (0, v1.digest()));
    for a in 0..v1.atom_end() {
        assert_eq!(f.text(AtomId(a)), v1.text(AtomId(a)));
    }
    for r in 0..v1.rel_end() {
        assert_eq!(f.rel_name(RelId(r)), v1.rel_name(RelId(r)));
    }
    let lists = Lists::default();
    same(&f.program(&lists).unwrap(), &v1.program(&lists).unwrap());
}

/// A rule written term by term is the rule the tree's encoder writes, hash for hash; and a rule copied from a version
/// of other ids (by its texts), or of the same (by its nodes), is the rule it was, as are its copied literals.
#[test]
fn the_builder_writes_what_the_tree_writes() {
    let lists = Lists::default();
    let p0 = every_part();
    let v0 = base(&p0);
    let rule = Rule { head: atom("p", vec![v("x"), Term::Int(1 << 40)]), body: vec![
        Literal::Atom(atom("a", vec![v("x"), Term::Wild, Term::Call("f".into(), [Term::Str("z".into())].into())])),
        Literal::Atom(Atom { neg: true, ..atom("b", vec![v("x"), Term::Top]) }),
        Literal::Cmp { l: v("x"), op: "<", r: Term::Int(3) },
    ].into() };
    let mut by_tree = v0.builder();
    let tree_id = by_tree.add(&rule).unwrap();
    let mut by_terms = v0.builder();
    let terms_id = by_terms.rule().head("p", &[T::Var("x"), T::Int(1 << 40)])
        .atom("a", &[T::Var("x"), T::Wild, T::Call("f", vec![T::Str("z")])])
        .not("b", &[T::Var("x"), T::Top])
        .cmp(T::Var("x"), CmpOp::Lt, T::Int(3))
        .finish().unwrap();
    assert_eq!(tree_id, terms_id);
    let (a, _) = by_tree.finish().unwrap();
    let (b, _) = by_terms.finish().unwrap();
    assert_eq!(a.rule(tree_id).unwrap().hash(), b.rule(terms_id).unwrap().hash());
    assert_eq!(a.digest(), b.digest());
    same(&a.program(&lists).unwrap(), &b.program(&lists).unwrap());
    // Copied into a version of other ids: a new base, with none of `a`'s names.
    let mut other = Builder::base();
    let r = a.rule(tree_id).unwrap();
    let copied = other.copy(&a, r);
    let lit = r.body().nth(1).unwrap();
    let head = r.head();
    let parts = other.rule().head_from(&a, r, head).lit(&a, r, lit).finish().unwrap();
    let (o, _) = other.finish().unwrap();
    assert_eq!(o.rule(copied).unwrap().hash(), r.hash());
    let mut tree = Tree::new(&o, &lists);
    assert_eq!(format!("{:?}", tree.rule(o.rule(copied).unwrap())), format!("{rule:?}"));
    let want = Rule { head: rule.head.clone(), body: vec![rule.body[1].clone()].into() };
    assert_eq!(format!("{:?}", tree.rule(o.rule(parts).unwrap())), format!("{want:?}"));
    // Copied within its stack: the same nodes.
    let mut again = a.builder();
    let id = again.copy(&a, a.rule(tree_id).unwrap());
    let (c, _) = again.finish().unwrap();
    assert_eq!(c.rule(id).unwrap().hash(), r.hash());
    // A body before its head, or no head, is refused.
    let mut bad = v0.builder();
    assert!(bad.rule().atom("a", &[]).head("p", &[]).finish().is_err());
    assert!(bad.rule().finish().is_err());
    drop(bad.rule().head("unfinished", &[]));
    let ok = bad.rule().head("kept", &[]).finish().unwrap();
    assert_eq!(ok, RuleId(v0.rule_end()));
    let (kept, _) = bad.finish().unwrap();
    assert_eq!(kept.size(), v0.size() + 1);
    same(&kept.program(&lists).unwrap(), &{
        let mut p = p0.clone();
        p.rules.push(Rule { head: atom("kept", vec![]), body: vec![].into() });
        p
    });
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
    let (v1, made) = edit(&v0, &p1);
    assert_eq!(made, Made::Folded);
    assert_eq!(v1.depth(), 0);
    same(&v1.program(&lists).unwrap(), &p1);
    assert_eq!(v1.digest(), base(&p1).digest());
}

/// An overlay's delta is the size of its edit, not of its parent: one rule added to a program ten times larger
/// (its rules, nine times over) makes a delta no larger.
#[test]
fn a_delta_is_the_size_of_its_edit() {
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
        let mut b = parent.builder();
        b.add(&rule).unwrap();
        let (v, made) = b.finish().unwrap();
        assert_eq!(made, Made::Overlay);
        let Version::Overlay { delta, .. } = &*v else { unreachable!() };
        sizes.push((encode(p).unwrap().len(), delta.bytes().len()));
    }
    assert!(sizes[1].0 > 5 * sizes[0].0, "{sizes:?}");
    assert_eq!(sizes[0].1, sizes[1].1, "{sizes:?}");
    assert!(sizes[1].1 < 2048, "{sizes:?}");
}

/// The analyses over `v` say what their tree forms say of `p`, `v`'s program: the strata (as sets, the version's
/// dependencies first), the edges, what the outputs need, the projections, what answers produce, and whether the
/// composed path accepts it.
fn analyses_agree(v: &Version, p: &Program) {
    use super::{About, analyses};
    use crate::code_rule_dsl::logic::front;
    use std::collections::{BTreeMap, BTreeSet};
    let about = About::of(v);
    let name = |r: RelId| v.rel_name(r).to_owned();
    match (front::strata(p), analyses::strata(v, &about)) {
        (Ok((sccs, edges)), Ok(s)) => {
            let want: BTreeSet<BTreeSet<String>> = sccs.into_iter().collect();
            let got: BTreeSet<BTreeSet<String>> = s.sccs.iter().map(|c| c.iter().map(|&r| name(r)).collect()).collect();
            assert_eq!(got, want, "strata");
            let at: std::collections::HashMap<RelId, usize> = s.sccs.iter().enumerate().flat_map(|(i, c)| c.iter().map(move |&r| (r, i))).collect();
            for (h, w, _) in s.edges.iter() {
                if let (Some(a), Some(b)) = (at.get(&h), at.get(&w)) {
                    assert!(b <= a, "{} before {}", name(w), name(h));
                }
            }
            let mut got: front::Edges = BTreeMap::new();
            for (h, w, neg) in s.edges.iter() {
                got.entry(name(h)).or_default().insert((name(w), neg));
            }
            let want: front::Edges = edges.into_iter().filter(|(_, e)| !e.is_empty()).collect();
            assert_eq!(got, want, "edges");
        }
        (Err(a), Err(b)) => assert_eq!(std::mem::discriminant(&a), std::mem::discriminant(&b), "{a} / {b}"),
        (a, b) => panic!("strata: tree {:?}, version {:?}", a.map(|_| ()), b.map(|_| ())),
    }
    let want = front::needed(p, p.outputs.clone());
    let need = analyses::needed(v, &about, v.outputs().map(|o| v.rel_of(o).expect("an output is a relation")));
    let got: BTreeSet<String> = (0..v.rel_end()).map(RelId).filter(|r| need[r.index()]).map(name).collect();
    assert_eq!(got, want, "needed");
    let want: BTreeMap<String, Vec<(usize, i64)>> = crate::code_rule_dsl::logic::plan::projections(p).into_iter().collect();
    let got: BTreeMap<String, Vec<(usize, i64)>> = analyses::projections(v).into_iter().map(|(r, e)| (name(r), e)).collect();
    assert_eq!(got, want, "projections");
    let want: Vec<(String, String)> = p.answers.iter().flat_map(|(req, ps)| ps.iter().map(move |x| (x.clone(), req.clone()))).collect();
    let got: Vec<(String, String)> = about.produced().map(|(x, req)| (name(x), name(req))).collect();
    assert_eq!(got, want, "produced");
    let produced: BTreeMap<String, String> = crate::code_rule_dsl::logic::answers::produced(p).into_iter().map(|(a, b)| (a.to_owned(), b.to_owned())).collect();
    assert_eq!(got.into_iter().collect::<BTreeMap<_, _>>(), produced, "produced, by relation");
    let want = crate::code_rule_dsl::logic::plan::compose::accepts(p).ok();
    let got = analyses::accepts(v, &about).ok();
    assert_eq!(got, want, "accepts");
}

/// A program with what the analyses read beside rules: answers, a lattice its aggregate is monotone in, a native,
/// projections with constants, and a stratum through a gated negation.
fn analysed() -> Program {
    let mut p = Program::default();
    let neg = |mut a: Atom| { a.neg = true; a };
    let mut gated = neg(atom("done_q", vec![v("x")]));
    gated.gated = true;
    p.rules = vec![
        Rule { head: atom("e", vec![v("x"), v("y")]), body: vec![Literal::Atom(atom("in", vec![v("x"), v("y"), Term::Int(3)]))].into() },
        Rule { head: atom("p", vec![v("x")]), body: vec![Literal::Atom(atom("e", vec![v("x"), Term::Wild]))].into() },
        Rule { head: atom("p", vec![v("y")]), body: vec![Literal::Atom(atom("p", vec![v("x")])), Literal::Atom(atom("e", vec![v("x"), v("y")]))].into() },
        Rule { head: atom("q", vec![v("x")]), body: vec![Literal::Atom(atom("p", vec![v("x")])), Literal::Atom(gated)].into() },
        Rule { head: atom("best", vec![v("x"), v("n")]), body: vec![Literal::Atom(atom("p", vec![v("x")])), Literal::Agg { var: "n".into(), op: AggOp::Max, over: Some("y".into()), atom: atom("best", vec![v("x"), v("y")]) }].into() },
        Rule { head: atom("lone", vec![v("x")]), body: vec![Literal::Atom(neg(atom("q", vec![v("x")]))), Literal::Atom(atom("p", vec![v("x")]))].into() },
        Rule { head: atom("ask", vec![v("x")]), body: vec![Literal::Atom(atom("lone", vec![v("x")]))].into() },
        Rule { head: atom("out", vec![v("x")]), body: vec![Literal::Atom(atom("told", vec![v("x")])), Literal::Atom(atom("r", vec![v("x")]))].into() },
    ];
    p.answers.insert("ask".into(), vec!["told".into()]);
    p.lattice.insert("best".into(), crate::code_rule_dsl::logic::front::LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None });
    p.natives.insert("r".into(), Native::Reach { seed: "lone".into(), edge: "e".into() });
    p.rules.push(Rule { head: atom("r", vec![v("x")]), body: vec![Literal::Atom(atom("seed", vec![v("x")]))].into() });
    // Padding, so that a small edit is an overlay or a view, not a fold.
    for k in 0..24 {
        p.rules.push(Rule { head: atom("pad", vec![v("x")]), body: vec![Literal::Atom(atom("in", vec![v("x"), v("y"), Term::Int(k)]))].into() });
    }
    p.inputs = vec!["in".into(), "seed".into()];
    p.outputs = vec!["out".into(), "best".into(), "pad".into()];
    p
}

/// The analyses over a version agree with their tree forms: on the pack programs, on a program with answers,
/// lattices and natives, on one that is not stratifiable, and on versions of a stack, whose ids are not a base's.
#[test]
fn the_analyses_over_a_version_say_what_the_tree_says() {
    let mut n = 0;
    for (_, prog) in pack_programs() {
        let Ok(bytes) = encode(&prog) else { continue };
        analyses_agree(&Version::open(&bytes).unwrap(), &prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
    let p = analysed();
    let b = base(&p);
    analyses_agree(&b, &p);
    analyses_agree(&base(&every_part()), &every_part());
    // Not stratifiable: `lone` read negatively in its own component.
    let mut bad = p.clone();
    bad.rules.push(Rule { head: atom("q", vec![v("x")]), body: vec![Literal::Atom(atom("lone", vec![v("x")]))].into() });
    analyses_agree(&base(&bad), &bad);
    // An overlay that adds a relation and a native, and a view that drops one.
    let mut more = p.clone();
    more.rules.push(Rule { head: atom("extra", vec![v("x")]), body: vec![Literal::Atom(atom("out", vec![v("x")]))].into() });
    more.natives.insert("far".into(), Native::Reach { seed: "extra".into(), edge: "e".into() });
    more.outputs.push("far".into());
    let (o, made) = edit(&b, &more);
    assert_eq!(made, Made::Overlay);
    analyses_agree(&o, &more);
    let mut less = p.clone();
    less.rules.retain(|r| r.head.rel.as_str() != "lone");
    let (w, made) = edit(&b, &less);
    assert_eq!(made, Made::View);
    analyses_agree(&w, &less);
    // The composed path's own kind of program.
    let plain = Program { rules: p.rules[..3].to_vec(), inputs: vec!["in".into()], outputs: vec!["p".into()], ..Program::default() };
    analyses_agree(&base(&plain), &plain);
    assert_eq!(super::analyses::accepts(&base(&plain), &super::About::of(&base(&plain))).ok(), Some(true));
}

/// The passes over a version make what the tree passes make of `p`: `normalize` (as a program, by digest, and its
/// sentinels), each derived rule's source, then `stated_apart` over the normalized version. The analyses agree on
/// what they make.
fn passes_agree(p: &Program) {
    use super::passes;
    use crate::code_rule_dsl::logic::plan;
    let lists = Lists::default();
    let v = base(p);
    let (tree, sentinels) = match (plan::sql_kernel::normalize(p), passes::normalize(&v)) {
        (Ok(t), Ok(g)) => (t, g),
        (Err(_), Err(_)) => return,
        (a, b) => panic!("normalize: tree {:?}, version {:?}", a.map(|_| ()).map_err(|e| e.to_string()), b.map(|_| ())),
    };
    let ((tp, ts), (nv, ns)) = (tree, sentinels);
    same(&nv.program(&lists).unwrap(), &tp);
    assert_eq!(nv.digest(), base(&tp).digest(), "normalized digest");
    let names: std::collections::BTreeSet<String> = ns.iter().map(|&r| nv.rel_name(r).to_owned()).collect();
    assert_eq!(names, ts, "sentinels");
    // Every derived rule came from one of `v`'s; each of `v`'s gave one rule, or a fact (a ground rule).
    for r in nv.rules() {
        let s = r.source().expect("a derived rule's source");
        assert_eq!(v.rel_name(v.rule(s).expect("its source is the input's").head().rel()), nv.rel_name(r.head().rel()));
    }
    for r in v.rules() {
        let made = nv.derived(r.id()).count();
        assert!(made == 1 || (made == 0 && r.body().next().is_none()), "{} made {made}", v.rel_name(r.head().rel()));
    }
    let mut tp2 = tp.clone();
    let pairs = plan::stated_apart(&mut tp2);
    let (sv, got) = passes::stated_apart(&nv).unwrap();
    same(&sv.program(&lists).unwrap(), &tp2);
    assert_eq!(sv.digest(), base(&tp2).digest(), "stated apart digest");
    let got: Vec<(String, String)> = got.iter().map(|&(a, b)| (sv.rel_name(a).to_owned(), sv.rel_name(b).to_owned())).collect();
    assert_eq!(got, pairs, "stated apart");
    analyses_agree(&sv, &tp2);
}

/// A program the passes change everywhere: relations of no columns (declared, ruled, with facts), a lattice of no
/// keys and one of keys, `top` in heads, bodies, calls and comparisons, ground rules, and stated facts beside rules.
fn normalizable() -> Program {
    use crate::code_rule_dsl::logic::front::LatticeDecl;
    let mut p = analysed();
    p.decls.insert("flag".into(), [].into());
    p.decls.insert("in".into(), [("a".into(), "int".into()), ("b".into(), "int".into()), ("c".into(), "int".into())].into());
    p.decls.insert("best".into(), [("x".into(), "int".into()), ("n".into(), "int".into())].into());
    p.decls.insert("lo".into(), [("n".into(), "int".into())].into());
    p.decls.insert("e".into(), [("x".into(), "int".into()), ("y".into(), "int".into())].into());
    p.lattice.insert("lo".into(), LatticeDecl { keys: 0, ty: "min".into(), merge: None, widen: None });
    p.facts.insert("flag".into(), vec![[].into()]);
    p.facts.insert("e".into(), vec![[Value::Int(1), Value::Int(2)].into()]);
    p.facts.insert("lo".into(), vec![[Value::Int(7)].into()]);
    p.rules.extend([
        Rule { head: atom("flag", vec![]), body: vec![Literal::Atom(atom("p", vec![Term::Wild]))].into() },
        Rule { head: atom("lo", vec![v("n")]), body: vec![Literal::Atom(atom("best", vec![Term::Wild, v("n")])), Literal::Atom(atom("flag", vec![]))].into() },
        Rule { head: atom("lo", vec![Term::Top]), body: vec![].into() },
        Rule { head: atom("best", vec![Term::Int(0), Term::Top]), body: vec![].into() },
        Rule { head: atom("seen", vec![v("x"), v("m")]), body: vec![
            Literal::Atom(atom("lo", vec![v("n")])),
            Literal::Cmp { l: v("m"), op: "=", r: v("n") },
            Literal::Cmp { l: v("m"), op: "!=", r: Term::Top },
            Literal::Atom(atom("in", vec![v("x"), Term::Call("max".into(), [v("m"), Term::Top].into()), Term::Wild])),
        ].into() },
    ]);
    p
}

/// The passes over a version are the tree's passes, on every sampled pack program and on a program they change
/// everywhere; and a `top` of two domains is refused by both.
#[test]
fn the_passes_over_a_version_are_the_tree_passes() {
    let mut n = 0;
    for (_, prog) in pack_programs() {
        if encode(&prog).is_err() {
            continue;
        }
        passes_agree(&prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
    let p = normalizable();
    crate::code_rule_dsl::logic::plan::sql_kernel::normalize(&p).expect("the tree normalizes it");
    passes_agree(&p);
    let (nv, _) = super::passes::normalize(&base(&p)).unwrap();
    assert!(nv.rules().count() < p.rules.len(), "ground rules became facts");
    // `top` beside variables of two lattices.
    let mut two = p.clone();
    two.rules.push(Rule { head: atom("out", vec![v("x")]), body: vec![
        Literal::Atom(atom("lo", vec![v("a")])),
        Literal::Atom(atom("best", vec![v("x"), v("b")])),
        Literal::Cmp { l: Term::Call("max".into(), [v("a"), v("b")].into()), op: "<", r: Term::Top },
    ].into() });
    assert!(crate::code_rule_dsl::logic::plan::sql_kernel::normalize(&two).is_err());
    assert!(super::passes::normalize(&base(&two)).is_err());
}

/// A rule rewritten as it is copied keeps what the rewrite does not change, records its source, and takes more
/// literals after; a fold keeps every source.
#[test]
fn a_rewrite_copies_what_it_does_not_change() {
    use super::{Rewrite, Site};
    struct Mark;
    impl Rewrite for Mark {
        fn prefix(&mut self, a: super::AtomRef<'_>, out: &mut Vec<T<'static>>) {
            if a.arity() == 1 {
                out.push(T::Int(9));
            }
        }
        fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
            matches!((site, t), (Site::Cmp(_), TermRef::Int(_))).then_some(T::Int(0))
        }
        fn keep(&mut self, l: super::LitRef<'_>) -> bool {
            !matches!(l.view(), Lit::Nearest { .. })
        }
    }
    let p = every_part();
    let v = base(&p);
    let mut b = Builder::derive(&v);
    let mut ids = Vec::new();
    for r in v.rules() {
        let id = b.rewrite(&v, r, &mut Mark).atom("extra", &[T::Wild]).finish().unwrap();
        ids.push((id, r.id()));
    }
    let (d, _) = b.finish().unwrap();
    for (id, src) in ids {
        let (got, from) = (d.rule(id).unwrap(), v.rule(src).unwrap());
        assert_eq!(got.source(), Some(src));
        assert_eq!(d.derived(src).map(|r| r.id()).collect::<Vec<_>>(), vec![id]);
        assert_eq!(got.head().arity(), from.head().arity() + usize::from(from.head().arity() == 1));
        let kept = from.body().filter(|l| !matches!(l.view(), Lit::Nearest { .. })).count();
        assert_eq!(got.body().count(), kept + 1);
        // Every variable keeps its name (a dropped literal's own are gone).
        let names = |r: super::RuleRef<'_>, v: &Version| r.vars().map(|a| v.text(a).to_owned()).collect::<std::collections::BTreeSet<_>>();
        assert!(names(got, &d).is_subset(&names(from, &v)));
        for l in got.body() {
            if let Lit::Cmp { l, r, .. } = l.view() {
                assert!(!matches!(l, TermRef::Int(x) if x != 0) && !matches!(r, TermRef::Int(x) if x != 0));
            }
        }
    }
    let f = d.fold().unwrap();
    for r in d.rules() {
        assert_eq!(f.rule(r.id()).unwrap().source(), r.source());
    }
    // Encoded again, the image opens and reads back.
    same(&Version::open(&encode(&d.program(&Lists::default()).unwrap()).unwrap()).unwrap().program(&Lists::default()).unwrap(), &d.program(&Lists::default()).unwrap());
}

/// The planner's reading of a rule (`plan::body::Body`) says what the tree says of it, on the pack programs and on a
/// program of every part: each literal's text, variables, whether it joins and when it can run (after each prefix of the
/// body, and after each literal alone), each constant, and the key the rule's observations are kept under.
#[test]
fn the_planners_reading_of_a_rule_is_the_trees() {
    use crate::code_rule_dsl::logic::plan::{self, body::{Body, VarSet, made}};
    use std::collections::BTreeSet;
    let mut progs: Vec<Program> = pack_programs().into_iter().map(|(_, p)| p).collect();
    progs.push(every_part());
    progs.push(analysed());
    let mut n = 0;
    for prog in &progs {
        let v = made(&prog.rules).unwrap();
        for (k, rule) in prog.rules.iter().enumerate() {
            let b = Body::of(&v, v.rule(RuleId(k as u32)).unwrap());
            assert_eq!(b.show_head(), rule.head.to_string());
            assert_eq!(b.len(), rule.body.len(), "{rule}");
            assert_eq!(b.observed_key(), plan::obs_rule(&rule.head, &rule.body), "{rule}");
            let names = |s: &VarSet| -> BTreeSet<String> { s.iter().map(|x| b.var(x).to_owned()).collect() };
            let slots = |names: &BTreeSet<String>| -> VarSet { VarSet::of(v.rule(RuleId(k as u32)).unwrap().vars().enumerate().filter(|(_, a)| names.contains(v.text(*a))).map(|(i, _)| super::Slot(i as u32))) };
            let mut bounds: Vec<BTreeSet<String>> = vec![BTreeSet::new()];
            for l in rule.body.iter() {
                let mut next = bounds.last().unwrap().clone();
                next.extend(l.vars());
                bounds.push(next);
                bounds.push(l.vars());
            }
            for (i, l) in rule.body.iter().enumerate() {
                assert_eq!(b.show(i), l.to_string());
                assert_eq!(names(b.vars(i)), l.vars(), "{l}");
                assert_eq!(b.is_join(i), plan::is_join(l), "{l}");
                for bound in &bounds {
                    assert_eq!(b.ready(i, &slots(bound)), plan::ready(l, bound), "{l} with {bound:?} bound");
                }
                if let (Some(a), Some(t)) = (b.atom(i), l.atom()) {
                    assert_eq!(b.rel(a), t.rel.as_str());
                    for (x, y) in a.args().zip(t.args.iter()) {
                        assert_eq!(b.constant(x), plan::constant(y));
                    }
                }
                n += 1;
            }
        }
    }
    assert!(n > 1000, "{n}");
}
