//! B1's checks (*0686: Operations* §13): a second writer, a writer beside a reader and a write to a frozen block each
//! fail when the operation starts, naming it, with nothing left pinned; a reference to a block not frozen fails;
//! reading a block read-pins what it refers to; an operation of each shape the twins need first (map, emit, sort,
//! merge, queue) gives the same facts at two tilings and in shuffled order; a version is registered only once checked
//! against the reference; a 31st slot is refused. (A referenced generation living until its referrer is rolled up is
//! `trace`'s.)

use super::*;
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Ref, word};
use rerail_blocks::region::{pin_block, whole};
use rerail_delta_twins::samples::{self, EVENS, MERGE as MERGE2, REACH, SORT, SQUARE};
use rerail_delta_twins::{NoParams, TwinPhase};

const NONE_P: ParamsLayout = ParamsLayout::of::<NoParams>();

fn lanes(name: &'static str, found: Found, across: Across, output: Output, bounds: &'static [Bound]) -> Phase {
    Phase::lanes(name, found, across, output, bounds)
}

/// A values block's facts: `(position, value)`.
fn values(_: &Table<'_>, _: usize, b: &[u8]) -> Vec<Vec<i64>> {
    samples::vals(b).iter().enumerate().map(|(i, &v)| vec![i as i64, v]).collect()
}

/// A visited set's facts: the nodes marked.
fn marked(_: &Table<'_>, _: usize, b: &[u8]) -> Vec<Vec<i64>> {
    samples::vals(b).iter().enumerate().filter(|&(_, &v)| v == 1).map(|(i, _)| vec![i as i64]).collect()
}

const R: Mode = Mode::Read;
const W: Mode = Mode::Write;
const S: Mode = Mode::Scratch;

fn slot(name: &'static str, mode: Mode, kind: u32) -> SlotSpec {
    SlotSpec { name, mode, kind }
}

fn sig(name: &'static str, slots: Vec<SlotSpec>, shape: Vec<Phase>, facts: FactsFn) -> &'static Signature {
    Box::leak(Box::new(Signature::new(name, Box::leak(slots.into_boxed_slice()), NONE_P, Box::leak(shape.into_boxed_slice()), Independence::Along("items"), facts)))
}

/// The five shapes' signatures, each placed on the six axes.
fn shapes() -> Vec<(&'static Signature, &'static [TwinPhase])> {
    use Across::*;
    use Found::*;
    use Output::*;
    let io = |w| vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS), slot("work", S, w)];
    vec![
        (sig("square", vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)], vec![lanes("map", Fixed, Nothing, ByIndex, &[])], values), SQUARE),
        (
            sig(
                "evens",
                io(kind::SCRATCH),
                vec![lanes("count", Fixed, Nothing, ByIndex, &[]), lanes("scan", Fixed, Combine, ByScan, &[]), lanes("place", Fixed, Nothing, ByScan, &[])],
                values,
            ),
            EVENS,
        ),
        (
            sig(
                "sort",
                io(kind::SCRATCH),
                vec![lanes("chunks", Fixed, Nothing, ByIndex, &[]), lanes("passes", Indirect, Rounds, ByIndex, &[Bound::Turns(16)]), lanes("settle", Fixed, Nothing, ByIndex, &[])],
                values,
            ),
            SORT,
        ),
        (sig("merge2", vec![slot("a", R, kind::FACTS), slot("b", R, kind::FACTS), slot("out", W, kind::FACTS)], vec![lanes("merge", Fixed, Nothing, ByIndex, &[])], values), MERGE2),
        (
            sig(
                "reach",
                io(kind::SCRATCH),
                vec![lanes("seed", Fixed, Nothing, InPlace, &[]), lanes("turn", Queue, Rounds, Claim, &[Bound::Turns(1024), Bound::Capacity(Over::Refuse)])],
                marked,
            ),
            REACH,
        ),
    ]
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut z = seed;
    move || {
        z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
        rerail_blocks::layer::mix(z)
    }
}

/// A values block of `v`, frozen.
fn values_block(dir: &mut Directory, v: &[i64]) -> BlockId {
    let id = dir.make(kind::FACTS, samples::vals_bytes(v.len()));
    {
        let b = dir.block(id).unwrap();
        let mut w = pin_block(&b, "fill", true).unwrap();
        samples::vals_mut(w.bytes_mut().unwrap(), v.len()).copy_from_slice(v);
    }
    dir.freeze(id).unwrap();
    id
}

