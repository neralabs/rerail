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

    fn name(&self, id: i64) -> String {
        self.atoms.name(&self.bench.dir, id)
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
        // A relation no document names (`k_decl` where none declares) has no symbol, and no rows.
        let Some(r) = self.atoms.find(&self.bench.dir, rel).and_then(|s| self.levels.levels[k].rels.get(&s)) else { return BTreeSet::new() };
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
const KERNEL: [(&str, &str); 13] = [
    ("kernel/k1-0-numbers.rerail", include_str!("../rerail/kernel/k1-0-numbers.rerail")),
    ("kernel/k1-1-read.rerail", include_str!("../rerail/kernel/k1-1-read.rerail")),
    ("kernel/k1-2-shared.rerail", include_str!("../rerail/kernel/k1-2-shared.rerail")),
    ("kernel/k1-3-rank.rerail", include_str!("../rerail/kernel/k1-3-rank.rerail")),
    ("kernel/k1-4-rules.rerail", include_str!("../rerail/kernel/k1-4-rules.rerail")),
    ("kernel/k1-5-steps.rerail", include_str!("../rerail/kernel/k1-5-steps.rerail")),
    ("kernel/k1-6-binds.rerail", include_str!("../rerail/kernel/k1-6-binds.rerail")),
    ("kernel/k1-7-place.rerail", include_str!("../rerail/kernel/k1-7-place.rerail")),
    ("kernel/k1-8-order.rerail", include_str!("../rerail/kernel/k1-8-order.rerail")),
    ("kernel/k1-9-turn.rerail", include_str!("../rerail/kernel/k1-9-turn.rerail")),
    ("kernel/k1-10-emit.rerail", include_str!("../rerail/kernel/k1-10-emit.rerail")),
    ("kernel/k1-11-graph.rerail", include_str!("../rerail/kernel/k1-11-graph.rerail")),
    ("kernel/k1-12-strata.rerail", include_str!("../rerail/kernel/k1-12-strata.rerail")),
];

/// The compiler (B§6, L4; B§10 step 9): the full lowerer, written in the language the kernel lowers. L3 reads it into
/// P4, which reads the user's files at L4.
const COMPILER: [(&str, &str); 23] = [
    ("compiler/c-1-read.rerail", include_str!("../rerail/compiler/c-1-read.rerail")),
    ("compiler/c-2-shared.rerail", include_str!("../rerail/compiler/c-2-shared.rerail")),
    ("compiler/c-3-rank.rerail", include_str!("../rerail/compiler/c-3-rank.rerail")),
    ("compiler/c-4-rules.rerail", include_str!("../rerail/compiler/c-4-rules.rerail")),
    ("compiler/c-5-steps.rerail", include_str!("../rerail/compiler/c-5-steps.rerail")),
    ("compiler/c-6-binds.rerail", include_str!("../rerail/compiler/c-6-binds.rerail")),
    ("compiler/c-7-place.rerail", include_str!("../rerail/compiler/c-7-place.rerail")),
    ("compiler/c-8-order.rerail", include_str!("../rerail/compiler/c-8-order.rerail")),
    ("compiler/c-9-turn.rerail", include_str!("../rerail/compiler/c-9-turn.rerail")),
    ("compiler/c-10-emit.rerail", include_str!("../rerail/compiler/c-10-emit.rerail")),
    ("compiler/c-11-graph.rerail", include_str!("../rerail/compiler/c-11-graph.rerail")),
    ("compiler/c-12-strata.rerail", include_str!("../rerail/compiler/c-12-strata.rerail")),
    ("compiler/c-13-declares.rerail", include_str!("../rerail/compiler/c-13-declares.rerail")),
    ("compiler/c-14-includes.rerail", include_str!("../rerail/compiler/c-14-includes.rerail")),
    ("compiler/c-15-plan.rerail", include_str!("../rerail/compiler/c-15-plan.rerail")),
    ("compiler/c-16-patterns.rerail", include_str!("../rerail/compiler/c-16-patterns.rerail")),
    ("compiler/c-17-pattern-root.rerail", include_str!("../rerail/compiler/c-17-pattern-root.rerail")),
    ("compiler/c-18-pattern-tree.rerail", include_str!("../rerail/compiler/c-18-pattern-tree.rerail")),
    ("compiler/c-19-pattern-rank.rerail", include_str!("../rerail/compiler/c-19-pattern-rank.rerail")),
    ("compiler/c-20-pattern-rule.rerail", include_str!("../rerail/compiler/c-20-pattern-rule.rerail")),
    ("compiler/c-21-phases.rerail", include_str!("../rerail/compiler/c-21-phases.rerail")),
    ("compiler/c-22-phase-own.rerail", include_str!("../rerail/compiler/c-22-phase-own.rerail")),
    ("compiler/c-23-phase-map.rerail", include_str!("../rerail/compiler/c-23-phase-map.rerail")),
];

/// What L3 reads: the stand-in for the compiler's files (Bootstrap §6: "P3 loads the full compiler"), a K0 file of
/// L2's whose `load` facts P3 then asks. Their strata are computed (K1-e): the rules read the facts' relations.
const ASK: &str = "load(\"u-facts.rerail\", \"u-facts\", \"rr\").\nload(\"u.rerail\", \"u\", \"rr\").\n";
const FACTS: &str = "e(1, 2).\ne(2, 3).\ne(3, 1).\ne(2, 4).\nb(3).\n";
/// K1: filters written before the atoms that bind their variables, which K0 refuses (B§7).
const K1_RULES: &str = "h(x, z) :- x != z, !b(y), e(x, y), e(y, z).\ng(x) :- e(x, y), y > 2, e(y, _).\n";
/// The same rules ordered by hand, in K0.
const K0_RULES: &str = "h(x, z) :- e(x, y), !b(y), e(y, z), x != z.\ng(x) :- e(x, y), y > 2, e(y, _).\n";

/// A run over the kernel: L2 loads `ask.rerail` and the kernel's files, the boot rows their strata; it exports P3, the
/// lowerer, which L3 runs over the files `ask.rerail` loads, exporting their rules lowered; L4 runs them.
fn kernel_run(name: &str, rules: &str) -> Run {
    kernel_files(name, ASK, &[("u-facts.rerail", FACTS), ("u.rerail", rules)])
}

/// A run over the kernel of L3's files `files`, which `ask` loads.
fn kernel_files(name: &str, ask: &str, files: &[(&str, &str)]) -> Run {
    let mut r = Run::new(name);
    std::fs::create_dir_all(r.root.join("kernel")).unwrap();
    for (path, text) in KERNEL {
        r.write(path, text, 50);
    }
    r.write("ask.rerail", ask, 50);
    for (path, text) in files {
        r.write(path, text, 50);
    }
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

/// A run over the compiler: L3 reads the compiler's files, and `c-ask.rerail`, whose facts `ask` name the program's
/// roots (`root(path, module)`) and its library (`p0_lib(dir)`); P4, the compiler, loads them and what they include
/// at L4 and lowers them, and L5 runs them.
fn compiler_files(name: &str, ask: &str, files: &[(&str, &str)]) -> Run {
    let l3: String = COMPILER.iter().map(|(p, _)| p).chain(std::iter::once(&"c-ask.rerail")).map(|p| format!("load(\"{p}\", \"{}\", \"rr\").\n", p.trim_end_matches(".rerail"))).collect();
    let all: Vec<(&str, &str)> = std::iter::once(("c-ask.rerail", ask)).chain(files.iter().copied()).collect();
    let r = kernel_files(name, &l3, &all);
    std::fs::create_dir_all(r.root.join("compiler")).unwrap();
    for (path, text) in COMPILER {
        r.write(path, text, 50);
    }
    r
}

/// The compiler's roots for one program and its facts, as `ASK` gives them to the kernel.
const ROOTS: &str = "root(\"u-facts.rerail\", \"u-facts\").\nroot(\"u.rerail\", \"u\").\np0_lib(\"lib\").\n";

/// The compiler over one program and its facts, as `kernel_run` gives them to the kernel.
fn compiler_run(name: &str, rules: &str) -> Run {
    compiler_files(name, ROOTS, &[("u-facts.rerail", FACTS), ("u.rerail", rules)])
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

/// K1's several heads, disjunctions and constants (B§6, L3; B§10 step 8): each program lowered by P3 and run gives the
/// rows written by hand and those of its hand-lowered K0 form, and a cold run the same. A clause is a rule a head, or
/// one an alternative of its one disjunction, at the top of its body or a group among its literals; a filter written
/// before its binding, in an alternative or outside a group, is placed after it as in any rule.
#[test]
fn k1s_heads_disjunctions_and_constants_lower_to_their_hand_written_rules() {
    type Want = &'static [(&'static str, &'static [&'static [i64]])];
    let cases: [(&str, &str, &str, Want); 9] = [
        ("heads", "a(x), c(x) :- e(x, _).\n", "a(x) :- e(x, _).\nc(x) :- e(x, _).\n", &[("a", &[&[1], &[2], &[3]]), ("c", &[&[1], &[2], &[3]])]),
        ("or", "h(x) :- e(x, _) ; e(_, x).\n", "h(x) :- e(x, _).\nh(x) :- e(_, x).\n", &[("h", &[&[1], &[2], &[3], &[4]])]),
        ("group", "h(x) :- e(x, y), (y = 1 ; y = 4).\n", "h(x) :- e(x, y), y = 1.\nh(x) :- e(x, y), y = 4.\n", &[("h", &[&[2], &[3]])]),
        ("consts", "c(7, x, 0) :- e(x, 1).\n", "c(7, x, 0) :- e(x, 1).\n", &[("c", &[&[7, 3, 0]])]),
        ("or-bind", "h(x) :- x > 2, e(x, _) ; x < 3, !b(x), e(_, x).\n", "h(x) :- e(x, _), x > 2.\nh(x) :- e(_, x), x < 3, !b(x).\n", &[("h", &[&[1], &[2], &[3]])]),
        ("group-bind", "h(x, y) :- y != x, (e(x, y) ; e(y, x)).\n", "h(x, y) :- e(x, y), y != x.\nh(x, y) :- e(y, x), y != x.\n",
            &[("h", &[&[1, 2], &[2, 3], &[3, 1], &[2, 4], &[2, 1], &[3, 2], &[1, 3], &[4, 2]])]),
        ("heads-or", "a(x), c(x) :- e(x, _) ; e(_, x).\n", "a(x) :- e(x, _).\na(x) :- e(_, x).\nc(x) :- e(x, _).\nc(x) :- e(_, x).\n",
            &[("a", &[&[1], &[2], &[3], &[4]]), ("c", &[&[1], &[2], &[3], &[4]])]),
        ("groups", "h(x, y) :- (e(x, y) ; e(y, x)), (x = 1 ; y = 1).\n",
            "h(x, y) :- e(x, y), x = 1.\nh(x, y) :- e(x, y), y = 1.\nh(x, y) :- e(y, x), x = 1.\nh(x, y) :- e(y, x), y = 1.\n",
            &[("h", &[&[1, 2], &[3, 1], &[2, 1], &[1, 3]])]),
        ("nested", "h(x) :- e(x, y), (e(y, z), (z = 1 ; z = 4) ; b(y)).\n",
            "h(x) :- e(x, y), e(y, z), z = 1.\nh(x) :- e(x, y), e(y, z), z = 4.\nh(x) :- e(x, y), b(y).\n", &[("h", &[&[1], &[2]])]),
    ];
    for (name, k1, k0, want) in cases {
        let mut r = kernel_run(&format!("phase-k1-{name}"), k1);
        r.run();
        let mut h = hand_run(&format!("phase-k1-{name}-hand"), k0);
        h.run();
        for &(rel, rows) in want {
            let want: BTreeSet<Vec<i64>> = rows.iter().map(|x| x.to_vec()).collect();
            assert_eq!(h.rows(1, rel), want, "{name}: {rel}, the hand-lowered form's rows");
            assert_eq!(r.rows(2, rel), want, "{name}: {rel}, lowered by P3");
        }
        let warm: Vec<_> = want.iter().map(|&(rel, _)| r.rows(2, rel)).collect();
        r.last = None;
        r.run();
        assert_eq!(want.iter().map(|&(rel, _)| r.rows(2, rel)).collect::<Vec<_>>(), warm, "{name}: cold");
    }
}

