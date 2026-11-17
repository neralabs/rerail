//! Measures a projection's kept parents (`Projected::up`) against its parents read off the tape, in each format
//! (the user, 2026-11-16: "drop the parent projections: yes, probably, but measure first"). One ask for every node a
//! pass, as `sizes`, `depths` and `ordinals` each make. Counts are the least over rounds run in turns: instructions
//! retired, cycles and thread CPU.
//!
//! `cargo run --profile profiling --example kept_parents -- DIR [ROUNDS]` (Python files).

// The engine registers its language packs as it loads.
extern crate rerail_engine;

use rerail_facts::code_rule_dsl::facts::core::{self, Facts};
use rerail_facts::code_rule_dsl::facts::vtab::syntax::{Dialect, Projected, Shape};
use rerail_facts::code_rule_dsl::language::GrammarDialect;
use rerail_facts::code_rule_dsl::lines::{LineTable, Terminators};
use rerail_facts::code_rule_dsl::tape::{Grammar, TapeImage, formats::Format};
use std::sync::Arc;

#[derive(Clone, Copy, Default)]
struct Cost {
    instructions: u64,
    cycles: u64,
    cpu: f64,
}

impl Cost {
    fn least(self, o: Cost) -> Cost {
        if self.instructions == 0 { o } else { Cost { instructions: self.instructions.min(o.instructions), cycles: self.cycles.min(o.cycles), cpu: self.cpu.min(o.cpu) } }
    }
    fn add(&mut self, o: Cost) {
        self.instructions += o.instructions;
        self.cycles += o.cycles;
        self.cpu += o.cpu;
    }
}

fn measure<T>(f: impl FnOnce() -> T) -> (T, Cost) {
    let cpu = || {
        let (u, s) = rerail_facts::code_rule_dsl::prof::thread_usage();
        u + s
    };
    let (c0, (i0, y0)) = (cpu(), rerail_compiler::native_cpu_codegen::retired());
    let out = std::hint::black_box(f());
    let ((i1, y1), c1) = (rerail_compiler::native_cpu_codegen::retired(), cpu());
    (out, Cost { instructions: i1 - i0, cycles: y1 - y0, cpu: c1 - c0 })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("a directory of Python files");
    let rounds: usize = args.next().map_or(5, |r| r.parse().expect("rounds"));
    let lang = GrammarDialect::Python;
    let grammar = Grammar::new(lang).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "py")).collect();
    files.sort();
    let sources: Vec<Arc<String>> = files.iter().map(|p| Arc::new(std::fs::read_to_string(p).unwrap())).collect();
    let images: Vec<TapeImage> = sources.iter().map(|s| TapeImage::parse(&grammar, s, &|| false).unwrap().unwrap().primary).collect();
    println!("{} files, {} bytes, {} nodes", files.len(), sources.iter().map(|s| s.len()).sum::<usize>(), images.iter().map(TapeImage::node_count).sum::<usize>());
    println!("{:<5} {:<22} {:>14} {:>14} {:>10}", "tape", "pass", "instructions", "cycles", "cpu ms");
    for format in Format::ALL {
        let facts: Vec<Arc<Facts>> = sources
            .iter()
            .zip(&images)
            .map(|(s, image)| {
                let lines = Arc::new(LineTable::new(s, Terminators::of(lang)));
                Arc::new(core::from_image(Arc::clone(s), &grammar, image.clone().in_format(format, s.as_bytes()), lines).unwrap())
            })
            .collect();
        let dialect = Dialect::new(core::facts_tag(&facts[0])).unwrap();
        let (mut walk, mut kept, mut tape) = (Cost::default(), Cost::default(), Cost::default());
        let mut locals = 0;
        for f in &facts {
            let (mut w, mut k, mut t) = (Cost::default(), Cost::default(), Cost::default());
            for _ in 0..rounds {
                let (p, c) = measure(|| Projected::new(1, Arc::clone(f), Some(&dialect), Shape::Core, false, None));
                w = w.least(c);
                let n = p.core.len();
                let (a, c) = measure(|| (0..n).map(|l| u64::from(p.parent(l))).sum::<u64>());
                k = k.least(c);
                let (b, c) = measure(|| (0..n).map(|l| u64::from(p.parent_off_tape(l))).sum::<u64>());
                t = t.least(c);
                assert_eq!(a, b, "the kept parents are the tape's");
            }
            locals += Projected::new(1, Arc::clone(f), Some(&dialect), Shape::Core, false, None).core.len();
            walk.add(w);
            kept.add(k);
            tape.add(t);
        }
        for (name, c) in [("walk (keeps up)", walk), ("parents kept", kept), ("parents off the tape", tape)] {
            println!("{:<5} {:<22} {:>14} {:>14} {:>10.2}", format.name(), name, c.instructions, c.cycles, c.cpu * 1e3);
        }
        println!("{:<5} kept parents: {} locals, {} bytes", format.name(), locals, 4 * locals);
    }
}