/// Each shape's launch over 1,000 values (or a 300-node graph), and what it must write.
fn case(name: &str, dir: &mut Directory) -> (Case, Vec<Vec<i64>>) {
    let mut r = rng(7);
    let v: Vec<i64> = (0..1000).map(|_| (r() % 5000) as i64 - 2500).collect();
    let out = |dir: &mut Directory, n: usize| dir.make(kind::FACTS, samples::vals_bytes(n));
    let work = |dir: &mut Directory, bytes: usize| dir.make(kind::SCRATCH, bytes);
    let rows = |v: Vec<i64>| v.into_iter().enumerate().map(|(i, x)| vec![i as i64, x]).collect::<Vec<_>>();
    match name {
        "square" => {
            let (i, o) = (values_block(dir, &v), out(dir, v.len()));
            (Case { slots: vec![i, o], params: vec![] }, rows(v.iter().map(|x| x * x).collect()))
        }
        "evens" => {
            let (i, o, w) = (values_block(dir, &v), out(dir, v.len()), work(dir, 8 + 8 * 8));
            (Case { slots: vec![i, o, w], params: vec![] }, rows(v.iter().copied().filter(|x| x % 2 == 0).collect()))
        }
        "sort" => {
            let (i, o, w) = (values_block(dir, &v), out(dir, v.len()), work(dir, 24 + 8 * v.len()));
            let mut s = v.clone();
            s.sort();
            (Case { slots: vec![i, o, w], params: vec![] }, rows(s))
        }
        "merge2" => {
            let (mut a, mut b) = (v[..400].to_vec(), v[400..].to_vec());
            a.sort();
            b.sort();
            let (ia, ib, o) = (values_block(dir, &a), values_block(dir, &b), out(dir, v.len()));
            let mut s = v.clone();
            s.sort();
            (Case { slots: vec![ia, ib, o], params: vec![] }, rows(s))
        }
        "reach" => {
            // 300 nodes; each has two edges, to (3i + 1) and (5i + 2) mod 300, but node 7's are cut: some unreachable.
            let n = 300usize;
            let mut offsets = vec![0u64];
            let mut targets = Vec::new();
            for i in 0..n {
                if i != 7 {
                    targets.push(((3 * i + 1) % n) as u64);
                    targets.push(((5 * i + 2) % n) as u64);
                }
                offsets.push(targets.len() as u64);
            }
            let g = dir.make(kind::FACTS, samples::graph_bytes(n, targets.len()));
            {
                let b = dir.block(g).unwrap();
                let mut w = pin_block(&b, "fill", true).unwrap();
                let bytes = w.bytes_mut().unwrap();
                use rerail_blocks::gens::{BODY_AT, set_word, words_mut};
                set_word::<u64>(bytes, BODY_AT, samples::graph_bytes(n, targets.len()) as u64);
                set_word::<u64>(bytes, BODY_AT + 8, n as u64);
                set_word::<u64>(bytes, BODY_AT + 16, targets.len() as u64);
                words_mut::<u64>(bytes, BODY_AT + 24, n + 1).copy_from_slice(&offsets);
                words_mut::<u64>(bytes, BODY_AT + 24 + 8 * (n + 1), targets.len()).copy_from_slice(&targets);
            }
            dir.freeze(g).unwrap();
            let (o, w) = (out(dir, n), work(dir, 32 + 8 * n));
            // The oracle: a plain search.
            let mut seen = vec![false; n];
            let mut stack = vec![0usize];
            seen[0] = true;
            while let Some(u) = stack.pop() {
                for &t in &targets[offsets[u] as usize..offsets[u + 1] as usize] {
                    if !seen[t as usize] {
                        seen[t as usize] = true;
                        stack.push(t as usize);
                    }
                }
            }
            let want = (0..n).filter(|&i| seen[i]).map(|i| vec![i as i64]).collect();
            (Case { slots: vec![g, o, w], params: vec![] }, want)
        }
        _ => unreachable!(),
    }
}

/// O§13: an operation of each shape the twins need first gives the same facts at two tilings and with its tiles in
/// shuffled order, and the right ones.
#[test]
fn each_shape_gives_the_same_facts_at_two_tilings_in_shuffled_order() {
    let tilings = [Tiling::WHOLE, Tiling { tile: 1, seed: 3 }, Tiling { tile: 3, seed: 11 }, Tiling { tile: 2, seed: 0 }];
    for (sig, twin) in shapes() {
        let mut reg = Registry::default();
        let op = reg.declare(sig).unwrap();
        reg.register_reference(Version::twin(op, "twin", twin, sig.shape)).unwrap();
        let mut first = None;
        for t in tilings {
            let mut dir = Directory::default();
            let (c, want) = case(sig.name, &mut dir);
            run_cpu(&reg, &dir, op, &c.slots, &c.params, t).unwrap_or_else(|e| panic!("{}: {e}", sig.name));
            let got = written_facts(sig, &dir, &c.slots).unwrap();
            let mut want = want;
            want.sort();
            assert_eq!(got[0], want, "{} at {t:?} wrote the right facts", sig.name);
            match &first {
                None => first = Some(got),
                Some(f) => assert_eq!(&got, f, "{}: the same facts at every tiling and order", sig.name),
            }
        }
    }
}

/// E1: a second writer, a writer beside a reader, and a write of a frozen block each refuse the start, naming
/// the operation and what holds the block, and leave nothing pinned.
#[test]
fn a_conflict_refuses_the_start_and_leaves_nothing_pinned() {
    let (sig, twin) = shapes().remove(0);
    let mut reg = Registry::default();
    let op = reg.declare(sig).unwrap();
    reg.register_reference(Version::twin(op, "twin", twin, sig.shape)).unwrap();
    let mut dir = Directory::default();
    let (c, _) = case("square", &mut dir);
    let (input, out) = (c.slots[0], c.slots[1]);
    let ob = dir.block(out).unwrap();

    let other = pin_block(&ob, "consolidate", true).unwrap();
    let e = run_cpu(&reg, &dir, op, &c.slots, &[], Tiling::WHOLE).unwrap_err();
    assert!(e.starts_with(&format!("square could not start: slot 1 ({out})")) && e.contains("a CPU write by consolidate"), "{e}");
    // Slot 0's read pin was taken, then let go.
    assert_eq!(whole(&dir.block(input).unwrap()).region().cpu_reads(), 0, "slot 0 was left unpinned");
    drop(other);

    let reader = pin_block(&ob, "step", false).unwrap();
    let e = run_cpu(&reg, &dir, op, &c.slots, &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("a CPU write by square while a CPU read by step"), "{e}");
    drop(reader);

    let e = run_cpu(&reg, &dir, op, &[input, input], &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("slot 1") && e.contains("frozen block"), "a write of a frozen block: {e}");

    let o2 = dir.make(kind::FACTS, 1 << 16);
    let e = run_cpu(&reg, &dir, op, &[o2, o2], &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("slot 1") && e.contains("a CPU read by square"), "one operation does not read and write one block: {e}");

    run_cpu(&reg, &dir, op, &c.slots, &[], Tiling::WHOLE).expect("nothing is left pinned by the refused starts");
}

