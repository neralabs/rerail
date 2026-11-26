//! Strata and levels (B§10): a stratum's rules over facts and over tapes, under the contract's laws; the refusals;
//! and B§8's loop, its loads and its program's δ, over files on disk.

use super::*;
use crate::contract::{Bench, Global, apply, harness_on, push_layer};
use crate::files::doc_of;
use crate::intern::{InternOps, intern};
use crate::k0::K0Ops;
use crate::l1::{L1, tape_row};

type Pairs = Vec<(Vec<i64>, i32)>;

/// A bench with the stratum's operations and L1, and the atoms: rules compiled from text by L1, read as a program.
struct P {
    bench: Bench,
    atoms: Atoms,
    iops: InternOps,
    l1: L1,
    ops: PhaseOps,
}

impl P {
    fn new() -> P {
        let mut bench = Bench::new().unwrap();
        let iops = InternOps::declare(&mut bench.reg).unwrap();
        let k0 = K0Ops::declare(&mut bench.reg).unwrap();
        let drops = DropOps::declare(&mut bench.reg).unwrap();
        let ops = PhaseOps::declare(&mut bench.reg, drops).unwrap();
        let atoms = Atoms::new(&mut bench.dir);
        P { bench, atoms, iops, l1: L1 { ops: k0, drops }, ops }
    }

    /// A document of `src` at `path` (its module the path less `.rerail`), interned: its row in a document set.
    fn doc(&mut self, src: &str, path: &str) -> Vec<i64> {
        let b = &mut self.bench;
        let ids = doc_of(&b.reg, &b.answers, &mut b.dir, &mut b.blobs, src).unwrap();
        let module = path.strip_suffix(".rerail").unwrap_or(path);
        let (s, t) = (b.blobs.get(&b.dir, &ids.source).unwrap().unwrap(), b.blobs.get(&b.dir, &ids.tape).unwrap().unwrap());
        let syms = intern(&b.reg, &self.iops, &mut b.dir, &mut b.blobs, self.atoms, s, t, path, module).unwrap();
        let d = self.s(module);
        tape_row(d, self.s(path), 0, &ids, &syms)
    }

    fn global(&self) -> Global {
        Global { block: self.atoms.0, count: u64::from(self.atoms.count(&self.bench.dir)) }
    }

    fn s(&self, x: &str) -> i64 {
        self.atoms.find(&self.bench.dir, x).unwrap_or_else(|| panic!("{x:?} not interned"))
    }

    /// L1's K-form rows of the rules `src`, module `module`.
    fn rows(&mut self, src: &str, module: &str) -> Vec<BTreeSet<Vec<i64>>> {
        let row = self.doc(src, &format!("{module}.rerail"));
        let mut t = Trace::new(&tape_shape()).unwrap().with_roll_up(RollUp::Never);
        let g = self.global();
        let b = &mut self.bench;
        let h = push_layer(&b.reg, &b.traces, &mut b.dir, &mut t, &[(row, 1)]).unwrap();
        apply(&self.l1, &mut b.cx(), &[(&t, Some(h))], &[g]).unwrap()
    }

    /// The program of the rules `src`, one module, or its refusal.
    fn program(&mut self, src: &str, module: &str) -> Result<Program, Refused> {
        let rows = self.rows(src, module);
        let l = self.bench.dir.read("atoms", &[self.atoms.0]).unwrap();
        Program::read(&rows, l.block(0))
    }

    fn known(&self) -> Known {
        Known::new(self.bench.dir.read("atoms", &[self.atoms.0]).unwrap().block(0))
    }

    /// Module `module`'s stratum in `p`, reading relations `shapes` (by name).
    fn module_op(&self, p: &Program, module: &str, shapes: &[(&str, Shape)]) -> Stratum {
        let m = self.s(module);
        let rules = p.modules[&m].rules.clone();
        let shapes: BTreeMap<i64, Shape> = shapes.iter().map(|(n, s)| (self.s(n), s.clone())).collect();
        let l = self.bench.dir.read("atoms", &[self.atoms.0]).unwrap();
        Stratum::new(self.ops, m, rules.clone(), rules, self.known(), &shapes, l.block(0)).unwrap()
    }

    /// `st` under the harness over a chain of its input ports' changes, `steps` (a step each port's pairs; the
    /// first state every port ∅), the cache dropped at a step chosen by each seed.
    fn harness(&mut self, st: &Stratum, steps: &[Vec<Pairs>]) {
        let ports = st.ports().unwrap();
        let mut ins: Vec<Trace> = ports.ins.iter().map(|p| Trace::new(&p.shape).unwrap().with_roll_up(RollUp::Never)).collect();
        let mut heads = vec![vec![None; ins.len()]];
        for step in steps {
            let mut now = heads.last().cloned().unwrap();
            for (k, pairs) in step.iter().enumerate() {
                if !pairs.is_empty() {
                    let b = &mut self.bench;
                    now[k] = Some(push_layer(&b.reg, &b.traces, &mut b.dir, &mut ins[k], pairs).unwrap());
                }
            }
            heads.push(now);
        }
        let globals = vec![vec![self.global()]; heads.len()];
        for seed in 1..=2u64 {
            let drop_at = 1 + (rerail_blocks::layer::mix(seed) % (heads.len() as u64 - 1)) as usize;
            harness_on(st, &mut self.bench, &mut ins, &heads, &globals, drop_at, None).unwrap_or_else(|b| panic!("{b:?}"));
        }
    }
}

fn fact(name: &str, arity: usize) -> (&str, Shape) {
    (name, head_shape(name, arity, false))
}

fn add(rows: &[&[i64]]) -> Pairs {
    rows.iter().map(|r| (r.to_vec(), 1)).collect()
}

fn drop(rows: &[&[i64]]) -> Pairs {
    rows.iter().map(|r| (r.to_vec(), -1)).collect()
}

fn both(a: Pairs, b: Pairs) -> Pairs {
    a.into_iter().chain(b).collect()
}

/// Δwave over facts (B§10 step 2): a join, an anti-join and a filter, cross-file rules by their delta terms, the
/// heads' derivation counts beside them, under every law of the contract.
#[test]
fn a_wave_of_three_rules_over_facts_passes_the_harness() {
    let mut p = P::new();
    let src = "two(x, z) :- edge(x, y), edge(y, z).\nlone(x) :- edge(x, _), !mark(x).\nup(x, y) :- edge(x, y), x < y.\n";
    let prog = p.program(src, "wave").unwrap();
    let st = p.module_op(&prog, "wave", &[fact("edge", 2), fact("mark", 1)]);
    assert!(!st.recursive);
    assert_eq!(st.ports().unwrap().outs.len(), 6, "three heads, each with its counts");
    p.harness(
        &st,
        &[
            vec![add(&[&[1, 2], &[2, 3], &[3, 1], &[2, 4]]), vec![]],
            vec![vec![], add(&[&[2]])],
            vec![vec![], vec![]],
            vec![both(drop(&[&[2, 3]]), add(&[&[4, 5]])), both(drop(&[&[2]]), add(&[&[4]]))],
            vec![drop(&[&[1, 2], &[3, 1]]), vec![]],
            vec![add(&[&[1, 2], &[2, 2]]), add(&[&[1]])],
        ],
    );
}

