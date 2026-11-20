use super::passes::Parts as _;
use super::fields::ProgramFields;
use super::version::{Made, Version};
use super::{AtomId, Builder, CmpOp, Extents, Grouped, Image, Lit, R, RelId, RuleId, RuleRef, Strings, T, TermRef};
use crate::code_rule_dsl::logic::front::{AggOp, LatticeDecl, Native, Value, ast::Gate};
use crate::code_rule_dsl::logic::program_facts::Field;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use crate::code_rule_dsl::logic::program_image::manager::VersionAnalyses as _;

/// `a` and `b` are the same program: the same text, and the same digest (every part's hash).
fn same(a: &Version, b: &Version) {
    assert_eq!(a.show(), b.show());
    assert_eq!(a.digest(), b.digest(), "the digests of\n{}", a.show());
}

/// `from` made again by another route: a derivation of all its parts, with the rules `keep` takes copied in their
/// order, then `edit`'s changes.
fn remade(from: &Version, keep: impl Fn(RuleRef<'_>) -> bool, edit: impl FnOnce(&mut Builder<'_>)) -> Arc<Version> {
    let mut b = Builder::derive(from);
    for r in from.rules().filter(|r| keep(*r)) {
        b.copy(from, r);
    }
    edit(&mut b);
    b.finish().unwrap().0
}

/// Every part of a program: names, the least number, strings that need escapes, each literal and aggregate, gated
/// negations, a relation with no rows, and the `.about` parts.
fn every_part_on(b: &mut Builder<'_>) {
    b.fields().declare("r", &[("c", "int"), ("d", "sym")]);
    b.fields().declare("nearest", &[("x", "int")]);
    b.fields().set_inputs(&["a", "b"]);
    b.fields().set_outputs(&["r"]);
    b.fields().set_facts("k", &[vec![Value::Int(i64::MIN), Value::Str("q\"\\\n\u{1}é".into())].into(), vec![Value::Int(-3), Value::Str(String::new().into())].into()]);
    b.fields().set_facts("empty", &[]);
    let x = || T::Var("x");
    let y = || T::Var("y");
    b.rule()
        .head("r", &[x(), T::Call("cat", vec![T::Var("top"), T::Str("s")])])
        .atom("a", &[x(), T::Var("count_"), T::Var("min")])
        .atom_with("b", &[T::Wild, T::Top], true, false)
        .atom_with("done_x", &[x()], true, true)
        .cmp(x(), CmpOp::Le, T::Int(i64::MIN))
        .cmp(T::Call("rx", vec![x(), T::Str("^a$")]), CmpOp::Eq, T::Int(1 << 40))
        .finish()
        .unwrap();
    b.rule()
        .head("nearest", &[T::Var("n")])
        .agg(T::Var("n"), &AggOp::Count, None, "a", &[x(), T::Wild])
        .agg(T::Var("m"), &AggOp::Min, Some(y()), "a", &[y()])
        .agg(T::Var("s"), &AggOp::Sum, None, "a", &[y()])
        .agg(T::Var("j"), &AggOp::Join { by: vec!["k".into(), "sep".into()], sep: ", ".into() }, Some(y()), "ast", &[y(), T::Var("k"), T::Var("sep")])
        .agg(T::Var("j2"), &AggOp::Join { by: vec![], sep: "".into() }, Some(y()), "a", &[y()])
        .ast(T::Var("d"), T::Var("n"), "{\"kind\": \"call\"}", &[("NAME", T::Var("v"))])
        .finish()
        .unwrap();
    b.rule().head("unit", &[]).finish().unwrap();
    b.fields().set_field(Field::Natives(&[("reach_x".to_owned(), Native::Reach { seed: "s".into(), edge: "e".into() })].into()));
    b.fields().set_field(Field::Kindsets(&[("Scope".to_owned(), vec!["module".to_owned(), "class".to_owned()])].into()));
    b.fields().set_field(Field::Demand(&["*".to_owned()]));
    b.fields().set_field(Field::Settings(&[("mode".to_owned(), "block".to_owned()), ("input".to_owned(), "r(IO=json)".to_owned())]));
    b.fields().set_field(Field::Nullary(&["unit".to_owned()].into()));
}

fn every_part() -> Arc<Version> {
    let mut b = Builder::base();
    every_part_on(&mut b);
    b.finish().unwrap().0
}

fn pack_files() -> Vec<std::path::PathBuf> {
    let packs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rule-syntax/packs");
    let mut files: Vec<_> = std::fs::read_dir(&packs).unwrap().flatten().filter(|d| d.path().is_dir()).flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten().map(|f| f.path())).filter(|p| p.extension().is_some_and(|e| e == "rr")).collect();
    files.sort();
    files
}

/// Every seventh pack program, as the text's test samples them.
fn pack_programs() -> Vec<(std::path::PathBuf, Arc<Version>)> {
    let files: Vec<_> = pack_files().into_iter().step_by(7).collect();
    crate::code_rule_dsl::logic::plan::corpus::phase0::keep_files(&files);
    files.into_iter().filter_map(|p| Some((p.clone(), crate::code_rule_dsl::logic::rr::parse_file_version(&p).ok()?))).collect()
}

/// Every part of a program reads back from its image as written, and so does every sampled pack program: its image,
/// opened, is the program, rule by rule, by head and by reader.
#[test]
fn a_program_reads_back_from_its_image() {
    let v = every_part();
    let back = Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
    same(&back, &v);
    handles_agree(&back, &v);
    assert!(back.show().contains("$j = join $y by ($k, $sep) sep \", \" : { ast($y, $k, $sep) }"), "{}", back.show());
    let mut n = 0;
    for (path, prog) in pack_programs() {
        let back = Version::open(prog.fold().unwrap().base_bytes().unwrap()).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        same(&back, &prog);
        handles_agree(&back, &prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
}

/// `a`'s handles say what `b`'s say: rule by rule (as written), by head, and by reader, by name.
fn handles_agree(a: &Version, b: &Version) {
    let (ra, rb): (Vec<_>, Vec<_>) = (a.rules().collect(), b.rules().collect());
    assert_eq!(ra.iter().map(|r| a.show_rule(*r)).collect::<Vec<_>>(), rb.iter().map(|r| b.show_rule(*r)).collect::<Vec<_>>());
    let order = |v: &Version, rs: &[RuleRef<'_>]| -> std::collections::HashMap<RuleId, usize> { let _ = v; rs.iter().enumerate().map(|(k, r)| (r.id(), k)).collect() };
    let (oa, ob) = (order(a, &ra), order(b, &rb));
    let names: BTreeSet<String> = ra.iter().flat_map(|r| std::iter::once(a.rel_name(r.head().rel()).to_owned()).chain(r.reads().map(|x| a.rel_name(x.rel()).to_owned()))).collect();
    for rel in names {
        let (xa, xb) = (a.rel(&rel).unwrap_or_else(|| panic!("relation {rel}")), b.rel(&rel).unwrap_or_else(|| panic!("relation {rel}")));
        let heads = |v: &Version, x: RelId, o: &std::collections::HashMap<RuleId, usize>| v.heads(x).map(|r| o[&r.id()]).collect::<Vec<_>>();
        assert_eq!(heads(a, xa, &oa), heads(b, xb, &ob), "heads of {rel}");
        let readers = |v: &Version, x: RelId, o: &std::collections::HashMap<RuleId, usize>| v.readers(x).map(|(r, at)| {
            assert_eq!(at.rel(), x);
            o[&r.id()]
        }).collect::<Vec<_>>();
        assert_eq!(readers(a, xa, &oa), readers(b, xb, &ob), "readers of {rel}");
    }
}

/// A damaged image is refused at open, never read wrong: bad magic, a short image, a region past the end, a node
/// whose size overruns its rule, a slot past its rule's variables, a node of no kind.
#[test]
fn a_damaged_image_is_refused() {
    let bytes = every_part().fold().unwrap().base_bytes().unwrap().to_vec();
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

/// A large pack program, for the versions' tests: the largest pack file that reads, with more than 40 rules and two
/// declarations.
fn big() -> Arc<Version> {
    let mut files = pack_files();
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map_or(0, |m| m.len())));
    files.iter().find_map(|p| crate::code_rule_dsl::logic::rr::parse_file_version(p).ok().filter(|p| p.rules().count() > 40 && p.declared_names().len() > 2)).expect("a large pack program")
}

/// The first rule's head relation of `v`.
fn first_head(v: &Version) -> String {
    v.rel_name(v.rules().next().unwrap().head().rel()).to_owned()
}

/// A view keeps a base's rules in ranges; an overlay adds, removes and replaces parts; three stacked are the program
/// they say, by rule, by head and by reader, and their digests are the digest of that program made again whole.
#[test]
fn views_and_overlays_are_the_programs_they_say() {
    let v0 = big();
    let (shown0, digest0) = (v0.show(), v0.digest());
    let head0 = first_head(&v0);
    let x = || T::Var("x");
    // A view: the first and last thirds.
    let n = v0.rule_end();
    let removed: Vec<u32> = (n / 3..2 * n / 3).collect();
    let mut b = v0.builder();
    for &g in &removed {
        b.remove(RuleId(g));
    }
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::View);
    let p1 = remade(&v0, |r| !removed.contains(&r.id().0), |_| {});
    same(&v1, &p1);
    handles_agree(&v1, &p1);
    // A view of the view: one more rule out.
    let mut b = v1.builder();
    b.remove(RuleId(0));
    let (v1v, made) = b.finish().unwrap();
    assert_eq!(made, Made::View);
    same(&v1v, &remade(&v0, |r| r.id().0 != 0 && !removed.contains(&r.id().0), |_| {}));
    // An overlay on the view: its delta over the view.
    let on_view = |b: &mut Builder<'_>| {
        b.rule().head("on_the_view", &[x()]).atom(head0.as_str(), &[x()]).finish().unwrap();
    };
    let mut b = v1.builder();
    b.remove(RuleId(0));
    on_view(&mut b);
    let (v1b, made) = b.finish().unwrap();
    assert_eq!((made, v1b.depth()), (Made::Overlay, 2));
    let p1b = remade(&v0, |r| r.id().0 != 0 && !removed.contains(&r.id().0), on_view);
    same(&v1b, &p1b);
    handles_agree(&v1b, &p1b);
    // An overlay on the base: two rules out, one in, a declaration changed, one gone, a fact relation added, the
    // inputs changed.
    let decls = v0.declared_names();
    let (d_changed, d_gone) = (decls[0].clone(), decls[1].clone());
    let inputs: Vec<String> = v0.input_names().into_iter().chain(["one_more".to_owned()]).collect();
    let made_here = |b: &mut Builder<'_>| {
        b.rule().head("made_here", &[x()]).atom(head0.as_str(), &[x()]).finish().unwrap();
    };
    let parts = |b: &mut Builder<'_>, facts: bool| {
        b.fields().declare(&d_changed, &[("only", "int")]);
        b.fields().undeclare(&d_gone);
        b.fields().declare("made_here", &[("x", "int")]);
        if facts {
            b.fields().set_facts("made_facts", &[vec![Value::Int(7)].into()]);
        }
        b.fields().set_inputs(&inputs);
    };
    let mut b = v0.builder();
    b.remove(RuleId(5));
    b.remove(RuleId(1));
    made_here(&mut b);
    parts(&mut b, true);
    let (v2, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    let p2 = remade(&v0, |r| ![1, 5].contains(&r.id().0), |b| {
        made_here(b);
        parts(b, true);
    });
    same(&v2, &p2);
    handles_agree(&v2, &p2);
    // Two more overlays on it: a stack three deep. The last three rules out (two of the base's and the new one), the
    // facts gone, an output added.
    let ids: Vec<RuleId> = v2.rules().map(|r| r.id()).collect();
    let last: Vec<RuleId> = ids[ids.len() - 3..].to_vec();
    let outputs: Vec<String> = v0.output_names().into_iter().chain(["made_here".to_owned()]).collect();
    let mut b = v2.builder();
    for &id in &last {
        b.remove(id);
    }
    b.fields().drop_facts("made_facts");
    b.fields().set_outputs(&outputs);
    let (v3, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    let mut b = v3.builder();
    made_here(&mut b);
    let (v4, _) = b.finish().unwrap();
    // v3 is two layers: a view (its rules out) and the overlay of its fields over it.
    assert_eq!(v4.depth(), 4);
    // The added rule goes last in an overlay: its order is the version's.
    let p4 = remade(&v0, |r| ![1, 5].contains(&r.id().0) && !last.contains(&r.id()), |b| {
        parts(b, false);
        b.fields().set_outputs(&outputs);
        made_here(b);
    });
    same(&v4, &p4);
    handles_agree(&v4, &p4);
    // The base is unchanged by all of it.
    assert_eq!((v0.show(), v0.digest()), (shown0, digest0));
}

/// A split part is a view of many ranges with its fields overlaid (decision 0244): it is the program made again of the
/// rules its heads keep, with those relations' natives, lattices and gates and its own inputs and outputs, by rule,
/// head and reader, with its digest; it costs its ranges and fields; and a view of it (the run's edits, the session's
/// cut) is the program that edit makes.
#[test]
fn a_part_is_a_view_with_its_fields_overlaid() {
    let big = big();
    // Fields of relations in both parts: a native, lattices and gates of each.
    let heads: Vec<String> = { let mut h: Vec<String> = big.rules().map(|r| big.rel_name(r.head().rel()).to_owned()).collect(); h.sort(); h.dedup(); h };
    assert!(heads.len() > 8, "{}", heads.len());
    let mut lattice = BTreeMap::new();
    let mut gates = Vec::new();
    for (i, rel) in heads.iter().take(6).enumerate() {
        lattice.insert(rel.clone(), LatticeDecl { keys: 1, ty: "int".into(), merge: (i % 2 == 0).then(|| "max".into()), widen: (i == 1).then_some(3) });
        gates.push((rel.clone(), Gate { rel: rel.clone(), level: i, src: (i == 2).then(|| "s".into()), quiet: vec![heads[i + 1].clone()], deps: vec![] }));
    }
    let mut natives = BTreeMap::new();
    natives.insert(heads[0].clone(), Native::Reach { seed: heads[1].clone(), edge: heads[2].clone() });
    natives.insert(heads[1].clone(), Native::Reach { seed: heads[2].clone(), edge: heads[3].clone() });
    let v0 = {
        let mut b = Builder::on(&big);
        b.fields().set_field(Field::Lattice(&lattice));
        b.fields().set_field(Field::Gates(&gates));
        b.fields().set_field(Field::Natives(&natives));
        // Facts named as about relations are (`gate`, `lattice`): the fields' overlay hides none of them.
        b.fields().set_facts("gate", &[vec![Value::Int(1)].into()]);
        b.fields().set_facts("lattice", &[vec![Value::Str("x".into())].into()]);
        b.finish().unwrap().0.fold().unwrap()
    };
    // Every other relation, so the part interleaves the program's rules: many ranges.
    let mine: BTreeSet<&str> = heads.iter().step_by(2).map(String::as_str).collect();
    let keep = |rel: &str| mine.contains(rel);
    let inputs: Vec<String> = v0.input_names().into_iter().take(2).chain(["exported".to_owned()]).collect();
    let outputs: Vec<String> = heads.iter().step_by(4).cloned().collect();
    let part = v0.part(&keep, &inputs, &outputs, super::PartFields { natives: &keep, lattice: &keep, gates: &keep }).unwrap();
    let fields_of = |b: &mut Builder<'_>| {
        b.fields().set_field(Field::Natives(&natives.iter().filter(|(r, _)| keep(r)).map(|(r, n)| (r.clone(), n.clone())).collect()));
        b.fields().set_field(Field::Lattice(&lattice.iter().filter(|(r, _)| keep(r)).map(|(r, l)| (r.clone(), l.clone())).collect()));
        b.fields().set_field(Field::Gates(&gates.iter().filter(|(r, _)| keep(r)).cloned().collect::<Vec<_>>()));
        b.fields().set_field(Field::Asked(&ProgramFields::asked(&*v0).into_iter().filter(|(r, _)| keep(r)).collect()));
        b.fields().set_inputs(&inputs);
        b.fields().set_outputs(&outputs);
    };
    let want = remade(&v0, |r| keep(v0.rel_name(r.head().rel())), fields_of);
    same(&part, &want);
    handles_agree(&part, &want);
    assert_eq!(part.size(), want.rules().count());
    // A view, its fields an overlay over it: one image of its own, the fields' delta, and no rule.
    let Version::Overlay { parent, delta, .. } = &*part else { panic!("the fields are an overlay") };
    let Version::View { parent: over, .. } = &**parent else { panic!("the rules are a view") };
    assert!(Arc::ptr_eq(over, &v0), "a view of the program itself");
    assert_eq!(delta.rules(), 0);
    // A view of the part: some of its rules out, as the run's edits do; its inputs changed, as they add inputs.
    let out: Vec<RuleId> = part.rules().step_by(6).map(|r| r.id()).collect();
    let mut b = part.builder();
    for &id in &out {
        b.remove(id);
    }
    let (cut, made) = b.finish().unwrap();
    assert_eq!(made, Made::View);
    let want_cut = remade(&v0, |r| keep(v0.rel_name(r.head().rel())) && !out.contains(&r.id()), fields_of);
    same(&cut, &want_cut);
    handles_agree(&cut, &want_cut);
    let mut b = Builder::on(&cut);
    let more: Vec<String> = inputs.iter().cloned().chain(["delta_in".to_owned()]).collect();
    b.fields().set_inputs(&more);
    let cut_in = b.finish_overlay().unwrap();
    same(&cut_in, &remade(&v0, |r| keep(v0.rel_name(r.head().rel())) && !out.contains(&r.id()), |b| {
        fields_of(b);
        b.fields().set_inputs(&more);
    }));
}