/// A slot takes the kind of block its signature names; the params are the declared size.
#[test]
fn a_slot_takes_its_kind_of_block_and_the_params_their_size() {
    let (sig, twin) = shapes().remove(0);
    let mut reg = Registry::default();
    let op = reg.declare(sig).unwrap();
    reg.register_reference(Version::twin(op, "twin", twin, sig.shape)).unwrap();
    let mut dir = Directory::default();
    let (c, _) = case("square", &mut dir);
    let atoms = dir.make(kind::ATOMS, 64);
    let e = run_cpu(&reg, &dir, op, &[c.slots[0], atoms], &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("slot 1 (out) takes a block of kind"), "{e}");
    let e = run_cpu(&reg, &dir, op, &c.slots, &[0u8; 3], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("3 bytes of params for NoParams (0)"), "{e}");
}

/// Operations §2: a block refers only to a frozen block; reading a block read-pins what it refers to, transitively,
/// and its launch's descriptor table gives each of them by id, where it lies; the pins go with the launch.
#[test]
fn references_are_to_frozen_blocks_and_reading_one_pins_what_it_refers_to() {
    let mut dir = Directory::default();
    let a = values_block(&mut dir, &[1, 2, 3]);
    let open = dir.make(kind::FACTS, 64);
    let b = dir.make(kind::FACTS, 64);
    let e = dir.refer(b, open, BODY_AT).unwrap_err();
    assert!(e.contains("not frozen"), "{e}");
    dir.refer(b, a, BODY_AT).unwrap();
    dir.freeze(b).unwrap();
    let c = dir.make(kind::FACTS, 64);
    dir.refer(c, b, BODY_AT).unwrap();
    let e = dir.refer(b, a, BODY_AT).unwrap_err();
    assert!(e.contains("a block frozen"), "a frozen block's references are fixed: {e}");

    let ab = dir.block(a).unwrap();
    {
        let l = dir.read("probe", &[c]).unwrap();
        let t = l.table();
        assert_eq!(t.rows().iter().map(|d| d.id).collect::<Vec<_>>(), vec![a.0, b.0, c.0], "the table: c and what it reaches");
        assert_eq!(samples::vals(t.block(a.0).unwrap()), &[1, 2, 3], "a reached block read by id, in place");
        assert_eq!(rerail_blocks::gens::refs(t.block(c.0).unwrap()), &[Ref { id: b.0, at: BODY_AT as u32 }], "c's reference table names b, and where in it");
        assert_eq!(whole(&ab).region().cpu_reads(), 1, "reading c read-pinned a, which b refers to");
    }
    assert_eq!(whole(&ab).region().cpu_reads(), 0, "the launch's pins went with it");

    // O§2: a launch's table names every block it can reach, its slots too. A slot pinned to write is its pin's row,
    // read through the slot, not the table; a slot pinned to read is read through either.
    let out = dir.make(kind::FACTS, 64);
    let l = dir.launch("probe", &[(c, false), (out, true)]).unwrap();
    let t = l.table();
    assert_eq!(t.rows().iter().map(|d| d.id).collect::<Vec<_>>(), vec![a.0, b.0, c.0, out.0], "the table: its slots and what they reach");
    assert!(t.is_written(out.0) && !t.is_written(c.0));
    assert!(t.block(out.0).is_none(), "a written slot is read through its pin");
    assert_eq!(rerail_blocks::gens::refs(t.block(c.0).unwrap())[0].id, b.0, "a slot read, by id");
    assert_eq!(t.row(out.0).unwrap().addr, l.slots[1].addr() as u64, "the written slot's row is its pin's");
}

/// D1: a 31st slot is refused where the operation is declared.
#[test]
fn a_thirty_first_slot_is_refused() {
    let slots: &'static [SlotSpec] = Box::leak(vec![slot("s", R, kind::FACTS); MAX_SLOTS + 1].into_boxed_slice());
    let s: &'static Signature = Box::leak(Box::new(Signature {
        name: "wide",
        slots,
        params: NONE_P,
        shape: Box::leak(vec![lanes("map", Found::Fixed, Across::Nothing, Output::ByIndex, &[])].into_boxed_slice()),
        independent: Independence::Not("a test"),
        facts: values,
    }));
    let e = Registry::default().declare(s).unwrap_err();
    assert!(e.contains("31 slots; an operation binds at most 30"), "{e}");
}

/// A broken square: the cube.
static CUBE: &[TwinPhase] = &[TwinPhase { name: "map", items: SQUARE[0].items, run: cube_tile, again: None }];

fn cube_tile(c: &mut rerail_delta_twins::Ctx<'_, '_, '_>, t: rerail_delta_twins::Tile) {
    let ([input], [out]) = c.views([0], [1]);
    let src = samples::vals(input).to_vec();
    let dst = samples::vals_mut(out, src.len());
    for i in t.items() {
        dst[i] = src[i] * src[i] * src[i];
    }
}