/// The edges of the chain the closure tests run: a cycle, a removal on it (rows held up only by the cycle go), its
/// return, a shortcut, a chain's removal.
fn closure_chain() -> Vec<Vec<Pairs>> {
    vec![
        vec![add(&[&[1, 2], &[2, 3], &[3, 1], &[3, 4]])],
        vec![drop(&[&[3, 1]])],
        vec![vec![]],
        vec![add(&[&[3, 1], &[4, 5]])],
        vec![add(&[&[1, 3]])],
        vec![drop(&[&[1, 2]])],
        vec![drop(&[&[2, 3], &[3, 4]])],
        vec![add(&[&[5, 1], &[2, 3]])],
    ]
}

/// Δfix (B§10 step 3): a transitive closure through edits, removals by levels (0364), under every law (each output
/// by its key: a level is a witness).
#[test]
fn a_transitive_closure_passes_the_harness_through_edits() {
    let mut p = P::new();
    let prog = p.program("path(x, y) :- edge(x, y).\npath(x, z) :- path(x, y), edge(y, z).\n", "fix").unwrap();
    let st = p.module_op(&prog, "fix", &[fact("edge", 2)]);
    assert!(st.recursive);
    let outs = st.ports().unwrap().outs;
    assert_eq!(outs.len(), 1, "a recursive head's levels are its last column, with no counts");
    assert!(outs[0].shape.levels);
    p.harness(&st, &closure_chain());
}

/// Each row's level is a witness (0364 §1): a derivation from rows of lower level holds, through every edit. An
/// added row gets one more than the highest level it reads (§2.4), so an edge's own row is at level 0.
#[test]
fn levels_stay_witnesses_through_edits() {
    let mut p = P::new();
    let prog = p.program("path(x, y) :- edge(x, y).\npath(x, z) :- path(x, y), edge(y, z).\n", "lv").unwrap();
    let st = p.module_op(&prog, "lv", &[fact("edge", 2)]);
    let mut edges = Trace::new(&head_shape("edge", 2, false)).unwrap().with_roll_up(RollUp::Never);
    let mut inst = Instance::new(&st, &mut p.bench.dir).unwrap();
    let g = p.global();
    let globals = [GlobalIn { block: g.block, before: Some(g.count), after: g.count }];
    let mut was = None;
    for step in closure_chain() {
        let b = &mut p.bench;
        let now = if step[0].is_empty() { was } else { Some(push_layer(&b.reg, &b.traces, &mut b.dir, &mut edges, &step[0]).unwrap()) };
        inst.step(&st, &mut b.cx(), &[(&edges, was, now)], &globals, None).unwrap();
        was = now;
        let e = rows_at(&b.dir, &edges, now).unwrap();
        let rows = rows_at(&b.dir, &inst.outs[0], inst.outs[0].head()).unwrap();
        for r in &rows {
            let (x, y, l) = (r[0], r[1], r[2]);
            let held = e.contains(&vec![x, y]) || rows.iter().any(|q| q[0] == x && q[2] < l && e.contains(&vec![q[1], y]));
            assert!(held, "path({x}, {y}) at level {l}: no derivation from lower levels, in {rows:?} over {e:?}");
            if l == 0 {
                assert!(e.contains(&vec![x, y]), "a level-0 row is an edge's");
            }
        }
    }
}

/// An anti-join whose matches come and go, a fact, and filters first (B§10's step 2): a `not` and a `cmp` of
/// constants run on the empty tuple.
#[test]
fn an_anti_join_whose_matches_come_and_go_with_facts_and_filters_first() {
    let mut p = P::new();
    let src = "free(x) :- item(x), !used(x).\nseven(7).\nquiet(1) :- !alarm(1).\nsure(2) :- 1 < 2.\nnever(3) :- 2 < 1.\n";
    let prog = p.program(src, "anti").unwrap();
    let st = p.module_op(&prog, "anti", &[fact("item", 1), fact("used", 1), fact("alarm", 1)]);
    p.harness(
        &st,
        &[
            vec![add(&[&[1], &[2], &[3]]), vec![], vec![]],
            vec![vec![], add(&[&[2]]), add(&[&[1]])],
            vec![vec![], add(&[&[1], &[3]]), vec![]],
            vec![vec![], drop(&[&[2]]), drop(&[&[1]])],
            vec![drop(&[&[3]]), drop(&[&[1], &[3]]), vec![]],
        ],
    );
}

/// B§7's refusals, each naming its rule.
#[test]
fn each_refusal_of_b7_names_its_rule() {
    let mut p = P::new();
    let cases = [
        ("a(x) :- b(x), !c(y).\n", "is not bound by an earlier step"),
        ("a(x) :- b(x), x < y.\n", "is not bound by an earlier step"),
        ("a(x, y) :- b(x).\n", "is bound by no step"),
        ("a(x) :- b(x), !a(x).\n", "its own stratum's head"),
    ];
    for (i, (src, why)) in cases.into_iter().enumerate() {
        let e = p.program(src, &format!("refused{i}")).unwrap_err();
        assert!(e.why.contains(why) && e.rule >= 0, "{src}: {e:?}");
    }
    // A view read with other columns than its own, and a stratum written by single-file and cross-file rules both:
    // refused when made.
    let prog = p.program("n(x) :- node(x).\n", "arity").unwrap();
    let l = p.bench.dir.read("atoms", &[p.atoms.0]).unwrap();
    let m = p.s("arity");
    let rules = prog.modules[&m].rules.clone();
    let e = Stratum::new(p.ops, m, rules.clone(), rules, p.known(), &BTreeMap::new(), l.block(0)).err().expect("refused");
    assert!(e.why.contains("view node of 5 columns with 1"), "{e:?}");
    std::mem::drop(l);
    let prog = p.program("h(d, t) :- kind(d, t, \"wildcard\").\nh(x, y) :- e(x, y).\n", "mixed").unwrap();
    let l = p.bench.dir.read("atoms", &[p.atoms.0]).unwrap();
    let m = p.s("mixed");
    let rules = prog.modules[&m].rules.clone();
    let e = Stratum::new(p.ops, m, rules.clone(), rules, p.known(), &BTreeMap::new(), l.block(0)).err().expect("refused");
    assert!(e.why.contains("single-file and cross-file"), "{e:?}");
}