/// A view may have up to a quarter of its parent's rules as ranges (decision 0244): past `MAX_RANGES` a large
/// program's view of many ranges is still a view, and past a quarter it is an overlay.
#[test]
fn a_view_may_have_a_quarter_of_its_parents_rules_as_ranges() {
    let v0 = big();
    let n = v0.rule_end();
    let edit = |every: u32| {
        let mut b = v0.builder();
        for g in (0..n).step_by(every as usize) {
            b.remove(RuleId(g));
        }
        b.finish().unwrap().1
    };
    // Every eighth out: about an eighth of the rules as ranges.
    if n / 8 > super::version::MAX_RANGES as u32 {
        assert_eq!(edit(8), Made::View);
    }
    // Every other out: half the rules as ranges, an overlay (folded: it removes half).
    assert_ne!(edit(2), Made::View);
}

/// Ids are the stack's: a delta's atoms, relations and rules follow its parent's, a parent's atom it makes a
/// relation keeps its id, every id of the parent means the same in the overlay, and a fold keeps them all but the
/// rules'.
#[test]
fn ids_are_the_same_through_a_stack_and_a_fold() {
    let v0 = {
        let mut b = Builder::base();
        every_part_on(&mut b);
        // Enough rules that two more are an overlay, not a fold.
        for k in 0..8 {
            b.rule().head(format!("pad_{k}").as_str(), &[T::Var("x")]).atom("a", &[T::Var("x")]).finish().unwrap();
        }
        b.finish().unwrap().0
    };
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
    same(&f, &v1);
}

/// A rule written term by term is copied into a version of other ids (by its texts), or of the same (by its nodes),
/// as the rule it was, hash for hash, as are its copied literals; and a body before its head, or no head, is refused.
#[test]
fn the_builder_copies_rules_by_texts_and_by_nodes() {
    let v0 = every_part();
    let mut by_terms = v0.builder();
    let id = by_terms.rule().head("p", &[T::Var("x"), T::Int(1 << 40)])
        .atom("a", &[T::Var("x"), T::Wild, T::Call("f", vec![T::Str("z")])])
        .not("b", &[T::Var("x"), T::Top])
        .cmp(T::Var("x"), CmpOp::Lt, T::Int(3))
        .finish().unwrap();
    let (a, _) = by_terms.finish().unwrap();
    let r = a.rule(id).unwrap();
    assert_eq!(a.show_rule(r), "p($x, 1099511627776) :- a($x, _, @f(\"z\")), !b($x, top), $x < 3");
    // Copied into a version of other ids: a new base, with none of `a`'s names.
    let mut other = Builder::base();
    let copied = other.copy(&a, r);
    let lit = r.body().nth(1).unwrap();
    let parts = other.rule().head_from(&a, r, r.head()).lit(&a, r, lit).finish().unwrap();
    let (o, _) = other.finish().unwrap();
    assert_eq!(o.rule(copied).unwrap().hash(), r.hash());
    assert_eq!(o.show_rule(o.rule(copied).unwrap()), a.show_rule(r));
    assert_eq!(o.show_rule(o.rule(parts).unwrap()), "p($x, 1099511627776) :- !b($x, top)");
    // Copied within its stack: the same nodes.
    let mut again = a.builder();
    let id2 = again.copy(&a, a.rule(id).unwrap());
    let (c, _) = again.finish().unwrap();
    assert_eq!(c.rule(id2).unwrap().hash(), r.hash());
    // A body before its head, or no head, is refused.
    let mut bad = v0.builder();
    assert!(bad.rule().atom("a", &[]).head("p", &[]).finish().is_err());
    assert!(bad.rule().finish().is_err());
    drop(bad.rule().head("unfinished", &[]));
    let ok = bad.rule().head("kept", &[]).finish().unwrap();
    assert_eq!(ok, RuleId(v0.rule_end()));
    let (kept, _) = bad.finish().unwrap();
    assert_eq!(kept.size(), v0.size() + 1);
    same(&kept, &remade(&v0, |_| true, |b| {
        b.rule().head("kept", &[]).finish().unwrap();
    }));
}

/// An edit that reaches half its parent is folded into a new base, which is the program.
#[test]
fn an_edit_of_half_its_parent_folds() {
    let v0 = big();
    let n = v0.rule_end();
    let inputs: Vec<String> = v0.input_names().into_iter().chain(["x".to_owned()]).collect();
    let mut b = v0.builder();
    for g in n / 3..n {
        b.remove(RuleId(g));
    }
    b.fields().set_inputs(&inputs);
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::Folded);
    assert_eq!(v1.depth(), 0);
    same(&v1, &remade(&v0, |r| r.id().0 < n / 3, |b| b.fields().set_inputs(&inputs)));
}

/// An overlay's delta is the size of its edit, not of its parent: one rule added to a program ten times larger
/// (its rules, nine times over) makes a delta no larger.
#[test]
fn a_delta_is_the_size_of_its_edit() {
    let small = big();
    let large = {
        let mut b = Builder::on(&small);
        for k in 0..9 {
            for r in small.rules() {
                let head = format!("{}_{k}", small.rel_name(r.head().rel()));
                let args: Vec<T<'_>> = r.head().args().map(|t| super::t_of(&small, r, t)).collect();
                let mut w = b.rule().head(head.as_str(), &args);
                for l in r.body() {
                    w = w.lit(&small, r, l);
                }
                w.finish().unwrap();
            }
        }
        b.finish().unwrap().0
    };
    assert_eq!(large.depth(), 0, "ten times the rules: a new base");
    let mut sizes = Vec::new();
    for parent in [&small, &large] {
        let mut b = parent.builder();
        b.rule().head("one", &[T::Var("x")]).atom("two", &[T::Var("x")]).finish().unwrap();
        let (v, made) = b.finish().unwrap();
        assert_eq!(made, Made::Overlay);
        let Version::Overlay { delta, .. } = &*v else { unreachable!() };
        sizes.push((parent.fold().unwrap().base_bytes().unwrap().len(), delta.bytes().len()));
    }
    assert!(sizes[1].0 > 5 * sizes[0].0, "{sizes:?}");
    assert_eq!(sizes[0].1, sizes[1].1, "{sizes:?}");
    assert!(sizes[1].1 < 2048, "{sizes:?}");
}

