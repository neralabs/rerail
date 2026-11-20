//! A Metal plan's steps and stores, and the barrier verifier (docs/parser-generators/17-tier-2-tdd-plan.md
//! §4). Nothing here runs a file.
//!
//! The plans themselves are made again for Metal, after the CPU, from the tree section's state
//! machine (docs/parser-generators/18-tree-as-grammar.md §5): the carry is the depth and its low
//! point, closes find their opens by nearest smaller value, and each level's transitions compose
//! as state functions (`block_monoid`). The old plans read the rows' start conditions, which the
//! grammar replaced. What stays is the plan's shape and its checks:
//!
//! - **Stores** in threadgroup or device memory, single or double-buffered; the threadgroup ones
//!   count against a budget well under 32 KB.
//! - **Steps:** loops (uniform or ragged), work with the stores it reads and writes, barriers.
//! - **The verifier:** the budget; SIMD operations and barriers only in uniform loops; a store read
//!   only after a barrier that follows another step's write, and rewritten only after one that
//!   follows another step's read (a double store's writes take its buffers in turn).

use std::fmt::Write as _;

/// Who runs the plan: Metal, or the CPU (one forward pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Target {
    Gpu,
    Cpu,
}

/// Where a store lives: a barrier on threadgroup memory orders only it; one on device memory
/// orders both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Mem {
    Threadgroup,
    Device,
}

/// Memory the steps share. Only threadgroup stores count against the budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Store {
    pub(super) name: String,
    pub(super) mem: Mem,
    /// Threadgroup bytes, both buffers if `double`.
    pub(super) bytes: u32,
    /// Two buffers, which writes take in turn: a write waits only for a barrier after the reads of
    /// the buffer it takes. The generated code keeps the same parity.
    pub(super) double: bool,
    pub(super) note: String,
}

/// A step of a plan. Every thread of the threadgroup runs every step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Step {
    /// A loop over tiles or rounds. `uniform`: every thread runs the same iterations.
    Loop { what: String, uniform: bool, by: String, body: Vec<Step> },
    /// Work, with the stores it reads and writes; `simd`: it uses SIMD-group operations.
    Work { what: String, simd: bool, reads: Vec<String>, writes: Vec<String> },
    Barrier(Mem),
}

/// A field of a candidate's words: bits `at..at + bits`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Field {
    pub(super) name: String,
    pub(super) at: u32,
    pub(super) bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Plan {
    pub(super) target: Target,
    pub(super) candidates: u32,
    /// S1's word: its table's bits and the identity's.
    pub(super) s1_bits: u32,
    pub(super) fields: Vec<Field>,
    pub(super) cand_words: u32,
    pub(super) stores: Vec<Store>,
    pub(super) steps: Vec<Step>,
    pub(super) budget: u32,
}

/// Threads per threadgroup, lanes per SIMD group, bytes per lane.
pub(super) const THREADS: u32 = 1024;
pub(super) const SIMD: u32 = 32;
pub(super) const LANE: u32 = 64;
/// The threadgroup memory a plan may use: well under 32 KB, so several threadgroups fit a core
/// and hide memory latency.
pub(super) const BUDGET: u32 = 10240;
/// A device store.
pub(super) fn device(name: &str) -> Store {
    Store { name: name.into(), mem: Mem::Device, bytes: 0, double: false, note: String::new() }
}



impl Plan {
    pub(in crate::code_rule_dsl) fn entry_words(&self) -> u32 {
        if self.target == Target::Cpu { 0 } else { 1 + self.candidates * self.cand_words }
    }

    pub(in crate::code_rule_dsl) fn threadgroup_bytes(&self) -> u32 {
        self.stores.iter().filter(|s| s.mem == Mem::Threadgroup).map(|s| s.bytes).sum()
    }

    fn store(&self, name: &str) -> Option<&Store> {
        self.stores.iter().find(|s| s.name == name)
    }