/// A single-file rule over tapes (B§10's one rule over a three-line tape): each document's rows at its grain, a
/// changed document's retracted from its counts and run again over its new tape, under every law.
#[test]
fn a_single_file_rule_over_tapes_passes_the_harness() {
    let mut p = P::new();
    let prog = p.program("wild(d, t) :- kind(d, t, \"wildcard\").\n", "sf").unwrap();
    let st = p.module_op(&prog, "sf", &[]);
    assert!(st.tape && st.ins.is_empty());
    let a1 = p.doc("x(_) :- y(_).\nz(1).\nw(_, _) :- v(_).\n", "a.rerail");
    let a2 = p.doc("x(_) :- y(1).\nz(_) :- q(_).\nw(2).\n", "a.rerail");
    let b = p.doc("u(_) :- t(_, _).\n", "b.rerail");
    p.harness(&st, &[vec![add(&[&a1])], vec![both(drop(&[&a1]), add(&[&a2]))], vec![add(&[&b])], vec![vec![]], vec![drop(&[&a2])]]);
    // The rows are the wildcards' nodes, a document's each partitioned by it.
    let d = p.s("a");
    let all = p.read_rows(&st, &[(&a1, 1)]);
    assert_eq!(all.len(), 5, "a1's five wildcards: {all:?}");
    assert!(all.iter().all(|r| r[0] == d));
}

impl P {
    /// `st`'s first head's rows over a document set of `docs`.
    fn read_rows(&mut self, st: &Stratum, docs: &[(&Vec<i64>, i32)]) -> BTreeSet<Vec<i64>> {
        let mut t = Trace::new(&tape_shape()).unwrap().with_roll_up(RollUp::Never);
        let pairs: Pairs = docs.iter().map(|(r, d)| ((*r).clone(), *d)).collect();
        let g = self.global();
        let b = &mut self.bench;
        let h = push_layer(&b.reg, &b.traces, &mut b.dir, &mut t, &pairs).unwrap();
        apply(st, &mut b.cx(), &[(&t, Some(h))], &[g]).unwrap().swap_remove(0)
    }
}

/// B§8's run over files on disk: L1 over l0, then the levels, with the run's facts.
struct Run {
    bench: Bench,
    root: std::path::PathBuf,
    docs: DocSet,
    atoms: Atoms,
    l1: L1,
    inst: Instance,
    levels: Levels,
    last: Option<crate::boot::Pins>,
    facts: Vec<(i64, Trace, Shape)>,
}

const L0_PATH: &str = "bootstrap-l0.rerail";

impl Run {
    fn new(name: &str) -> Run {
        let mut bench = Bench::new().unwrap();
        let (root, store) = crate::files::tests::root(name);
        crate::files::tests::write(&root.join(L0_PATH), crate::L0, 60);
        let atoms = Atoms::new(&mut bench.dir);
        let world = crate::files::World::new(root.clone(), store, bench.answers, &bench.dir, [None; 3]).unwrap();
        let iops = InternOps::declare(&mut bench.reg).unwrap();
        let k0 = K0Ops::declare(&mut bench.reg).unwrap();
        let drops = DropOps::declare(&mut bench.reg).unwrap();
        let l1 = L1 { ops: k0, drops };
        let levels = Levels::new(PhaseOps::declare(&mut bench.reg, drops).unwrap());
        let inst = Instance::new(&l1, &mut bench.dir).unwrap();
        let docs = DocSet::new(world, iops, drops, &bench.dir, None, None).unwrap();
        Run { bench, root, docs, atoms, l1, inst, levels, last: None, facts: Vec::new() }
    }

    /// The run kept in store `folder` over root `root`, at its last pins (none: a new store): L1's instance, the
    /// document set and the levels resumed where the last run left them, and the run's facts where level 0 last read
    /// them (its pins' `seen`).
    fn open(root: &std::path::Path, folder: &std::path::Path) -> Run {
        let store = crate::files::Store::open(folder).unwrap();
        let (mut bench, pins) = Bench::open(&store.kept()).unwrap();
        let atoms = Atoms::open(&mut bench.dir, &store.atoms()).unwrap();
        let world = crate::files::World::new(root.to_path_buf(), store, bench.answers, &bench.dir, pins.as_ref().map_or([None; 3], |p| p.1.world)).unwrap();
        let iops = InternOps::declare(&mut bench.reg).unwrap();
        let k0 = K0Ops::declare(&mut bench.reg).unwrap();
        let drops = DropOps::declare(&mut bench.reg).unwrap();
        let l1 = L1 { ops: k0, drops };
        let mut levels = Levels::new(PhaseOps::declare(&mut bench.reg, drops).unwrap());
        let place = bench.dir.lasting();
        let p = pins.as_ref().map(|p| &p.1);
        let inst = Instance::new(&l1, &mut bench.dir).unwrap().resumed(place, p.map_or(&[][..], |p| &p.outs), p.and_then(|p| p.want), p.map_or(&[][..], |p| &p.asks));
        let docs = DocSet::new(world, iops, drops, &bench.dir, p.and_then(|p| p.tape), pins.as_ref().map(|p| p.0)).unwrap();
        let mut facts = Vec::new();
        if let Some(p) = p {
            levels.resume(&bench.dir, atoms, place, &inst.outs, &p.levels).unwrap();
            for &(rel, h) in &p.levels[0].seen {
                let h = h.expect("a fact read");
                facts.push((rel, Trace::at_layer(&bench.dir, place, h).unwrap(), rows_shape(&bench.dir, h).unwrap()));
            }
        }
        let last = pins.map(|(_, p)| crate::boot::Pins { atoms: Global { block: atoms.0, ..p.atoms }, ..p });
        Run { bench, root: root.to_path_buf(), docs, atoms, l1, inst, levels, last, facts }
    }

    /// Names interned as the engine interns any text: a document's strings (with the facts' relations').
    fn names(&mut self, names: &[&str]) {
        let names: Vec<&str> = names.iter().copied().chain(["load", "boot"]).collect();
        let src = format!("names({}).\n", names.iter().map(|n| format!("{n:?}")).collect::<Vec<_>>().join(", "));
        let b = &mut self.bench;
        let ids = doc_of(&b.reg, &b.answers, &mut b.dir, &mut b.blobs, &src).unwrap();
        let (s, t) = (b.blobs.get(&b.dir, &ids.source).unwrap().unwrap(), b.blobs.get(&b.dir, &ids.tape).unwrap().unwrap());
        intern(&b.reg, &self.docs.iops, &mut b.dir, &mut b.blobs, self.atoms, s, t, "names.rerail", "names").unwrap();
    }

    fn s(&self, x: &str) -> i64 {
        self.atoms.find(&self.bench.dir, x).unwrap_or_else(|| panic!("{x:?} not interned"))
    }