/// E3 and Operations §3: the twin is the one reference, registered first; another version is registered only with
/// a check against it that found the same facts, at the tilings the check ran.
#[test]
fn a_version_is_registered_only_once_checked_against_the_reference() {
    let (sig, twin) = shapes().remove(0);
    let mut reg = Registry::default();
    let op = reg.declare(sig).unwrap();
    let ukl = Version { imp: Impl::Ukl { source: "", entry: "square" }, status: Status::Candidate, devices: Devices::METAL, ..Version::twin(op, "square ukl", twin, sig.shape) };
    assert!(reg.register_reference(ukl).unwrap_err().contains("not the twin"));
    reg.register_reference(Version::twin(op, "twin", twin, sig.shape)).unwrap();
    assert!(reg.register_reference(Version::twin(op, "twin 2", twin, sig.shape)).unwrap_err().contains("a second reference"));

    let case = |dir: &mut Directory| case("square", dir).0;
    let tilings = [Tiling::WHOLE, Tiling { tile: 7, seed: 5 }];
    let e = reg.check(&ukl, &case, &tilings).err().unwrap();
    assert!(e.contains("no device was given"), "a UKL version runs on a device: {e}");
    let cube = Version { status: Status::Candidate, ..Version::twin(op, "cube", CUBE, sig.shape) };
    let e = reg.check(&cube, &case, &tilings).err().unwrap();
    assert!(e.contains("wrote other facts than the reference"), "{e}");

    let tiled = Version { status: Status::Default, ..Version::twin(op, "square, tiled", twin, sig.shape) };
    let checked = reg.check(&tiled, &case, &tilings).unwrap();
    let other = Version { name: "unchecked", ..tiled };
    assert!(reg.register(other, reg.check(&tiled, &case, &tilings).unwrap()).unwrap_err().contains("not the version checked"));
    reg.register(tiled, checked).unwrap();
    assert_eq!(reg.versions(op).len(), 2);
    assert!(reg.declare(sig).unwrap_err().contains("declared twice"));
}

/// A twin's phases are its signature's, and a phase that runs again bounds its turns.
#[test]
fn a_twin_fits_its_signature() {
    let shapes = shapes();
    let (square, _) = shapes[0];
    let (_, sort) = shapes[2];
    let mut reg = Registry::default();
    let op = reg.declare(square).unwrap();
    let e = reg.register_reference(Version::twin(op, "sort as square", sort, square.shape)).unwrap_err();
    assert!(e.contains("the twin's phases (chunks, passes, settle) are not its shape's (map)"), "{e}");
    let unbounded = sig("unbounded", vec![slot("in", R, kind::FACTS)], vec![lanes("chunks", Found::Fixed, Across::Nothing, Output::ByIndex, &[]), lanes("passes", Found::Indirect, Across::Rounds, Output::ByIndex, &[]), lanes("settle", Found::Fixed, Across::Nothing, Output::ByIndex, &[])], values);
    // Refused when declared: a phase by rounds bounds its turns (`Phase::fits_axes`).
    let e = reg.declare(unbounded).unwrap_err();
    assert!(e.contains("unbounded: phase passes: relates its units by rounds, and its shape bounds no turns"), "{e}");

    // A version runs by its own shape: the sort's reference, then a version of it whose shape bounds its passes at
    // two turns, which the check runs by that bound and refuses (a case of more than two passes' items).
    let (sort_sig, _) = shapes[2];
    let op = reg.declare(sort_sig).unwrap();
    reg.register_reference(Version { status: Status::Reference, ..Version::twin(op, "sort", sort, sort_sig.shape) }).unwrap();
    let tight: &'static [Phase] = Box::leak(sort_sig.shape.iter().map(|p| if p.name == "passes" { Phase { bounds: &[Bound::Turns(2)], ..*p } } else { *p }).collect::<Vec<_>>().into_boxed_slice());
    let v = Version { status: Status::Candidate, ..Version::twin(op, "sort, two passes", sort, tight) };
    // 3,000 values: chunks of 256, so four merge passes.
    let sort_case = |dir: &mut Directory| {
        let v: Vec<i64> = (0..3_000i64).map(|i| (i * 7919) % 3_001).collect();
        let (i, o, w) = (values_block(dir, &v), dir.make(kind::FACTS, samples::vals_bytes(v.len())), dir.make(kind::SCRATCH, 24 + 8 * v.len()));
        Case { slots: vec![i, o, w], params: vec![] }
    };
    assert!(reg.check(&Version { name: "sort, as the reference", status: Status::Candidate, ..Version::twin(op, "", sort, sort_sig.shape) }, &sort_case, &[Tiling::WHOLE]).is_ok());
    let Err(e) = reg.check(&v, &sort_case, &[Tiling::WHOLE]) else { panic!("a version run past its shape's bound is refused") };
    assert!(e.contains("phase passes ran past its bound of 2 turns"), "{e}");
}

/// Params are declared once in Rust; UKL's declaration comes from that one.
#[test]
fn params_give_ukl_their_layout() {
    let u = ParamsLayout::of::<rerail_delta_twins::trace::MergeP>().ukl();
    assert!(u.contains("typedef struct MergeP {\n    uint32_t head;\n    uint32_t count;\n    uint32_t below;\n    uint32_t pad;\n} MergeP;"), "{u}");
    assert!(u.contains("UKL_OFFSETOF(MergeP, below) == 8"), "{u}");
    assert_eq!(Mode::Read.ukl(), "ROBUFFER");
    assert_eq!(Mode::Scratch.ukl(), "RWBUFFER");
}