    /// The plan's barriers, in order.
    pub(in crate::code_rule_dsl) fn barriers(&self) -> usize {
        fn count(steps: &[Step]) -> usize {
            steps
                .iter()
                .map(|s| match s {
                    Step::Barrier(_) => 1,
                    Step::Loop { body, .. } => count(body),
                    Step::Work { .. } => 0,
                })
                .sum()
        }
        count(&self.steps)
    }

    /// The plan with its `k`th barrier removed: for the verifier's tests.
    pub(in crate::code_rule_dsl) fn without_barrier(&self, k: usize) -> Plan {
        fn cut(steps: &mut Vec<Step>, k: &mut usize) -> bool {
            let mut i = 0;
            while i < steps.len() {
                match &mut steps[i] {
                    Step::Barrier(_) if *k == 0 => {
                        steps.remove(i);
                        return true;
                    }
                    Step::Barrier(_) => *k -= 1,
                    Step::Loop { body, .. } => {
                        if cut(body, k) {
                            return true;
                        }
                    }
                    Step::Work { .. } => {}
                }
                i += 1;
            }
            false
        }
        let mut p = self.clone();
        cut(&mut p.steps, &mut { k });
        p
    }

    /// The verifier: the budget; SIMD operations and barriers only in uniform loops; and every
    /// shared store read after a barrier that follows another thread's write, and rewritten after
    /// a barrier that follows another thread's read.
    pub(in crate::code_rule_dsl) fn verify(&self) -> Result<(), String> {
        if self.threadgroup_bytes() > self.budget {
            return Err(format!("{} bytes of threadgroup memory, over the budget of {}", self.threadgroup_bytes(), self.budget));
        }
        self.check(&self.steps)
    }

    fn check(&self, steps: &[Step]) -> Result<(), String> {
        fn ragged(steps: &[Step]) -> bool {
            steps.iter().any(|s| match s {
                Step::Barrier(_) => true,
                Step::Work { simd, .. } => *simd,
                Step::Loop { body, .. } => ragged(body),
            })
        }
        fn uniform(steps: &[Step]) -> Result<(), String> {
            for s in steps {
                if let Step::Loop { what, uniform: u, body, .. } = s {
                    if !*u && ragged(body) {
                        return Err(format!("{what}: a SIMD operation or a barrier in a loop that is not uniform"));
                    }
                    uniform(body)?;
                }
            }
            Ok(())
        }
        uniform(steps)?;
        let mut state = Hazards::default();
        self.walk(steps, &mut state)
    }

    /// Walks the steps, each loop's body three times, so that a double store's two buffers both
    /// meet the iteration before.
    fn walk(&self, steps: &[Step], h: &mut Hazards) -> Result<(), String> {
        for s in steps {
            match s {
                Step::Barrier(m) => h.barrier(*m, |name| self.store(name).map_or(Mem::Device, |s| s.mem)),
                Step::Loop { body, .. } => {
                    for _ in 0..3 {
                        self.walk(body, h)?;
                    }
                }
                Step::Work { what, reads, writes, .. } => {
                    for r in reads {
                        if let Some(w) = h.written.get(r)
                            && w != what {
                                return Err(format!("`{what}` reads {r}, written by `{w}`, with no barrier between"));
                            }
                        let buffer = h.buffer(r, false);
                        h.read.insert((r.clone(), buffer), what.clone());
                    }
                    for x in writes {
                        let double = self.store(x).is_some_and(|s| s.double);
                        let buffer = h.buffer(x, double);
                        if let Some(r) = h.read.get(&(x.clone(), buffer))
                            && r != what {
                                return Err(format!("`{what}` rewrites {x} while `{r}` may still read it: no barrier between"));
                            }
                        h.written.insert(x.clone(), what.clone());
                    }
                }
            }
        }
        Ok(())
    }