    /// The run's fact `rel`'s rows moved to `rows` (a layer of their change).
    fn fact(&mut self, rel: &str, rows: &[Vec<i64>]) {
        let r = self.s(rel);
        if !self.facts.iter().any(|f| f.0 == r) {
            let shape = head_shape(rel, rows[0].len(), false);
            let t = Trace::new(&shape).unwrap().with_roll_up(RollUp::Never).resumed(self.bench.dir.lasting(), None);
            self.facts.push((r, t, shape));
        }
        let t = &mut self.facts.iter_mut().find(|f| f.0 == r).unwrap().1;
        let have = rows_at(&self.bench.dir, t, t.head()).unwrap();
        let want: BTreeSet<Vec<i64>> = rows.iter().cloned().collect();
        let pairs: Pairs = have.difference(&want).map(|r| (r.clone(), -1)).chain(want.difference(&have).map(|r| (r.clone(), 1))).collect();
        if !pairs.is_empty() {
            let b = &mut self.bench;
            push_layer(&b.reg, &b.traces, &mut b.dir, t, &pairs).unwrap();
        }
    }

    /// B§8's run from the last pins.
    fn run(&mut self) -> Work {
        self.try_run().unwrap()
    }

    /// B§8's run from the last pins, or its refusal.
    fn try_run(&mut self) -> Result<Work, String> {
        let last = self.last.take();
        let facts: Vec<FactIn<'_>> = self.facts.iter().map(|(rel, t, shape)| FactIn { rel: *rel, trace: t, shape: shape.clone() }).collect();
        let module = L0_PATH.strip_suffix(".rerail").unwrap();
        let (pins, _, work) = crate::boot::run(&self.l1, &mut self.inst, &mut self.levels, L0_PATH, module, self.atoms, &facts, &mut self.docs, &mut self.bench, last.as_ref())?;
        self.last = Some(pins);
        Ok(work)
    }

    /// Level `k`'s relation `rel`'s rows, each its relation's columns (a level dropped).
    fn rows(&self, k: usize, rel: &str) -> BTreeSet<Vec<i64>> {
        let Some(r) = self.levels.levels[k].rels.get(&self.s(rel)) else { return BTreeSet::new() };
        let n = r.shape.columns.len() - usize::from(r.shape.levels);
        rows_at(&self.bench.dir, &r.head, r.head.head()).unwrap().into_iter().map(|mut x| {
            x.truncate(n);
            x
        }).collect()
    }

    fn write(&self, path: &str, text: &str, ago: u64) {
        crate::files::tests::write(&self.root.join(path), text, ago);
    }
}

/// The self-reading check (B§10, 0686: "P2 run over `l0`'s own tape gives P2 again, row for row"): L2 is P2 run as a
/// level over the document its facts load, l0; its K-form is L1's. The level after it loads nothing and exports no
/// program, which ends the loop.
#[test]
fn p2_run_as_a_level_over_l0s_tape_gives_p2() {
    let mut r = Run::new("phase-self");
    r.names(&[L0_PATH, "bootstrap-l0", "rr"]);
    let (path, module, rr) = (r.s(L0_PATH), r.s("bootstrap-l0"), r.s("rr"));
    r.fact("load", &[vec![path, module, rr]]);
    r.run();
    assert_eq!(r.levels.levels.len(), 2, "L2 exports P3, and L3, loading nothing, exports none");
    for (k, (name, _)) in RELATIONS.iter().enumerate() {
        let p2 = rows_at(&r.bench.dir, &r.inst.outs[k], r.inst.outs[k].head()).unwrap();
        assert_eq!(r.rows(0, name), p2, "{name}: P2 over l0's tape is P2");
    }
    assert!(!r.rows(0, "k_rule").is_empty());
}

const A: &str = "base(1).\nbase(2).\ntwice(x, y) :- base(x), base(y), x < y.\n";
const B: &str = "top(x) :- twice(x, _).\n";

/// Loads (B§10): L2 runs P2 over the two K0 files its facts load, and its K-form is L1's of them, their `k_order`
/// from the `boot` facts; L3 runs that program, its strata in `k_order`. Then the program's δ: an edit of a file moves
/// L2's K-form and L3's results with it; an unchanged run is the identity; a cold run gives the same.
#[test]
fn a_level_loads_its_files_and_the_next_runs_their_program_through_edits() {
    let mut r = Run::new("phase-loads");
    r.write("a.rerail", A, 50);
    r.write("b.rerail", B, 50);
    r.names(&["a.rerail", "a", "b.rerail", "b", "rr"]);
    let (pa, ma, pb, mb, rr) = (r.s("a.rerail"), r.s("a"), r.s("b.rerail"), r.s("b"), r.s("rr"));
    r.fact("load", &[vec![pa, ma, rr], vec![pb, mb, rr]]);
    r.fact("boot", &[vec![pa, 1], vec![pb, 2]]);
    r.run();
    assert_eq!(r.levels.levels.len(), 2, "L2 exports a and b's program; L3 runs it and exports none");
    // L2's K-form is L1's of the two files.
    let (da, db) = (r.docs.key(&r.bench.dir, r.atoms, "a.rerail").unwrap(), r.docs.key(&r.bench.dir, r.atoms, "b.rerail").unwrap());
    let mut t = Trace::new(&tape_shape()).unwrap().with_roll_up(RollUp::Never);
    let rows: Pairs = rows_at(&r.bench.dir, &r.docs.tape, r.docs.tape.head()).unwrap().into_iter().filter(|x| x[0] == da || x[0] == db).map(|x| (x, 1)).collect();
    assert_eq!(rows.len(), 2);
    let g = Global { block: r.atoms.0, count: u64::from(r.atoms.count(&r.bench.dir)) };
    let l1 = {
        let b = &mut r.bench;
        let h = push_layer(&b.reg, &b.traces, &mut b.dir, &mut t, &rows).unwrap();
        apply(&r.l1, &mut b.cx(), &[(&t, Some(h))], &[g]).unwrap()
    };
    for (k, (name, _)) in RELATIONS.iter().enumerate().filter(|(_, n)| n.0 != "k_order") {
        assert_eq!(r.rows(0, name), l1[k], "{name}");
    }
    assert_eq!(r.rows(0, "k_order"), BTreeSet::from([vec![da, 1], vec![db, 2]]));
    let check = |r: &Run, twice: &[[i64; 2]], top: &[i64]| {
        assert_eq!(r.rows(1, "twice"), twice.iter().map(|x| x.to_vec()).collect(), "twice");
        assert_eq!(r.rows(1, "top"), top.iter().map(|&x| vec![x]).collect(), "top");
    };
    check(&r, &[[1, 2]], &[1]);
    // Unchanged: the identity.
    let work = r.run();
    assert!(work.is_checks_only(), "{work:?}");
    // a's edit: its module's rules removed and its new ones run cold (O3), and b's over a's new rows.
    r.write("a.rerail", &format!("{A}base(3).\n"), 40);
    r.run();
    check(&r, &[[1, 2], [1, 3], [2, 3]], &[1, 2]);
    // b's edit.
    r.write("b.rerail", "top(x) :- twice(_, x).\n", 30);
    r.run();
    check(&r, &[[1, 2], [1, 3], [2, 3]], &[2, 3]);
    // A file no longer loaded: its program gone, and what it wrote.
    r.fact("load", &[vec![pa, ma, rr]]);
    r.run();
    assert_eq!(r.rows(0, "k_order"), BTreeSet::from([vec![da, 1]]));
    check(&r, &[[1, 2], [1, 3], [2, 3]], &[]);
    // Cold: the same rows.
    let warm: Vec<BTreeSet<Vec<i64>>> = ["twice", "top", "k_rule"].iter().map(|n| r.rows(1, n).into_iter().chain(r.rows(0, n)).collect()).collect();
    r.last = None;
    r.run();
    let cold: Vec<BTreeSet<Vec<i64>>> = ["twice", "top", "k_rule"].iter().map(|n| r.rows(1, n).into_iter().chain(r.rows(0, n)).collect()).collect();
    assert_eq!(cold, warm);
}