/// A running sum: item `i` is the sum of items `0..=i`, read from item `i - 1`'s: not independent.
const RUNNING: &[TwinPhase] = &[TwinPhase { name: "map", items: SQUARE[0].items, run: running_tile, again: None }];

fn running_tile(c: &mut rerail_delta_twins::Ctx<'_, '_, '_>, t: rerail_delta_twins::Tile) {
    let ([input], [out]) = c.views([0], [1]);
    let src = samples::vals(input);
    let dst = samples::vals_mut(out, src.len());
    for i in t.items() {
        dst[i] = if i == 0 { 0 } else { dst[i - 1] } + src[i];
    }
}

/// Operations §4's independence axis: an operation that says its items are not independent runs whole and in order at
/// any tiling asked; one that claims they are when they are not is caught by the check, at tiles in shuffled order.
#[test]
fn the_independence_axis_is_kept_and_checked() {
    let shape: &'static [Phase] = Box::leak(vec![lanes("map", Found::Fixed, Across::Ordered, Output::ByIndex, &[])].into_boxed_slice());
    let slots: &'static [SlotSpec] = Box::leak(vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)].into_boxed_slice());
    let not = Box::leak(Box::new(Signature::new("running", slots, NONE_P, shape, Independence::Not("item i reads item i - 1"), values)));
    // The false claim: items independent, and the phase's units sharing nothing, so its tiles run in any order.
    let unordered: &'static [Phase] = Box::leak(vec![lanes("map", Found::Fixed, Across::Nothing, Output::ByIndex, &[])].into_boxed_slice());
    let along = Box::leak(Box::new(Signature::new("running, claimed independent", slots, NONE_P, unordered, Independence::Along("items"), values)));
    let make = |dir: &mut Directory| {
        let v: Vec<i64> = (1..=600).collect();
        let (i, o) = (values_block(dir, &v), dir.make(kind::FACTS, samples::vals_bytes(v.len())));
        Case { slots: vec![i, o], params: vec![] }
    };
    let want: Vec<Vec<i64>> = (1..=600i64).map(|i| vec![i - 1, i * (i + 1) / 2]).collect();
    let tilings = [Tiling::WHOLE, Tiling { tile: 7, seed: 3 }, Tiling { tile: 1, seed: 11 }];

    let mut reg = Registry::default();
    let op = reg.declare(not).unwrap();
    reg.register_reference(Version::twin(op, "twin", RUNNING, shape)).unwrap();
    for t in tilings {
        let mut dir = Directory::default();
        let c = make(&mut dir);
        run_cpu(&reg, &dir, op, &c.slots, &[], t).unwrap();
        assert_eq!(written_facts(not, &dir, &c.slots).unwrap()[0], want, "not independent: run whole at {t:?}");
    }
    let again = Version { status: Status::Default, ..Version::twin(op, "running again", RUNNING, shape) };
    reg.check(&again, &make, &tilings).expect("whole at every tiling");

    let mut reg = Registry::default();
    let op = reg.declare(along).unwrap();
    reg.register_reference(Version::twin(op, "twin", RUNNING, unordered)).unwrap();
    let again = Version { status: Status::Default, ..Version::twin(op, "running again", RUNNING, unordered) };
    let Err(e) = reg.check(&again, &make, &tilings) else { panic!("a false claim of independence is caught") };
    assert!(e.contains("wrote other facts"), "{e}");

    // Its units in order (a carry from one to the next, Operations §4's "phases in order"): tiled, its tiles run in
    // order whatever the tiling's seed, so the same twin is right at every tiling.
    let in_order = Box::leak(Box::new(Signature::new("running, in order", slots, NONE_P, shape, Independence::Along("items, in order"), values)));
    let mut reg = Registry::default();
    let op = reg.declare(in_order).unwrap();
    reg.register_reference(Version::twin(op, "twin", RUNNING, shape)).unwrap();
    let again = Version { status: Status::Default, ..Version::twin(op, "running again", RUNNING, shape) };
    reg.check(&again, &make, &tilings).expect("an ordered phase's tiles run in order at every tiling");
}

/// A phase of items made at the launch's start (`Found::Fixed`) whose count an earlier phase changed.
const GROWS: &[TwinPhase] = &[
    TwinPhase { name: "first", items: |_| 1, run: grow_tile, again: None },
    TwinPhase { name: "then", items: |c| samples::vals(c.read(1)).first().map_or(0, |&n| n as u32), run: |_, _| {}, again: None },
];

fn grow_tile(c: &mut rerail_delta_twins::Ctx<'_, '_, '_>, _: rerail_delta_twins::Tile) {
    let ([], [out]) = c.views([], [1]);
    samples::vals_mut(out, 1)[0] = 5;
}

