//! 0156's E9 against the engine's own lookups: the base rows `tape16 base --dump <dir>` writes, kept as a frame
//! gives them (`InRel::kept`), probed as a join probes a kept input: `InRel::index` on the probed column, its
//! order made on the first probe (`Csr::runs`, 0186) and read by key prints (0185); `RERAIL_KEY_PRINTS=off` (the
//! order alone) and `hash` (the hash table before 0112) for the other two. Every node is probed once a relation,
//! and the rows found are checked against the lab's count.

use rerail_blocks::vector::Vector;
use super::*;

fn load(path: &std::path::Path) -> Vector<i64> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    bytes.as_chunks::<8>().0.iter().map(|b| i64::from_le_bytes(*b)).collect()
}

#[test]
#[ignore = "a measurement: RERAIL_E9_ROWS names a `tape16 base --dump` directory (0156 E9)"]
fn base_rows_probed_by_the_engine() {
    let dir = std::path::PathBuf::from(std::env::var("RERAIL_E9_ROWS").expect("RERAIL_E9_ROWS"));
    let mode = std::env::var("RERAIL_KEY_PRINTS").unwrap_or_else(|_| "prints".into());
    let meta = std::fs::read_to_string(dir.join("meta")).unwrap();
    let mut rels: HashMap<String, Arc<InRel>> = HashMap::default();
    println!("| relation | probed column | mode | rows | first probe: ms | ns/row | probes | ns/probe | found |");
    println!("|---|---|---|---|---|---|---|---|---|");
    let nodes: i64 = meta.lines().find(|l| l.starts_with("node ")).and_then(|l| l.split(' ').nth(2)).unwrap().parse().unwrap();
    for line in meta.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let (name, w, n, col, want): (&str, usize, usize, usize, i64) = (f[0], f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].parse().unwrap(), f[4].parse().unwrap());
        let rel = Arc::clone(rels.entry(name.to_owned()).or_insert_with(|| {
            let values = load(&dir.join(format!("{name}.rows")));
            assert_eq!(values.len(), w * n, "{name}: rows of its width");
            InRel::kept(vec![b'i'; w], values, n)
        }));
        // The index and its order, as the first probe makes them.
        let (c0, t0) = (thread_cpu(), Instant::now());
        let csr = rel.index(&[col]);
        let order = csr.runs(&rel);
        let first = probe(&csr, order, &rel, 0);
        let (build_cpu, build_wall) = (thread_cpu() - c0, t0.elapsed().as_secs_f64());
        // Every node once.
        let (c0, t0) = (thread_cpu(), Instant::now());
        let mut found = 0i64;
        for id in 0..nodes {
            found += probe(&csr, order, &rel, id) as i64;
        }
        let (cpu, wall) = (thread_cpu() - c0, t0.elapsed().as_secs_f64());
        std::hint::black_box(first);
        println!(
            "| {name} | {col} | {mode} | {n} | {:.1} (wall {:.1}) | {:.1} | {nodes} | {:.1} (wall {:.1}) | {} |",
            build_cpu * 1e3, build_wall * 1e3, build_cpu * 1e9 / n as f64, cpu * 1e9 / nodes as f64, wall * 1e9 / nodes as f64,
            if found == want { "as the lab".to_owned() } else { format!("{found}, the lab {want}") }
        );
        assert_eq!(found, want, "{name} by column {col}: the rows found");
    }
}

/// The rows of `rel` whose probed column holds `key`, as a join's probe finds them.
fn probe(csr: &Csr, order: Option<&Index>, rel: &InRel, key: i64) -> usize {
    match order {
        Some(ix) => ix.find(&rel.rows, rel.k, &[key], None).len(),
        None => csr.get(rel, &[key]).len(),
    }
}