/// A program edit that makes a stratum recursive changes its heads' shape (a level column, 0364): the level runs
/// again from ∅, and the levels after it too, with the same rows a cold run gives.
#[test]
fn an_edit_that_makes_a_stratum_recursive_runs_its_level_from_empty() {
    let mut r = Run::new("phase-reshape");
    r.write("a.rerail", "e(1, 2).\ne(2, 3).\np(x, y) :- e(x, y).\n", 50);
    r.names(&["a.rerail", "a", "rr"]);
    let (pa, ma, rr) = (r.s("a.rerail"), r.s("a"), r.s("rr"));
    r.fact("load", &[vec![pa, ma, rr]]);
    r.run();
    assert_eq!(r.rows(1, "p"), BTreeSet::from([vec![1, 2], vec![2, 3]]));
    r.write("a.rerail", "e(1, 2).\ne(2, 3).\np(x, y) :- e(x, y).\np(x, z) :- p(x, y), e(y, z).\n", 40);
    r.run();
    assert_eq!(r.rows(1, "p"), BTreeSet::from([vec![1, 2], vec![2, 3], vec![1, 3]]));
    let p = r.levels.levels[1].rels.get(&r.s("p")).unwrap();
    assert!(p.shape.levels && p.counts.is_none());
}

/// What a run keeps lasts across processes (B§8's `last`): a later process opens the pins, and its levels, their
/// programs read again from where each level before left its K-form, and the run's facts, where level 0 last read
/// them, start where the last run left them. An unchanged root is the identity; an edit then is the same as in the
/// first process.
#[test]
fn a_later_process_resumes_the_levels_where_the_last_run_left_them() {
    let _racy = crate::files::tests::racy_window();
    let (root, store) = crate::files::tests::root("phase-restart");
    let folder = store.folder.clone();
    std::mem::drop(store);
    crate::files::tests::write(&root.join("a.rerail"), A, 50);
    crate::files::tests::write(&root.join("b.rerail"), B, 50);
    crate::files::tests::past_racy_window();
    let rows = |r: &Run| (r.rows(0, "k_rule"), r.rows(1, "twice"), r.rows(1, "top"));
    let first = {
        let mut r = Run::open(&root, &folder);
        assert!(r.last.is_none());
        r.names(&["a.rerail", "a", "b.rerail", "b", "rr"]);
        let (pa, ma, pb, mb, rr) = (r.s("a.rerail"), r.s("a"), r.s("b.rerail"), r.s("b"), r.s("rr"));
        r.fact("load", &[vec![pa, ma, rr], vec![pb, mb, rr]]);
        r.fact("boot", &[vec![pa, 1], vec![pb, 2]]);
        r.run();
        rows(&r)
    };
    assert_eq!(first.2, BTreeSet::from([vec![1]]));
    crate::files::forget_observer(&folder);
    let mut r = Run::open(&root, &folder);
    assert!(r.last.is_some(), "the last run's pins");
    assert_eq!((r.levels.levels.len(), r.facts.len()), (2, 2), "the levels and the facts, resumed");
    assert_eq!(rows(&r), first, "the levels' relations, where the last run left them");
    let work = r.run();
    assert!(work.is_checks_only(), "an unchanged root: {work:?}");
    r.write("a.rerail", &format!("{A}base(3).\n"), 40);
    r.run();
    assert_eq!(r.rows(1, "twice"), BTreeSet::from([vec![1, 2], vec![1, 3], vec![2, 3]]));
    assert_eq!(r.rows(1, "top"), BTreeSet::from([vec![1], vec![2]]));
}

/// The kernel's files (B§10 step 7), by path relative to the bootstrap's root, in stratum order: K1's reader, then
/// the binding lowerer, one file a stratum (O4).
const KERNEL: [(&str, &str); 5] = [
    ("kernel/k1-0-read.rerail", include_str!("../rerail/kernel/k1-0-read.rerail")),
    ("kernel/k1-1-steps.rerail", include_str!("../rerail/kernel/k1-1-steps.rerail")),
    ("kernel/k1-2-binds.rerail", include_str!("../rerail/kernel/k1-2-binds.rerail")),
    ("kernel/k1-3-place.rerail", include_str!("../rerail/kernel/k1-3-place.rerail")),
    ("kernel/k1-4-emit.rerail", include_str!("../rerail/kernel/k1-4-emit.rerail")),
];

/// What L3 reads: the stand-in for the compiler's files (Bootstrap §6: "P3 loads the full compiler"), a K0 file of
/// L2's whose `load` facts P3 then asks, with their files' strata: the facts first, as a base the rules negate.
const ASK: &str = "load(\"u-facts.rerail\", \"u-facts\", \"rr\").\nboot(\"u-facts.rerail\", 1).\nload(\"u.rerail\", \"u\", \"rr\").\nboot(\"u.rerail\", 2).\n";
const FACTS: &str = "e(1, 2).\ne(2, 3).\ne(3, 1).\ne(2, 4).\nb(3).\n";
/// K1: filters written before the atoms that bind their variables, which K0 refuses (B§7).
const K1_RULES: &str = "h(x, z) :- x != z, !b(y), e(x, y), e(y, z).\ng(x) :- e(x, y), y > 2, e(y, _).\n";
/// The same rules ordered by hand, in K0.
const K0_RULES: &str = "h(x, z) :- e(x, y), !b(y), e(y, z), x != z.\ng(x) :- e(x, y), y > 2, e(y, _).\n";

