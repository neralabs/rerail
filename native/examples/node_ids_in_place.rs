//! Measures node ids answered in place (the granule table and its rank index, decision 0353 step 2) against the numbered
//! array (`DefIds::low`), in each format: a point read of every node's id, the scan of a document's ids in preorder,
//! and the node of every id (the array's way back, a sorted copy made on first use, against the table's select).
//! Counts are the least over rounds run in turns: instructions retired, cycles and thread CPU.
//!
//! `cargo run --profile profiling --example node_ids_in_place -- ROUNDS PATH...` (Python files, or directories of them).

// The engine registers its language packs as it loads.
extern crate rerail_engine;

use rerail_facts::code_rule_dsl::facts::core::{self, Facts};
use rerail_facts::code_rule_dsl::facts::vtab::syntax::{Dialect, Projected, Shape};
use rerail_facts::code_rule_dsl::language::GrammarDialect;
use rerail_facts::code_rule_dsl::lines::terminators;
use rerail_facts::code_rule_dsl::tape::{Grammar, TapeImage, formats::Format};
use std::path::PathBuf;
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

fn python_files(path: PathBuf, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(&path).unwrap().flatten().map(|e| e.path()).collect();
        entries.sort();
        for e in entries {
            python_files(e, out);
        }
    } else if path.extension().is_some_and(|e| e == "py") {
        out.push(path);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: usize = args.next().expect("rounds").parse().expect("rounds");
    let mut files = Vec::new();
    for a in args {
        python_files(PathBuf::from(a), &mut files);
    }
    let lang = GrammarDialect::Python;
    let grammar = Grammar::new(lang).unwrap();
    let sources: Vec<Arc<String>> = files.iter().map(|p| Arc::new(std::fs::read_to_string(p).unwrap())).collect();
    let images: Vec<TapeImage> = sources.iter().map(|s| TapeImage::parse(&grammar, s, &|| false).unwrap().unwrap().primary).collect();
    println!("{} files, {} bytes, {} tape nodes", files.len(), sources.iter().map(|s| s.len()).sum::<usize>(), images.iter().map(TapeImage::node_count).sum::<usize>());
    println!("{:<5} {:<26} {:>14} {:>14} {:>10}", "tape", "pass", "instructions", "cycles", "cpu ms");
    for format in Format::ALL {
        let facts: Vec<Arc<Facts>> = sources
            .iter()
            .zip(&images)
            .map(|(s, image)| {
                let lines = Arc::new(image.lines(s, terminators(lang), false));
                Arc::new(core::from_image(Arc::clone(s), &grammar, image.clone().in_format(format, s.as_bytes()), lines).unwrap())
            })
            .collect();
        let dialect = Dialect::new(core::facts_tag(&facts[0])).unwrap();
        let mut total = [Cost::default(); 6];
        let (mut ids, mut granules, mut table_bytes, mut biggest) = (0usize, 0usize, 0usize, 0u32);
        for f in &facts {
            let p = Projected::new(1, Arc::clone(f), Some(&dialect), Shape::Core, false, None);
            let reference = p.reference(&dialect);
            let low = &reference.low[..];
            let table = p.table().expect("ids by definition");
            let tokens = p.token_order();
            // Every numbered item's tape node, nodes then tokens: what a reader holds when it asks for an id.
            let items: Vec<u32> = p.nodes().map(|(_, x)| x).chain(tokens.iter().map(|&(x, _)| x)).collect();
            ids += items.len();
            granules += table.len();
            table_bytes += table.bytes();
            biggest = biggest.max(table.largest());
            for (k, &x) in items.iter().enumerate() {
                let got = table.id(p.ranks(), x).map(|(l, _)| l);
                if got != Some(low[k]) {
                    let parent = f.parent(x).map(|q| (q, dialect.name(f, q), f.named(q)));
                    panic!("{}: item {k} (tape {x}, {:?} {:?}, parent {parent:?}): in place {got:x?}, numbered {:x}", files[facts.iter().position(|g| Arc::ptr_eq(g, f)).unwrap()].display(), dialect.name(f, x), f.text(x), low[k]);
                }
            }
            let mut least = [Cost::default(); 6];
            for _ in 0..rounds {
                let passes: [&dyn Fn() -> u64; 6] = [
                    &|| low.iter().map(|&l| u64::from(l)).sum(),
                    &|| items.iter().map(|&x| u64::from(table.id(p.ranks(), x).unwrap().0)).sum(),
                    &|| {
                        let mut s = 0u64;
                        table.scan(p.ranks(), |_, l, _| s += u64::from(l));
                        s
                    },
                    &|| {
                        let q = Projected::new(1, Arc::clone(f), Some(&dialect), Shape::Core, false, None);
                        low.iter().map(|&l| u64::from(q.local_of(l).unwrap())).sum()
                    },
                    &|| low.iter().map(|&l| u64::from(table.node(p.ranks(), l).unwrap().0)).sum(),
                    &|| Projected::new(1, Arc::clone(f), Some(&dialect), Shape::Core, false, None).len() as u64,
                ];
                let mut sums = [0u64; 6];
                for (k, pass) in passes.iter().enumerate() {
                    let (s, c) = measure(pass);
                    sums[k] = s;
                    least[k] = least[k].least(c);
                }
                assert_eq!(sums[0], sums[1], "the point reads are the array's");
                assert_eq!(sums[0], sums[2], "the scan is the array's");
            }
            for k in 0..6 {
                total[k].add(least[k]);
            }
        }
        // The way back's pass includes a projection made for it (its sorted copy is made on first use): less the
        // projection's own cost, measured apart.
        let back = Cost { instructions: total[3].instructions - total[5].instructions, cycles: total[3].cycles - total[5].cycles, cpu: total[3].cpu - total[5].cpu };
        for (name, c) in [
            ("id: array", total[0]),
            ("id: point read in place", total[1]),
            ("id: scan in place", total[2]),
            ("node: array's way back", back),
            ("node: select in place", total[4]),
            ("(projection, for scale)", total[5]),
        ] {
            println!("{:<5} {:<26} {:>14} {:>14} {:>10.2}", format.name(), name, c.instructions, c.cycles, c.cpu * 1e3);
        }
        println!("{:<5} {ids} ids, {granules} granules, table {table_bytes} bytes (array {} bytes), largest granule {biggest} items", format.name(), 4 * ids);
    }
}
