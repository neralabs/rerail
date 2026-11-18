use super::passes::Parts as _;
use super::fields::{FieldsMut, ProgramFields};
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
    b.declare("r", &[("c", "int"), ("d", "sym")]);
    b.declare("nearest", &[("x", "int")]);
    b.set_inputs(&["a", "b"]);
    b.set_outputs(&["r"]);
    b.set_facts("k", &[vec![Value::Int(i64::MIN), Value::Str("q\"\\\n\u{1}é".into())].into(), vec![Value::Int(-3), Value::Str(String::new().into())].into()]);
    b.set_facts("empty", &[]);
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
    b.set_field(Field::Natives(&[("reach_x".to_owned(), Native::Reach { seed: "s".into(), edge: "e".into() })].into()));
    b.set_field(Field::Kindsets(&[("Scope".to_owned(), vec!["module".to_owned(), "class".to_owned()])].into()));
    b.set_field(Field::Demand(&["*".to_owned()]));
    b.set_field(Field::Settings(&[("mode".to_owned(), "block".to_owned()), ("input".to_owned(), "r(IO=json)".to_owned())]));
    b.set_field(Field::Nullary(&["unit".to_owned()].into()));
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
        b.declare(&d_changed, &[("only", "int")]);
        b.undeclare(&d_gone);
        b.declare("made_here", &[("x", "int")]);
        if facts {
            b.set_facts("made_facts", &[vec![Value::Int(7)].into()]);
        }
        b.set_inputs(&inputs);
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
    b.drop_facts("made_facts");
    b.set_outputs(&outputs);
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
        b.set_outputs(&outputs);
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
        b.set_field(Field::Lattice(&lattice));
        b.set_field(Field::Gates(&gates));
        b.set_field(Field::Natives(&natives));
        // Facts named as about relations are (`gate`, `lattice`): the fields' overlay hides none of them.
        b.set_facts("gate", &[vec![Value::Int(1)].into()]);
        b.set_facts("lattice", &[vec![Value::Str("x".into())].into()]);
        b.finish().unwrap().0.fold().unwrap()
    };
    // Every other relation, so the part interleaves the program's rules: many ranges.
    let mine: BTreeSet<&str> = heads.iter().step_by(2).map(String::as_str).collect();
    let keep = |rel: &str| mine.contains(rel);
    let inputs: Vec<String> = v0.input_names().into_iter().take(2).chain(["exported".to_owned()]).collect();
    let outputs: Vec<String> = heads.iter().step_by(4).cloned().collect();
    let part = v0.part(&keep, &inputs, &outputs, super::PartFields { natives: &keep, lattice: &keep, gates: &keep }).unwrap();
    let fields_of = |b: &mut Builder<'_>| {
        b.set_field(Field::Natives(&natives.iter().filter(|(r, _)| keep(r)).map(|(r, n)| (r.clone(), n.clone())).collect()));
        b.set_field(Field::Lattice(&lattice.iter().filter(|(r, _)| keep(r)).map(|(r, l)| (r.clone(), l.clone())).collect()));
        b.set_field(Field::Gates(&gates.iter().filter(|(r, _)| keep(r)).cloned().collect::<Vec<_>>()));
        b.set_field(Field::Asked(&ProgramFields::asked(&*v0).into_iter().filter(|(r, _)| keep(r)).collect()));
        b.set_inputs(&inputs);
        b.set_outputs(&outputs);
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
    b.set_inputs(&more);
    let cut_in = b.finish_overlay().unwrap();
    same(&cut_in, &remade(&v0, |r| keep(v0.rel_name(r.head().rel())) && !out.contains(&r.id()), |b| {
        fields_of(b);
        b.set_inputs(&more);
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
    b.set_inputs(&inputs);
    let (v1, made) = b.finish().unwrap();
    assert_eq!(made, Made::Folded);
    assert_eq!(v1.depth(), 0);
    same(&v1, &remade(&v0, |r| r.id().0 < n / 3, |b| b.set_inputs(&inputs)));
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
    b.set_field(Field::Answers(&[("ask".to_owned(), vec!["told".to_owned()])].into()));
    b.set_field(Field::Lattice(&[("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None })].into()));
    b.set_field(Field::Natives(&[("r".to_owned(), Native::Reach { seed: "lone".into(), edge: "e".into() })].into()));
    b.set_inputs(&["in", "seed"]);
    b.set_outputs(&["out", "best", "pad"]);
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
    b.set_inputs(&["in"]);
    b.set_outputs(&["p"]);
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
    b.declare("flag", &[]);
    b.declare("in", &[("a", "int"), ("b", "int"), ("c", "int")]);
    b.declare("best", &[("x", "int"), ("n", "int")]);
    b.declare("lo", &[("n", "int")]);
    b.declare("e", &[("x", "int"), ("y", "int")]);
    b.set_field(Field::Lattice(&[
        ("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None }),
        ("lo".to_owned(), LatticeDecl { keys: 0, ty: "min".into(), merge: None, widen: None }),
    ].into()));
    b.set_facts("flag", &[Vec::<Value>::new().into()]);
    b.set_facts("e", &[vec![Value::Int(1), Value::Int(2)].into()]);
    b.set_facts("lo", &[vec![Value::Int(7)].into()]);
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
    b.set_field(Field::Natives(&natives));
    b.set_field(Field::Lattice(&[
        ("best".to_owned(), LatticeDecl { keys: 1, ty: "max".into(), merge: None, widen: None }),
        ("joined".to_owned(), LatticeDecl { keys: 2, ty: "sym".into(), merge: Some("type_join".into()), widen: Some(3) }),
    ].into()));
    b.set_field(Field::Answers(&[("ask".to_owned(), vec!["told".to_owned(), "also".to_owned()]), ("ask2".to_owned(), vec!["told2".to_owned()])].into()));
    b.set_field(Field::Gates(&[
        ("done_x".to_owned(), Gate { src: Some("x_demand".into()), rel: "x".into(), level: 2, quiet: vec!["q1".into(), "q0".into()], deps: vec!["done_y".into()] }),
        ("done_y".to_owned(), Gate { src: None, rel: "y".into(), level: 1, quiet: vec![], deps: vec![] }),
    ]));
    b.set_field(Field::Qualifiers(&[("r".to_owned(), ["inline".to_owned(), "no_magic".to_owned()].into()), ("nearest".to_owned(), ["no_inline".to_owned()].into())].into()));
    b.set_field(Field::Tapes(&[("r".to_owned(), [1usize].into())].into()));
    b.set_field(Field::Demand(&["r".to_owned(), "*".to_owned(), "nearest".to_owned()]));
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

/// A relation's identity (`plan::identities`) is a digest of the text `plan::tree_text` writes from the version's
/// handles: the text the tree's `Debug` wrote, byte for byte, or every kept stratum's key would change. These are
/// that text's lines for `every_field`'s rules and relations, as the tree wrote them (0276 step 6 froze them when
/// the tree went).
#[test]
fn a_relations_identity_text_is_kept() {
    use crate::code_rule_dsl::logic::plan::{body, body::Placed, fields::Fields, tree_text};
    let v = every_field();
    let rules: Vec<String> = (0..v.rules().count()).map(|k| tree_text::rule(&body::nth(&v, k))).collect();
    let placed = Placed::whole(v.clone());
    let f = Fields::of(&placed).unwrap();
    let rels: Vec<String> = ["r", "k", "empty", "reach_x", "joined", "unit", "no_such_relation"].iter().map(|rel| format!("{}\n{}", tree_text::shape(&f, rel), tree_text::leaf(&f, rel))).collect();
    let (want_rules, want_rels): (&[&str], &[&str]) = (IDENTITY_RULES, IDENTITY_RELS);
    assert_eq!(rules, want_rules, "{rules:#?}");
    assert_eq!(rels, want_rels, "{rels:#?}");
}

const IDENTITY_RULES: &[&str] = &[
    "Rule { head: Atom { rel: \"r\", args: [Var(\"x\"), Call(\"cat\", [Var(\"top\"), Str(\"s\")])], neg: false, gated: false }, body: [Atom(Atom { rel: \"a\", args: [Var(\"x\"), Var(\"count_\"), Var(\"min\")], neg: false, gated: false }), Atom(Atom { rel: \"b\", args: [Wild, Top], neg: true, gated: false }), Atom(Atom { rel: \"done_x\", args: [Var(\"x\")], neg: true, gated: true }), Cmp { l: Var(\"x\"), op: \"<=\", r: Int(-9223372036854775808) }, Cmp { l: Call(\"rx\", [Var(\"x\"), Str(\"^a$\")]), op: \"=\", r: Int(1099511627776) }] }",
    "Rule { head: Atom { rel: \"nearest\", args: [Var(\"n\")], neg: false, gated: false }, body: [Agg { var: \"n\", op: Count, over: None, atom: Atom { rel: \"a\", args: [Var(\"x\"), Wild], neg: false, gated: false } }, Agg { var: \"m\", op: Min, over: Some(\"y\"), atom: Atom { rel: \"a\", args: [Var(\"y\")], neg: false, gated: false } }, Agg { var: \"s\", op: Sum, over: None, atom: Atom { rel: \"a\", args: [Var(\"y\")], neg: false, gated: false } }, Agg { var: \"j\", op: Join { by: [\"k\", \"sep\"], sep: \", \" }, over: Some(\"y\"), atom: Atom { rel: \"ast\", args: [Var(\"y\"), Var(\"k\"), Var(\"sep\")], neg: false, gated: false } }, Agg { var: \"j2\", op: Join { by: [], sep: \"\" }, over: Some(\"y\"), atom: Atom { rel: \"a\", args: [Var(\"y\")], neg: false, gated: false } }, Ast { document: \"d\", node: \"n\", matcher: \"{\\\"kind\\\": \\\"call\\\"}\", captures: [(\"NAME\", \"v\")] }] }",
    "Rule { head: Atom { rel: \"unit\", args: [], neg: false, gated: false }, body: [] }",
];
const IDENTITY_RELS: &[&str] = &[
    "(Some([(\"c\", \"int\"), (\"d\", \"sym\")]), None, None, None, false)\nSome([(\"c\", \"int\"), (\"d\", \"sym\")])\nNone",
    "(None, Some([[Int(-9223372036854775808), Str(\"q\\\"\\\\\\n\\u{1}é\")], [Int(-3), Str(\"\")]]), None, None, false)\nNone\nSome([[Int(-9223372036854775808), Str(\"q\\\"\\\\\\n\\u{1}é\")], [Int(-3), Str(\"\")]])",
    "(None, Some([]), None, None, false)\nNone\nSome([])",
    "(None, None, Some(Reach { seed: \"s\", edge: \"e\" }), None, false)\nNone\nNone",
    "(None, None, None, Some(LatticeDecl { keys: 2, ty: \"sym\", merge: Some(\"type_join\"), widen: Some(3) }), false)\nNone\nNone",
    "(None, None, None, None, true)\nNone\nNone",
    "(None, None, None, None, false)\nNone\nNone",
];

// ---- the pass manager (decision 0281) ----

/// A program of three heads over two inputs, with stated facts: `e` read by `p` and `q`, `f` by `r`'s twelve rules.
fn managed() -> Arc<Version> {
    let mut b = Builder::base();
    for (n, k) in [("e", 2), ("f", 1), ("p", 1), ("q", 1), ("r", 1)] {
        let cols: Vec<String> = (0..k).map(|i| format!("c{i}")).collect();
        b.declare(n, &cols.iter().map(|c| (c.as_str(), "int")).collect::<Vec<_>>());
    }
    b.set_facts("f", &[vec![Value::Int(1)].into()]);
    b.set_inputs(&["e"]);
    b.set_outputs(&["p", "r"]);
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
        w.on()?.drop_facts("f");
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
            b.declare("a", &[("n", "node")]);
            b.set_outputs(&["a"]);
            b.set_field(Field::DocScopes(scopes.as_deref()));
            b.set_field(Field::TextGlobs(&globs));
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
        b.set_facts("none", &[]);
        b.set_facts("one", &[vec![Value::Int(1)].into()]);
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
        b.declare("a", &[("n", "node")]);
        b.set_outputs(&["a"]);
        b.set_field(Field::AstPatterns(&patterns));
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