/// A run over the kernel: L2 loads `ask.rerail` and the kernel's files, the boot rows their strata; it exports P3, the
/// lowerer, which L3 runs over the files `ask.rerail` loads, exporting their rules lowered; L4 runs them.
fn kernel_run(name: &str, rules: &str) -> Run {
    let mut r = Run::new(name);
    std::fs::create_dir_all(r.root.join("kernel")).unwrap();
    for (path, text) in KERNEL {
        r.write(path, text, 50);
    }
    r.write("ask.rerail", ASK, 50);
    r.write("u-facts.rerail", FACTS, 50);
    r.write("u.rerail", rules, 50);
    let modules: Vec<(String, String)> = std::iter::once(("ask.rerail".to_string(), "ask".to_string()))
        .chain(KERNEL.iter().map(|(p, _)| (p.to_string(), p.trim_start_matches("kernel/").trim_end_matches(".rerail").to_string())))
        .collect();
    let names: Vec<&str> = modules.iter().flat_map(|(p, m)| [p.as_str(), m.as_str()]).chain(["rr"]).collect();
    r.names(&names);
    let rr = r.s("rr");
    let load: Vec<Vec<i64>> = modules.iter().map(|(p, m)| vec![r.s(p), r.s(m), rr]).collect();
    let boot: Vec<Vec<i64>> = modules.iter().zip(1..).map(|((p, _), k)| vec![r.s(p), k]).collect();
    r.fact("load", &load);
    r.fact("boot", &boot);
    r
}

/// The same files read at L2's place, the a/b test's way: L2 reads them in K0, L3 runs them.
fn hand_run(name: &str, rules: &str) -> Run {
    let mut r = Run::new(name);
    r.write("hand-facts.rerail", FACTS, 50);
    r.write("hand.rerail", rules, 50);
    r.names(&["hand-facts.rerail", "hand-facts", "hand.rerail", "hand", "rr"]);
    let (pf, mf, p, m, rr) = (r.s("hand-facts.rerail"), r.s("hand-facts"), r.s("hand.rerail"), r.s("hand"), r.s("rr"));
    r.fact("load", &[vec![pf, mf, rr], vec![p, m, rr]]);
    r.fact("boot", &[vec![pf, 1], vec![p, 2]]);
    r
}

/// The first K1 feature, in rules (B§10 step 7; the rebuild plan, O2 and O4): `kernel/`'s binding lowerer, in K0, one
/// file a stratum, places a body's filters after their variables are bound, at keys (p, f) with no arithmetic. A K1
/// rule lowered by P3 and run gives the same rows as its hand-ordered K0 form, through edits, unchanged and cold;
/// the K1 rule as written is refused by K0's executor.
#[test]
fn a_k1_rule_lowered_by_p3_gives_the_rows_of_its_hand_ordered_k0_form() {
    let mut r = kernel_run("phase-k1", K1_RULES);
    r.run();
    assert_eq!(r.levels.levels.len(), 3, "L2 exports P3; L3 lowers u's files and exports them; L4 runs them and exports none");
    let mut h = hand_run("phase-k1-hand", K0_RULES);
    h.run();
    let rows = |r: &Run, k: usize| (r.rows(k, "h"), r.rows(k, "g"));
    let want = (BTreeSet::from([vec![1, 3], vec![1, 4], vec![3, 2]]), BTreeSet::from([vec![2]]));
    assert_eq!(rows(&h, 1), want, "the hand-ordered form's rows");
    assert_eq!(rows(&r, 2), want, "the lowered form's rows");
    // The filters' keys: `x != z` after the atom binding z (written at 0, now at (3, 0)), `!b(y)` after the one binding
    // y ((2, 1)), `y > 2` where it was bound ((0, 1)); the atoms where they were read.
    let keys = |kind: &str, rel: &str| -> BTreeSet<(i64, i64)> {
        let (kind, rel) = (r.s(kind), r.s(rel));
        r.rows(1, "k_step").into_iter().filter(|x| x[4] == kind && x[5] == rel).map(|x| (x[2], x[3])).collect()
    };
    assert_eq!(keys("cmp", "!="), BTreeSet::from([(3, 0)]));
    assert_eq!(keys("not", "b"), BTreeSet::from([(2, 1)]));
    assert_eq!(keys("cmp", ">"), BTreeSet::from([(0, 1)]));
    assert_eq!(keys("atom", "e"), BTreeSet::from([(0, -1), (2, -1), (3, -1)]), "e's steps at (k, -1)");
    // Unchanged: the identity at every level.
    let work = r.run();
    assert!(work.is_checks_only(), "{work:?}");
    // Edits, the same on both sides: an edge gone and one added; a rule's literals reordered, which K0 takes as written.
    let facts = "e(1, 2).\ne(2, 3).\ne(3, 1).\ne(3, 5).\ne(5, 6).\nb(3).\n";
    r.write("u-facts.rerail", facts, 40);
    h.write("hand-facts.rerail", facts, 40);
    r.run();
    h.run();
    assert_eq!(rows(&r, 2), rows(&h, 1), "the facts' edit");
    r.write("u.rerail", "h(x, z) :- e(y, z), x != z, e(x, y), !b(y).\ng(x) :- y > 2, e(y, _), e(x, y).\n", 30);
    r.run();
    assert_eq!(rows(&r, 2), rows(&h, 1), "the rules' reorder");
    // Cold: the same rows.
    let warm = rows(&r, 2);
    r.last = None;
    r.run();
    assert_eq!(rows(&r, 2), warm, "cold");
    // The K1 rules as written, run at L2's place with no lowering: K0's executor refuses the filter before its binding.
    let mut raw = hand_run("phase-k1-raw", K1_RULES);
    let e = raw.try_run().unwrap_err();
    assert!(e.contains("module hand.rerail") && e.contains("variable x is not bound by an earlier step"), "{e}");
    // A level reads its documents before its rules run, so a `load` its program derives is refused.
    r.write("ask.rerail", "load(p, \"u\", \"rr\") :- boot(p, 2).\nboot(\"u.rerail\", 2).\n", 20);
    let e = r.try_run().unwrap_err();
    assert!(e.contains("module ask.rerail") && e.contains("a load that is not a fact"), "{e}");
}

/// The planner's memo is its module's operator cache, kept from one run of a level to the next (Operations §6.1: "in
/// memory across runs"): a line planned in one run takes its kept order under the budget in the next (0122), and the
/// rows are a cold run's.
#[test]
fn a_levels_memo_lasts_from_one_run_to_the_next() {
    let mut r = Run::new("phase-memo");
    let base = |extra: &str| format!("{}{extra}", (0..6).map(|i| format!("e({i}, {}).\nf({}, {i}).\n", i + 1, i + 1)).collect::<String>());
    r.write("base.rerail", &base(""), 50);
    r.write("rule.rerail", "p(x, z) :- e(x, y), f(y, w), e(w, z).\n", 50);
    r.names(&["base.rerail", "base", "rule.rerail", "rule", "rr"]);
    let (pb, mb, pr, mr, rr) = (r.s("base.rerail"), r.s("base"), r.s("rule.rerail"), r.s("rule"), r.s("rr"));
    r.fact("load", &[vec![pb, mb, rr], vec![pr, mr, rr]]);
    r.fact("boot", &[vec![pb, 1], vec![pr, 2]]);
    let cold = r.run();
    assert!(cold.planned.priced > 0, "{:?}", cold.planned);
    // Two edits of the same shape: the first prices its delta's line, the second takes it kept.
    r.write("base.rerail", &base("e(7, 8).\n"), 40);
    let first = r.run();
    assert!(first.planned.priced > 0, "{:?}", first.planned);
    r.write("base.rerail", &base("e(7, 8).\ne(8, 9).\n"), 30);
    let second = r.run();
    assert!(second.planned.kept > 0, "the memo the run before left: {:?}", second.planned);
    let warm = r.rows(1, "p");
    r.last = None;
    r.run();
    assert_eq!(r.rows(1, "p"), warm, "cold");
}