    pub(in crate::code_rule_dsl) fn text(&self) -> String {
        let mut out = String::new();
        if self.target == Target::Cpu {
            let _ = writeln!(out, "plan cpu");
        } else {
            let _ = writeln!(out, "plan gpu");
            let _ = writeln!(out, "  threads {THREADS}, simd {SIMD}, lane {LANE} bytes, tile {} bytes", THREADS * LANE);
            let words = self.entry_words();
            let _ = writeln!(out, "  entry: {words} words, {} bytes, in device memory as {words} arrays, candidate-major", words * 4);
            if self.s1_bits > 0 {
                let _ = writeln!(out, "    word 0: s1: {} bits, identity 1", self.s1_bits - 1);
            }
            let ranges: Vec<String> = (0..self.candidates).map(|c| format!("{}-{}", 1 + c * self.cand_words, (c + 1) * self.cand_words)).collect();
            let _ = writeln!(out, "    candidate words {}:", ranges.join(", "));
            let fields: Vec<String> = self.fields.iter().map(|f| format!("{} {}..{}", f.name, f.at, f.at + f.bits)).collect();
            let _ = writeln!(out, "      {}", fields.join(", "));
            let _ = writeln!(out, "  threadgroup memory: {} of {} bytes", self.threadgroup_bytes(), self.budget);
            for s in self.stores.iter().filter(|s| s.mem == Mem::Threadgroup) {
                let _ = writeln!(out, "    {} {}: {}", s.name, s.bytes, s.note);
            }
        }
        let _ = writeln!(out, "  steps");
        fn print_steps(out: &mut String, steps: &[Step], indent: usize) {
            let pad = " ".repeat(indent);
            for s in steps {
                match s {
                    Step::Loop { what, uniform, by, body } => {
                        let _ = writeln!(out, "{pad}{what}, {}, by {by}", if *uniform { "uniform" } else { "ragged" });
                        print_steps(out, body, indent + 2);
                    }
                    Step::Work { what, reads, writes, .. } => {
                        let mut line = format!("{pad}{what}");
                        if !reads.is_empty() {
                            let _ = write!(line, "; reads {}", reads.join(", "));
                        }
                        if !writes.is_empty() {
                            let _ = write!(line, "; writes {}", writes.join(", "));
                        }
                        let _ = writeln!(out, "{line}");
                    }
                    Step::Barrier(m) => {
                        let _ = writeln!(out, "{pad}barrier {}", if *m == Mem::Threadgroup { "threadgroup" } else { "device" });
                    }
                }
            }
        }
        print_steps(&mut out, &self.steps, 4);
        if self.target == Target::Gpu {
            let per = |i: usize| match &self.steps.get(i) {
                Some(Step::Loop { body, .. }) => body.iter().filter(|s| matches!(s, Step::Barrier(_))).count(),
                _ => 0,
            };
            let between = self.steps.iter().filter(|s| matches!(s, Step::Barrier(_))).count();
            let _ = writeln!(out, "  barriers: pass A {} per tile, scan {} per round, pass B {} per tile, {between} between passes", per(0), per(2), per(4));
        }
        out
    }
}

/// The verifier's state: each store's writer and readers since its last barrier. A double
/// store's writes take its two buffers in turn, and a read reads the buffer last written; so a
/// write waits only for a barrier after the reads of the buffer it takes.
#[derive(Default)]
struct Hazards {
    written: std::collections::HashMap<String, String>,
    read: std::collections::HashMap<(String, u32), String>,
    writes: std::collections::HashMap<String, u32>,
}

impl Hazards {
    /// The buffer a write takes (`write`), or a read reads.
    fn buffer(&mut self, store: &str, write: bool) -> u32 {
        let n = self.writes.entry(store.to_owned()).or_insert(0);
        if write {
            *n += 1;
            *n % 2
        } else {
            *n % 2
        }
    }

    fn barrier(&mut self, m: Mem, mem: impl Fn(&str) -> Mem) {
        let keep = |name: &String| m == Mem::Threadgroup && mem(name) == Mem::Device;
        self.written.retain(|k, _| keep(k));
        self.read.retain(|k, _| keep(&k.0));
    }
}