/// Operations §4's other axes, as the executor runs them: a phase of a fixed count has the count it had when the
/// launch started (one an earlier phase made is indirect), and a phase runs again exactly when its units relate by
/// rounds.
#[test]
fn a_phase_runs_by_its_axes() {
    let slots: &'static [SlotSpec] = Box::leak(vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)].into_boxed_slice());
    let make = |dir: &mut Directory| {
        let v: Vec<i64> = vec![0];
        let (i, o) = (values_block(dir, &v), dir.make(kind::FACTS, samples::vals_bytes(1)));
        Case { slots: vec![i, o], params: vec![] }
    };
    let run = |found: Found| {
        let shape: &'static [Phase] = Box::leak(vec![lanes("first", Found::Fixed, Across::Nothing, Output::ByIndex, &[]), lanes("then", found, Across::Nothing, Output::ByIndex, &[])].into_boxed_slice());
        let sig = Box::leak(Box::new(Signature::new("grows", slots, NONE_P, shape, Independence::Along("items"), values)));
        let mut reg = Registry::default();
        let op = reg.declare(sig).unwrap();
        reg.register_reference(Version::twin(op, "twin", GROWS, shape)).unwrap();
        let mut dir = Directory::default();
        let c = make(&mut dir);
        run_cpu(&reg, &dir, op, &c.slots, &[], Tiling::WHOLE)
    };
    let e = run(Found::Fixed).unwrap_err();
    assert!(e.contains("phase then has a fixed count of items, 0 when the launch started, and 5 when it ran"), "{e}");
    run(Found::Indirect).expect("an indirect count is made by the phases before it");

    let shapes = shapes();
    let (_, sort) = shapes[2];
    let mut reg = Registry::default();
    let not_rounds = sig("sort, passes in order", vec![slot("in", R, kind::FACTS)], vec![lanes("chunks", Found::Fixed, Across::Nothing, Output::ByIndex, &[]), lanes("passes", Found::Indirect, Across::Ordered, Output::ByIndex, &[Bound::Turns(16)]), lanes("settle", Found::Fixed, Across::Nothing, Output::ByIndex, &[])], values);
    let op = reg.declare(not_rounds).unwrap();
    let e = reg.register_reference(Version::twin(op, "sort", sort, not_rounds.shape)).unwrap_err();
    assert!(e.contains("phase passes runs again, and its shape's units do not relate by rounds"), "{e}");
    let rounds = sig("square by rounds", vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)], vec![lanes("map", Found::Fixed, Across::Rounds, Output::ByIndex, &[Bound::Turns(4)])], values);
    let op = reg.declare(rounds).unwrap();
    let e = reg.register_reference(Version::twin(op, "square", SQUARE, rounds.shape)).unwrap_err();
    assert!(e.contains("phase map relates its units by rounds, and its twin has no finisher"), "{e}");
}

/// E1 on a device: a launch handed to its fence (`Launch::to_fence`) holds every slot by its mode and every block it
/// reaches to read until the fence completes: another launch that writes a slot it reads, or reads the slot it writes,
/// is refused until then. On a discrete GPU's domain each block reached is the directory's replica there
/// (`Directory::replicate`): a launch with a block not placed there is refused and given back as it was, and the
/// device's table rows name each replica's address (`gens::device_rows_in`).
#[test]
fn a_launch_on_a_device_holds_its_blocks_until_its_fence_completes() {
    use rerail_blocks::ledger::Fence;
    use rerail_blocks::region::Mirror;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicU64, Ordering::SeqCst};
    let mut dir = Directory::default();
    let a = values_block(&mut dir, &(0..100).collect::<Vec<_>>());
    let out = dir.make(kind::FACTS, 64);
    dir.refer(out, a, BODY_AT).unwrap();
    let l = dir.launch("k", &[(out, true), (a, false)]).unwrap();
    let f = Fence::new();
    assert!(l.to_fence(&f, "k", 0).is_ok());
    let e = dir.launch("rival", &[(out, false)]).err().expect("a reader of the slot the device writes is refused");
    assert!(e.contains("rival could not start") && e.contains("a GPU write by k"), "{e}");
    assert!(dir.launch("probe", &[(a, false)]).is_ok(), "a reader beside the device's read");
    f.mark_committed();
    f.complete();
    assert!(dir.launch("next", &[(out, true)]).is_ok(), "the device's pins end with its fence");

    /// A discrete GPU's copy of a block, at an address of its own.
    struct Copy(Mutex<Vec<u8>>, u64, Arc<AtomicU64>);
    impl Mirror for Copy {
        fn upload(&self, host: &[u8], _: &Fence) {
            *self.0.lock().unwrap() = host.to_vec();
            self.2.fetch_add(1, SeqCst);
        }
        fn download(&self, host: &mut [u8]) {
            host.copy_from_slice(&self.0.lock().unwrap());
        }
        fn device_address(&self) -> Option<u64> {
            Some(self.1)
        }
    }
    let ups = Arc::new(AtomicU64::new(0));
    let copy = |addr: u64| Box::new(Copy(Mutex::new(Vec::new()), addr, ups.clone()));
    dir.replicate(out, 1, copy(0x1_0000_0000)).unwrap();
    assert!(dir.replicate(out, 1, copy(0x1_1000_0000)).is_err(), "one replica a domain");
    let l = dir.launch("k", &[(out, true)]).unwrap();
    let Err(refused) = l.to_fence(&Fence::new(), "k", 1) else { panic!("a block reached has no replica in domain 1") };
    let (l, e) = *refused;
    assert!(e.contains("no replica"), "{e}");
    let rows = l.table().rows().to_vec();
    assert!(rerail_blocks::gens::device_rows_in(&rows, &l.pins(), 1).is_err(), "nor a table row there");
    drop(l);
    dir.replicate(a, 1, copy(0x2_0000_0000)).unwrap();
    let l = dir.launch("k", &[(out, true)]).unwrap();
    let rows = l.table().rows().to_vec();
    let dev = rerail_blocks::gens::device_rows_in(&rows, &l.pins(), 1).unwrap();
    let at = |id: BlockId| dev.iter().find(|d| d.id == id.0).map(|d| d.addr);
    assert_eq!((at(out), at(a)), (Some(0x1_0000_0000), Some(0x2_0000_0000)), "each row names its replica");
    assert!(dev.iter().zip(&rows).all(|(d, r)| (d.id, d.kind, d.version, d.len) == (r.id, r.kind, r.version, r.len)));
    let g = Fence::new();
    assert!(l.to_fence(&g, "k", 1).is_ok());
    assert_eq!(ups.load(SeqCst), 2, "each replica brought to the latest version once");
    assert!(dir.launch("rival", &[(out, false)]).is_err(), "held until the fence completes");
    g.mark_committed();
    g.complete();
}

