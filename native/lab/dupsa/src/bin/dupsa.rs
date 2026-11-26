//! ```text
//! dupsa lines <root> [--include <prefix>]... [--ext <ext>]... [--min 30] [--out <file.tsv>]
//!     Jeff's rule on a suffix array: blocks of at least `min` counted lines repeated anywhere in the scope.
//! dupsa nodes <capture> [--min 200] [--min-lines 0] [--cells row|code-slot] [--out <file.tsv>]
//!     The same over format 16 tape nodes, one suffix array a language (`rerail-engine capture-tapes`).
//! ```
//!
//! Output: one line a copy, `path<TAB>first line<TAB>last line<TAB>units<TAB>other places`, in path order; a
//! summary on stderr.

use rerail_dupsa_lab::multi::{Docs, MultiSuffixArray};
use rerail_dupsa_lab::nodes::{Capture, Cells, NodeDocs};
use rerail_dupsa_lab::{lines, report};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

fn cpu() -> f64 {
    let mut u: libc_rusage = unsafe { std::mem::zeroed() };
    unsafe { getrusage(0, &mut u) };
    let t = |v: [i64; 2]| v[0] as f64 + v[1] as f64 / 1e6;
    t(u.utime) + t(u.stime)
}

#[repr(C)]
struct libc_rusage {
    utime: [i64; 2],
    stime: [i64; 2],
    rest: [i64; 14],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut libc_rusage) -> i32;
}

fn peak_mb() -> f64 {
    let mut u: libc_rusage = unsafe { std::mem::zeroed() };
    unsafe { getrusage(0, &mut u) };
    // ru_maxrss: bytes on macOS, KiB on Linux.
    let max = u.rest[0] as f64;
    if cfg!(target_os = "macos") { max / 1e6 } else { max / 1e3 }
}

struct Opts {
    pos: Vec<String>,
    include: Vec<String>,
    ext: Vec<String>,
    min: Option<u32>,
    min_lines: u32,
    cells: Cells,
    out: Option<String>,
}

fn opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { pos: Vec::new(), include: Vec::new(), ext: Vec::new(), min: None, min_lines: 0, cells: Cells::Row, out: None };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--include" => o.include.push(val()?),
            "--ext" => o.ext.push(val()?),
            "--min" => o.min = Some(val()?.parse().map_err(|e| format!("--min: {e}"))?),
            "--min-lines" => o.min_lines = val()?.parse().map_err(|e| format!("--min-lines: {e}"))?,
            "--cells" => {
                o.cells = match val()?.as_str() {
                    "row" => Cells::Row,
                    "code-slot" => Cells::CodeSlot,
                    c => return Err(format!("--cells {c}: row or code-slot")),
                }
            }
            "--out" => o.out = Some(val()?),
            _ => o.pos.push(a.clone()),
        }
    }
    Ok(o)
}

struct Row {
    path: String,
    first: u32,
    last: u32,
    units: u32,
    others: Vec<String>,
}