/// B§10 step 8's test: a dozen plain-Datalog programs, each lowered by P3 and run against its hand-written rows and its
/// hand-lowered K0 form's (run at L2's place), warm and cold: recursion, mutual recursion, joins, negation, filters
/// written before their bindings, disjunctions, groups, several heads, wildcards and constants.
#[test]
fn a_dozen_plain_datalog_programs_lowered_by_k1_give_their_hand_lowered_rows() {
    type Want = &'static [(&'static str, &'static [&'static [i64]])];
    const T: &[&[i64]] = &[&[1, 1], &[1, 2], &[1, 3], &[1, 4], &[2, 1], &[2, 2], &[2, 3], &[2, 4], &[3, 1], &[3, 2], &[3, 3], &[3, 4]];
    let cases: [(&str, &str, &str, Want); 12] = [
        ("tc", "t(x, y) :- e(x, y).\nt(x, z) :- t(x, y), e(y, z).\n", "t(x, y) :- e(x, y).\nt(x, z) :- t(x, y), e(y, z).\n", &[("t", T)]),
        ("tc-or", "t(x, y) :- e(x, y) ; t(x, z), e(z, y).\n", "t(x, y) :- e(x, y).\nt(x, y) :- t(x, z), e(z, y).\n", &[("t", T)]),
        ("sg", "sg(x, y) :- x != y, e(p, x), e(p, y).\n", "sg(x, y) :- e(p, x), e(p, y), x != y.\n", &[("sg", &[&[3, 4], &[4, 3]])]),
        ("not-b", "nb(x, y) :- !b(y), e(x, y).\n", "nb(x, y) :- e(x, y), !b(y).\n", &[("nb", &[&[1, 2], &[3, 1], &[2, 4]])]),
        ("sym", "s(x, y) :- e(x, y) ; e(y, x).\n", "s(x, y) :- e(x, y).\ns(x, y) :- e(y, x).\n",
            &[("s", &[&[1, 2], &[2, 1], &[2, 3], &[3, 2], &[3, 1], &[1, 3], &[2, 4], &[4, 2]])]),
        ("reach", "r(y) :- e(1, y).\nr(y) :- r(x), e(x, y).\n", "r(y) :- e(1, y).\nr(y) :- r(x), e(x, y).\n", &[("r", &[&[1], &[2], &[3], &[4]])]),
        ("triangle", "tri(x, y, z) :- e(x, y), e(y, z), e(z, x).\n", "tri(x, y, z) :- e(x, y), e(y, z), e(z, x).\n", &[("tri", &[&[1, 2, 3], &[2, 3, 1], &[3, 1, 2]])]),
        ("group-not", "h(x) :- e(x, y), (y = 3 ; !b(y)).\n", "h(x) :- e(x, y), y = 3.\nh(x) :- e(x, y), !b(y).\n", &[("h", &[&[1], &[2], &[3]])]),
        ("heads", "a(x, y), c(y, x) :- e(x, y).\n", "a(x, y) :- e(x, y).\nc(y, x) :- e(x, y).\n",
            &[("a", &[&[1, 2], &[2, 3], &[3, 1], &[2, 4]]), ("c", &[&[2, 1], &[3, 2], &[1, 3], &[4, 2]])]),
        ("odd-even", "odd(x, y) :- e(x, y).\nodd(x, z) :- e(y, z), even(x, y).\neven(x, z) :- odd(x, y), e(y, z).\n",
            "odd(x, y) :- e(x, y).\nodd(x, z) :- even(x, y), e(y, z).\neven(x, z) :- odd(x, y), e(y, z).\n", &[("odd", T), ("even", T)]),
        ("sink", "sink(y) :- e(_, y), !e(y, _).\n", "sink(y) :- e(_, y), !e(y, _).\n", &[("sink", &[&[4]])]),
        ("lt", "lt(x, y) :- x < y, e(x, y).\n", "lt(x, y) :- e(x, y), x < y.\n", &[("lt", &[&[1, 2], &[2, 3], &[2, 4]])]),
    ];
    for (name, k1, k0, want) in cases {
        let mut r = kernel_run(&format!("phase-dozen-{name}"), k1);
        r.run();
        let mut h = hand_run(&format!("phase-dozen-{name}-hand"), k0);
        h.run();
        for &(rel, rows) in want {
            let want: BTreeSet<Vec<i64>> = rows.iter().map(|x| x.to_vec()).collect();
            assert_eq!(h.rows(1, rel), want, "{name}: {rel}, the hand-lowered form's rows");
            assert_eq!(r.rows(2, rel), want, "{name}: {rel}, lowered by P3");
        }
        r.last = None;
        r.run();
        for &(rel, rows) in want {
            assert_eq!(r.rows(2, rel), rows.iter().map(|x| x.to_vec()).collect::<BTreeSet<_>>(), "{name}: {rel}, cold");
        }
    }
}