// ---- Bounds (Operations §4, the bounds axis): each held by the executor ---------------------------------------------

/// A values block's `i`th value as an item's report: each item reports its value (steps, a stack's depth).
fn report_items(c: &Ctx<'_, '_, '_>) -> u32 {
    samples::vals(c.read(0)).len() as u32
}

fn report_steps(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    for i in t.items() {
        let n = samples::vals(c.read(0))[i] as u32;
        if !c.steps(n) {
            return;
        }
    }
}

fn report_depth(c: &mut Ctx<'_, '_, '_>, t: Tile) {
    for i in t.items() {
        let n = samples::vals(c.read(0))[i] as u32;
        if !c.depth(n) {
            return;
        }
    }
}

/// One item: the phase's sample size written to the out block.
fn take_sample(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let n = c.sample();
    let ([], [o]) = c.views([], [1]);
    samples::vals_mut(o, 1)[0] = i64::from(n);
}

/// One item: every input value appended to an allocation of the out block, after asking room for it; the allocation's
/// place and length in the out block's body. Run again over a grown block, it finds its own write and does nothing.
fn append(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let n = samples::vals(c.read(0)).len();
    if word::<u64>(c.read(1), BODY_AT + 8) != 0 || !c.room(1, 8 * n) {
        return;
    }
    let v: Vec<i64> = samples::vals(c.read(0)).to_vec(); // copy: not block data (a test twin's input values)
    let pin = c.pin_mut(1);
    let at = pin.block().alloc(8 * n, 64);
    let b = pin.bytes_mut().expect("a write pin");
    for (k, x) in v.iter().enumerate() {
        b[at + 8 * k..at + 8 * k + 8].copy_from_slice(&x.to_ne_bytes());
    }
    rerail_blocks::gens::set_word::<u64>(b, BODY_AT, at as u64);
    rerail_blocks::gens::set_word::<u64>(b, BODY_AT + 8, n as u64);
}

fn appended(_: &Table<'_>, _: usize, b: &[u8]) -> Vec<Vec<i64>> {
    let (at, n) = (word::<u64>(b, BODY_AT) as usize, word::<u64>(b, BODY_AT + 8) as usize);
    (0..n).map(|k| vec![k as i64, i64::from_ne_bytes(b[at + 8 * k..at + 8 * k + 8].try_into().unwrap())]).collect()
}

const ONE: fn(&Ctx<'_, '_, '_>) -> u32 = |_| 1;
static STEPS_TWIN: [TwinPhase; 1] = [TwinPhase { name: "walk", items: report_items, run: report_steps, again: None }];
static DEPTH_TWIN: [TwinPhase; 1] = [TwinPhase { name: "walk", items: report_items, run: report_depth, again: None }];
static SAMPLE_TWIN: [TwinPhase; 1] = [TwinPhase { name: "trial", items: ONE, run: take_sample, again: None }];
static APPEND_TWIN: [TwinPhase; 1] = [TwinPhase { name: "append", items: ONE, run: append, again: None }];

fn bounded(name: &'static str, slots: Vec<SlotSpec>, phase: Phase, facts: FactsFn) -> &'static Signature {
    sig(name, slots, vec![phase], facts)
}

fn walk(inside: Inside, bounds: &'static [Bound]) -> Phase {
    Phase { name: "walk", found: Found::Fixed, unit: Unit::Lane, inside, across: Across::Nothing, output: Output::ByIndex, bounds }
}

fn declared(sig: &'static Signature, twin: &'static [TwinPhase]) -> Result<(Registry, OpId), String> {
    let mut reg = Registry::default();
    let op = reg.declare(sig)?;
    reg.register_reference(Version::twin(op, "twin", twin, sig.shape))?;
    Ok((reg, op))
}

#[test]
fn an_item_past_its_steps_or_its_stack_depth_fails_the_launch() {
    let io = || vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)];
    for (name, inside, bound, twin, what) in [
        ("steps", Inside::Nothing, &[Bound::Steps(10)][..], &STEPS_TWIN[..], "steps"),
        ("depth", Inside::Stacks, &[Bound::StackDepth(10)][..], &DEPTH_TWIN[..], "stack depth"),
    ] {
        let (reg, op) = declared(bounded(name, io(), walk(inside, bound), values), twin).unwrap();
        for (v, ok) in [(vec![3, 10, 7], true), (vec![3, 11, 7], false)] {
            let mut dir = Directory::default();
            let (i, o) = (values_block(&mut dir, &v), dir.make(kind::FACTS, samples::vals_bytes(1)));
            let r = run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE);
            if ok {
                r.unwrap();
            } else {
                let e = r.unwrap_err();
                assert!(e.contains(&format!("an item's {what} reached 11, past its bound of 10")), "{e}");
            }
        }
    }
    // A twin reporting what its phase's shape does not bound fails: its twin does what its axes say it does not.
    let (reg, op) = declared(bounded("unbounded steps", io(), walk(Inside::Nothing, &[]), values), &STEPS_TWIN).unwrap();
    let mut dir = Directory::default();
    let (i, o) = (values_block(&mut dir, &[1]), dir.make(kind::FACTS, samples::vals_bytes(1)));
    let e = run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("reports steps, and its phase's shape bounds none"), "{e}");
}

