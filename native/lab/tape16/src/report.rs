//! Tables: one row per solution, columns per tradeoff metric.

use crate::solution::{Lang, Row};

fn per(v: f64, n: u64) -> f64 {
    if n == 0 { 0.0 } else { v / n as f64 }
}

pub const GRID_HEADER: &str = "| solution | B/node | ×v15 stored | ×v15 trained | ×source stored | entropy B/node | placement B/node | dict KB | dup % | enc ins/node | enc ns/node | dec ins/node | dec ns/node | positions ns/node | random read µs | errors |";

pub fn grid_row(r: &Row, t: &Lang) -> String {
    let n = t.nodes;
    format!(
        "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.3} | {} | {:.1} | {:.0} | {:.1} | {:.0} | {:.1} | {:.1} | {:.2} | {} |",
        r.solution,
        per((t.stored + t.placement) as f64, n),
        (t.stored + t.placement) as f64 / t.v15_stored.max(1) as f64,
        (t.stored + t.placement) as f64 / t.v15_trained.max(1) as f64,
        (t.stored + t.placement) as f64 / t.source_stored.max(1) as f64,
        per(t.entropy, n),
        per(t.placement as f64, n),
        t.dict_bytes / 1024,
        100.0 * t.dup_bytes as f64 / t.stored.max(1) as f64,
        per(r.encode.instructions as f64, n),
        per(r.encode.nanos as f64, n),
        per(r.decode.instructions as f64, n),
        per(r.decode.nanos as f64, n),
        per(r.positions.nanos as f64, n),
        r.random.nanos as f64 / r.random_reads.max(1) as f64 / 1000.0,
        r.errors,
    )
}

pub fn print_grid(rows: &[Row]) {
    println!("{GRID_HEADER}");
    println!("|{}", "---|".repeat(GRID_HEADER.matches('|').count() - 1));
    for r in rows {
        println!("{}", grid_row(r, &r.total()));
    }
}

pub fn print_langs(r: &Row) {
    println!("\n{} by language\n", r.solution);
    println!("| language | files | nodes | source KB | v15 raw B/node | v15 stored B/node | v15 trained B/node | laid B/node | stored B/node | source stored B/node | blocks |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, l) in &r.by_lang {
        let n = l.nodes;
        println!(
            "| {name} | {} | {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {} |",
            l.files,
            n,
            l.source_bytes / 1024,
            per(l.v15_raw as f64, n),
            per(l.v15_stored as f64, n),
            per(l.v15_trained as f64, n),
            per(l.laid as f64, n),
            per((l.stored + l.placement) as f64, n),
            per(l.source_stored as f64, n),
            l.blocks,
        );
    }
}