/// K-form's rows of level `k`, by relation, each symbol column as its text (two runs intern in their own orders): a
/// rule's head, a step's kind and relation, a variable's name, a string, a declaration's relation and type.
fn kform_rows(r: &Run, k: usize) -> Vec<(&'static str, BTreeSet<Vec<String>>)> {
    let syms: &[(&str, &[usize])] = &[("k_rule", &[2]), ("k_step", &[4, 5]), ("k_var", &[2]), ("k_str", &[2]), ("k_decl", &[1, 3])];
    crate::kform::RELATIONS
        .iter()
        .map(|(n, _)| {
            let cols = syms.iter().find(|(m, _)| m == n).map_or(&[][..], |(_, c)| c);
            let rows = r.rows(k, n).into_iter().map(|row| row.iter().enumerate().map(|(i, &v)| if cols.contains(&i) && v > 0 { r.name(v) } else { v.to_string() }).collect()).collect();
            (*n, rows)
        })
        .collect()
}

/// The compiler (B§6, L4; B§10 step 9): the full lowerer, written with what the kernel lowers (arithmetic, counts, a
/// maximum, disjunctions), read by P3 at L3 into P4. P4 lowers each program into the K-form P3 makes of it, which runs
/// to the same rows. And the self-hosting check (B§6: "P4 reading the compiler/ files must give the K-form of P4
/// itself"): P4 reading the compiler's files gives the rows P3 made of them.
#[test]
fn the_compiler_lowers_as_the_kernel_does_and_reads_itself_into_itself() {
    let programs: [(&str, &str, &[&str]); 7] = [
        ("tc", "t(x, y) :- e(x, y) ; t(x, z), e(z, y).\n", &["t"]),
        ("filters", "h(x, z) :- x != z, !b(y), e(x, y), e(y, z).\ng(x) :- e(x, y), y > 2, e(y, _).\n", &["h", "g"]),
        ("groups", "h(x) :- e(x, y), (y = 3 ; !b(y)).\na(x, y), c(y, x) :- e(x, y).\n", &["h", "a", "c"]),
        ("arith", "n(x, y + 1) :- e(x, y).\nm(z) :- e(x, y), z = x * 10 - y, z > 5.\n", &["n", "m"]),
        ("agg", "deg(x, n) :- e(x, _), n = count : { e(x, _) }.\ns(x, n) :- e(x, _), n = sum y : { e(x, y) }.\n", &["deg", "s"]),
        ("calls", "c(x, @max(x, y)) :- e(x, y).\nl(t) :- e(x, _), t = to_string(x + 100).\n", &["c", "l"]),
        ("source", "src(x) :- e(x, _), !e(_, x).\nnot3(x) :- e(x, _), !b(x).\n", &["not3"]),
    ];
    for (name, rules, rels) in programs {
        let mut k = kernel_run(&format!("phase-compiler-{name}-kernel"), rules);
        k.run();
        let mut c = compiler_run(&format!("phase-compiler-{name}"), rules);
        c.run();
        for ((rel, a), (_, b)) in kform_rows(&k, 1).into_iter().zip(kform_rows(&c, 2)) {
            assert_eq!(b, a, "{name}: {rel}, P4's K-form against P3's");
        }
        for rel in rels {
            assert!(!k.rows(2, rel).is_empty(), "{name}: {rel} holds rows");
            // A text column (`l`'s) compares by its text: two runs intern in their own orders.
            let named = |r: &Run, rows: BTreeSet<Vec<i64>>| -> BTreeSet<Vec<String>> { rows.into_iter().map(|row| row.iter().map(|&v| if *rel == "l" { r.name(v) } else { v.to_string() }).collect()).collect() };
            assert_eq!(named(&c, c.rows(3, rel)), named(&k, k.rows(2, rel)), "{name}: {rel}, run");
        }
    }
    // The self-hosting check: L4 reads the compiler's files, and P4's K-form of them is P4's own.
    let ask: String = COMPILER.iter().map(|(p, _)| format!("root(\"{p}\", \"{}\").\n", p.trim_end_matches(".rerail"))).chain(["p0_lib(\"lib\").\n".to_string()]).collect();
    let mut c = compiler_files("phase-compiler-self", &ask, &[]);
    c.run();
    let p4 = kform_rows(&c, 1);
    let again = kform_rows(&c, 2);
    let modules: BTreeSet<String> = again.iter().flat_map(|(_, rows)| rows.iter().map(|r| r[0].clone())).collect();
    assert_eq!(modules.len(), COMPILER.len(), "every compiler file read at L4");
    for ((rel, a), (_, b)) in p4.into_iter().zip(again) {
        let a: BTreeSet<Vec<String>> = a.into_iter().filter(|r| modules.contains(&r[0])).collect();
        assert!(!a.is_empty() || matches!(rel, "k_wild" | "k_decl" | "k_num"), "{rel}: P4 holds rows");
        assert_eq!(b, a, "{rel}: P4 reading the compiler gives P4");
    }
}