/// A level's run ends with its critical path over its strata (0203 §4): a line of more than `CRITICAL_ROWS` of work on
/// it is marked critical; an edit that does little marks none.
#[test]
fn a_levels_run_marks_its_critical_lines() {
    let mut r = Run::new("phase-critical");
    let base = |n: i64| (0..n).map(|i| format!("a({i}).\nb({i}).\n")).collect::<String>();
    r.write("base.rerail", &base(120), 50);
    r.write("rule.rerail", "p(x, y) :- a(x), b(y).\n", 50);
    r.names(&["base.rerail", "base", "rule.rerail", "rule", "rr"]);
    let (pb, mb, pr, mr, rr) = (r.s("base.rerail"), r.s("base"), r.s("rule.rerail"), r.s("rule"), r.s("rr"));
    r.fact("load", &[vec![pb, mb, rr], vec![pr, mr, rr]]);
    r.fact("boot", &[vec![pb, 1], vec![pr, 2]]);
    let cold = r.run();
    assert!(cold.planned.critical >= 1, "{:?}", cold.planned);
    assert_eq!(r.rows(1, "p").len(), 120 * 120);
    r.write("base.rerail", &format!("{}a(500).\n", base(120)), 40);
    let edit = r.run();
    assert_eq!(edit.planned.critical, 0, "{:?}", edit.planned);
    assert_eq!(r.rows(1, "p").len(), 121 * 120);
}

/// Declarations are facts of the program (0361): `key(rel, k, col)` in a module of an earlier stratum, the relation
/// named by its text. A level's lines they cover take their default plans, unpriced, with a cold run's rows; a
/// declaration the rows break is caught, and the line is planned from statistics.
#[test]
fn a_levels_declared_keys_give_its_lines_default_plans() {
    let base = (0..40).map(|i| format!("e({i}, {}).\nf({}, {}).\ng({}, {i}).\n", i + 1, i + 1, i + 2, i + 2)).collect::<String>();
    let declared = |decl: &str| {
        let mut r = Run::new("phase-declared");
        r.write("base.rerail", &base, 50);
        r.write("decl.rerail", decl, 50);
        r.write("rule.rerail", "p(x, w) :- e(x, y), f(y, z), g(z, w).\n", 50);
        r.names(&["base.rerail", "base", "decl.rerail", "decl", "rule.rerail", "rule", "rr"]);
        let s = |n: &str| r.s(n);
        let (pb, mb, pd, md, pr, mr, rr) = (s("base.rerail"), s("base"), s("decl.rerail"), s("decl"), s("rule.rerail"), s("rule"), s("rr"));
        r.fact("load", &[vec![pb, mb, rr], vec![pd, md, rr], vec![pr, mr, rr]]);
        r.fact("boot", &[vec![pb, 1], vec![pd, 1], vec![pr, 2]]);
        let cold = r.run();
        (r, cold)
    };
    // Undeclared (the same facts under another relation): its lines are priced.
    let keys = "(\"e\", 0, 1).\n_(\"f\", 0, 0).\n_(\"f\", 1, 1).\n_(\"g\", 0, 0).\n";
    let (_, plain) = declared(&format!("other{}", keys.replace("_(", "other(")));
    let (mut r, cold) = declared(&format!("key{}", keys.replace("_(", "key(")));
    assert_eq!((plain.planned.declared, plain.planned.broken), (0, 0), "{:?}", plain.planned);
    assert!(cold.planned.declared >= 1, "the cold line: {:?}", cold.planned);
    assert!(cold.planned.priced < plain.planned.priced, "{:?} {:?}", plain.planned, cold.planned);
    assert_eq!(cold.planned.broken, 0, "{:?}", cold.planned);
    assert_eq!(r.rows(1, "p").len(), 40);
    // f's first column is no key once two rows share it: e's change probes f at y = 1 and finds three rows. Caught,
    // and planned from statistics; the rows those of the facts.
    r.write("base.rerail", &format!("{base}f(1, 99).\nf(1, 98).\ng(99, 7).\ng(98, 8).\ne(50, 1).\n"), 40);
    let edit = r.run();
    assert!(edit.planned.broken >= 1, "{:?}", edit.planned);
    // e(50, 1) breaks e's key at y too (e(0, 1)): each fact named is one the rows broke.
    let truly = [(r.s("f"), 0b01), (r.s("e"), 0b10)];
    assert!(!edit.broken.is_empty() && edit.broken.iter().all(|b| truly.contains(b)), "{:?}", edit.broken);
    let p = r.rows(1, "p");
    assert_eq!(p.len(), 45);
    assert!([[0, 7], [0, 8], [50, 0], [50, 7], [50, 8]].iter().all(|x| p.contains(x.as_slice())), "{p:?}");
    let warm = r.rows(1, "p");
    r.last = None;
    r.run();
    assert_eq!(r.rows(1, "p"), warm, "cold");
}

/// A derived relation's key, inferred through its rule (0361): `q` copies `f` keeping its key, `r` keeps `g`'s, and
/// a later stratum's line over `e`, `q` and `r` takes its default plan from what the earlier strata's rules give
/// them. With `f` and `g` undeclared, nothing is inferred and the line is priced; the rows are the same.
#[test]
fn a_derived_relations_inferred_key_gives_a_later_line_its_default_plan() {
    let base = (0..40).map(|i| format!("e({i}, {}).\nf({}, {}).\ng({}, {i}).\n", i + 1, i + 1, i + 2, i + 2)).collect::<String>();
    let run = |rel: &str| {
        let mut r = Run::new("phase-inferred");
        r.write("base.rerail", &base, 50);
        r.write("decl.rerail", &format!("{rel}(\"e\", 0, 1).\n{rel}(\"f\", 0, 0).\n{rel}(\"g\", 0, 0).\n"), 50);
        r.write("mid.rerail", "q(y, z) :- f(y, z), y < 1000.\nr(z, w) :- g(z, w).\n", 50);
        r.write("rule.rerail", "p(x, w) :- e(x, y), q(y, z), r(z, w).\n", 50);
        r.names(&["base.rerail", "base", "decl.rerail", "decl", "mid.rerail", "mid", "rule.rerail", "rule", "rr"]);
        let rr = r.s("rr");
        let files = [("base.rerail", "base", 1), ("decl.rerail", "decl", 1), ("mid.rerail", "mid", 2), ("rule.rerail", "rule", 3)];
        let load: Vec<Vec<i64>> = files.iter().map(|(p, m, _)| vec![r.s(p), r.s(m), rr]).collect();
        let boot: Vec<Vec<i64>> = files.iter().map(|(p, _, k)| vec![r.s(p), *k]).collect();
        r.fact("load", &load);
        r.fact("boot", &boot);
        let cold = r.run();
        let p = r.rows(1, "p");
        (cold, p)
    };
    let (plain, want) = run("other");
    let (keyed, got) = run("key");
    assert_eq!((got.len(), &got), (40, &want));
    assert_eq!(plain.planned.declared, 0, "{:?}", plain.planned);
    assert!(keyed.planned.declared >= 1, "p's line, by q's and r's inferred keys: {:?}", keyed.planned);
}