/// A program with what the analyses read beside rules: answers, a lattice its aggregate is monotone in, a native,
/// projections with constants, and a stratum through a gated negation.
fn analysed_on(b: &mut Builder<'_>) {
    let x = || T::Var("x");
    let y = || T::Var("y");
    b.rule().head("e", &[x(), y()]).atom("in", &[x(), y(), T::Int(3)]).finish().unwrap();
    b.rule().head("p", &[x()]).atom("e", &[x(), T::Wild]).finish().unwrap();
    b.rule().head("p", &[y()]).atom("p", &[x()]).atom("e", &[x(), y()]).finish().unwrap();
    b.rule().head("q", &[x()]).atom("p", &[x()]).atom_with("done_q", &[x()], true, true).finish().unwrap();
    b.rule().head("best", &[x(), T::Var("n")]).atom("p", &[x()]).agg(T::Var("n"), &AggOp::Max, Some(y()), "best", &[x(), y()]).finish().unwrap();
    b.rule().head("lone", &[x()]).not("q", &[x()]).atom("p", &[x()]).finish().unwrap();
    b.rule().head("ask", &[x()]).atom("lone", &[x()]).finish().unwrap();
    b.rule().head("out", &[x()]).atom("told", &[x()]).atom("r", &[x()]).finish().unwrap();
    b.rule().head("r", &[x()]).atom("seed", &[x()]).finish().unwrap();
    // Padding, so that a small edit is an overlay or a view, not a fold.
    for k in 0..24 {
        b.rule().head("pad", &[x()]).atom("in", &[x(), y(), T::Int(k)]).finish().unwrap();
    }
    b.fields().set_field(Field::Answers(&[("ask".to_owned(), vec!["told".to_owned()])].into()));
    b.fields().set_field(Field::Lattice(&[("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None })].into()));
    b.fields().set_field(Field::Natives(&[("r".to_owned(), Native::Reach { seed: "lone".into(), edge: "e".into() })].into()));
    b.fields().set_inputs(&["in", "seed"]);
    b.fields().set_outputs(&["out", "best", "pad"]);
}

fn analysed() -> Arc<Version> {
    let mut b = Builder::base();
    analysed_on(&mut b);
    b.finish().unwrap().0
}

/// The composed path accepts a program of its own kind: joins of inputs, one recursive stratum.
#[test]
fn the_composed_path_accepts_a_plain_program() {
    let mut b = Builder::base();
    let x = || T::Var("x");
    let y = || T::Var("y");
    b.rule().head("e", &[x(), y()]).atom("in", &[x(), y(), T::Int(3)]).finish().unwrap();
    b.rule().head("p", &[x()]).atom("e", &[x(), T::Wild]).finish().unwrap();
    b.rule().head("p", &[y()]).atom("p", &[x()]).atom("e", &[x(), y()]).finish().unwrap();
    b.fields().set_inputs(&["in"]);
    b.fields().set_outputs(&["p"]);
    let plain = b.finish().unwrap().0;
    assert_eq!(super::analyses::accepts(&plain, &super::About::of(&plain)).ok(), Some(true));
}

/// What the planner's passes over a version make of `p` (`passes::normalize`, then `passes::stated_apart` over what it
/// made): the sentinels are the relations of no columns and the lattices of no keys (and the declared nullary ones),
/// each declared with its presence column; every derived rule has its source, and each of `p`'s gave one rule, or a
/// fact (a ground rule); each relation's stated facts beside its rules go apart, to a relation of its own.
fn passes_hold(v: &Arc<Version>) {
    use super::fields::ProgramFields;
    use super::passes;
    use std::collections::BTreeSet;
    let Ok((nv, ns)) = passes::normalize(&v) else { return };
    let names: BTreeSet<String> = ns.iter().map(|&r| nv.rel_name(r).to_owned()).collect();
    let lattice = ProgramFields::lattice(&**v);
    let nullary = ProgramFields::nullary(&**v);
    let want: BTreeSet<String> = v
        .declared_names()
        .into_iter()
        .filter(|r| v.columns_of(r).is_some_and(|c| c.is_empty()) || lattice.get(r).is_some_and(|l| l.keys == 0) || nullary.contains(r))
        .collect();
    assert_eq!(names, want, "sentinels");
    for r in &names {
        let (before, after) = (v.columns_of(r).unwrap().len(), nv.columns_of(r).unwrap().len());
        assert!(after == before + 1 || (after == before && nullary.contains(r)), "{r}: {before} columns, then {after}");
    }
    // Every derived rule came from one of `v`'s; each of `v`'s gave one rule, or a fact (a ground rule). A program
    // normalize leaves as it was is its input (the manager keeps no new base of it).
    if !Arc::ptr_eq(&nv, &v) {
        for r in nv.rules() {
            let s = r.source().expect("a derived rule's source");
            assert_eq!(v.rel_name(v.rule(s).expect("its source is the input's").head().rel()), nv.rel_name(r.head().rel()));
        }
        for r in v.rules() {
            let made = nv.derived(r.id()).count();
            assert!(made == 1 || (made == 0 && r.body().next().is_none()), "{} made {made}", v.rel_name(r.head().rel()));
        }
    }
    let (sv, pairs) = passes::stated_apart(&nv).unwrap();
    for &(a, b) in &pairs {
        let (a, b) = (sv.rel_name(a), sv.rel_name(b));
        assert!(sv.declares(b) && !sv.has_fact_rows(a), "{a}'s facts apart as {b}:\n{}", sv.show());
        assert!(sv.rules().any(|r| sv.rel_name(r.head().rel()) == a && r.body().filter_map(|l| l.atom()).any(|x| sv.rel_name(x.rel()) == b)), "{a} reads {b}:\n{}", sv.show());
    }
}

/// A program the passes change everywhere: relations of no columns (declared, ruled, with facts), a lattice of no
/// keys and one of keys, `top` in heads, bodies, calls and comparisons, ground rules, and stated facts beside rules.
fn normalizable_on(b: &mut Builder<'_>) {
    analysed_on(b);
    b.fields().declare("flag", &[]);
    b.fields().declare("in", &[("a", "int"), ("b", "int"), ("c", "int")]);
    b.fields().declare("best", &[("x", "int"), ("n", "int")]);
    b.fields().declare("lo", &[("n", "int")]);
    b.fields().declare("e", &[("x", "int"), ("y", "int")]);
    b.fields().set_field(Field::Lattice(&[
        ("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None }),
        ("lo".to_owned(), LatticeDecl { keys: 0, ty: "min".into(), merge: None, widen: None }),
    ].into()));
    b.fields().set_facts("flag", &[Vec::<Value>::new().into()]);
    b.fields().set_facts("e", &[vec![Value::Int(1), Value::Int(2)].into()]);
    b.fields().set_facts("lo", &[vec![Value::Int(7)].into()]);
    let (x, n, m) = (|| T::Var("x"), || T::Var("n"), || T::Var("m"));
    b.rule().head("flag", &[]).atom("p", &[T::Wild]).finish().unwrap();
    b.rule().head("lo", &[n()]).atom("best", &[T::Wild, n()]).atom("flag", &[]).finish().unwrap();
    b.rule().head("lo", &[T::Top]).finish().unwrap();
    b.rule().head("best", &[T::Int(0), T::Top]).finish().unwrap();
    b.rule()
        .head("seen", &[x(), m()])
        .atom("lo", &[n()])
        .cmp(m(), CmpOp::Eq, n())
        .cmp(m(), CmpOp::Ne, T::Top)
        .atom("in", &[x(), T::Call("max", vec![m(), T::Top]), T::Wild])
        .finish()
        .unwrap();
}

/// The planner's passes over a version hold (`passes_hold`) on every sampled pack program and on a program they change
/// everywhere; and a `top` of two domains is refused.
#[test]
fn the_planners_passes_over_a_version_hold() {
    let mut n = 0;
    for (_, prog) in pack_programs() {
        passes_hold(&prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
    let p = {
        let mut b = Builder::base();
        normalizable_on(&mut b);
        b.finish().unwrap().0
    };
    passes_hold(&p);
    let (nv, _) = super::passes::normalize(&p).expect("it normalizes");
    assert!(nv.rules().count() < p.rules().count(), "ground rules became facts");
    // `top` beside variables of two lattices.
    let two = {
        let mut b = Builder::base();
        normalizable_on(&mut b);
        b.rule()
            .head("out", &[T::Var("x")])
            .atom("lo", &[T::Var("a")])
            .atom("best", &[T::Var("x"), T::Var("b")])
            .cmp(T::Call("max", vec![T::Var("a"), T::Var("b")]), CmpOp::Lt, T::Top)
            .finish()
            .unwrap();
        b.finish().unwrap().0
    };
    assert!(super::passes::normalize(&two).is_err());
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
    let v = every_part();
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
    // Its image opens and reads back as the program.
    same(&Version::open(f.base_bytes().unwrap()).unwrap(), &d);
}

/// `every_part` with a field of every other kind `plan::run` reads: a native of each kind, lattices with a merge and a
/// widening, answers, gates, qualifiers, tape columns and demand in an order.
fn every_field() -> Arc<Version> {
    let mut b = Builder::base();
    every_part_on(&mut b);
    let natives: BTreeMap<String, Native> = [
        ("reach_x", Native::Reach { seed: "s".into(), edge: "e".into() }),
        ("dfs_x", Native::DfsFirst { edge: "e".into(), target: "t".into(), roots: Some("roots".into()) }),
        ("dfs_y", Native::DfsFirst { edge: "e".into(), target: "t".into(), roots: None }),
        ("match_x", Native::MatchAll { input: "src".into(), pattern: "(a)(b)?".into(), groups: vec!["g2".into(), "g1".into()], positions: true }),
        ("match_y", Native::MatchAll { input: "src".into(), pattern: "c".into(), groups: vec![], positions: false }),
        ("tt_x", Native::TextTest { input: "src".into(), column: 0, op: "contains".into(), patterns: vec!["b".into(), "a".into()] }),
        ("comp_x", Native::Components { edge: "e".into(), own: "own".into() }),
        ("through_x", Native::ReachThrough { reach: "reach_x".into(), map: "m".into() }),
        ("py_x", Native::Python { kind: "calls".into(), input: "src".into(), targets: Some("tg".into()) }),
        ("py_y", Native::Python { kind: "defs".into(), input: "src".into(), targets: None }),
        ("cand_x", Native::PythonCandidates { input: "src".into(), targets: "tg".into(), context: "ctx".into(), member: "mem".into() }),
    ]
    .into_iter()
    .map(|(r, n)| (r.to_owned(), n))
    .collect();
    b.fields().set_field(Field::Natives(&natives));
    b.fields().set_field(Field::Lattice(&[
        ("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None }),
        ("joined".to_owned(), LatticeDecl { keys: 2, ty: "sym".into(), merge: Some("type_join".into()), widen: Some(3) }),
    ].into()));
    b.fields().set_field(Field::Answers(&[("ask".to_owned(), vec!["told".to_owned(), "also".to_owned()]), ("ask2".to_owned(), vec!["told2".to_owned()])].into()));
    b.fields().set_field(Field::Gates(&[
        ("done_x".to_owned(), Gate { src: Some("x_demand".into()), rel: "x".into(), level: 2, quiet: vec!["q1".into(), "q0".into()], deps: vec!["done_y".into()] }),
        ("done_y".to_owned(), Gate { src: None, rel: "y".into(), level: 1, quiet: vec![], deps: vec![] }),
    ]));
    b.fields().set_field(Field::Qualifiers(&[("r".to_owned(), ["inline".to_owned(), "no_magic".to_owned()].into()), ("nearest".to_owned(), ["no_inline".to_owned()].into())].into()));
    b.fields().set_field(Field::Tapes(&[("r".to_owned(), [1usize].into())].into()));
    b.fields().set_field(Field::Demand(&["r".to_owned(), "*".to_owned(), "nearest".to_owned()]));
    b.finish().unwrap().0
}

/// A native's fields, by name, as the program states them.
fn native_tree(f: &crate::code_rule_dsl::logic::plan::fields::Fields<'_>, s: &super::NativeSpec) -> Native {
    use super::NativeSpec as S;
    let r = |x: RelId| f.rel_name(x).to_owned();
    let t = |x: AtomId| f.text(x).to_owned();
    match s {
        S::DfsFirst { edge, target, roots } => Native::DfsFirst { edge: r(*edge), target: r(*target), roots: roots.map(r) },
        S::MatchAll { input, pattern, groups, positions } => Native::MatchAll { input: r(*input), pattern: t(*pattern), groups: groups.iter().map(|&g| t(g)).collect(), positions: *positions },
        S::TextTest { input, column, op, patterns } => Native::TextTest { input: r(*input), column: *column, op: t(*op), patterns: patterns.iter().map(|&p| t(p)).collect() },
        S::PythonCandidates { input, targets, context, member } => Native::PythonCandidates { input: r(*input), targets: r(*targets), context: t(*context), member: t(*member) },
        S::Python { kind, input, targets } => Native::Python { kind: t(*kind), input: r(*input), targets: targets.map(r) },
        S::Components { edge, own } => Native::Components { edge: r(*edge), own: r(*own) },
        S::Reach { seed, edge } => Native::Reach { seed: r(*seed), edge: r(*edge) },
        S::ReachThrough { reach, map } => Native::ReachThrough { reach: r(*reach), map: r(*map) },
        S::Rank { query, counted, group, op } => Native::Rank { query: r(*query), counted: r(*counted), group: *group, op: t(*op) },
    }
}

/// Every field `plan::run` reads of `v` (all but its rules), read through `Fields` from its image, says what the
/// program's own readers (`ProgramFields`) say, by text.
fn fields_agree(v: &Arc<Version>) {
    use crate::code_rule_dsl::logic::plan::{body::Placed, fields::Fields};
    // The program's own readers (`ProgramFields`), the reference.
    let (inputs, outputs) = (v.input_names(), v.output_names());
    let decls: BTreeMap<String, Vec<(String, String)>> = v.declared_names().into_iter().map(|r| {
        let cols = v.columns_of(&r).unwrap();
        (r, cols)
    }).collect();
    let facts: BTreeMap<String, Vec<_>> = v.fact_names().into_iter().map(|r| {
        let rows = ProgramFields::fact_rows(&**v, &r);
        (r, rows)
    }).collect();
    let (lattice, natives, answers, pgates) = (ProgramFields::lattice(&**v), ProgramFields::natives(&**v), ProgramFields::answers(&**v), ProgramFields::gates(&**v));
    let (qualifiers, tapes, kindsets, nullary) = (ProgramFields::qualifiers(&**v), ProgramFields::tapes(&**v), ProgramFields::kindsets(&**v), ProgramFields::nullary(&**v));
    let (demand, shapes) = (ProgramFields::demand(&**v), ProgramFields::shapes(&**v));
    let placed = Placed::whole(v.clone());
    let f = Fields::of(&placed).expect("a whole image");
    // Read once, and shared by the clones.
    let again = placed.clone();
    assert!(std::ptr::eq(f.about, Fields::of(&placed).unwrap().about));
    assert!(std::ptr::eq(f.about, Fields::of(&again).unwrap().about));
    fn strs(xs: &[String]) -> Vec<&str> {
        xs.iter().map(String::as_str).collect()
    }
    assert_eq!(f.inputs().collect::<Vec<_>>(), strs(&inputs));
    assert_eq!(f.outputs().collect::<Vec<_>>(), strs(&outputs));
    for r in &inputs {
        assert!(f.is_input(r), "{r}");
    }
    for r in &outputs {
        assert!(f.is_output(r), "{r}");
    }
    // Declarations.
    assert_eq!(f.decls().len(), decls.len());
    for (rel, cols) in &decls {
        let want: Vec<(&str, &str)> = cols.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
        assert_eq!(f.columns(rel.as_str()), Some(want.clone()), "{rel}");
        assert_eq!(f.arity(rel.as_str()), Some(cols.len()));
        for (c, (_, t)) in want.iter().enumerate() {
            assert_eq!(f.column_type(rel.as_str(), c), Some(*t));
        }
    }
    // Facts.
    assert_eq!(f.fact_rels().len(), facts.len());
    for (rel, rows) in &facts {
        let got: Vec<Vec<String>> = f.facts(rel).unwrap_or_else(|| panic!("facts of {rel}")).rows().map(|row| row.map(|c| f.cell_text(c)).collect()).collect();
        let want: Vec<Vec<String>> = rows.iter().map(|row| row.iter().map(|x| match x {
            Value::Int(i) => i.to_string(),
            Value::Str(s) => format!("{:?}", s.as_str()),
            Value::Branded(b) => b.to_string(),
        }).collect()).collect();
        assert_eq!(got, want, "{rel}");
        assert_eq!(f.has_facts(rel), !rows.is_empty());
    }
    // Lattices.
    assert_eq!(f.lattices().len(), lattice.len());
    for (rel, l) in &lattice {
        let d = f.lattice_decl(rel).unwrap_or_else(|| panic!("lattice {rel}"));
        assert_eq!((d.keys, f.text(d.ty), d.merge.map(|m| f.text(m)), d.widen), (l.keys, l.ty.as_str(), l.merge.as_deref(), l.widen), "{rel}");
        assert_eq!(f.lattice_name(rel), Some(l.lattice()));
        assert!(f.lattice(rel));
    }
    // Natives, each of its fields.
    assert_eq!(f.natives().len(), natives.len());
    for (rel, n) in &natives {
        let got = f.native(rel).unwrap_or_else(|| panic!("native {rel}"));
        assert_eq!(&native_tree(&f, &got.spec), n, "{rel}");
        assert_eq!(got.reads.iter().map(|&r| f.rel_name(r)).collect::<Vec<_>>(), n.reads(), "{rel}");
    }
    // Answers, in order.
    assert_eq!(f.answers().len(), answers.values().map(Vec::len).sum::<usize>());
    for (req, ps) in &answers {
        assert_eq!(f.answers_of(req), strs(ps), "{req}");
        assert!(f.request(req) && f.answer(req));
    }
    // Gates, in order.
    let gates: Vec<(String, Option<String>, String, usize, Vec<String>, Vec<String>)> = f.gates().iter().map(|g| (f.text(g.key).to_owned(), g.src.map(|s| f.text(s).to_owned()), g.rel.map_or("", |r| f.rel_name(r)).to_owned(), g.level, g.quiet.iter().map(|&r| f.rel_name(r).to_owned()).collect(), g.deps.iter().map(|&r| f.rel_name(r).to_owned()).collect())).collect();
    let want: Vec<_> = pgates.iter().map(|(k, g)| (k.clone(), g.src.clone(), g.rel.clone(), g.level, g.quiet.clone(), g.deps.clone())).collect();
    assert_eq!(gates, want);
    for (k, _) in &pgates {
        assert!(f.gate(k) && f.demanded_or_gate(k));
    }
    // Qualifiers, tapes, kind sets, nullary and demand.
    for (rel, ws) in &qualifiers {
        assert_eq!(f.qualifiers_of(rel).into_iter().collect::<BTreeSet<_>>(), ws.iter().map(String::as_str).collect(), "{rel}");
        for w in ws {
            assert!(f.qualified(rel, w));
        }
    }
    assert_eq!(f.about.qualifiers.len(), qualifiers.values().map(BTreeSet::len).sum::<usize>());
    for (rel, cols) in &tapes {
        assert_eq!(f.tapes_of(rel), cols.iter().copied().collect::<Vec<_>>(), "{rel}");
    }
    assert_eq!(f.tapes().len(), tapes.values().map(BTreeSet::len).sum::<usize>());
    assert_eq!(f.kindsets().len(), kindsets.len());
    for (set, kinds) in &kindsets {
        let got: Vec<&str> = f.kindset(set).unwrap_or_else(|| panic!("kind set {set}")).iter().map(|&k| f.text(k)).collect();
        assert_eq!(got, strs(kinds), "{set}");
    }
    assert_eq!(f.nullaries().collect::<BTreeSet<_>>(), nullary.iter().map(String::as_str).collect());
    for r in &nullary {
        assert!(f.nullary(r));
    }
    assert_eq!(f.demand().collect::<Vec<_>>(), strs(&demand));
    for r in &demand {
        assert!(f.demanded(r));
    }
    // Shapes, in order.
    assert_eq!(f.shapes().iter().map(|s| (f.text(s.head), f.text(s.lit), f.text(s.text))).collect::<Vec<_>>(), shapes.iter().map(|s| (s.head.as_str(), s.lit.as_str(), s.text.as_str())).collect::<Vec<_>>());
}

/// A version's fields (`plan::fields::Fields`, read once from its `Placed`) say what its program's readers say: on the
/// pack programs, and on programs with a field of every kind.
#[test]
fn a_versions_fields_say_what_its_readers_say() {
    let mut n = 0;
    for (_, prog) in pack_programs() {
        fields_agree(&prog);
        n += 1;
    }
    assert!(n > 50, "{n}");
    fields_agree(&every_part());
    fields_agree(&analysed());
    fields_agree(&every_field());
}

/// A relation's identity (`plan::identities`, 0173 step 6) is of its rules' and parts' hashes as the version keeps
/// them: the same for the same program made again (parsed again, or derived with its rules copied), and another when
/// a rule of its stratum, its declaration or its facts change.
#[test]
fn a_relations_identity_is_its_versions_hashes() {
    use crate::code_rule_dsl::logic::plan::{self, body, body::Placed, fields::Fields};
    let ids = |text: &str| {
        let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
        let placed = Placed::whole(v.clone());
        let f = Fields::of(&placed).unwrap();
        let vs = body::Versions::of_placed(placed.clone());
        let rules: Vec<_> = (0..v.rules().count()).map(body::RuleKey::program).collect();
        let sccs = [BTreeSet::from(["r".to_owned()])];
        (plan::identities(f, &vs, &sccs, &[rules])["r"], plan::leaf_identity(f, "e"))
    };
    let base = ".decl e(x: number, y: number)\n.input e\n.decl r(x: number, y: number)\n.output r\ne(1, 2).\nr(x, y) :- e(x, y).\n";
    let (r, e) = ids(base);
    assert_eq!(ids(base), (r, e), "made again, the same");
    assert_ne!(ids(&base.replace("r(x, y) :- e(x, y)", "r(y, x) :- e(x, y)")).0, r, "a rule edit");
    assert_ne!(ids(&base.replace(".decl r(x: number, y: number)", ".decl r(x: number, z: number)")).0, r, "a declaration edit");
    let (r2, e2) = ids(&base.replace("e(1, 2).", "e(1, 3)."));
    assert_ne!(e2, e, "a fact edit changes the input's identity");
    assert_ne!(r2, r, "and every identity that reads it");
}

// ---- the pass manager (decision 0281) ----

/// A program of three heads over two inputs, with stated facts: `e` read by `p` and `q`, `f` by `r`'s twelve rules.
fn managed() -> Arc<Version> {
    let mut b = Builder::base();
    for (n, k) in [("e", 2), ("f", 1), ("p", 1), ("q", 1), ("r", 1)] {
        let cols: Vec<String> = (0..k).map(|i| format!("c{i}")).collect();
        b.fields().declare(n, &cols.iter().map(|c| (c.as_str(), "int")).collect::<Vec<_>>());
    }
    b.fields().set_facts("f", &[vec![Value::Int(1)].into()]);
    b.fields().set_inputs(&["e"]);
    b.fields().set_outputs(&["p", "r"]);
    let (x, y) = (|| T::Var("x"), || T::Var("y"));
    b.rule().head("p", &[x()]).atom("e", &[x(), T::Wild]).finish().unwrap();
    b.rule().head("q", &[y()]).atom("e", &[T::Wild, y()]).finish().unwrap();
    b.rule().head("r", &[x()]).atom("f", &[x()]).finish().unwrap();
    b.rule().head("r", &[x()]).atom("p", &[x()]).finish().unwrap();
    // Rules that read neither `e` nor `q`, so an edit of `e`'s readers is a small part of the program.
    for k in 1..9 {
        b.rule().head("r", &[T::Int(k)]).atom("f", &[T::Int(k)]).finish().unwrap();
    }
    b.finish().unwrap().0
}

/// Renames every read of `from` to `to`, and counts the rules it is shown.
struct Rename {
    from: RelId,
    to: &'static str,
    seen: usize,
}
impl super::Rewrite for Rename {
    fn atom_as(&mut self, a: super::AtomRef<'_>) -> Option<(String, bool, bool)> {
        (a.rel() == self.from).then(|| (self.to.to_owned(), a.neg(), a.gated()))
    }
}
impl super::manager::RulePass for Rename {
    const NAME: &'static str = "rename";
    // It reads its rule alone: a rerun visits an edit's added rules (`seen` is the test's count, not a result).
    const WHOLE: super::manager::Reads = super::manager::Reads::DECLS;
    fn begin(&mut self, _: &super::manager::Cx<'_>) -> Result<super::manager::Select, String> {
        Ok(super::manager::Select::Readers([self.from].into_iter().collect()))
    }
    fn rule(&mut self, _: &super::manager::Cx<'_>, _: super::RuleRef<'_>, out: &mut super::manager::RuleOut<'_, '_>) -> Result<super::manager::Step, String> {
        self.seen += 1;
        let mut rw = Rename { from: self.from, to: self.to, seen: 0 };
        out.rewrite(&mut rw)
    }
}

/// A rule pass costs the rules it selects: renaming one relation's reads visits its readers alone, and the edit is
/// an overlay of the version whose delta is those rules out and their rewrites in, never a copy of the program.
#[test]
fn a_rule_pass_costs_the_rules_it_selects() {
    use super::manager::{Delta, Form, Pipeline, Reads};
    let v = managed();
    let e = v.rel("e").unwrap();
    let readers: Vec<RuleId> = v.readers(e).map(|(r, _)| r.id()).collect();
    assert_eq!(readers.len(), 2);
    let mut pass = Rename { from: e, to: "e2", seen: 0 };
    let mut p = Pipeline::new("test", v.clone());
    p.rules(&mut pass);
    let (out, stages) = p.finish().unwrap();
    assert_eq!(pass.seen, 2, "visited its selection alone");
    assert_eq!((stages[0].visited, stages[0].form), (2, Form::Overlay));
    let d = Delta::of(&v, &out);
    assert_eq!(d.removed.iter().collect::<Vec<_>>(), readers);
    assert_eq!(d.added.len(), 2);
    assert!(!d.rebuilt);
    assert_eq!(d.touched, Reads::RULES, "no field changed");
    let e2 = out.rel("e2").unwrap();
    assert_eq!(out.readers(e2).count(), 2);
    assert_eq!(out.readers(e).count(), 0);
    assert_eq!(out.size(), v.size());
}

/// Counts the rules of `rel` and keeps every rule: a pass that changes nothing.
struct Keep;
impl super::manager::RulePass for Keep {
    const NAME: &'static str = "keep";
    fn rule(&mut self, _: &super::manager::Cx<'_>, _: super::RuleRef<'_>, _: &mut super::manager::RuleOut<'_, '_>) -> Result<super::manager::Step, String> {
        Ok(super::manager::Step::Keep)
    }
}

/// Drops the stated facts of `f`: an edit of facts alone.
struct DropFacts;
impl super::manager::ProgramPass for DropFacts {
    const NAME: &'static str = "drop facts";
    fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
        w.on()?.fields().drop_facts("f");
        cx.decide("dropped", || "f".into());
        Ok(())
    }
}

/// Drops the rules of `q`.
struct DropQ;
impl super::manager::ProgramPass for DropQ {
    const NAME: &'static str = "drop q";
    fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
        let q = cx.v().rel("q").unwrap();
        w.keep(|r| r.head().rel() != q)
    }
}

/// An analysis is computed once per version, held by it, and carried to an edit's output when the edit misses what
/// it reads: the strata over a facts edit, not over a rules edit; a pass that keeps every rule leaves the version as
/// it was. A debug build checks each carried one against a fresh computation.
#[test]
fn analyses_carry_over_edits_that_miss_what_they_read() {
    use super::analyses::{Needed, Strata};
    use super::manager::{Form, Pipeline, Reads};
    let v = managed();
    let mut p = Pipeline::new("test", v.clone());
    let s0 = p.get::<Strata>().unwrap();
    p.get::<Needed>().unwrap();
    assert_eq!(v.analyses().computed("strata"), 1);
    p.rules(&mut Keep);
    assert!(Arc::ptr_eq(p.version(), &v), "a pass that keeps every rule makes no version");
    p.program(&mut DropFacts);
    assert_eq!(p.stages()[1].touched, Reads::FACTS);
    let facts_edit = p.version().clone();
    let s1 = p.get::<Strata>().unwrap();
    assert!(Arc::ptr_eq(&s0, &s1), "carried, not computed again");
    assert_eq!(facts_edit.analyses().computed("strata"), 0);
    p.program(&mut DropQ);
    assert_eq!((p.stages()[2].form, p.stages()[2].touched), (Form::View, Reads::RULES));
    assert!(!p.store().holds::<Strata>() && p.store().holds::<super::About>(), "a rules edit drops the strata, keeps the about rows");
    p.get::<Strata>().unwrap();
    assert_eq!(p.store().computed("strata"), 1, "a rules edit computes it again");
    assert_eq!(p.store().computed("about"), 0, "the about rows carried through all three");
    assert_eq!(p.decisions().len(), 1);
    let (out, stages) = p.finish().unwrap();
    assert_eq!(stages.iter().map(|s| s.form).collect::<Vec<_>>(), [Form::Unchanged, Form::Overlay, Form::View]);
    assert_eq!(out.size(), 11);
}

/// A version's analyses are its own: every `Placed` of it reads one computation of its components and fields, as
/// the run's readers make a `Placed` of a version again and again (`Lowered::placed`).
#[test]
fn every_placed_of_a_version_shares_its_analyses() {
    use crate::code_rule_dsl::logic::plan::{body::Placed, fields::Fields};
    let v = managed();
    let a = Placed::whole(v.clone()).components().unwrap();
    let b = Placed::whole(v.clone()).components().unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    Fields::of(&Placed::whole(v.clone())).unwrap();
    Fields::of(&Placed::whole(v.clone())).unwrap();
    assert_eq!((v.analyses().computed("components"), v.analyses().computed("fields"), v.analyses().computed("about")), (1, 1, 1));
}

/// A rule pass that writes for a rule it says it kept fails its pipeline, named by the pass.
#[test]
fn a_pass_that_writes_for_a_kept_rule_fails() {
    struct Liar;
    impl super::manager::RulePass for Liar {
        const NAME: &'static str = "liar";
        fn rule(&mut self, _: &super::manager::Cx<'_>, _: super::RuleRef<'_>, out: &mut super::manager::RuleOut<'_, '_>) -> Result<super::manager::Step, String> {
            out.rule().head("p", &[T::Int(1)]).finish()?;
            Ok(super::manager::Step::Keep)
        }
    }
    let mut p = super::manager::Pipeline::new("test", managed());
    p.rules(&mut Liar);
    let e = p.finish().err().expect("refused");
    assert!(e.contains("liar") && e.contains("kept"), "{e}");
}

/// Renames the reads of `from` (by name) in every rule it is shown, keeping the rest: a pass over all rules, so its
/// output is a derivation. Asks for `Needed` when `needs`, so a rerun compares it.
struct RenameAll {
    from: &'static str,
    to: &'static str,
    needs: bool,
    seen: usize,
}
impl super::manager::RulePass for RenameAll {
    const NAME: &'static str = "rename all";
    const WHOLE: super::manager::Reads = super::manager::Reads::DECLS;
    fn begin(&mut self, cx: &super::manager::Cx<'_>) -> Result<super::manager::Select, String> {
        if self.needs {
            cx.get::<super::analyses::Needed>()?;
        }
        Ok(super::manager::Select::All)
    }
    fn rule(&mut self, cx: &super::manager::Cx<'_>, r: super::RuleRef<'_>, out: &mut super::manager::RuleOut<'_, '_>) -> Result<super::manager::Step, String> {
        self.seen += 1;
        let from = cx.v().rel(self.from).unwrap();
        if !r.reads().any(|a| a.rel() == from) {
            return Ok(super::manager::Step::Keep);
        }
        out.rewrite(&mut Rename { from, to: self.to, seen: 0 })
    }
}

/// `v` less rules `gone`, plus `add`: an overlay, as a session's edit of a program is.
fn edited(v: &Arc<Version>, gone: &[RuleId], add: &[Add]) -> Arc<Version> {
    let mut b = Builder::on(v);
    for &g in gone {
        b.remove(g);
    }
    for r in add {
        let x = || T::Var("x");
        match *r {
            Add::ReadsE(head) => b.rule().head(head, &[x()]).atom("e", &[x(), T::Wild]),
            Add::ROfF(k) => b.rule().head("r", &[T::Int(k)]).atom("f", &[T::Int(k)]),
            Add::PReadsQ => b.rule().head("p", &[x()]).atom("q", &[x()]),
        }
        .finish()
        .unwrap();
    }
    let (out, made) = b.finish().unwrap();
    assert_eq!(made, Made::Overlay);
    out
}

/// A rule an edit adds: `head(x) :- e(x, _)`, `r(k) :- f(k)`, or `p(x) :- q(x)`.
#[derive(Clone, Copy)]
enum Add {
    ReadsE(&'static str),
    ROfF(i64),
    PReadsQ,
}

/// A rule pass run again over an edit of its input (0281 §3) visits the edit's added rules alone, drops what came
/// from its removed ones, and makes the program a run from scratch makes, over two edits in a row: as an edit of
/// its input (`Rename`, the readers of `e`) and as a derivation (`RenameAll`, every rule).
#[test]
fn a_rule_pass_run_again_visits_the_edits_rules_alone() {
    use super::manager::{Form, Pipeline, Ran};
    let v0 = managed();
    let e = v0.rel("e").unwrap();
    let readers: Vec<RuleId> = v0.readers(e).map(|(r, _)| r.id()).collect();
    let r3 = v0.rules().nth(6).unwrap().id();
    // Edit 1: one reader of `e` and one other rule out; one reader and one other rule in.
    let v1 = edited(&v0, &[readers[0], r3], &[Add::ReadsE("q"), Add::ROfF(20)]);
    let added1: Vec<RuleId> = super::manager::Delta::of(&v0, &v1).added;
    // Edit 2: edit 1's added reader out, and one more reader in.
    let v2 = edited(&v1, &[added1[0]], &[Add::ReadsE("p")]);

    let scratch = |v: &Arc<Version>, by_rules: bool| {
        let mut p = Pipeline::new("scratch", v.clone());
        if by_rules {
            p.rules(&mut Rename { from: v.rel("e").unwrap(), to: "e2", seen: 0 });
        } else {
            p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        }
        p.finish().unwrap().0
    };
    for by_rules in [true, false] {
        let mut p = Pipeline::keeping("test", v0.clone());
        let run = |p: &mut Pipeline| -> usize {
            if by_rules {
                let mut pass = Rename { from: e, to: "e2", seen: 0 };
                p.rules(&mut pass);
                pass.seen
            } else {
                let mut pass = RenameAll { from: "e", to: "e2", needs: false, seen: 0 };
                p.rules(&mut pass);
                pass.seen
            }
        };
        assert_eq!(run(&mut p), if by_rules { 2 } else { 12 });
        let mut last = p.done().unwrap();
        assert_eq!(last.stages()[0].form, if by_rules { Form::Overlay } else { Form::Derived });
        for (v, visits) in [(&v1, if by_rules { 1 } else { 2 }), (&v2, 1)] {
            let mut p = Pipeline::again(last, v.clone());
            assert_eq!(run(&mut p), visits, "visits the edit's added rules alone");
            last = p.done().unwrap();
            let s = &last.stages()[0];
            assert_eq!((s.ran, s.visited), (Ran::Delta, visits));
            let want = scratch(v, by_rules);
            let got = last.output();
            assert_eq!((got.digest(), got.size()), (want.digest(), want.size()), "the program a run from scratch makes");
            assert_eq!(got.readers(got.rel("e").unwrap()).count(), 0);
        }
        // The same input again: the step is reused.
        let mut p = Pipeline::again(last, v2.clone());
        let before = p.version().clone();
        assert_eq!(run(&mut p), 0);
        let again = p.done().unwrap();
        assert_eq!(again.stages()[0].ran, Ran::Reused);
        assert!(Arc::ptr_eq(&before, &v2));
    }
}

/// A pipeline's run kept in a store (decision 0333) and opened in a new process reruns on an edit there as it does
/// in its own: the edit rebased onto the stored run's input by rule hash, each rule pass visits the edit's added
/// rules alone, the analyses it asked for are read back from the store for the last input, not computed, and the
/// output is a run from scratch's.
#[test]
fn a_stored_run_reruns_on_an_edit_in_a_new_process() {
    use super::manager::{Pipeline, Ran, Runs, analysis, rebase};
    use crate::code_rule_dsl::logic::program_text::StoredRuns;
    use crate::content_store::ContentStore;
    let v0 = managed();
    let e = v0.rel("e").unwrap();
    let readers: Vec<RuleId> = v0.readers(e).map(|(r, _)| r.id()).collect();
    let r3 = v0.rules().nth(6).unwrap().id();
    let v1 = edited(&v0, &[readers[0], r3], &[Add::ReadsE("q"), Add::ROfF(20)]);
    // Each process reads its program afresh: no version of the last process's is shared.
    let fresh = |v: &Arc<Version>| Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
    let scratch = |v: &Arc<Version>| {
        let mut p = Pipeline::new("scratch", v.clone());
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        p.finish().unwrap().0
    };
    let path = crate::code_rule_dsl::fixtures::temp();
    let key = [7u8; 32];
    // The first process: a run from scratch, kept.
    {
        let store = ContentStore::open(&path).unwrap();
        let mut p = Pipeline::keeping("test", fresh(&v0));
        // An analysis of the input, kept with the run.
        p.get::<super::analyses::Needed>().unwrap();
        let mut pass = RenameAll { from: "e", to: "e2", needs: false, seen: 0 };
        p.rules(&mut pass);
        assert_eq!(pass.seen, 12);
        StoredRuns(&store).keep(&key, &p.done().unwrap()).unwrap();
        drop(store);
        ContentStore::forget(&path);
    }
    // The next: the run opened, the edit rebased onto its input, run again.
    let store = ContentStore::open(&path).unwrap();
    let last = StoredRuns(&store).open(&key, "test").unwrap().expect("the run kept");
    let input = last.input().clone();
    let v = rebase(&input, &fresh(&v1)).unwrap().expect("an edit of the last input");
    assert_eq!(super::manager::Delta::of(&input, &v).size(), 4, "two rules out, two in");
    let mut p = Pipeline::again(last, v.clone());
    let mut pass = RenameAll { from: "e", to: "e2", needs: false, seen: 0 };
    p.rules(&mut pass);
    assert_eq!(pass.seen, 2, "visits the edit's added rules alone");
    let run = p.done().unwrap();
    assert_eq!(run.stages()[0].ran, Ran::Delta);
    analysis::<super::analyses::Needed>(&input).unwrap();
    assert_eq!((input.analyses().loaded("needed"), input.analyses().computed("needed")), (1, 0), "the last input's analysis read back, not computed");
    let want = scratch(&v1);
    assert_eq!((run.output().digest(), run.output().size()), (want.digest(), want.size()), "the program a run from scratch makes");
    drop(store);
    ContentStore::forget(&path);
}

/// `text`'s version with request record `asked` (decision 0331), as one base image, as a lowered program's is.
fn asked_base(text: &str, asked: &[(&str, Vec<BTreeSet<String>>)]) -> Arc<Version> {
    let read = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
    let mut b = Builder::on(&read);
    b.fields().set_asked(&asked.iter().map(|(r, by)| ((*r).to_owned(), by.clone())).collect());
    let (v, _) = b.finish().unwrap();
    Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap()
}

/// A rebased version (`manager::rebase`) holds the last input's rules in its order and the new program's others after
/// them: the request record is restated by rule, so each request rule keeps the readers the new program gave it
/// (decision 0331). Taken by place, `q`'s two rules swapped their readers: `a_library_held_once_is_read_by_each_policy_in_its_own_scope`
/// failed `same_program` in the runs that found their last run stored.
#[test]
fn a_rebased_version_keeps_each_request_rules_readers() {
    use std::collections::BTreeSet;
    let program = |order: &[usize], extra: &str| {
        let rules = ["q(n) :- k(n), n < 3.", "q(n) :- k(n), n > 5."];
        let mut text = String::from(".decl q(n: number)\n.decl a(n: number)\n.decl b(n: number)\n.decl k(n: number)\n.input k\n");
        for &i in order {
            text.push_str(rules[i]);
            text.push('\n');
        }
        text.push_str("a(n) :- q(n).\nb(n) :- q(n).\n");
        text.push_str(extra);
        // The rule `< 3` serves `a`, the rule `> 5` serves `b`, wherever each lies.
        asked_base(&text, &[("q", order.iter().map(|&i| BTreeSet::from([["a", "b"][i].to_owned()])).collect())])
    };
    let last = program(&[0, 1], "");
    let new = program(&[1, 0], ".decl c(n: number)\nc(n) :- a(n).\n");
    let v = super::manager::rebase(&last, &new).unwrap().expect("an edit of the last input");
    // The case: `q`'s rules lie in the last input's order, the new program's reversed.
    let first = |x: &Version| x.heads(x.rel("q").unwrap()).next().map(|r| x.show_rule(r)).unwrap();
    assert!(first(&v).contains("< 3") && first(&new).contains("> 5"), "{} / {}", first(&v), first(&new));
    assert!(super::same_program(&v, &new), "{}", super::differences(&v, &new));
}

/// A rerun runs a step whole when it cannot run on the edit alone: a program pass, a rule pass that keeps the
/// default `WHOLE`, and a rule pass whose analyses the edit changed. Across the edit, the analyses the last input
/// held are carried to the new input.
#[test]
fn a_rerun_runs_a_step_whole_when_the_edit_reaches_past_its_rules() {
    use super::analyses::Strata;
    use super::manager::{Pipeline, Ran};
    let v0 = managed();
    let mut p = Pipeline::keeping("test", v0.clone());
    p.get::<Strata>().unwrap();
    p.rules(&mut Keep).rules(&mut RenameAll { from: "f", to: "f2", needs: true, seen: 0 }).program(&mut DropQ);
    let mut last = p.done().unwrap();
    // `q` is read by an output's rule now: `Needed` changes, so the pass that asked for it runs whole.
    for (add, ran) in [(Add::ROfF(30), Ran::Delta), (Add::PReadsQ, Ran::Whole)] {
        let v = edited(last.input(), &[], &[add]);
        let mut p = Pipeline::again(last, v.clone());
        p.rules(&mut Keep).rules(&mut RenameAll { from: "f", to: "f2", needs: true, seen: 0 }).program(&mut DropQ);
        last = p.done().unwrap();
        let ran_by: Vec<Ran> = last.stages().iter().map(|s| s.ran).collect();
        assert_eq!(ran_by, [Ran::Whole, ran, Ran::Whole]);
        assert!(v.analyses().holds::<super::About>(), "carried from the last input");
        assert_eq!(v.analyses().computed("about"), 0);
    }
}

/// Drops the rule `r(k) :- f(k)` of its `k`, which is its setting (a keyed step's key holds it), and counts its runs.
struct DropR {
    k: i64,
    runs: usize,
}
impl super::manager::ProgramPass for DropR {
    const NAME: &'static str = "drop r";
    fn settings(&self) -> Vec<u8> {
        self.k.to_le_bytes().to_vec()
    }
    fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
        self.runs += 1;
        let v = cx.v().clone();
        let k = self.k.to_string();
        cx.decide("dropped r", || k.clone());
        w.keep(|r| v.show_rule(r) != format!("r({k}) :- f({k})"))
    }
}

/// Counts the runs of a pass in front of it.
struct Counted<P> {
    pass: P,
    runs: usize,
}
impl<P: super::manager::ProgramPass> super::manager::ProgramPass for Counted<P> {
    const NAME: &'static str = P::NAME;
    fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
        self.runs += 1;
        self.pass.run(cx, w)
    }
}

/// A keyed pipeline (0281 §6) stores each step under its key, and a new process (the store opened again, nothing of
/// the last one held) runs only the passes after the latest step the store holds: a changed setting reruns its pass
/// and the ones after it, a caller's key part changes every step after it, and the output is the unkeyed pipeline's.
#[test]
fn a_new_process_reruns_only_the_passes_after_a_stored_step() {
    use super::manager::{Pipeline, Ran};
    use crate::code_rule_dsl::logic::program_text::StoredSteps;
    use crate::content_store::ContentStore;
    let path = crate::code_rule_dsl::fixtures::temp();
    let v = managed();
    // One process: the store opened, the pipeline run, the store closed and forgotten.
    let run = |k: i64, part: Option<&[u8]>| -> (Arc<Version>, Vec<Ran>, [usize; 3], Vec<String>) {
        let store = ContentStore::open(&path).unwrap();
        let steps = StoredSteps(&store);
        let (mut a, mut b, mut c) = (Counted { pass: DropFacts, runs: 0 }, Counted { pass: DropQ, runs: 0 }, DropR { k, runs: 0 });
        let mut p = Pipeline::keyed("test", v.clone(), &steps);
        if let Some(part) = part {
            p.key_part("olds", part);
        }
        p.program(&mut a).program(&mut b).program(&mut c);
        let decided: Vec<String> = p.decisions().iter().map(|d| format!("{}: {}: {}", d.pass, d.what, d.detail)).collect();
        let (out, stages) = p.finish().unwrap();
        drop(store);
        ContentStore::forget(&path);
        (out, stages.iter().map(|s| s.ran).collect(), [a.runs, b.runs, c.runs], decided)
    };
    let unkeyed = |k: i64| -> Arc<Version> {
        let mut p = Pipeline::new("test", v.clone());
        p.program(&mut DropFacts).program(&mut DropQ).program(&mut DropR { k, runs: 0 });
        p.finish().unwrap().0
    };
    let same = |a: &Arc<Version>, b: &Arc<Version>| {
        assert_eq!((a.size(), a.digest()), (b.size(), b.digest()));
        assert_eq!(super::manager::identity(a), super::manager::identity(b), "the same rules, in the same order");
    };
    let want = ["drop facts: dropped: f".to_owned()];

    let (out, ran, runs, decided) = run(1, None);
    assert_eq!(runs, [1, 1, 1], "a store holding nothing runs every pass");
    assert_eq!(ran, [Ran::Whole; 3]);
    same(&out, &unkeyed(1));
    assert_eq!(decided[..1], want);

    // A new process, the last pass's setting changed: the first two steps are read from the store, the last runs.
    let (out, ran, runs, decided) = run(2, None);
    assert_eq!(runs, [0, 0, 1], "only the pass after the stored steps runs");
    assert_eq!(ran, [Ran::Stored, Ran::Stored, Ran::Whole]);
    same(&out, &unkeyed(2));
    assert_eq!(decided, [want[0].clone(), "drop r: dropped r: 2".to_owned()], "the stored steps' decisions read back");

    // Again, nothing changed: every step is stored, and the output is opened from the last one's image.
    let (out, ran, runs, _) = run(2, None);
    assert_eq!(runs, [0, 0, 0]);
    assert_eq!(ran, [Ran::Stored; 3]);
    same(&out, &unkeyed(2));

    // A caller's key part (0298's old rows' signature) in front of every step: none is found, each runs.
    let (out, _, runs, _) = run(2, Some(&7u64.to_le_bytes()));
    assert_eq!(runs, [1, 1, 1]);
    same(&out, &unkeyed(2));
    let (_, _, runs, _) = run(2, Some(&7u64.to_le_bytes()));
    assert_eq!(runs, [0, 0, 0], "found again under the same part");
}

/// A run that keeps a pipeline's steps, and a later run that finds them, each name the steps' sources in its manifest
/// (Q146, decision 0480 §7): the step facts and each step's image, the same sources both times, so a collection between
/// the two runs keeps them. Read as the run's manifest reads them, by an item at the record's end.
#[test]
fn a_found_step_names_its_sources_as_its_keeper_did() {
    use super::manager::{Pipeline, Ran};
    use crate::code_rule_dsl::logic::program_text::StoredSteps;
    use crate::content_store::{ContentStore, SourceId};
    static NAMED: std::sync::Mutex<Vec<SourceId>> = std::sync::Mutex::new(Vec::new());
    fn takes(cx: &crate::content_store::record::Apply<'_>, _: crate::content_store::record::codec::Body) -> crate::content_store::StoreResult<()> {
        *NAMED.lock().unwrap() = cx.store.take_named_sources();
        Ok(())
    }
    let path = crate::code_rule_dsl::fixtures::temp();
    let v = managed();
    let run = || -> (Vec<Ran>, Vec<SourceId>) {
        let store = ContentStore::open(&path).unwrap();
        let ran = {
            let _waves = crate::content_store::write_back::run_waves();
            let steps = StoredSteps(&store);
            let mut p = Pipeline::keyed("test", v.clone(), &steps);
            p.program(&mut DropFacts).program(&mut DropQ);
            let (_, stages) = p.finish().unwrap();
            store.record("test/the manifest's names", Vec::new(), &(), None, takes);
            store.written().unwrap();
            stages.iter().map(|s| s.ran).collect()
        };
        drop(store);
        ContentStore::forget(&path);
        let mut named = std::mem::take(&mut *NAMED.lock().unwrap());
        named.sort_by_key(|id| format!("{id:?}"));
        named.dedup();
        (ran, named)
    };
    let (ran, kept) = run();
    assert_eq!(ran, [Ran::Whole; 2]);
    assert!(kept.len() >= 3, "each step's facts and the images: {kept:?}");
    let (ran, found) = run();
    assert_eq!(ran, [Ran::Stored; 2], "both steps found");
    assert_eq!(found, kept, "the run that finds them names what the run that kept them named");
}

/// A chain of edits past `REBASE_DEPTH` runs again over each one (decision 0385): at that depth the last run is made
/// shallow, each deep version flat (a view of one base, its ids, size and digest the deep one's), so the next edit is
/// still an edit of its input; the output is a run from scratch's every time.
#[test]
fn a_chain_of_edits_past_the_rebase_depth_runs_again_over_each() {
    use super::manager::{Pipeline, REBASE_DEPTH, Ran, rebase};
    let v0 = managed();
    let fresh = |v: &Arc<Version>| Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
    let scratch = |v: &Arc<Version>| {
        let mut p = Pipeline::new("scratch", v.clone());
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        p.finish().unwrap().0
    };
    let mut p = Pipeline::keeping("test", fresh(&v0));
    p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
    let mut last = p.done().unwrap();
    let mut prog = v0.clone();
    let mut flattened = 0;
    for i in 0..2 * REBASE_DEPTH + 2 {
        let gone = prog.rules().nth(6).unwrap().id();
        prog = fresh(&edited(&prog, &[gone], &[Add::ROfF(30 + i as i64)]));
        let deep = last.input().clone();
        last = last.shallow();
        if deep.depth() >= REBASE_DEPTH {
            flattened += 1;
            let flat = last.input();
            assert_eq!(flat.depth(), 0, "edit {i}: made flat");
            assert_eq!((flat.size(), flat.digest()), (deep.size(), deep.digest()), "edit {i}: the same program");
            assert!(flat.rules().map(|r| (r.id(), r.hash())).eq(deep.rules().map(|r| (r.id(), r.hash()))), "edit {i}: the same rules under the same ids");
        }
        let v = rebase(last.input(), &prog).unwrap().expect("an edit of the last input, however many came before");
        let mut p = Pipeline::again(last, v);
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        last = p.done().unwrap();
        assert_eq!(last.stages()[0].ran, Ran::Delta, "edit {i}: run again over the edit");
        let want = scratch(&prog);
        assert_eq!((last.output().digest(), last.output().size()), (want.digest(), want.size()), "edit {i}: the program a run from scratch makes: {}", super::differences(&want, last.output()));
    }
    assert!(flattened >= 2, "made flat {flattened} times");
}

/// `program_facts`' own tests, which read and write through the image: above the image's crate (decision 0377).
mod program_facts_tests {
    /// The encoding names each string by its first use and puts the table first: the bytes the two-pass writer made
    /// (strings counted, then the rows written), and they decode to the facts.
    #[test]
    fn facts_encode_as_two_passes_did_and_decode_back() {
        use crate::code_rule_dsl::logic::front::Value;
        use crate::code_rule_dsl::logic::program_facts::Facts;
        let mut facts = Facts::new();
        facts.insert("b".into(), vec![vec![Value::Str("x".into()), Value::Int(-3)].into(), vec![Value::Str("b".into()), Value::Str("y".into())].into()]);
        facts.insert("a".into(), vec![vec![Value::Str("y".into()), Value::Str("z".into()), Value::Int(7)].into()]);
        let mut want = Vec::new();
        let u = |out: &mut Vec<u8>, x: u64| out.extend_from_slice(&x.to_le_bytes());
        // Strings by first use: a, y, z, b, x.
        let table = ["a", "y", "z", "b", "x"];
        u(&mut want, 5);
        for x in table {
            u(&mut want, x.len() as u64);
            want.extend_from_slice(x.as_bytes());
        }
        u(&mut want, 2);
        let id = |x: &str| table.iter().position(|t| *t == x).unwrap() as u64;
        for (rel, rows) in &facts {
            u(&mut want, id(rel));
            u(&mut want, rows.len() as u64);
            for row in rows {
                u(&mut want, row.len() as u64);
                for v in row {
                    match v {
                        Value::Int(i) => {
                            want.push(0);
                            want.extend_from_slice(&i.to_le_bytes());
                        }
                        Value::Branded(b) => {
                            want.push(2 + b.class as u8);
                            want.extend_from_slice(&u64::from(b.atom).to_le_bytes());
                        }
                        Value::Str(x) => {
                            want.push(1);
                            u(&mut want, id(x));
                        }
                    }
                }
            }
        }
        let got = crate::code_rule_dsl::logic::program_facts::encode(&facts);
        assert_eq!(got, want);
        assert_eq!(crate::code_rule_dsl::logic::program_facts::decode(&got).unwrap(), facts);
    }

    /// The documents a program reads (`doc_scopes`) and its text globs survive its image: a stored program read back
    /// reads the same documents, not all of them.
    #[test]
    fn document_scopes_read_back_as_written() {
        use crate::code_rule_dsl::logic::program_facts::Field;
        use crate::code_rule_dsl::logic::program_image::{Builder, Version, fields::ProgramFields};
        for scopes in [None, Some(vec![(vec!["impl/**".to_owned(), "api/**".to_owned()], vec!["**/tests/**".to_owned()]), (vec![], vec![])])] {
            let globs = if scopes.is_some() { vec!["**/*.md".to_owned(), "docs/*.rst".to_owned()] } else { Vec::new() };
            let mut b = Builder::base();
            b.fields().declare("a", &[("n", "node")]);
            b.fields().set_outputs(&["a"]);
            b.fields().set_field(Field::DocScopes(scopes.as_deref()));
            b.fields().set_field(Field::TextGlobs(&globs));
            let v = b.finish().unwrap().0;
            let back = Version::open(v.base_bytes().unwrap()).unwrap();
            assert_eq!(ProgramFields::doc_scopes(&*back), scopes);
            assert_eq!(ProgramFields::text_globs(&*back), globs);
        }
    }

    /// A relation the program has facts of but no rows (a constant folded to nothing) reads back as one: a stored
    /// program is the program prepared, and its file-local part has the same memo key (`memo::program_digest`).
    #[test]
    fn a_relation_of_no_facts_reads_back_as_one() {
        use crate::code_rule_dsl::logic::front::Value;
        use crate::code_rule_dsl::logic::program_image::{Builder, Version, fields::ProgramFields};
        let mut b = Builder::base();
        b.fields().set_facts("none", &[]);
        b.fields().set_facts("one", &[vec![Value::Int(1)].into()]);
        let v = b.finish().unwrap().0;
        let back = Version::open(v.base_bytes().unwrap()).unwrap();
        assert_eq!(ProgramFields::fact_names(&*back), ["none", "one"]);
        assert!(ProgramFields::states_facts(&*back, "none") && ProgramFields::fact_rows(&*back, "none").is_empty());
        assert_eq!(ProgramFields::fact_rows(&*back, "one"), [crate::code_rule_dsl::logic::front::List::from(vec![Value::Int(1)])]);
    }

    /// The AST patterns a program matches survive its image: a stored program read back (a prepared program,
    /// O-stored-patterns) still asks the batch for `ast_match` rows. Without them, a run that reuses it finds no
    /// pattern matches.
    #[test]
    fn ast_patterns_read_back_as_written() {
        use crate::code_rule_dsl::logic::program_facts::Field;
        use crate::code_rule_dsl::logic::program_image::{Builder, Version, fields::ProgramFields};
        let patterns: std::collections::BTreeSet<String> = ["$O[:-$K]".to_owned(), "print($$$A)".to_owned()].into();
        let mut b = Builder::base();
        b.fields().declare("a", &[("n", "node")]);
        b.fields().set_outputs(&["a"]);
        b.fields().set_field(Field::AstPatterns(&patterns));
        let v = b.finish().unwrap().0;
        let back = Version::open(v.base_bytes().unwrap()).unwrap();
        assert_eq!(ProgramFields::ast_patterns(&*back), patterns);
    }
}


/// `Builder::rewrite` renames an aggregate's result variable through `Rewrite::var`, as it renames the variable's
/// other occurrences: before, the result kept its name, so a rule whose variables were renamed (a library module's
/// helpers renumbered at its link, decision 0377) read `$rr_agg17 = count { … }, 0 = $rr_agg21`.
#[test]
fn a_rewrite_renames_an_aggregates_result_too() {
    use super::{Rewrite, Slot};
    let v = crate::code_rule_dsl::logic::rr::parse_version(".decl e(x: number)\n.input e\n.decl out(n: number)\nout(n) :- n = count : { e(_) }, n > 1.\n.output out\n").unwrap();
    let r = v.rules().find(|r| v.rel_name(r.head().rel()) == "out").expect("out's rule");
    let n = r.vars().position(|a| v.text(a) == "n").expect("its variable n");
    let mut b = Builder::base();
    let m = b.atom("m");
    struct Renamed(Slot, AtomId);
    impl Rewrite for Renamed {
        fn var(&mut self, s: Slot) -> Option<AtomId> {
            (s == self.0).then_some(self.1)
        }
    }
    b.rewrite(&v, r, &mut Renamed(Slot(n as u32), m)).finish().unwrap();
    let w = b.finish().unwrap().0;
    let shown = w.show();
    assert!(shown.contains("$m = count") && !shown.contains("$n"), "{shown}");
}

/// `managed()` with a request record (decision 0174's `asked` rows): `q`'s one rule serves `p`'s readers. A base of
/// its own, as a process reads it.
fn managed_asked() -> Arc<Version> {
    let v = managed();
    let mut b = v.builder();
    b.fields().set_asked(&std::collections::BTreeMap::from([("q".to_owned(), vec![std::collections::BTreeSet::from(["p".to_owned()])])]));
    let (o, _) = b.finish().unwrap();
    Version::open(o.fold().unwrap().base_bytes().unwrap()).unwrap()
}

/// `v` with request record `asked`, as one base image (`managed_asked`'s way).
fn asked_of(v: &Arc<Version>, asked: &super::about::Asked) -> Arc<Version> {
    let mut b = v.builder();
    b.fields().set_asked(asked);
    let (o, _) = b.finish().unwrap();
    Version::open(o.fold().unwrap().base_bytes().unwrap()).unwrap()
}

/// The request record over a rerun (decision 0393): a step run again over an edit onto its last output carries the
/// last run's record of that step on, so the run's output states the record a run from scratch states. An edit that
/// adds a request's rule runs the step whole, its record carried from its input.
#[test]
fn a_request_record_over_an_edit_is_a_run_from_scratchs() {
    use super::manager::{Pipeline, Ran, rebase};
    let fresh = |v: &Arc<Version>| Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
    let scratch = |v: &Arc<Version>| {
        let mut p = Pipeline::new("scratch", v.clone());
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        p.finish().unwrap().0
    };
    let v0 = managed_asked();
    assert_eq!(v0.asked().get("q").map(Vec::len), Some(1), "the input states a record");
    let mut p = Pipeline::keeping("test", v0.clone());
    p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
    let mut last = p.done().unwrap();
    assert_eq!(last.output().asked(), scratch(&v0).asked());
    let mut prog = v0.clone();
    for (i, (add, ran)) in [(Add::ROfF(30), Ran::Delta), (Add::ROfF(31), Ran::Delta), (Add::ReadsE("q"), Ran::Whole), (Add::ROfF(32), Ran::Delta)].into_iter().enumerate() {
        let gone: Vec<RuleId> = if ran == Ran::Delta { vec![prog.rules().filter(|r| prog.rel_name(r.head().rel()) == "r").nth(5).unwrap().id()] } else { Vec::new() };
        prog = fresh(&edited(&prog, &gone, &[add]));
        if ran == Ran::Whole {
            // The added rule is `q`'s second: its record is stated again, an entry a rule, as a lowering states it.
            // A record of another length gates nothing (`AskedByRule::of`), and there would be none to carry.
            let by = |r: &str| std::collections::BTreeSet::from([r.to_owned()]);
            prog = asked_of(&prog, &std::collections::BTreeMap::from([("q".to_owned(), vec![by("p"), by("q")])]));
        }
        let v = rebase(last.input(), &prog).unwrap().expect("an edit of the last input");
        let mut p = Pipeline::again(last, v);
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        last = p.done().unwrap();
        assert_eq!(last.stages()[0].ran, ran, "edit {i}");
        let want = scratch(&prog);
        assert!(!want.asked().is_empty(), "edit {i}: a record to carry");
        assert_eq!(last.output().asked(), want.asked(), "edit {i}: the record a run from scratch states");
        assert_eq!((last.output().digest(), last.output().size()), (want.digest(), want.size()), "edit {i}: the program a run from scratch makes: {}", super::differences(&want, last.output()));
    }
}

/// A stored run is read where the store placed it (decision 0394): its images, its analyses' rows and its facts in
/// one segment, uncompressed, read there mapped, and each version opened over the mapping, not decompressed, hashed
/// or copied. An image stored before the run (a prepared program's, `store_encoded`) is in a segment from its write,
/// as every source is (store floor 7), and is read there before the keep and after it. Its request record is kept
/// with its steps (decision 0393), so a rerun in a new process over an edit states a run from
/// scratch's.
#[test]
fn a_stored_run_is_read_in_place() {
    use super::manager::{Pipeline, Ran, Runs, rebase};
    use crate::code_rule_dsl::logic::program_text::{Opened, StoredRuns, image_key};
    use crate::content_store::ContentStore;
    use sha2::{Digest as _, Sha256};
    let fresh = |v: &Arc<Version>| Version::open(v.fold().unwrap().base_bytes().unwrap()).unwrap();
    let scratch = |v: &Arc<Version>| {
        let mut p = Pipeline::new("scratch", v.clone());
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        p.finish().unwrap().0
    };
    let v0 = managed_asked();
    let path = crate::code_rule_dsl::fixtures::temp();
    let key = [9u8; 32];
    {
        let store = ContentStore::open(&path).unwrap();
        let bytes = v0.base_bytes().unwrap();
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        store.put_source(bytes, &[image_key(&digest)]).unwrap();
        let held = |store: &ContentStore| store.sources_by_alias_in_place(&[image_key(&digest)]).unwrap().remove(0).expect("the input's image held");
        assert!(held(&store).in_place(), "a prepared program's image is in a segment from its write");
        let mut p = Pipeline::keeping("test", v0.clone());
        p.get::<super::analyses::Needed>().unwrap();
        p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
        let run = p.done().unwrap();
        StoredRuns(&store).keep(&key, &run).unwrap();
        assert!(held(&store).in_place(), "the keep left the input's image in place");
        // Kept again under the same key: its facts kept once, no error.
        StoredRuns(&store).keep(&key, &run).unwrap();
        drop(store);
        ContentStore::forget(&path);
    }
    let store = ContentStore::open(&path).unwrap();
    let (last, opened) = StoredRuns(&store).open_counted(&key, "test").unwrap().expect("the run kept");
    assert!(opened.images >= 2, "{opened:?}");
    assert_eq!(opened, Opened { images: opened.images, in_place: opened.images, facts_in_place: true, rows_in_place: true }, "every image, the rows and the facts read in place");
    assert!(last.input().is_shared() && last.output().is_shared(), "opened over the mapping, not copied");
    let prog = fresh(&edited(&v0, &[v0.rules().nth(6).unwrap().id()], &[Add::ROfF(40)]));
    let v = rebase(last.input(), &prog).unwrap().expect("an edit of the last input");
    let mut p = Pipeline::again(last, v);
    p.rules(&mut RenameAll { from: "e", to: "e2", needs: false, seen: 0 });
    let run = p.done().unwrap();
    assert_eq!(run.stages()[0].ran, Ran::Delta);
    let want = scratch(&prog);
    assert_eq!(run.output().asked(), want.asked(), "the record a run from scratch states");
    assert_eq!((run.output().digest(), run.output().size()), (want.digest(), want.size()));
    drop(store);
    ContentStore::forget(&path);
}
/// A debug check compares a version with the tree's image as one program (`same_program`): the digest is blind to the
/// rules' order, and the request record (decision 0331), stated by a rule's place among its relation's rules, is
/// compared by rule. The same rules in another order, each with its readers, are one program; a reader on the other
/// rule is not, though its digest is the same.
#[test]
fn a_record_by_place_is_compared_by_rule() {
    use std::collections::BTreeSet;
    let text = |swap: bool| {
        let (x, y) = if swap { ("b", "a") } else { ("a", "b") };
        format!(".decl a(x: number)\n.input a\n.decl b(x: number)\n.input b\n.decl q(x: number)\n.output q\nq(x) :- {x}(x).\nq(x) :- {y}(x).\n")
    };
    let image = |swap: bool, readers: [&str; 2]| asked_base(&text(swap), &[("q", readers.iter().map(|r| BTreeSet::from([(*r).to_owned()])).collect())]);
    let (first, moved, wrong) = (image(false, ["r1", "r2"]), image(true, ["r2", "r1"]), image(true, ["r1", "r2"]));
    assert_ne!(first.digest(), moved.digest(), "the record by place differs");
    assert!(super::same_program(&first, &moved), "{}", super::differences(&first, &moved));
    // The digest alone takes it for the same: a sum over the rules, and the same rows by place.
    assert_eq!(first.digest(), wrong.digest());
    assert!(!super::same_program(&first, &wrong), "a reader on the other rule");
}

/// A step that drops a request relation's record (`prune`'s `revived`: its readers all went, its rules kept) leaves
/// it dropped: the record the pipeline states at its end holds the relations still requested, not those it began with.
#[test]
fn a_request_record_a_step_drops_stays_dropped() {
    use std::collections::BTreeSet;
    struct DropRecord;
    impl super::manager::ProgramPass for DropRecord {
        const NAME: &'static str = "drop the record";
        fn run(&mut self, _: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
            w.on()?.fields().keep_asked(&|r| r != "q");
            Ok(())
        }
    }
    // Then a step that changes rules (drops `t`'s), after which the pipeline states the record again.
    struct DropT;
    impl super::manager::ProgramPass for DropT {
        const NAME: &'static str = "drop t";
        fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
            let t = cx.v().rel("t").unwrap();
            w.keep(|r| r.head().rel() != t)
        }
    }
    let text = ".decl a(x: number)\n.input a\n.decl b(x: number)\n.input b\n.decl q(x: number)\n.output q\nq(x) :- a(x).\nq(x) :- b(x).\n\
                .decl s(x: number)\n.output s\ns(x) :- a(x).\n.decl t(x: number)\n.output t\nt(x) :- b(x).\n";
    let v = asked_base(text, &[("q", vec![BTreeSet::from(["r1".to_owned()]), BTreeSet::from(["r2".to_owned()])]), ("s", vec![BTreeSet::from(["r3".to_owned()])])]);
    let mut pipe = super::manager::Pipeline::new("test", v);
    pipe.program(&mut DropRecord).program(&mut DropT);
    let out = pipe.finish().unwrap().0;
    let asked = out.asked();
    assert!(out.rel("q").is_some_and(|q| out.heads(q).count() == 2), "q's rules are kept");
    assert!(!asked.contains_key("q"), "{asked:?}");
    assert_eq!(asked.get("s"), Some(&vec![BTreeSet::from(["r3".to_owned()])]));
}

/// Q226: a step that changes a request relation's rules and does not state the record (here drops `q`'s first
/// rule) leaves it by place over other rules. The pipeline states it again by rule at once, so the next pass reads it
/// aligned (`request rules another subsumes` read it misaligned and kept it whole), not only at the pipeline's end.
#[test]
fn a_request_record_a_step_misaligns_is_stated_again_at_once() {
    use std::collections::BTreeSet;
    struct DropFirstQ;
    impl super::manager::ProgramPass for DropFirstQ {
        const NAME: &'static str = "drop q's first rule";
        fn run(&mut self, cx: &super::manager::Cx<'_>, w: &mut super::manager::ProgramW<'_>) -> Result<(), String> {
            let v = cx.v();
            let first = v.heads(v.rel("q").unwrap()).next().unwrap().id();
            w.keep(|r| r.id() != first)
        }
    }
    let decls = ".decl a(x: number)\n.input a\n.decl b(x: number)\n.input b\n.decl c(x: number)\n.input c\n.decl q(x: number)\n.output q\n";
    let by = |r: &str| BTreeSet::from([r.to_owned()]);
    let v = asked_base(&format!("{decls}q(x) :- a(x).\nq(x) :- b(x).\nq(x) :- c(x).\n"), &[("q", vec![by("r1"), by("r2"), by("r3")])]);
    let mut pipe = super::manager::Pipeline::new("test", v);
    pipe.program(&mut DropFirstQ);
    let now = pipe.current().unwrap().clone();
    assert_eq!(now.asked().get("q"), Some(&vec![by("r2"), by("r3")]), "stated by rule after the step");
    // As the program states it without that rule: the rule dropped, its entry with it.
    let want = asked_base(&format!("{decls}q(x) :- b(x).\nq(x) :- c(x).\n"), &[("q", vec![by("r2"), by("r3")])]);
    assert!(super::same_program(&want, &now), "{}", super::differences(&want, &now));
}

/// Each relation's layout, declared or inferred (`layouts`, decision 0361 step 3).
mod layouts {
    use crate::code_rule_dsl::logic::program_image::layouts::{Layouts, Origin};
    use crate::code_rule_dsl::logic::program_image::{RelId, Version};
    use std::collections::BTreeSet;
    use crate::code_rule_dsl::logic::front::{Fan, LayoutFact as F};
    use std::sync::Arc;

    const BASE: &str = ".decl e(p: node, c: node, i: number)\n.input e\n\
                        .layout e key(c) key(p, i) fanout(p -> c, few) order(i dense in p) tree(p -> c) local\n\
                        .decl k(n: node, s: symbol)\n.input k\n.layout k key(n) local\n";

    /// The facts of each relation `rules` derive over `BASE`.
    fn of(rules: &str) -> (Arc<Version>, Layouts) {
        let v = crate::code_rule_dsl::logic::rr::parse_version(&format!("{BASE}{rules}")).unwrap();
        let l = crate::code_rule_dsl::logic::program_image::manager::analysis::<Layouts>(&v).unwrap();
        (v.clone(), (*l).clone())
    }
    fn facts(v: &Version, l: &Layouts, rel: &str) -> Vec<F> {
        l.of(v.rel(rel).unwrap()).cloned().collect()
    }

    /// An input the transducer makes from a kind set has the engine's facts (decision 0361 step 5): `nearest`'s a key
    /// on its node, an axis's a fan-out from it; one with facts declared keeps its own.
    #[test]
    fn a_relation_made_from_a_kind_set_has_the_engines_facts() {
        use crate::code_rule_dsl::logic::front::{Fan, LayoutValues};
        let (v, l) = of(".decl rr_nearest_ks1(n: node, a: node)\n.input rr_nearest_ks1\n\
                         .decl rr_inside_ks2(n: node, a: node)\n.input rr_inside_ks2\n\
                         .decl rr_has_ks3(n: node, a: node)\n.input rr_has_ks3\n.layout rr_has_ks3 key(n)\n\
                         .decl f(n: node)\n.output f\nf(n) :- rr_nearest_ks1(n, _), rr_inside_ks2(n, _), rr_has_ks3(n, _).\n");
        let ancestors = F::Values { col: 1, values: LayoutValues::OneIn(4) };
        assert_eq!(facts(&v, &l, "rr_nearest_ks1"), [F::Key(vec![0]), F::Local, ancestors.clone()]);
        assert_eq!(facts(&v, &l, "rr_inside_ks2"), [F::Fanout { from: vec![0], to: vec![1], fan: Fan::About(2) }, F::Local, ancestors]);
        assert_eq!(facts(&v, &l, "rr_has_ks3"), [F::Key(vec![0])], "declared: its own");
        assert!(l.0.get(&v.rel("rr_nearest_ks1").unwrap()).unwrap().iter().all(|(_, o)| *o == Origin::Made));
    }

    /// A projection keeps the key its columns hold, the fan-out and the tree; a filter keeps them too, and the order
    /// but not its density; the relation renamed keeps the order dense (decision 0361 step 3).
    #[test]
    fn a_projection_or_a_filter_keeps_the_facts_its_columns_hold() {
        let (v, l) = of(".decl f(c: node, p: node)\n.output f\nf(c, p) :- e(p, c, _).\n\
                         .decl g(p: node, c: node, i: number)\n.output g\ng(p, c, i) :- e(p, c, i), i > 0.\n\
                         .decl h(p: node, c: node, i: number)\n.output h\nh(p, c, i) :- e(p, c, i).\n");
        assert_eq!(facts(&v, &l, "f"), [F::Key(vec![0]), F::Fanout { from: vec![1], to: vec![0], fan: Fan::Few }, F::Tree { from: 1, to: 0 }, F::Local]);
        let g = facts(&v, &l, "g");
        assert!(g.contains(&F::Key(vec![1])) && g.contains(&F::Key(vec![0, 2])) && g.contains(&F::Local), "{g:?}");
        assert!(g.contains(&F::Order { col: 2, within: vec![0], dense: false }), "{g:?}");
        assert!(facts(&v, &l, "h").contains(&F::Order { col: 2, within: vec![0], dense: true }));
    }

    /// A join along a key keeps the other side's key: `c` fixes `e`'s row and, by `k`'s key, `s`. Two atoms that
    /// share no node are neither keyed by one side nor local.
    #[test]
    fn a_join_along_a_key_keeps_the_key_and_a_cross_product_neither() {
        let (v, l) = of(".decl j(c: node, s: symbol)\n.output j\nj(c, s) :- e(_, c, _), k(c, s).\n\
                         .decl x(a: node, b: node)\n.output x\nx(a, b) :- e(_, a, _), e(_, b, _).\n");
        assert_eq!(facts(&v, &l, "j"), [F::Key(vec![0]), F::Local]);
        assert_eq!(facts(&v, &l, "x"), []);
    }

    /// A relation of two rules, or a recursive one, keeps `local` where every rule gives it, and its declared facts
    /// alone besides.
    #[test]
    fn a_union_or_a_recursion_keeps_local_alone() {
        let (v, l) = of(".decl u(c: node)\n.output u\nu(c) :- e(_, c, _).\nu(n) :- k(n, _).\n\
                         .decl t(a: node, b: node)\n.output t\nt(a, b) :- e(a, b, _).\nt(a, c) :- t(a, b), e(b, c, _).\n\
                         .decl d(a: node, b: node)\n.layout d key(b)\n.output d\nd(a, b) :- d(a, c), e(c, b, _).\nd(a, b) :- e(a, b, _).\n");
        assert_eq!(facts(&v, &l, "u"), [F::Local]);
        assert_eq!(facts(&v, &l, "t"), [F::Local]);
        assert_eq!(facts(&v, &l, "d"), [F::Key(vec![1]), F::Local]);
        // A branch that joins two documents' nodes unjoined is not local, nor is the union.
        let (v, l) = of(".decl w(a: node, b: node)\n.output w\nw(a, b) :- e(a, b, _).\nw(a, b) :- e(_, a, _), e(_, b, _).\n");
        assert_eq!(facts(&v, &l, "w"), []);
    }

    /// Over a Python policy's program (python/ast.rr's library and core.rr): the library's derived relations take
    /// facts from core.rr's base ones. The counts are printed for decision 0361's record.
    #[test]
    fn the_python_library_infers_facts_of_its_derived_relations() {
        let text = ".policy \"t:a\" revision 1\n.language python\n#include \"python/ast.rr\"\n.violation Fn(at: node) \"a call\"\nFn(c) :- call_name(c, _).\n";
        let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
        let l = crate::code_rule_dsl::logic::program_image::manager::analysis::<Layouts>(&v).unwrap();
        let by = |o: fn(&Origin) -> bool| l.0.values().filter(|fs| fs.iter().any(|(_, x)| o(x))).count();
        let (declared, inferred) = (by(|o| *o == Origin::Declared), by(|o| matches!(o, Origin::Rule(_))));
        let keyed = l.0.values().filter(|fs| fs.iter().any(|(f, o)| matches!(o, Origin::Rule(_)) && matches!(f, F::Key(_)))).count();
        let local = l.0.values().filter(|fs| fs.iter().any(|(f, o)| matches!(o, Origin::Rule(_)) && *f == F::Local)).count();
        let derived: BTreeSet<RelId> = v.rules().map(|r| r.head().rel()).collect();
        eprintln!("layouts: {} relations derived; {declared} declared, {inferred} with inferred facts ({keyed} keyed, {local} local)", derived.len());
        assert!(declared >= 9 && inferred > 0 && keyed > 0, "{}", l.show(&v));
    }

    /// EXPLAIN's lines: each relation's facts, their columns named, each with where it came from.
    #[test]
    fn explain_names_each_facts_columns_and_origin() {
        let (v, l) = of(".decl f(c: node, p: node)\n.output f\nf(c, p) :- e(p, c, _).\n");
        let shown = l.show(&v);
        assert!(shown.contains("e: key(c) (declared), key(p, i) (declared)"), "{shown}");
        assert!(shown.contains("f: key(c) (by rule #"), "{shown}");
        assert!(shown.contains("tree(p -> c) (by rule #"), "{shown}");
    }
}

/// A relation's facts are a set (decision 0456 K3): the same facts in another order are the same program, its digest
/// and each relation's identity unchanged. A phase's export maintained from kept rows comes in their canonical order,
/// one derived anew in another; the cone's rounds' space is named by the program's digest, and its kept relations by
/// their identities, so an order-sensitive digest lost every kept round when a library file went.
#[test]
fn facts_in_another_order_are_the_same_program() {
    use crate::code_rule_dsl::logic::plan::{body::Placed, fields::Fields, leaf_identity};
    let text = |rows: &[i64]| {
        let facts: String = rows.iter().map(|r| format!("k({r}).\n")).collect();
        format!(".decl k(x: number)\n.input k\n.decl r(x: number)\n.output r\n{facts}r(x) :- k(x).\n")
    };
    let of = |rows: &[i64]| {
        let p = Placed::whole(crate::code_rule_dsl::logic::rr::parse_version(&text(rows)).unwrap());
        (p.digest(), leaf_identity(Fields::of(&p).unwrap(), "k"))
    };
    let (a, b) = (of(&[1, 2, 3]), of(&[3, 1, 2]));
    assert_eq!(a.0, b.0, "the program's digest");
    assert_eq!(a.1, b.1, "the relation's identity");
    // Other rows are another program.
    assert_ne!(of(&[1, 2]).0, a.0);
}

/// A rule written literal by literal on an open rule (`Builder::open_rule`, the lowering's writer, decision 0276 A3)
/// is the rule `RuleBuilder` writes, byte for byte; a rule ended once (`close_rule_once`) leaves out its repeat.
#[test]
fn an_open_rule_writes_what_the_rule_builder_writes() {
    let x = || T::Var("x");
    let join = AggOp::Join { by: vec!["k".into()], sep: ", ".into() };
    let mut a = Builder::base();
    a.rule()
        .head("r", &[x(), T::Call("cat", vec![T::Top, T::Str("s")])])
        .atom("a", &[x(), T::Wild])
        .atom_with("b", &[x()], true, true)
        .cmp(x(), CmpOp::Le, T::Int(i64::MIN))
        .agg(T::Var("j"), &join, Some(T::Var("y")), "a", &[T::Var("y"), T::Var("k")])
        .ast(T::Var("d"), x(), "{\"kind\": \"call\"}", &[("NAME", T::Var("v"))])
        .finish()
        .unwrap();
    let mut b = Builder::base();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..2 {
        let mut o = b.open_rule();
        b.put_head(&mut o, "r", &[x(), T::Call("cat", vec![T::Top, T::Str("s")])]);
        b.put_atom(&mut o, "a", &[x(), T::Wild], false, false);
        b.put_atom(&mut o, "b", &[x()], true, true);
        b.put_cmp(&mut o, x(), CmpOp::Le, T::Int(i64::MIN));
        b.put_agg(&mut o, "j", &join, Some("y"), "a", &[T::Var("y"), T::Var("k")]);
        b.put_ast(&mut o, "d", "x", "{\"kind\": \"call\"}", &[("NAME", "v")]);
        b.close_rule_once(o, &mut seen).unwrap();
    }
    assert_eq!(a.image().unwrap().bytes(), b.image().unwrap().bytes());
    let mut c = Builder::base();
    let o = c.open_rule();
    assert!(c.close_rule(o).is_err(), "a rule with no head is refused");
    assert_eq!(c.finish().unwrap().0.rules().count(), 0);
}

/// A derivation's kept rules copied as runs of nodes (`Builder::keep_run`, 0276 §3) are the rules `keep` copies one
/// by one: the same program, rule hashes and sources, and an image that opens as it; a gap in the run (a rule left
/// out) splits it.
#[test]
fn a_run_of_kept_rules_is_the_rules_kept_one_by_one() {
    let v = every_part();
    let kept: Vec<RuleRef<'_>> = v.rules().enumerate().filter(|(i, _)| i % 3 != 1).map(|(_, r)| r).collect();
    let mut one = Builder::derive(&v);
    for r in &kept {
        one.keep(&v, *r);
    }
    let (one, _) = one.finish().unwrap();
    let mut run = Builder::derive(&v);
    run.keep_run(&v, &kept);
    let (run, _) = run.finish().unwrap();
    same(&one, &run);
    assert!(one.rules().map(|r| (r.hash(), r.source())).eq(run.rules().map(|r| (r.hash(), r.source()))));
    assert!(run.rules().all(|r| r.source().is_some()));
    same(&Version::open(run.fold().unwrap().base_bytes().unwrap()).unwrap(), &run);
}