/// Includes and the module plan (B§6, L4; L4-g option 1), as the compiler's rules at L4. An include resolves to its
/// first candidate that is there, the library's before its file's own directory; a root not in T's shape reads
/// `core.rerail` first; an include no file resolves is `p0_missing`, its candidates `p0_absent`; each file's scope is
/// the files its includes reach, and an include cycle settles. What the includes reach is the program L5 runs. An
/// edit and a file appearing move the graph as a cold run gives it.
#[test]
fn includes_resolve_by_the_compilers_rules_into_the_program() {
    const A: &str = "#include \"b.rerail\"\n#include \"dup.rerail\"\n#include \"nowhere.rerail\"\ngot(x) :- core_fact(x).\nall(x) :- bb(x).\ndl(x) :- dup_lib(x).\n";
    let roots = "root(\"src/a.rerail\", \"a\").\nroot(\"t.rerail\", \"t\").\np0_lib(\"lib\").\n";
    let files = [
        ("src/a.rerail", A),
        ("src/b.rerail", "#include \"c.rerail\"\nbb(2).\n"),
        ("src/c.rerail", "#include \"a.rerail\"\ncc(3).\n"),
        ("src/dup.rerail", "dup_src(9).\n"),
        ("lib/core.rerail", "core_fact(1).\n"),
        ("lib/dup.rerail", "dup_lib(1).\n"),
        ("t.rerail", ".decl doc(d: number, p: symbol, m: symbol)\nt_fact(4).\n"),
    ];
    let c = compiler_files("phase-includes-compiler", roots, &[]);
    for dir in ["src", "lib"] {
        std::fs::create_dir_all(c.root.join(dir)).unwrap();
    }
    for (path, text) in files {
        c.write(path, text, 50);
    }
    let mut c = c;
    c.run();
    let s = |r: &Run, cells: &[&str]| -> Vec<i64> { cells.iter().map(|x| x.parse::<i64>().unwrap_or_else(|_| r.s(x))).collect() };
    let set = |r: &Run, rows: &[&[&str]]| -> BTreeSet<Vec<i64>> { rows.iter().map(|row| s(r, row)).collect() };
    let includes = |r: &Run, cycle: bool| {
        let mut rows: Vec<&[&str]> = vec![
            &["src/a.rerail", "-1", "core.rerail", "lib/core.rerail"],
            &["src/a.rerail", "0", "b.rerail", "src/b.rerail"],
            &["src/a.rerail", "20", "dup.rerail", "lib/dup.rerail"],
            &["src/b.rerail", "0", "c.rerail", "src/c.rerail"],
        ];
        if cycle {
            rows.push(&["src/c.rerail", "0", "a.rerail", "src/a.rerail"]);
        }
        set(r, &rows)
    };
    assert_eq!(c.rows(2, "p0_include"), includes(&c, true), "each include's file");
    assert_eq!(c.rows(2, "p0_missing"), set(&c, &[&["src/a.rerail", "42", "nowhere.rerail"]]), "the include no file resolves");
    let absent = ["lib/nowhere.rerail", "src/nowhere.rerail", "lib/b.rerail", "lib/c.rerail", "lib/a.rerail"];
    assert_eq!(c.rows(2, "p0_absent"), absent.iter().map(|p| vec![c.s(p)]).collect(), "the candidates not there");
    let scope = ["src/b.rerail", "lib/dup.rerail", "lib/core.rerail", "src/c.rerail"];
    assert_eq!(c.rows(2, "p0_scope").into_iter().filter(|r| r[0] == c.s("src/a.rerail")).collect::<BTreeSet<_>>(), scope.iter().map(|p| vec![c.s("src/a.rerail"), c.s(p)]).collect(), "a's scope");
    assert!(c.rows(2, "p0_module_rank").contains(&vec![c.s("src/a.rerail"), 5]), "a's rank: the files it reaches, itself by the cycle");
    // The program L5 runs: what the includes reach, the library's `dup.rerail` and not the other.
    for (rel, rows) in [("got", vec![vec![1]]), ("all", vec![vec![2]]), ("dl", vec![vec![1]]), ("t_fact", vec![vec![4]])] {
        assert_eq!(c.rows(3, rel), rows.into_iter().collect::<BTreeSet<_>>(), "{rel}");
    }
    assert!(c.rows(3, "dup_src").is_empty(), "a candidate after the one an include resolves to is not loaded");
    // The cycle edited away, then a cold run: the same graph.
    c.write("src/c.rerail", "cc(3).\n", 40);
    c.run();
    assert_eq!(c.rows(2, "p0_include"), includes(&c, false), "after the edit");
    assert!(c.rows(2, "p0_module_rank").contains(&vec![c.s("src/a.rerail"), 4]), "a's rank after the edit");
    c.last = None;
    c.run();
    assert_eq!(c.rows(2, "p0_include"), includes(&c, false), "cold");
    // A file appearing where an include missed: the include resolves to it.
    c.write("src/nowhere.rerail", "here(5).\n", 30);
    c.run();
    assert!(c.rows(2, "p0_missing").is_empty(), "nothing missing once the file is there");
    assert!(c.rows(2, "p0_include").contains(&s(&c, &["src/a.rerail", "42", "nowhere.rerail", "src/nowhere.rerail"])), "the include resolves to the new file");
    assert_eq!(c.rows(3, "here"), [vec![5]].into_iter().collect(), "and its rules are the program's");
}

/// Patterns (B§6, L4; B§11: lowered "into view joins in rules"; L4-h), as the compiler's rules: `n ~ `code``, the code
/// a document of its own (`text:`), its rule a chain of views over the code's tree in that document's module, and the
/// literal an atom of it. Each form against hand-written rows: a root down a chain of only children, a selector, a
/// leaf's text, `$$$` alone, before and after others, a metavariable named twice (the same name), an operator (a
/// field's token), `$_`, and a literal whose clause names no document. An edit of the documents it reads and one of
/// its code are maintained as a cold run gives them; a list of two `$$$` is refused, naming it.
#[test]
fn patterns_lower_into_view_joins_by_the_compilers_rules() {
    const T: &str = "edge(a, b).\nedge(c, d) :- x(c).\nedge(e).\nedge(z, z).\nedge(p, q, w).\nother(f, g).\nr(x, y) :- edge(x, y), x < y, y > x.\n";
    const U: &str = "load(\"t.rerail\", \"t\", \"rr\").\n\
heads(a, b) :- node(d, n, _, _, _), n ~ `edge($A, $B).`, text(d, A, a), text(d, B, b).\n\
bodies(x) :- node(d, n, _, _, _), n ~ `q(1) :- edge($X, $Y).` at \"atom\", text(d, X, x).\n\
any(n) :- node(_, n, _, _, _), n ~ `edge($$$).`.\n\
last(b) :- node(d, n, _, _, _), n ~ `edge($$$, $L).`, text(d, L, b).\n\
ends(a, b) :- node(d, n, _, _, _), n ~ `edge($F, $$$, $L).`, text(d, F, a), text(d, L, b).\n\
same(a) :- node(d, n, _, _, _), n ~ `edge($X, $X).`, text(d, X, a).\n\
lt(a) :- node(d, n, _, _, _), n ~ `q(1) :- $A < $B.` at \"comparison\", text(d, A, a).\n\
first(a) :- node(d, n, _, _, _), n ~ `other($A, $_).`, text(d, A, a).\n\
loose(1) :- n ~ `other($A, $B).`.\n";
    let roots = "root(\"u.rerail\", \"u\").\np0_lib(\"lib\").\n";
    let mut r = compiler_files("phase-patterns", roots, &[("u.rerail", U), ("t.rerail", T)]);
    r.run();
    let texts = |r: &Run, rows: &[&[&str]]| -> BTreeSet<Vec<i64>> { rows.iter().map(|row| row.iter().map(|x| x.parse::<i64>().unwrap_or_else(|_| r.s(x))).collect()).collect() };
    let check = |r: &Run, what: &str| {
        assert_eq!(r.rows(3, "heads"), texts(r, &[&["a", "b"], &["c", "d"], &["z", "z"]]), "{what}: heads of two arguments");
        assert_eq!(r.rows(3, "bodies"), texts(r, &[&["x"]]), "{what}: a selector's atom");
        assert_eq!(r.rows(3, "any").len(), 5, "{what}: `$$$` alone: every edge head");
        assert_eq!(r.rows(3, "last"), texts(r, &[&["b"], &["d"], &["e"], &["z"], &["w"]]), "{what}: after `$$$`, from the end");
        assert_eq!(r.rows(3, "ends"), texts(r, &[&["a", "b"], &["c", "d"], &["z", "z"], &["p", "w"]]), "{what}: around `$$$`");
        assert_eq!(r.rows(3, "same"), texts(r, &[&["z"]]), "{what}: a metavariable named twice");
        assert_eq!(r.rows(3, "lt"), texts(r, &[&["x"]]), "{what}: an operator's token");
        assert_eq!(r.rows(3, "first"), texts(r, &[&["f"]]), "{what}: `$_`");
        assert_eq!(r.rows(3, "loose"), texts(r, &[&["1"]]), "{what}: no document named");
    };
    check(&r, "cold");
    // An edit of the documents the patterns read: a head more, a comparison turned.
    r.write("t.rerail", &format!("{T}edge(k, l).\ns(u, v) :- v > u.\n"), 40);
    r.run();
    assert!(r.rows(3, "heads").contains(&texts(&r, &[&["k", "l"]]).pop_first().unwrap()), "the new head");
    r.write("t.rerail", T, 30);
    r.run();
    check(&r, "edited back");
    r.last = None;
    r.run();
    check(&r, "cold again");
    // An edit of a pattern's code: another text, another document.
    r.write("u.rerail", &U.replace("`edge($X, $X).`", "`edge($X, z)`"), 20);
    let e = r.try_run().expect_err("a code that does not parse");
    assert!(e.contains("text:edge(__mv_X, z)"), "{e}");
    r.write("u.rerail", &U.replace("`edge($X, $X).`", "`edge($X, z).`"), 10);
    r.last = None;
    r.run();
    assert_eq!(r.rows(3, "same"), texts(&r, &[&["z"]]), "the edited code");
    // A list of two `$$$`.
    r.write("u.rerail", "load(\"t.rerail\", \"t\", \"rr\").\ntwo(n) :- node(_, n, _, _, _), n ~ `edge($$$, $$$).`.\n", 5);
    r.last = None;
    let e = r.try_run().expect_err("two $$$");
    assert!(e.contains("a list of two $$$"), "{e}");
}