/// A tree and a dense order declare keys (0361): `tree(f, 1, 0)` gives each of f's first column one second, and
/// `order(e, 1, 0)` numbers e's second column within its first. The line takes its default plan from them, and an
/// edit breaking the tree (a second row at one child) is caught as a broken key is.
#[test]
fn a_declared_tree_and_order_give_their_keys_and_are_checked() {
    let base = (0..40).map(|i| format!("e({i}, 0).\nf({i}, {}).\ng({}, {i}).\n", i + 1, i + 1)).collect::<String>();
    let mut r = Run::new("phase-tree");
    r.write("base.rerail", &base, 50);
    r.write("decl.rerail", "order(\"e\", 1, 0).\ntree(\"f\", 1, 0).\ntree(\"g\", 1, 0).\n", 50);
    r.write("rule.rerail", "p(x, w) :- e(x, n), f(x, z), g(z, w).\n", 50);
    r.names(&["base.rerail", "base", "decl.rerail", "decl", "rule.rerail", "rule", "rr"]);
    let rr = r.s("rr");
    let files = [("base.rerail", "base", 1), ("decl.rerail", "decl", 1), ("rule.rerail", "rule", 2)];
    let load: Vec<Vec<i64>> = files.iter().map(|(p, m, _)| vec![r.s(p), r.s(m), rr]).collect();
    let boot: Vec<Vec<i64>> = files.iter().map(|(p, _, k)| vec![r.s(p), *k]).collect();
    r.fact("load", &load);
    r.fact("boot", &boot);
    let cold = r.run();
    assert!(cold.planned.declared >= 1, "{:?}", cold.planned);
    assert_eq!(cold.planned.broken, 0, "{:?}", cold.planned);
    assert_eq!(r.rows(1, "p").len(), 40);
    // f(3, _) twice: 3 has two parents. The next edit's change to e at 3 probes f by it, f's both rows there.
    r.write("base.rerail", &format!("{base}f(3, 77).\ng(77, 5).\n"), 40);
    r.run();
    assert_eq!(r.rows(1, "p").len(), 41);
    r.write("base.rerail", &format!("{base}f(3, 77).\ng(77, 5).\ne(3, 1).\n"), 30);
    let edit = r.run();
    assert!(edit.planned.broken >= 1, "{:?}", edit.planned);
    assert_eq!(edit.broken, vec![(r.s("f"), 0b01)], "f's tree, probed by the child, named");
    assert_eq!(r.rows(1, "p").len(), 41, "e(3, 1) derives what e(3, 0) does");
}

/// 0361's algorithms from the facts: a recursion along a relation declared a tree runs cold in one wave, unplanned
/// (Operations, class 30). Its rows and their levels are those of Δfix's rounds over the same facts undeclared, and an
/// edit moving a subtree runs Δfix by levels from them to the rows a cold run gives. A head naming the wave
/// (`by(anc, wave)`) takes it with no tree declared.
#[test]
fn a_recursion_along_a_declared_tree_runs_in_one_wave_with_the_rounds_levels() {
    let par = |moved: bool| (1..40).map(|i| format!("par({i}, {}).\n", if moved && i == 5 { 3 } else { i / 2 })).collect::<String>();
    let run = |rel: &str| {
        let mut r = Run::new(&format!("phase-along-{rel}"));
        r.write("base.rerail", &par(false), 50);
        let decl = if rel == "by" { "by(\"anc\", \"wave\").\n".to_string() } else { format!("{rel}(\"par\", 1, 0).\n") };
        r.write("decl.rerail", &decl, 50);
        r.write("rule.rerail", "anc(x, z) :- par(x, z).\nanc(x, z) :- par(x, y), anc(y, z).\n", 50);
        r.names(&["base.rerail", "base", "decl.rerail", "decl", "rule.rerail", "rule", "rr"]);
        let rr = r.s("rr");
        let files = [("base.rerail", "base", 1), ("decl.rerail", "decl", 1), ("rule.rerail", "rule", 2)];
        let load: Vec<Vec<i64>> = files.iter().map(|(p, m, _)| vec![r.s(p), r.s(m), rr]).collect();
        let boot: Vec<Vec<i64>> = files.iter().map(|(p, _, k)| vec![r.s(p), *k]).collect();
        r.fact("load", &load);
        r.fact("boot", &boot);
        let cold = r.run();
        r
            .levels
            .levels
            .get(1)
            .and_then(|l| l.rels.get(&r.s("anc")))
            .map(|t| rows_at(&r.bench.dir, &t.head, t.head.head()).unwrap())
            .map(|levelled| (r, cold, levelled))
            .expect("anc")
    };
    let (mut rounds, plain, by_rounds) = run("other");
    let (mut wave, along, by_wave) = run("tree");
    assert_eq!((plain.planned.waves, along.planned.waves), (0, 1), "{:?} {:?}", plain.planned, along.planned);
    assert!(along.planned.lines < plain.planned.lines, "the rounds' lines unplanned: {:?} {:?}", plain.planned, along.planned);
    assert_eq!(by_wave, by_rounds, "the rows and their levels");
    // Named (the override): no tree declared, `by(anc, wave)`.
    let (_, named, by_name) = run("by");
    assert_eq!(named.planned.waves, 1, "{:?}", named.planned);
    assert_eq!(by_name, by_rounds);
    // anc(i, 0) for every i, and 1 + 2 + … per depth: a heap of 40 nodes.
    assert!(wave.rows(1, "anc").contains(&vec![39, 0]));
    // 5 moves from under 2 to under 3: both by Δfix from what each run kept, the rows of a cold run.
    for r in [&mut rounds, &mut wave] {
        r.write("base.rerail", &par(true), 40);
        r.run();
    }
    let edited = wave.rows(1, "anc");
    assert_eq!(edited, rounds.rows(1, "anc"));
    assert!(edited.contains(&vec![11, 3]) && !edited.contains(&vec![11, 2]), "11's parent 5 is under 3");
    wave.last = None;
    wave.run();
    assert_eq!(wave.rows(1, "anc"), edited, "cold");
}
