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
    // "s" is a string constant of the base; here it becomes a relation.
    let s = v0.find("s").unwrap();
    assert_eq!(v0.rel_of(s), None);
    b.rule().head("s", &[T::Var("x")]).atom("a", &[T::Var("x"), T::Wild, T::Wild]).finish().unwrap();
    b.rule().head("brand_new", &[T::Str("never seen")]).finish().unwrap();
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    assert_eq!(v1.find("s"), Some(s));
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