/// Phase programs (B§6's L4 row, the passes as phases; B§8's phase DAG; decision 0083 §1): a file that declares
/// `<phase>__<export>` reads that phase's export; the phase's file (`<phase>.rerail` in `p0_phase_lib`) loads as an
/// include does, runs in a fact space of its own (its relations renamed by the compiler's pass, c-21..c-23), and is
/// ordered before its readers. A phase reads another's export; a reader aggregates one. A read of a relation a phase
/// does not export is refused, as is an export with a node column. A phase whose file is not there is absent, and
/// runs once its file appears. Edits give what a cold run gives.
#[test]
fn phase_programs_run_in_fact_spaces_of_their_own_before_their_readers() {
    const ST: &str = ".phase st\n.export def\n#include \"st-facts.rerail\"\n.decl def(x: symbol)\n\
inner(x) :- base(x), x != \"c\".\ndef(x) :- inner(x).\n";
    const ST_FACTS: &str = "base(\"a\").\nbase(\"b\").\nbase(\"c\").\n";
    const ND: &str = ".phase nd\n.export two\n.decl st__def(x: symbol)\ntwo(x, y) :- st__def(x), st__def(y), x != y.\n";
    // The reader's own `inner`: without a phase's own fact space, two modules would write it.
    const U: &str = ".decl st__def(x: symbol)\n.decl nd__two(x: symbol, y: symbol)\n.decl zz__q(x: symbol)\n\
inner(x) :- st__def(x), x = \"a\".\nseen(x) :- inner(x).\npair(x, y) :- nd__two(x, y).\nn(k) :- k = count : { st__def(_) }.\nq(x) :- zz__q(x).\n\
deco(z) :- st__def(x), y = @cat(x, \"!\"), z = @cat(y, \"?\").\n";
    let roots = "root(\"u.rerail\", \"u\").\np0_lib(\"lib\").\np0_phase_lib(\"phases\").\n";
    let mut r = compiler_files("phase-programs", roots, &[("u.rerail", U)]);
    std::fs::create_dir_all(r.root.join("phases")).unwrap();
    r.write("phases/st.rerail", ST, 50);
    r.write("phases/st-facts.rerail", ST_FACTS, 50);
    r.write("phases/nd.rerail", ND, 50);
    r.run();
    let one = |r: &Run, xs: &[&str]| -> BTreeSet<Vec<i64>> { xs.iter().map(|x| vec![r.s(x)]).collect() };
    let check = |r: &Run, what: &str, defs: &[&str]| {
        assert_eq!(r.rows(3, "st__def"), one(r, defs), "{what}: the export, renamed");
        assert_eq!(r.rows(3, "st/inner"), one(r, defs), "{what}: the phase's own relation, in its own space");
        assert_eq!(r.rows(3, "inner"), one(r, &["a"]), "{what}: the reader's own relation of the same name");
        assert_eq!(r.rows(3, "seen"), one(r, &["a"]), "{what}: read by the reader");
        let pairs: BTreeSet<Vec<i64>> = defs.iter().flat_map(|x| defs.iter().filter(move |y| *y != x).map(move |y| (x, y))).map(|(x, y)| vec![r.s(x), r.s(y)]).collect();
        assert_eq!(r.rows(3, "pair"), pairs, "{what}: a phase reading another's export");
        assert_eq!(r.rows(3, "n"), [vec![defs.len() as i64]].into_iter().collect(), "{what}: an aggregate of an export");
        let deco: Vec<String> = defs.iter().map(|x| format!("{x}!?")).collect();
        assert_eq!(r.rows(3, "deco"), deco.iter().map(|x| vec![r.s(x)]).collect(), "{what}: a chain of answers, each after the one it reads");
        assert!(r.rows(3, "def").is_empty() && r.rows(3, "two").is_empty(), "{what}: no phase relation under its own name");
        assert_eq!(r.rows(2, "p0_phase").len(), 4, "{what}: u reads st, nd and zz; nd reads st");
    };
    check(&r, "cold", &["a", "b"]);
    assert!(r.rows(2, "p0_absent").contains(&vec![r.s("phases/zz.rerail")]), "a phase not there");
    assert!(r.rows(3, "q").is_empty());
    // A phase's facts edited, and back.
    r.write("phases/st-facts.rerail", "base(\"a\").\nbase(\"b\").\nbase(\"c\").\nbase(\"d\").\n", 40);
    r.run();
    check(&r, "a phase's fact added", &["a", "b", "d"]);
    r.write("phases/st-facts.rerail", ST_FACTS, 30);
    r.run();
    check(&r, "back", &["a", "b"]);
    // The phase not there appears, and runs.
    r.write("phases/zz.rerail", ".phase zz\n.export q\nq(\"z\").\n", 30);
    r.run();
    assert_eq!(r.rows(3, "q"), one(&r, &["z"]), "the phase appeared");
    assert!(!r.rows(2, "p0_absent").contains(&vec![r.s("phases/zz.rerail")]));
    r.last = None;
    r.run();
    check(&r, "cold again", &["a", "b"]);
    assert_eq!(r.rows(3, "q"), one(&r, &["z"]));
    // A read of a relation the phase does not export.
    r.write("u.rerail", &format!("{U}.decl st__inner(x: symbol)\nbad(x) :- st__inner(x).\n"), 20);
    let e = r.try_run().expect_err("an unexported read");
    assert!(e.contains("phase st does not export inner"), "{e}");
    // An export with a node column.
    r.write("u.rerail", U, 15);
    r.write("phases/zz.rerail", ".phase zz\n.export q, w\n.decl w(x: node)\nq(\"z\").\nw(x) :- node(_, x, _, _, _).\n", 15);
    r.last = None;
    let e = r.try_run().expect_err("a node export");
    assert!(e.contains("phase zz's export w has a node column"), "{e}");
    // A phase's own relation is its stratum's: an aggregate of it in its own file is refused, as for any module.
    r.write("phases/zz.rerail", ".phase zz\n.export q\nq(k) :- k = count : { zz_in(_) }.\nzz_in(1).\n", 10);
    r.last = None;
    let e = r.try_run().expect_err("an aggregate of its own stratum");
    assert!(e.contains("zz/zz_in"), "{e}");
}