#[test]
fn a_phase_bounds_what_its_axes_say_it_does() {
    let io = || vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)];
    let refused = |s: &'static Signature| Registry::default().declare(s).unwrap_err();
    let e = refused(bounded("stacks unbounded", io(), walk(Inside::Stacks, &[]), values));
    assert!(e.contains("its units hold stacks, and it bounds no stack depth"), "{e}");
    let e = refused(bounded("depth, no stacks", io(), walk(Inside::Nothing, &[Bound::StackDepth(4)]), values));
    assert!(e.contains("bounds a stack's depth, and its units hold no stacks"), "{e}");
    let claim = Phase { output: Output::Claim, ..walk(Inside::Nothing, &[]) };
    let e = refused(bounded("claim unbounded", io(), claim, values));
    assert!(e.contains("its output is claimed, and it bounds no capacity"), "{e}");
    let e = refused(bounded("twice", io(), walk(Inside::Nothing, &[Bound::Steps(1), Bound::Steps(2)]), values));
    assert!(e.contains("is given twice"), "{e}");
    let rw = vec![slot("in", R, kind::FACTS), slot("out", Mode::ReadWrite, kind::FACTS)];
    let e = refused(bounded("rerun rw", rw, walk(Inside::Nothing, &[Bound::Capacity(Over::Rerun)]), appended));
    assert!(e.contains("runs the launch again past a capacity, and slot out is read and written"), "{e}");
}

#[test]
fn a_sample_is_its_bound_and_a_phase_without_one_has_none() {
    let io = vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)];
    let trial = Phase { name: "trial", ..walk(Inside::Nothing, &[Bound::Sample(5)]) };
    let (reg, op) = declared(bounded("sample", io.clone(), trial, values), &SAMPLE_TWIN).unwrap();
    let mut dir = Directory::default();
    let (i, o) = (values_block(&mut dir, &[0]), dir.make(kind::FACTS, samples::vals_bytes(1)));
    run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE).unwrap();
    assert_eq!(samples::vals(dir.read("t", &[o]).unwrap().block(0))[0], 5);
    let (reg, op) = declared(bounded("no sample", io, Phase { name: "trial", ..walk(Inside::Nothing, &[]) }, values), &SAMPLE_TWIN).unwrap();
    let e = run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE).unwrap_err();
    assert!(e.contains("reports a sample, and its phase's shape bounds none"), "{e}");
}

#[test]
fn a_slot_past_its_capacity_is_refused_grown_or_run_again() {
    let io = || vec![slot("in", R, kind::FACTS), slot("out", W, kind::FACTS)];
    let v: Vec<i64> = (0..2000).collect();
    let want: Vec<Vec<i64>> = v.iter().map(|&x| vec![x, x]).collect();
    for over in [Over::Refuse, Over::Grow, Over::Rerun] {
        let phase = Phase { name: "append", output: Output::Claim, ..walk(Inside::Nothing, Box::leak(Box::new([Bound::Capacity(over)]))) };
        let (reg, op) = declared(bounded(Box::leak(format!("append {over:?}").into_boxed_str()), io(), phase, appended), &APPEND_TWIN).unwrap();
        let mut dir = Directory::default();
        let i = values_block(&mut dir, &v);
        // A capacity of 4 KiB in a reservation of 1 MiB: the 16,000 bytes do not fit until it grows.
        let o = dir.make_capped(kind::FACTS, 16, 4096, 1 << 20);
        let r = run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE);
        match over {
            Over::Refuse => {
                let e = r.unwrap_err();
                assert!(e.contains("slot 1 (out) needs a capacity of") && e.contains("refused"), "{e}");
            }
            Over::Grow | Over::Rerun => {
                r.unwrap();
                assert!(dir.capacity(o) > 16_000, "{over:?}: grown to {}", dir.capacity(o));
                let l = dir.read("t", &[o]).unwrap();
                assert_eq!(appended(&l.table(), 1, l.block(0)), want, "{over:?}");
            }
        }
        // Past the reservation: refused, naming it.
        if over != Over::Refuse {
            let mut dir = Directory::default();
            let big: Vec<i64> = (0..10_000).collect();
            let i = values_block(&mut dir, &big);
            let o = dir.make_capped(kind::FACTS, 16, 4096, 8192);
            let e = run_cpu(&reg, &dir, op, &[i, o], &[], Tiling::WHOLE).unwrap_err();
            assert!(e.contains("past its reservation"), "{over:?}: {e}");
        }
    }
}

#[test]
fn every_launch_counts_in_the_evaluation_and_one_past_its_budget_fails() {
    let (sig, twin) = shapes().into_iter().find(|(s, _)| s.name == "evens").unwrap();
    let (reg, op) = declared(sig, twin).unwrap();
    let mut dir = Directory::default();
    let (c, _) = case("evens", &mut dir);
    // Alone, a launch is an evaluation of its own: three phases, three launches.
    run_cpu(&reg, &dir, op, &c.slots, &c.params, Tiling::WHOLE).unwrap();
    assert_eq!(dir.launches(), 3);
    let e = {
        let _e = dir.evaluation(5);
        run_cpu(&reg, &dir, op, &c.slots, &c.params, Tiling::WHOLE).unwrap();
        run_cpu(&reg, &dir, op, &c.slots, &c.params, Tiling::WHOLE).unwrap_err()
    };
    assert!(e.contains("launch 6 of an evaluation whose budget is 5 (0601): evens.count 2, evens.scan 2, evens.place 2"), "{e}");
}