fn write(rows: &mut [Row], out: Option<&str>) -> Result<(), String> {
    rows.sort_by(|a, b| (&a.path, a.first).cmp(&(&b.path, b.first)));
    let mut w: Box<dyn Write> = match out {
        Some(p) => Box::new(std::io::BufWriter::new(std::fs::File::create(p).map_err(|e| format!("{p}: {e}"))?)),
        None => Box::new(std::io::stdout().lock()),
    };
    for r in rows.iter() {
        writeln!(w, "{}\t{}\t{}\t{}\t{}", r.path, r.first, r.last, r.units, r.others.join(", ")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn lines_command(o: Opts) -> Result<(), String> {
    let [root] = &o.pos[..] else { return Err("usage: dupsa lines <root> [--include <prefix>]... [--min 30]".into()) };
    let min = o.min.unwrap_or(30);
    let (t0, c0) = (Instant::now(), cpu());
    let docs = lines::read(Path::new(root), &o.include, &o.ext);
    let read = (t0.elapsed().as_secs_f64(), cpu() - c0);
    let symbols: usize = docs.ids.iter().map(Vec::len).sum();
    let (t1, c1) = (Instant::now(), cpu());
    let sa = MultiSuffixArray::new(&docs);
    let built = (t1.elapsed().as_secs_f64(), cpu() - c1);
    let blocks = report::blocks(&sa, min);
    let place = |d: u32, j: u32| format!("{}:{}", docs.paths[d as usize], docs.lines[d as usize][j as usize]);
    let mut rows: Vec<Row> = blocks
        .iter()
        .map(|b| {
            let (d, ls) = (b.doc as usize, &docs.lines[b.doc as usize]);
            let mut others: Vec<String> = b.others.iter().map(|&(e, j)| place(e, j)).collect();
            others.sort();
            Row { path: docs.paths[d].clone(), first: ls[b.first as usize], last: ls[b.end as usize - 1], units: b.end - b.first, others }
        })
        .collect();
    write(&mut rows, o.out.as_deref())?;
    eprintln!(
        "lines: {} files, {} counted lines, {} distinct; read {:.2} s ({:.2} s CPU), suffix array + LCP {:.2} s ({:.2} s CPU); {} blocks at {min} lines; peak {:.0} MB",
        docs.count(), symbols, docs.distinct, read.0, read.1, built.0, built.1, rows.len(), peak_mb()
    );
    Ok(())
}

fn nodes_command(o: Opts) -> Result<(), String> {
    let [cap] = &o.pos[..] else { return Err("usage: dupsa nodes <capture> [--min 200] [--min-lines 0] [--cells row|code-slot]".into()) };
    let min = o.min.unwrap_or(200);
    let mut capture = Capture::load(Path::new(cap))?;
    // Jeff's scope: the include prefixes, his excludes, and the extensions asked for.
    capture.files.retain(|f| {
        (o.include.is_empty() || o.include.iter().any(|i| f.path.starts_with(i.as_str())))
            && !lines::excluded(&f.path)
            && (o.ext.is_empty() || f.path.rsplit_once('.').is_some_and(|(_, e)| o.ext.iter().any(|x| x == e)))
    });
    let mut tags: Vec<u8> = capture.files.iter().map(|f| f.tag).collect();
    tags.sort_unstable();
    tags.dedup();
    let mut rows = Vec::new();
    for tag in tags {
        let (t0, c0) = (Instant::now(), cpu());
        let docs = match NodeDocs::new(&capture, tag, o.cells) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("tag {tag}: {e}");
                continue;
            }
        };
        let ranked = (t0.elapsed().as_secs_f64(), cpu() - c0);
        let nodes: usize = (0..docs.count()).map(|d| docs.doc_len(d)).sum();
        let (t1, c1) = (Instant::now(), cpu());
        let sa = MultiSuffixArray::new(&docs);
        let built = (t1.elapsed().as_secs_f64(), cpu() - c1);
        let blocks = report::blocks(&sa, min);
        let before = rows.len();
        for b in &blocks {
            let d = b.doc as usize;
            let (start, stop) = docs.span(d, b.first as usize, b.end as usize);
            let (first, last) = (docs.line(d, start), docs.line(d, stop.saturating_sub(1).max(start)));
            if last + 1 - first < o.min_lines {
                continue;
            }
            let mut others: Vec<String> = b
                .others
                .iter()
                .map(|&(e, j)| {
                    let e = e as usize;
                    let (s, _) = docs.span(e, j as usize, j as usize + 1);
                    format!("{}:{}", docs.path(e), docs.line(e, s))
                })
                .collect();
            others.sort();
            rows.push(Row { path: docs.path(d).to_owned(), first, last, units: b.end - b.first, others });
        }
        eprintln!(
            "tag {tag}: {} files, {} nodes, {} distinct rows; ranked {:.2} s ({:.2} s CPU), suffix array + LCP {:.2} s ({:.2} s CPU); {} blocks at {min} nodes, {} kept",
            docs.count(), nodes, docs.distinct(), ranked.0, ranked.1, built.0, built.1, blocks.len(), rows.len() - before
        );
    }
    write(&mut rows, o.out.as_deref())?;
    eprintln!("nodes: {} blocks; peak {:.0} MB", rows.len(), peak_mb());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("lines") => opts(&args[1..]).and_then(lines_command),
        Some("nodes") => opts(&args[1..]).and_then(nodes_command),
        _ => Err("usage: dupsa lines <root> … | dupsa nodes <capture> …".into()),
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