/// Lattices (B§6, L4; `core.rerail`; L4-i): a relation whose last column is a lattice keeps one row a key, the join
/// of the values derived for it: `flat<>` (one value, or top when two differ), `max<>`, `min<>`, a `.lattice` by its
/// join function (`shortlex_min`, as `rf_shortlex` is), and `widen k` (top once a key's value moved k times). Through
/// recursion, a cycle included, and read by a later module. An edit gives what a cold run gives.
#[test]
fn lattices_keep_each_keys_join_by_the_executors_rounds() {
    const U: &str = ".decl cv(n: symbol, v: flat<>)\n.decl best(n: symbol, v: max<>)\n.decl low(n: symbol, v: min<>)\n\
.decl far(n: symbol, v: max<>)\n.decl near(n: symbol, v: min<>)\n\
.lattice sl<symbol> { Lub -> @shortlex_min }\n.decl route(c: symbol, r: sl<>) widen 64\n\
.decl grow(n: symbol, v: max<>) widen 3\n.decl climb(n: symbol, v: max<>)\n.decl few(n: symbol, v: max<>) widen 3\n\
cv(n, v) :- src(n, v).\nbest(n, v) :- src(n, v).\nlow(n, v) :- src(n, v).\n\
far(\"x\", 0).\nfar(c, w) :- far(b, v), f(b, c), w = v + 1.\nnear(\"x\", 0).\nnear(c, w) :- near(b, v), f(b, c), w = v + 1.\n\
route(\"x\", \"x\").\nroute(c, s) :- route(b, r), e(b, c), s = cat(r, c).\n\
grow(\"g\", 0).\ngrow(n, w) :- grow(n, v), w = v + 1, w < 100.\nclimb(\"g\", 0).\nclimb(n, w) :- climb(n, v), w = v + 1, w < 100.\n\
few(\"g\", 0).\nfew(n, w) :- few(n, v), w = v + 1, w < 3.\n";
    const FACTS: &str = "src(\"a\", 1).\nsrc(\"a\", 1).\nsrc(\"b\", 1).\nsrc(\"b\", 2).\n\
f(\"x\", \"y\").\nf(\"y\", \"z\").\nf(\"x\", \"z\").\n\
e(\"x\", \"y\").\ne(\"y\", \"z\").\ne(\"x\", \"z\").\ne(\"z\", \"w\").\ne(\"w\", \"x\").\n";
    const V: &str = "two(n) :- far(n, 2).\nflat_top(n) :- cv(n, v), v = 9223372036854775807.\n";
    let roots = "root(\"facts.rerail\", \"facts\").\nroot(\"u.rerail\", \"u\").\nroot(\"v.rerail\", \"v\").\np0_lib(\"lib\").\n";
    let mut r = compiler_files("phase-lattices", roots, &[("facts.rerail", FACTS), ("u.rerail", U), ("v.rerail", V)]);
    r.run();
    let top = i64::MAX;
    let rows = |r: &Run, cells: &[(&str, i64)]| -> BTreeSet<Vec<i64>> { cells.iter().map(|(k, v)| vec![r.s(k), *v]).collect() };
    let syms = |r: &Run, cells: &[(&str, &str)]| -> BTreeSet<Vec<i64>> { cells.iter().map(|(k, v)| vec![r.s(k), r.s(v)]).collect() };
    let check = |r: &Run, what: &str, b: i64| {
        assert_eq!(r.rows(3, "cv"), rows(r, &[("a", 1), ("b", if b == 1 { 1 } else { top })]), "{what}: flat");
        assert_eq!(r.rows(3, "best"), rows(r, &[("a", 1), ("b", b)]), "{what}: max");
        assert_eq!(r.rows(3, "low"), rows(r, &[("a", 1), ("b", 1)]), "{what}: min");
        assert_eq!(r.rows(3, "far"), rows(r, &[("x", 0), ("y", 1), ("z", 2)]), "{what}: max through recursion");
        assert_eq!(r.rows(3, "near"), rows(r, &[("x", 0), ("y", 1), ("z", 1)]), "{what}: min through recursion");
        assert_eq!(r.rows(3, "route"), syms(r, &[("x", "x"), ("y", "xy"), ("z", "xz"), ("w", "xzw")]), "{what}: a lattice by its join, through a cycle");
        assert_eq!(r.rows(3, "grow"), rows(r, &[("g", top)]), "{what}: widened");
        assert_eq!(r.rows(3, "climb"), rows(r, &[("g", 99)]), "{what}: not widened");
        assert_eq!(r.rows(3, "few"), rows(r, &[("g", 2)]), "{what}: two moves, under its widen of 3");
        assert_eq!(r.rows(3, "two"), [vec![r.s("z")]].into_iter().collect(), "{what}: read by a later module");
        let tops: BTreeSet<Vec<i64>> = if b == 1 { BTreeSet::new() } else { [vec![r.s("b")]].into_iter().collect() };
        assert_eq!(r.rows(3, "flat_top"), tops, "{what}: top read");
    };
    check(&r, "cold", 2);
    r.write("facts.rerail", &FACTS.replace("src(\"b\", 2)", "src(\"b\", 1)"), 40);
    r.run();
    check(&r, "edited", 1);
    r.write("facts.rerail", FACTS, 30);
    r.run();
    check(&r, "edited back", 2);
    r.last = None;
    r.run();
    check(&r, "cold again", 2);
}

/// L4's aggregates (B§10 step 9; L4-f): `count`, `sum`, `min`, `max` and `mean` as delta operators the executor knows
/// by symbol, each a step folding the rows of its body's atom that match its keys (the variables it shares with its
/// clause, bound before it), its own variables only its, in a head or bound by `=`. `count` of none is 0. An edit is
/// maintained by changed groups and gives what a cold run gives. A body of several literals, and an aggregate of its
/// own stratum's head, are refused.
#[test]
fn aggregates_fold_their_groups_and_are_maintained_by_changed_groups() {
    type Want = &'static [&'static [i64]];
    let cases: [(&str, &str, &str, Want); 8] = [
        ("count", "deg(x, n) :- e(x, _), n = count : { e(x, _) }.\n", "deg", &[&[1, 1], &[2, 2], &[3, 1]]),
        ("all", "total(n) :- n = count : { e(_, _) }.\n", "total", &[&[4]]),
        ("sum", "s(x, n) :- e(x, _), n = sum y : { e(x, y) }.\n", "s", &[&[1, 2], &[2, 7], &[3, 1]]),
        ("min", "lo(x, n) :- e(x, _), n = min y : { e(x, y) }.\n", "lo", &[&[1, 2], &[2, 3], &[3, 1]]),
        ("max", "hi(x, n) :- e(x, _), n = max y : { e(x, y) }.\n", "hi", &[&[1, 2], &[2, 4], &[3, 1]]),
        ("none", "ind(x, n) :- e(x, _), n = count : { b(x) }.\n", "ind", &[&[1, 0], &[2, 0], &[3, 1]]),
        ("head", "h(x, count : { e(_, x) }) :- e(x, _).\n", "h", &[&[1, 1], &[2, 1], &[3, 1]]),
        ("mean", "m(n) :- n = mean y : { e(_, y) }.\n", "m", &[&[2]]),
    ];
    for (name, text, rel, want) in cases {
        let mut r = kernel_run(&format!("phase-agg-{name}"), text);
        r.run();
        let want: BTreeSet<Vec<i64>> = want.iter().map(|x| x.to_vec()).collect();
        assert_eq!(r.rows(2, rel), want, "{name}");
    }
    // An edit of the facts: each aggregate's changed groups only, as a cold run of the edited facts gives.
    let all = "deg(x, n) :- e(x, _), n = count : { e(x, _) }.\ns(x, n) :- e(x, _), n = sum y : { e(x, y) }.\nhi(x, n) :- e(x, _), n = max y : { e(x, y) }.\nind(x, n) :- e(x, _), n = count : { b(x) }.\ntotal(n) :- n = count : { e(_, _) }.\n";
    let mut r = kernel_run("phase-agg-edit", all);
    r.run();
    r.write("u-facts.rerail", "e(1, 2).\ne(2, 4).\ne(3, 1).\ne(3, 4).\ne(1, 5).\nb(1).\n", 40);
    r.run();
    let want: [(&str, &[&[i64]]); 5] = [
        ("deg", &[&[1, 2], &[2, 1], &[3, 2]]),
        ("s", &[&[1, 7], &[2, 4], &[3, 5]]),
        ("hi", &[&[1, 5], &[2, 4], &[3, 4]]),
        ("ind", &[&[1, 1], &[2, 0], &[3, 0]]),
        ("total", &[&[5]]),
    ];
    for (rel, rows) in want {
        assert_eq!(r.rows(2, rel), rows.iter().map(|x| x.to_vec()).collect::<BTreeSet<_>>(), "{rel}, after the edit");
    }
    r.last = None;
    r.run();
    for (rel, rows) in want {
        assert_eq!(r.rows(2, rel), rows.iter().map(|x| x.to_vec()).collect::<BTreeSet<_>>(), "{rel}, cold");
    }
    for (name, text, why) in [
        ("several", "bad(n) :- n = count : { e(x, y), b(y) }.\n", "not one atom"),
        ("own", "p(x) :- e(x, _).\nq(n) :- n = count : { p(_) }.\n", "aggregates relation p, its own stratum's head"),
    ] {
        let mut r = kernel_run(&format!("phase-agg-{name}"), text);
        let e = r.try_run().expect_err(name);
        assert!(e.contains(why), "{name}: {e}");
    }
}

/// Loads a level's rules derive from its own documents (B§10 step 9, includes): each round answers the loads asked,
/// then runs the strata, until a round asks nothing new. An include cycle settles. An edit that drops an include lets
/// its file go, as a cold run gives it. An include of a file that is not there is a row of `missing`, and the file
/// appearing loads it.
#[test]
fn a_level_loads_what_its_rules_derive_until_nothing_new_is_asked() {
    const RULES: &str = "load(\"a.rerail\", \"a.rerail\", \"rr\").\n\
load(p, p, \"rr\") :- kind(d, i, \"include\"), field(d, i, \"path\", s), value(d, s, p).\n\
seen(p) :- doc(_, p, _).\n\
gap(p) :- missing(p).\n";
    let mut r = kernel_run("phase-includes", RULES);
    r.write("a.rerail", "#include \"b.rerail\"\nx(1).\n", 50);
    r.write("b.rerail", "#include \"c.rerail\"\n#include \"a.rerail\"\nx(2).\n", 50);
    r.write("c.rerail", "x(3).\n", 50);
    r.run();
    let seen = |r: &Run, paths: &[&str]| -> BTreeSet<Vec<i64>> { paths.iter().map(|p| vec![r.s(p)]).collect() };
    assert_eq!(r.rows(2, "seen"), seen(&r, &["a.rerail", "b.rerail", "c.rerail"]), "every file the includes reach");
    r.write("b.rerail", "#include \"a.rerail\"\nx(2).\n", 40);
    r.run();
    assert_eq!(r.rows(2, "seen"), seen(&r, &["a.rerail", "b.rerail"]), "after the edit");
    r.last = None;
    r.run();
    assert_eq!(r.rows(2, "seen"), seen(&r, &["a.rerail", "b.rerail"]), "cold");
    // An include of no file: the load answer's miss, a row of `missing` (D3), and the file appearing loads it.
    r.write("b.rerail", "#include \"gone.rerail\"\n", 30);
    r.run();
    assert_eq!(r.rows(2, "missing"), seen(&r, &["gone.rerail"]), "the miss, a row");
    assert_eq!(r.rows(2, "gap"), seen(&r, &["gone.rerail"]), "read by a rule");
    assert_eq!(r.rows(2, "seen"), seen(&r, &["a.rerail", "b.rerail"]), "the miss is no document");
    r.write("gone.rerail", "x(4).\n", 20);
    r.run();
    assert!(r.rows(2, "missing").is_empty() && r.rows(2, "gap").is_empty(), "no miss once the file is there");
    assert_eq!(r.rows(2, "seen"), seen(&r, &["a.rerail", "b.rerail", "gone.rerail"]), "the file appearing");
}

/// L4's functors (B§10 step 9; L4-f option 3): `@f(…)` and `f(…)` as calls of functions the executor knows by name,
/// each a step of kind `call` placed as an operator is, its answer a number or a text's symbol. A text no symbol had
/// is interned by the launch's driver, which runs the launch again. No answer (a text that is no number, a zero
/// divisor) gives no row. An edit is maintained as a cold run gives it. A function the executor does not know, and a
/// call of another arity, are refused.
#[test]
fn calls_answer_by_function_and_text_answers_are_interned() {
    const W: &str = "w(\"ab\", 2).\nw(\"cde\", 2).\nw(\"x.y\", 3).\n";
    const RULES: &str = "len(s, n) :- w(s, _), n = strlen(s).\n\
bang(t) :- w(s, _), t = cat(s, \"!\").\n\
dotz(t) :- w(s, 3), t = @dot(s, \"z\").\n\
hd(@head(s)) :- w(s, _).\n\
nest(t) :- w(s, _), t = cat(@head(s), \"-\", @to_string(strlen(s))).\n\
mx(x, @max(x, 2)) :- e(x, _).\n\
q(x, @div(4, x - 1)) :- e(x, _).\n\
num(n) :- w(s, _), n = to_number(s).\n\
fit(s) :- w(s, n), n = strlen(s).\n\
two(t) :- t = cat(\"a\", \"b\").\n";
    const ASK_W: &str = "load(\"u-facts.rerail\", \"u-facts\", \"rr\").\nload(\"u-w.rerail\", \"u-w\", \"rr\").\nload(\"u.rerail\", \"u\", \"rr\").\n";
    let mut r = kernel_files("phase-call", ASK_W, &[("u-facts.rerail", FACTS), ("u-w.rerail", W), ("u.rerail", RULES)]);
    r.run();
    let want = |r: &Run, rows: &[&[&str]]| -> BTreeSet<Vec<i64>> {
        // A number, or a text's symbol (`'` before a text written as digits).
        rows.iter().map(|row| row.iter().map(|c| c.strip_prefix('\'').map_or_else(|| c.parse::<i64>().unwrap_or_else(|_| r.s(c)), |t| r.s(t))).collect()).collect()
    };
    let check = |r: &Run, when: &str, cases: &[(&str, &[&[&str]])]| {
        for (rel, rows) in cases {
            assert_eq!(r.rows(2, rel), want(r, rows), "{rel}, {when}");
        }
    };
    let first: [(&str, &[&[&str]]); 10] = [
        ("len", &[&["ab", "2"], &["cde", "3"], &["x.y", "3"]]),
        ("bang", &[&["ab!"], &["cde!"], &["x.y!"]]),
        ("dotz", &[&["x.y.z"]]),
        ("hd", &[&["ab"], &["cde"], &["x"]]),
        ("nest", &[&["ab-2"], &["cde-3"], &["x-3"]]),
        ("mx", &[&["1", "2"], &["2", "2"], &["3", "3"]]),
        ("q", &[&["2", "4"], &["3", "2"]]),
        ("num", &[]),
        ("fit", &[&["ab"], &["x.y"]]),
        ("two", &[&["ab"]]),
    ];
    check(&r, "first", &first);
    // An edit: a text added, one taken away, one changed; as the edit gives and as a cold run gives.
    r.write("u-w.rerail", "w(\"ab\", 2).\nw(\"x.y\", 1).\nw(\"7\", 1).\nw(\"pq.rs\", 3).\n", 40);
    r.run();
    let after: [(&str, &[&[&str]]); 7] = [
        ("len", &[&["ab", "2"], &["x.y", "3"], &["'7", "1"], &["pq.rs", "5"]]),
        ("bang", &[&["ab!"], &["x.y!"], &["7!"], &["pq.rs!"]]),
        ("dotz", &[&["pq.rs.z"]]),
        ("hd", &[&["ab"], &["x"], &["'7"], &["pq"]]),
        ("nest", &[&["ab-2"], &["x-3"], &["7-1"], &["pq-5"]]),
        ("num", &[&["7"]]),
        ("fit", &[&["ab"], &["'7"]]),
    ];
    check(&r, "after the edit", &after);
    r.last = None;
    r.run();
    check(&r, "cold", &after);
    for (name, rules, why) in [
        ("unknown", "bad(t) :- w(s, _), t = nosuch(s).\n", "a call of nosuch, which is no function"),
        ("arity", "bad(t) :- w(s, _), t = strlen(s, s).\n", "strlen of 2 arguments"),
    ] {
        let mut r = kernel_files(&format!("phase-call-{name}"), ASK_W, &[("u-facts.rerail", FACTS), ("u-w.rerail", W), ("u.rerail", rules)]);
        let e = r.try_run().expect_err(name);
        assert!(e.contains(why), "{name}: {e}");
    }
}

/// K1's arithmetic (B§6, L3; B§10 step 8; K1-a): each expression a step of an operator (a Pure answer, B§3 sentence 3),
/// its answer a variable named by its node, placed after the atom that binds the last variable under it and run in
/// postorder there (an operand before its operator, an operator before its filter), one of constants alone first. Each
/// program lowered by P3 gives the rows written by hand, its rule's steps the hand-lowered form's, and a cold run the
/// same. `=` of a variable no step binds binds it; a division by zero has no row.
#[test]
fn k1s_arithmetic_lowers_to_operator_steps_in_postorder() {
    type Steps = &'static [(&'static str, &'static str)];
    type Want = &'static [&'static [i64]];
    let cases: [(&str, &str, Steps, Want); 8] = [
        ("head", "h(x + 1) :- e(x, _).\n", &[("atom", "e"), ("atom", "+")], &[&[2], &[3], &[4]]),
        ("nested", "h(x, (x + y) * 2 - 1) :- e(x, y).\n", &[("atom", "e"), ("atom", "+"), ("atom", "*"), ("atom", "-")], &[&[1, 5], &[2, 9], &[3, 7], &[2, 11]]),
        ("filter", "h(x) :- x * 2 > y + 1, e(x, y).\n", &[("atom", "e"), ("atom", "*"), ("atom", "+"), ("cmp", ">")], &[&[3]]),
        ("bind", "h(x, z) :- e(x, y), z = y * 10.\n", &[("atom", "e"), ("atom", "*"), ("cmp", "=")], &[&[1, 20], &[2, 30], &[3, 10], &[2, 40]]),
        ("unary", "h(x, -x, x / 2, x % 2) :- e(x, _).\n", &[("atom", "e"), ("atom", "-"), ("atom", "/"), ("atom", "%")], &[&[1, -1, 0, 1], &[2, -2, 1, 0], &[3, -3, 1, 1]]),
        ("const", "h(x, 2 * 3) :- e(x, y), y > 1 + 0.\n", &[("atom", "*"), ("atom", "+"), ("atom", "e"), ("cmp", ">")], &[&[1, 6], &[2, 6]]),
        ("zero", "h(x, 6 / (x - 1)) :- e(x, _).\n", &[("atom", "e"), ("atom", "-"), ("atom", "/")], &[&[2, 6], &[3, 3]]),
        ("in-atom", "h(x) :- e(x, y), e(y + 1, _).\n", &[("atom", "e"), ("atom", "+"), ("atom", "e")], &[&[1], &[3]]),
    ];
    for (name, k1, steps, want) in cases {
        let mut r = kernel_run(&format!("phase-k1a-{name}"), k1);
        r.run();
        let want: BTreeSet<Vec<i64>> = want.iter().map(|x| x.to_vec()).collect();
        assert_eq!(r.rows(2, "h"), want, "{name}: lowered by P3");
        assert_eq!(steps_in_order(&r, 1, "h"), steps.iter().map(|&(k, x)| (k.to_string(), x.to_string())).collect::<Vec<_>>(), "{name}: its steps");
        r.last = None;
        r.run();
        assert_eq!(r.rows(2, "h"), want, "{name}: cold");
    }
}

/// K1's declarations (B§6, L3; B§10 step 8; K1-d): a `.decl` is K-form rows, `k_decl`, a column each with its type,
/// and the program runs as before. A head or a step of another arity than its relation's declaration is refused, as
/// is a column declared of two types.
#[test]
fn k1s_declarations_are_k_form_rows_and_an_arity_against_them_is_refused() {
    let mut r = kernel_run("phase-k1d", ".decl h(x: number)\n.decl e(a: number, b: symbol)\nh(x) :- e(x, _).\n");
    r.run();
    assert_eq!(r.rows(2, "h"), BTreeSet::from([vec![1], vec![2], vec![3]]));
    let decls: BTreeSet<(String, i64, String)> = r.rows(1, "k_decl").into_iter().map(|x| (r.name(x[1]), x[2], r.name(x[3]))).collect();
    let want = [("h", 0, "number"), ("e", 0, "number"), ("e", 1, "symbol")].map(|(a, i, t)| (a.to_string(), i, t.to_string()));
    assert_eq!(decls, BTreeSet::from(want));
    for (name, text, why) in [
        ("head", ".decl h(x: number, y: number)\nh(x) :- e(x, _).\n", "head h of 1 columns, declared with 2"),
        ("step", ".decl e(a: number)\nh(x) :- e(x, _).\n", "reads e of 2 columns, declared with 1"),
        ("twice", ".decl h(x: number)\n.decl h(x: symbol)\nh(x) :- e(x, _).\n", "relation h's column 0 is declared of type number and of symbol"),
    ] {
        let mut r = kernel_run(&format!("phase-k1d-{name}"), text);
        let e = r.try_run().err().unwrap_or_else(|| panic!("{name}: run, not refused: {:?}", r.rows(1, "k_decl")));
        assert!(e.contains(why), "{name}: {e}");
    }
}

/// K1's computed strata (B§6, L3: "strata computed"; O4; K1-e): a module's stratum is its longest chain of modules whose
/// heads it reads, an atom's or a negation's, through the numbers' chain; the files' order in the ask is not theirs.
/// Modules that read none of each other share a stratum. A cycle of modules has no stratum, and is refused.
#[test]
fn k1s_strata_are_computed_from_the_modules_reads_and_a_cycle_is_refused() {
    let ask = "load(\"c.rerail\", \"c\", \"rr\").\nload(\"b.rerail\", \"b\", \"rr\").\nload(\"a.rerail\", \"a\", \"rr\").\nload(\"z.rerail\", \"z\", \"rr\").\n";
    let files = [
        ("a.rerail", "e(1, 2).\ne(2, 3).\ne(3, 1).\n"),
        ("b.rerail", "p(x, z) :- e(x, y), e(y, z).\n"),
        ("c.rerail", "q(x) :- e(x, _), !p(x, 3).\n"),
        ("z.rerail", "w(1).\n"),
    ];
    let mut r = kernel_files("phase-k1e", ask, &files);
    r.run();
    let order: BTreeSet<(String, i64)> = r.rows(1, "k_order").into_iter().map(|x| (r.docs.path_of(&r.bench.dir, r.atoms, x[0]), x[1])).collect();
    let want = [("a.rerail", 0), ("b.rerail", 1), ("c.rerail", 2), ("z.rerail", 0)].map(|(p, k)| (p.to_string(), k));
    assert_eq!(order, BTreeSet::from(want), "one row a module, its longest chain");
    assert_eq!(r.rows(2, "q"), BTreeSet::from([vec![2], vec![3]]), "c after b: p is (1, 3), (2, 1), (3, 2), so only 1 has p(x, 3)");
    let cycle = [("a.rerail", "e(1, 2).\n"), ("b.rerail", "p(x) :- e(x, _), !q(x).\n"), ("c.rerail", "q(x) :- p(x).\n"), ("z.rerail", "w(1).\n")];
    let mut r = kernel_files("phase-k1e-cycle", ask, &cycle);
    let e = r.try_run().expect_err("a cycle of modules is refused");
    assert!(e.contains("no k_order row"), "{e}");
}

/// The steps of the rule level `k`'s K-form writes `head` (one rule), in their key order: (kind, relation or operator).
fn steps_in_order(r: &Run, k: usize, head: &str) -> Vec<(String, String)> {
    let rules: Vec<i64> = r.rows(k, "k_rule").into_iter().filter(|x| x[2] == r.s(head)).map(|x| x[1]).collect();
    assert_eq!(rules.len(), 1, "one rule writes {head}");
    let mut steps: Vec<Vec<i64>> = r.rows(k, "k_step").into_iter().filter(|x| x[1] == rules[0]).collect();
    steps.sort_by_key(|x| (x[2], x[3]));
    steps.into_iter().map(|x| (r.name(x[4]), r.name(x[5]))).collect()
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
    // Each lowered rule's steps, in their key order, are the hand-ordered form's: `x != z` after the atom binding z,
    // `!b(y)` after the one binding y, `y > 2` after its own; a placed filter at (p, f), every atom at its node
    // (k, -1).
    let order = |head: &str| steps_in_order(&r, 1, head);
    assert_eq!(order("h"), [("atom", "e"), ("not", "b"), ("atom", "e"), ("cmp", "!=")].map(|(k, x)| (k.to_string(), x.to_string())));
    assert_eq!(order("g"), [("atom", "e"), ("cmp", ">"), ("atom", "e")].map(|(k, x)| (k.to_string(), x.to_string())));
    let moved: BTreeSet<String> = r.rows(1, "k_step").into_iter().filter(|x| x[3] != -1).map(|x| r.name(x[5])).collect();
    assert_eq!(moved, BTreeSet::from(["!=".to_string(), ">".to_string(), "b".to_string()]), "every filter placed, at (p, f)");
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
    // The same files asked by a rule (B§10 step 9: a `load` the program derives, answered in the level's next round):
    // the same rows.
    r.write("ask.rerail", "load(p, m, \"rr\") :- boot(p, m).\nboot(\"u-facts.rerail\", \"u-facts\").\nboot(\"u.rerail\", \"u\").\n", 20);
    r.run();
    assert_eq!(rows(&r, 2), warm, "loads a rule derives");
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
