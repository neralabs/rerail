//! ```text
//! dupsa lines <root> [--include <prefix>]... [--ext <ext>]... [--min 30] [--out <file.tsv>]
//!     Jeff's rule on a suffix array: blocks of at least `min` counted lines repeated anywhere in the scope.
//! dupsa tlines <capture> [--key text|shape] [--include <prefix>]... [--ext <ext>]... [--min 30] [--out <file.tsv>]
//!     The hybrid: Jeff's counted lines and blocks, each line keyed by the tape leaves that start on it.
//! dupsa nodes <capture> [--min 200] [--min-lines 0] [--cells row|code-slot|indent|local] [--out <file.tsv>]
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

/// Instructions this process has retired (macOS's `proc_pid_rusage`, `ri_instructions` of `rusage_info_v4`: after
/// its 16-byte uuid, the 30th u64); 0 elsewhere. Unlike CPU time, the machine's load does not change it.
fn instr() -> u64 {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
            fn getpid() -> i32;
        }
        let mut b = [0u64; 64];
        // RUSAGE_INFO_V4 = 4.
        if unsafe { proc_pid_rusage(getpid(), 4, b.as_mut_ptr()) } == 0 {
            return b[2 + 29];
        }
    }
    0
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
    key: rerail_dupsa_lab::nodes::LineKey,
    out: Option<String>,
    build: String,
    whole_doc: bool,
    check: bool,
    compact_every: usize,
    save: Option<String>,
    load: Option<String>,
}

fn opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { pos: Vec::new(), include: Vec::new(), ext: Vec::new(), min: None, min_lines: 0, cells: Cells::Row, key: rerail_dupsa_lab::nodes::LineKey::Text, out: None, build: "doubling".into(), whole_doc: false, check: false, compact_every: 0, save: None, load: None };
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
                    "indent" => Cells::Indent,
                    "local" => Cells::Local,
                    c => return Err(format!("--cells {c}: row, code-slot, indent or local")),
                }
            }
            "--key" => {
                o.key = match val()?.as_str() {
                    "text" => rerail_dupsa_lab::nodes::LineKey::Text,
                    "shape" => rerail_dupsa_lab::nodes::LineKey::Shape,
                    k => return Err(format!("--key {k}: text or shape")),
                }
            }
            "--out" => o.out = Some(val()?),
            "--build" => o.build = val()?,
            "--whole-doc" => o.whole_doc = true,
            "--save" => o.save = Some(val()?),
            "--load" => o.load = Some(val()?),
            "--check" => o.check = true,
            "--compact-every" => o.compact_every = val()?.parse().map_err(|e| format!("--compact-every: {e}"))?,
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
    let (t0, c0) = (Instant::now(), cpu());
    let docs = lines::read(Path::new(root), &o.include, &o.ext);
    report_lines("lines", docs, (t0.elapsed().as_secs_f64(), cpu() - c0), &o)
}

/// `tlines <capture> [--key text|shape] …`: the hybrid, Jeff's counted lines keyed by their tape leaves.
fn tlines_command(o: Opts) -> Result<(), String> {
    let [cap] = &o.pos[..] else { return Err("usage: dupsa tlines <capture> [--key text|shape] [--min 30]".into()) };
    let mut capture = Capture::load(Path::new(cap))?;
    capture.files.retain(|f| {
        (o.include.is_empty() || o.include.iter().any(|i| f.path.starts_with(i.as_str())))
            && !lines::excluded(&f.path)
            && (o.ext.is_empty() || f.path.rsplit_once('.').is_some_and(|(_, e)| o.ext.iter().any(|x| x == e)))
    });
    let (t0, c0) = (Instant::now(), cpu());
    let docs = rerail_dupsa_lab::nodes::tape_lines(&capture, o.key);
    report_lines("tlines", docs, (t0.elapsed().as_secs_f64(), cpu() - c0), &o)
}

fn report_lines(what: &str, docs: lines::LineDocs, read: (f64, f64), o: &Opts) -> Result<(), String> {
    let min = o.min.unwrap_or(30);
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
        "{what}: {} files, {} counted lines, {} distinct; read {:.2} s ({:.2} s CPU), suffix array + LCP {:.2} s ({:.2} s CPU); {} blocks at {min} lines; peak {:.0} MB",
        docs.count(), symbols, docs.distinct, read.0, read.1, built.0, built.1, rows.len(), peak_mb()
    );
    Ok(())
}

fn nodes_command(o: Opts) -> Result<(), String> {
    let [cap] = &o.pos[..] else { return Err("usage: dupsa nodes <capture> [--min 200] [--min-lines 0] [--cells row|code-slot|indent|local]".into()) };
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

/// `probe <capture> <path:line> <path:line> [--cells …]`: the two places' nodes from the first that starts on each
/// line, side by side, and the first rows where their symbols differ, each cell shown.
fn probe_command(o: Opts) -> Result<(), String> {
    let [cap, a, b] = &o.pos[..] else { return Err("usage: dupsa probe <capture> <path:line> <path:line>".into()) };
    let capture = Capture::load(Path::new(cap))?;
    let place = |s: &str| -> Result<(String, u32), String> {
        let (p, l) = s.rsplit_once(':').ok_or("path:line")?;
        Ok((p.to_owned(), l.parse().map_err(|e| format!("{s}: {e}"))?))
    };
    let ((pa, la), (pb, lb)) = (place(a)?, place(b)?);
    let tag = capture.files.iter().find(|f| f.path == pa).ok_or(format!("{pa} not captured"))?.tag;
    capture.files.iter().find(|f| f.path == pb).ok_or(format!("{pb} not captured"))?;
    let docs = NodeDocs::new(&capture, tag, o.cells)?;
    let doc = |p: &str| (0..docs.count()).find(|&d| docs.path(d) == p).unwrap();
    let (da, db) = (doc(&pa), doc(&pb));
    let first = |d: usize, l: u32| (0..docs.doc_len(d)).find(|&j| docs.line(d, docs.span(d, j, j + 1).0) == l).unwrap();
    let (ja, jb) = (first(da, la), first(db, lb));
    let mut shown = 0;
    let mut same = 0;
    for k in 0..docs.doc_len(da).min(docs.doc_len(db)).saturating_sub(ja.max(jb)) {
        let (x, y) = (docs.value(da, ja + k, o.cells), docs.value(db, jb + k, o.cells));
        if x == y {
            same += 1;
            continue;
        }
        let shift = if matches!(o.cells, Cells::Indent | Cells::Local) { 80 } else { 72 };
        let cells = |v: u128| format!("code {} slot {} gap {:#x} len {}", v & 0xff, (v >> 8) & 0xffff_ffff, (v >> 40) & 0x1_ffff_ffff, v >> shift);
        let line = |d: usize, j: usize| docs.line(d, docs.span(d, j, j + 1).0);
        println!("node +{k} (after {same} equal): {}:{} {}  |  {}:{} {}", pa, line(da, ja + k), cells(x), pb, line(db, jb + k), cells(y));
        same = 0;
        shown += 1;
        if shown == 8 {
            break;
        }
    }
    Ok(())
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `explain <capture>`: every method's symbols for a small capture, as JSON, for the explorer's method pages: per
/// method, per document, each unit's source line, a label (what the symbol is made of) and its key (the interned
/// symbol, shared across documents).
fn explain_command(o: Opts) -> Result<(), String> {
    let [cap] = &o.pos[..] else { return Err("usage: dupsa explain <capture>".into()) };
    let capture = Capture::load(Path::new(cap))?;
    let mut methods = Vec::new();
    // Lines: Jeff's normalisation over the captured sources.
    {
        let mut ids = std::collections::HashMap::new();
        let mut norm = String::new();
        let mut docs = Vec::new();
        for f in &capture.files {
            let text = String::from_utf8_lossy(&capture.bytes[f.source.clone()]).into_owned();
            let mut units = Vec::new();
            for (k, raw) in text.split('\n').enumerate() {
                if lines::normalise(raw, lines::hash_comments(&f.path), &mut norm) {
                    let next = ids.len();
                    let id = *ids.entry(norm.clone()).or_insert(next);
                    units.push(format!("[{},{},{}]", k + 1, json_str(&norm), id));
                }
            }
            docs.push(units);
        }
        methods.push(("lines", docs));
    }
    // Tape tokens and shapes.
    for (name, key) in [("tlines-text", rerail_dupsa_lab::nodes::LineKey::Text), ("tlines-shape", rerail_dupsa_lab::nodes::LineKey::Shape)] {
        let ld = rerail_dupsa_lab::nodes::tape_lines(&capture, key);
        let mut docs = Vec::new();
        for (d, _) in capture.files.iter().enumerate() {
            let toks = rerail_dupsa_lab::nodes::tape_line_tokens(&capture, d);
            let units: Vec<String> = ld.ids[d]
                .iter()
                .zip(&ld.lines[d])
                .map(|(&id, &line)| {
                    let label = toks
                        .iter()
                        .find(|(l, _)| *l == line)
                        .map(|(_, ts)| {
                            ts.iter()
                                .map(|(k, t)| if key == rerail_dupsa_lab::nodes::LineKey::Text { format!("{k}:{t}") } else { format!("{k}:{}", t.len()) })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    format!("[{line},{},{id}]", json_str(&label))
                })
                .collect();
            docs.push(units);
        }
        methods.push((name, docs));
    }
    // Nodes: rows and local symbols, one language (the capture's first file's).
    let tag = capture.files.first().ok_or("an empty capture")?.tag;
    for (name, cells) in [("nodes-row", Cells::Row), ("nodes-local", Cells::Local)] {
        let nd = NodeDocs::new(&capture, tag, cells)?;
        let mut docs = Vec::new();
        for d in 0..nd.count() {
            let units: Vec<String> = (0..nd.doc_len(d))
                .map(|j| {
                    let (kind, line) = nd.kind_and_line(d, j);
                    let v = nd.value(d, j, cells);
                    let label = match cells {
                        Cells::Row => format!("{kind} | code {} slot {} gap {} len {}", v & 0xff, (v >> 8) & 0xffff_ffff, (v >> 40) & 0xffff_ffff, v >> 72),
                        _ => {
                            let gap = (v >> 40) & 0x1_ffff_ffff;
                            let gap = if gap >> 32 == 1 { format!("newline+{}", gap & 0xffff_ffff) } else { gap.to_string() };
                            format!("{kind} | code {} {} gap {gap} len {}", v & 0xff, if (v >> 8) & 1 == 1 { "leaf" } else { "inner" }, v >> 80)
                        }
                    };
                    format!("[{line},{},{}]", json_str(&label), nd.at(d, j))
                })
                .collect();
            docs.push(units);
        }
        methods.push((name, docs));
    }
    let docs_json: Vec<String> = capture
        .files
        .iter()
        .map(|f| format!("{{\"path\":{},\"source\":{}}}", json_str(&f.path), json_str(&String::from_utf8_lossy(&capture.bytes[f.source.clone()]))))
        .collect();
    let methods_json: Vec<String> = methods
        .iter()
        .map(|(m, docs)| format!("{}:[{}]", json_str(m), docs.iter().map(|u| format!("[{}]", u.join(","))).collect::<Vec<_>>().join(",")))
        .collect();
    println!("{{\"docs\":[{}],\"methods\":{{{}}}}}", docs_json.join(","), methods_json.join(","));
    Ok(())
}

/// `readcost <capture> [--include/--ext] [--cells]`: where the node suffix array's time goes: the symbol reads it
/// makes, one read's cost, and the same build over the symbols read once.
fn readcost_command(o: Opts) -> Result<(), String> {
    use rerail_dupsa_lab::multi::Docs;
    let [cap] = &o.pos[..] else { return Err("usage: dupsa readcost <capture>".into()) };
    let mut capture = Capture::load(Path::new(cap))?;
    capture.files.retain(|f| {
        (o.include.is_empty() || o.include.iter().any(|i| f.path.starts_with(i.as_str())))
            && (o.ext.is_empty() || f.path.rsplit_once('.').is_some_and(|(_, e)| o.ext.iter().any(|x| x == e)))
    });
    let tag = capture.files[0].tag;
    let docs = NodeDocs::new(&capture, tag, o.cells)?;
    struct Counting<'a, D: Docs>(&'a D, std::cell::Cell<u64>);
    impl<D: Docs> Docs for Counting<'_, D> {
        fn count(&self) -> usize { self.0.count() }
        fn doc_len(&self, d: usize) -> usize { self.0.doc_len(d) }
        fn at(&self, d: usize, j: usize) -> u32 { self.1.set(self.1.get() + 1); self.0.at(d, j) }
        fn upper(&self) -> u32 { self.0.upper() }
    }
    struct Read(Vec<Vec<u32>>, u32);
    impl Docs for Read {
        fn count(&self) -> usize { self.0.len() }
        fn doc_len(&self, d: usize) -> usize { self.0[d].len() }
        fn at(&self, d: usize, j: usize) -> u32 { self.0[d][j] }
        fn upper(&self) -> u32 { self.1 }
    }
    let n: usize = (0..docs.count()).map(|d| docs.doc_len(d)).sum();
    let c0 = cpu();
    let read = Read((0..docs.count()).map(|d| (0..docs.doc_len(d)).map(|j| docs.at(d, j)).collect()).collect(), docs.upper());
    let once = cpu() - c0;
    let c1 = cpu();
    let a = MultiSuffixArray::new(&read);
    let over_array = cpu() - c1;
    let counting = Counting(&docs, std::cell::Cell::new(0));
    let c2 = cpu();
    let b = MultiSuffixArray::new(&counting);
    let in_place = cpu() - c2;
    let reads = counting.1.get();
    assert_eq!(a.entries, b.entries);
    eprintln!("{n} symbols; every symbol read once: {once:.2} s CPU ({:.0} ns a read)", 1e9 * once / n as f64);
    eprintln!("suffix array + LCP reading the tape: {in_place:.2} s CPU, {reads} reads ({:.1} a symbol; {:.2} s of reads at one read's cost)", reads as f64 / n as f64, reads as f64 * once / n as f64);
    eprintln!("suffix array + LCP over the symbols read once: {over_array:.2} s CPU; same order");
    struct Flat(Vec<u32>);
    impl rerail_dupsa_lab::sais::Text for Flat {
        fn len(&self) -> usize { self.0.len() }
        fn at(&self, i: usize) -> u32 { self.0[i] }
    }
    let flat = Flat(read.0.iter().flat_map(|d| d.iter().map(|&x| x + 1).chain(std::iter::once(0))).collect());
    let c3 = cpu();
    let sa = rerail_dupsa_lab::sais::suffix_array(&flat, read.1 + 2);
    let c4 = cpu();
    let _ = rerail_dupsa_lab::sais::lcp(&flat, &sa);
    let c5 = cpu();
    eprintln!("over one flat array (no document lookup per read): SA-IS {:.2} s, LCP {:.2} s CPU", c4 - c3, c5 - c4);
    Ok(())
}

/// One source of document versions for `incr`: its documents, their paths, and a block's file lines.
struct Source {
    docs: &'static dyn rerail_dupsa_lab::multi::Docs,
    paths: Vec<String>,
    lines: Box<dyn Fn(u32, u32, u32) -> (u32, u32)>,
}

type Key = (u32, u32, u32, Vec<(u32, u32)>);

/// `incr lines <base root> <steps> | incr nodes <base capture> <steps>`: the findings kept up to date over a series
/// of changes. `<steps>` holds one directory per step (`0001/`, …: each changed file's new version at its path),
/// optionally `0001.deleted` (paths, one a line) and, for nodes, `0001.cap` (the step's files captured).
/// Options: `--build doubling|sais` (the base), `--whole-doc` (re-add a changed file's every window), `--check`
/// (rebuild every step and compare), `--compact-every N`.
fn incr_command(o: Opts) -> Result<(), String> {
    use rerail_dupsa_lab::multi::Docs;
    let [kind, base_at, steps] = &o.pos[..] else { return Err("usage: dupsa incr lines|nodes <base> <steps dir>".into()) };
    let steps_dir = Path::new(steps).to_path_buf();
    let mut names: Vec<String> = std::fs::read_dir(&steps_dir)
        .map_err(|e| format!("{steps}: {e}"))?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let (c0, i0) = (cpu(), instr());
    // A saved index, mapped: "DUPIX002", the method's own part (lines: the base's line documents and dictionary;
    // nodes: the dictionary's rows), the base, the blocks. Every array is read in place.
    let saved: Option<&'static [u8]> = match &o.load {
        Some(p) => Some(rerail_dupsa_lab::store::map(Path::new(p))?),
        None => None,
    };
    if let Some(b) = saved
        && !b.starts_with(b"DUPIX002")
    {
        return Err("not a DUPIX002 index".into());
    }
    let mut at = 8usize;
    // The method's part of the index, written when saving (before any step changes the dictionary).
    let mut head: Vec<u8> = Vec::new();
    let line_source = |docs: &'static dyn rerail_dupsa_lab::multi::Docs, paths: Vec<String>, at_line: Box<dyn Fn(u32, u32) -> u32>| Source {
        docs,
        paths,
        lines: Box::new(move |nd, f, e| (at_line(nd, f), at_line(nd, e - 1))),
    };
    let (base, mut load): (Source, Box<dyn FnMut(&str) -> Result<(Source, Vec<String>), String>>) = match kind.as_str() {
        "lines" => {
            let (base, mut dict) = match saved {
                Some(b) => {
                    let m: &'static lines::MappedLines = Box::leak(Box::new(lines::MappedLines::read(b, &mut at)));
                    let dict = lines::LineDict::read(b, &mut at);
                    (line_source(m, m.paths.clone(), Box::new(move |nd, u| m.lines[(m.offsets[nd as usize] + u) as usize])), dict)
                }
                None => {
                    let root = Path::new(base_at).to_path_buf();
                    let mut dict = lines::LineDict::new();
                    let docs: &'static lines::LineDocs = Box::leak(Box::new(lines::read_with(&root, &lines::files(&root, &o.include, &o.ext), &mut dict)));
                    if o.save.is_some() {
                        lines::MappedLines::write(docs, &mut head);
                        dict.write(&mut head);
                    }
                    (line_source(docs, docs.paths.clone(), Box::new(move |nd, u| docs.lines[nd as usize][u as usize])), dict)
                }
            };
            let (include, ext, dir) = (o.include.clone(), o.ext.clone(), steps_dir.clone());
            (base, Box::new(move |name: &str| {
                let root = dir.join(name);
                let docs: &'static lines::LineDocs = Box::leak(Box::new(lines::read_with(&root, &lines::files(&root, &include, &ext), &mut dict)));
                let deleted = std::fs::read_to_string(dir.join(format!("{name}.deleted"))).unwrap_or_default().lines().map(str::to_owned).collect();
                Ok((line_source(docs, docs.paths.clone(), Box::new(move |nd, u| docs.lines[nd as usize][u as usize])), deleted))
            }))
        }
        "nodes" => {
            let scope = |c: &mut Capture, o: &Opts| {
                c.files.retain(|f| {
                    (o.include.is_empty() || o.include.iter().any(|i| f.path.starts_with(i.as_str())))
                        && !lines::excluded(&f.path)
                        && (o.ext.is_empty() || f.path.rsplit_once('.').is_some_and(|(_, e)| o.ext.iter().any(|x| x == e)))
                });
            };
            let mut capture = Capture::load(Path::new(base_at))?;
            scope(&mut capture, &o);
            let tag = capture.files.first().ok_or("no files in scope")?.tag;
            capture.files.retain(|f| f.tag == tag);
            let capture: &'static Capture = Box::leak(Box::new(capture));
            let docs: &'static NodeDocs<'static> = Box::leak(Box::new(match saved {
                Some(b) => {
                    // The rows as two halves (an array of u128 would want 16-byte alignment).
                    let (lo, hi) = (rerail_dupsa_lab::store::view::<u64>(b, &mut at), rerail_dupsa_lab::store::view::<u64>(b, &mut at));
                    let values: Vec<u128> = lo.iter().zip(hi).map(|(&l, &h)| u128::from(l) | u128::from(h) << 64).collect();
                    NodeDocs::with_values(capture, tag, o.cells, &values)?
                }
                None => NodeDocs::new(capture, tag, o.cells)?,
            }));
            if o.save.is_some() {
                let v = docs.values();
                rerail_dupsa_lab::store::put(&mut head, &v.iter().map(|&x| x as u64).collect::<Vec<_>>());
                rerail_dupsa_lab::store::put(&mut head, &v.iter().map(|&x| (x >> 64) as u64).collect::<Vec<_>>());
            }
            let span_lines = |docs: &'static NodeDocs<'static>| -> Box<dyn Fn(u32, u32, u32) -> (u32, u32)> {
                Box::new(move |nd, f, e| {
                    let (start, stop) = docs.span(nd as usize, f as usize, e as usize);
                    (docs.line(nd as usize, start), docs.line(nd as usize, stop.saturating_sub(1).max(start)))
                })
            };
            let paths = (0..docs.count()).map(|d| docs.path(d).to_owned()).collect();
            let base = Source { docs, paths, lines: span_lines(docs) };
            let (dir, oc, mut prev) = (steps_dir.clone(), Opts { pos: Vec::new(), out: None, ..o_clone(&o) }, docs);
            (base, Box::new(move |name: &str| {
                let mut c = Capture::load(&dir.join(format!("{name}.cap")))?;
                scope(&mut c, &oc);
                c.files.retain(|f| f.tag == tag);
                let c: &'static Capture = Box::leak(Box::new(c));
                let docs: &'static NodeDocs<'static> = Box::leak(Box::new(NodeDocs::extending(c, tag, oc.cells, prev)?));
                prev = docs;
                let deleted = std::fs::read_to_string(dir.join(format!("{name}.deleted"))).unwrap_or_default().lines().map(str::to_owned).collect();
                let paths = (0..docs.count()).map(|d| docs.path(d).to_owned()).collect();
                Ok((Source { docs, paths, lines: span_lines(docs) }, deleted))
            }))
        }
        k => return Err(format!("incr {k}: lines or nodes")),
    };
    let read_base = cpu() - c0;
    let i_read = instr() - i0;
    let w = o.min.unwrap_or(if kind == "lines" { 30 } else { 150 });
    let mut sources = vec![base];
    let mut path_doc: std::collections::HashMap<String, u32> =
        sources[0].paths.iter().enumerate().map(|(d, p)| (p.clone(), d as u32)).collect();
    let mut paths = sources[0].paths.clone();
    let cur = rerail_dupsa_lab::incr::Current::of(sources[0].docs);
    let (c1, i1) = (cpu(), instr());
    let b = match o.build.as_str() {
        _ if saved.is_some() => rerail_dupsa_lab::incr::Base::read(saved.unwrap(), &mut at),
        "sais" => rerail_dupsa_lab::incr::Base::from_suffix_array(&cur, &MultiSuffixArray::new(&cur), w),
        "doubling" => rerail_dupsa_lab::incr::Base::doubling(&cur, w),
        x => return Err(format!("--build {x}: doubling or sais")),
    };
    let built = cpu() - c1;
    let i_built = instr() - i1;
    let (n, windows, classes, bytes) = (cur.at.len(), b.sa.len(), b.classes(), b.bytes());
    let (c2, i2) = (cpu(), instr());
    let mut ix = match saved {
        Some(bytes) => rerail_dupsa_lab::incr::Index::with_blocks(b, cur, bytes, &mut at),
        None => rerail_dupsa_lab::incr::Index::new(b, cur),
    };
    ix.whole_doc = o.whole_doc;
    let first_blocks = cpu() - c2;
    let i_blocks = instr() - i2;
    // Findings as reported (at least `min_lines` lines), sorted: for comparing a rebuild's.
    let keyed = |ix: &rerail_dupsa_lab::incr::Index, blocks: Vec<rerail_dupsa_lab::report::Block>, sources: &[Source]| -> Vec<Key> {
        let mut v: Vec<Key> = blocks
            .into_iter()
            .filter(|b| {
                let (s, nd) = ix.cur.at[b.doc as usize];
                let (f, l) = (sources[s as usize].lines)(nd, b.first, b.end);
                l + 1 - f >= o.min_lines
            })
            .map(|b| (b.doc, b.first, b.end, b.others))
            .collect();
        v.sort();
        v
    };
    // One document's findings, as reported (at least `min_lines` lines).
    let keyed_doc = |ix: &rerail_dupsa_lab::incr::Index, d: usize, sources: &[Source]| -> Vec<Key> {
        ix.doc_findings(d)
            .into_iter()
            .filter(|b| {
                let (s, nd) = ix.cur.at[b.doc as usize];
                let (f, l) = (sources[s as usize].lines)(nd, b.first, b.end);
                l + 1 - f >= o.min_lines
            })
            .map(|b| (b.doc, b.first, b.end, b.others))
            .collect()
    };
    let i3 = instr();
    // The last findings, per document: computed (a cold run), or the ones the saved index holds (a warm one).
    let mut prev: Vec<Vec<Key>> = match saved {
        Some(bytes) => {
            let v = rerail_dupsa_lab::store::view::<u32>(bytes, &mut at);
            let mut k = 1;
            (0..v[0])
                .map(|d| {
                    let n = v[k];
                    k += 1;
                    (0..n)
                        .map(|_| {
                            let (f, e, m) = (v[k], v[k + 1], v[k + 2] as usize);
                            let others = (0..m).map(|i| (v[k + 3 + 2 * i], v[k + 4 + 2 * i])).collect();
                            k += 3 + 2 * m;
                            (d, f, e, others)
                        })
                        .collect()
                })
                .collect()
        }
        None => (0..ix.blocks.len()).map(|d| keyed_doc(&ix, d, &sources)).collect(),
    };
    eprintln!("instructions: base read {i_read}, index {i_built}, blocks {i_blocks}, first findings {}", instr() - i3);
    if let Some(p) = &o.save {
        let mut out = b"DUPIX002".to_vec();
        out.extend_from_slice(&head);
        ix.base.write(&mut out);
        ix.write_blocks(&mut out);
        let mut v = vec![prev.len() as u32];
        for ks in &prev {
            v.push(ks.len() as u32);
            for (_, f, e, others) in ks {
                v.extend_from_slice(&[*f, *e, others.len() as u32]);
                for &(od, ou) in others {
                    v.extend_from_slice(&[od, ou]);
                }
            }
        }
        rerail_dupsa_lab::store::put(&mut out, &v);
        std::fs::write(p, &out).map_err(|e| format!("{p}: {e}"))?;
        eprintln!("saved {p}: {:.0} MB", out.len() as f64 / 1e6);
    }
    eprintln!(
        "{kind}: {n} files; base read {read_base:.2} s CPU, {} {built:.2} s, blocks {first_blocks:.2} s; {windows} windows of {w}, {classes} classes, {:.0} MB; {} findings",
        if saved.is_some() { "index loaded".to_owned() } else { format!("built ({})", o.build) }, bytes as f64 / 1e6, prev.iter().map(Vec::len).sum::<usize>()
    );
    println!("step\tfiles\tdead\tadded\tfresh\ttouched\taffected\trecomputed\tload_ms\tupdate_ms\tdiff_ms\tfindings\tadded\tremoved\tcopies\tmoved\trebuild_ms\tcheck\tcompact_ms\toverlay\tupdate_instr\tdiff_instr\trebuild_instr");
    for name in &names {
        let c3 = cpu();
        let i3 = instr();
        let (src, deleted) = load(name)?;
        let load_ms = 1e3 * (cpu() - c3);
        eprintln!("instructions: step {name} load {}", instr() - i3);
        ix.cur.sources.push(src.docs);
        let s = ix.cur.sources.len() as u32 - 1;
        let (c4, i4) = (cpu(), instr());
        for (nd, p) in src.paths.iter().enumerate() {
            let d = *path_doc.entry(p.clone()).or_insert_with(|| {
                paths.push(p.clone());
                paths.len() as u32 - 1
            });
            ix.set(d, (s, nd as u32));
        }
        for p in &deleted {
            if let Some(&d) = path_doc.get(p) {
                ix.set(d, (u32::MAX, 0));
            }
        }
        sources.push(src);
        let st = ix.finish();
        let update_ms = 1e3 * (cpu() - c4);
        let update_i = instr() - i4;
        // What changed in the findings. Outside the affected documents every finding is as it was, at places moved
        // by the changes: their last findings are only moved. In the affected ones they are recomputed, and compared
        // with their last ones moved.
        let (c5, i5) = (cpu(), instr());
        let map = |d: u32, u: u32, wl: u32| match ix.last_changes.get(&d) {
            Some(ch) => ch.map(u, wl),
            None => Some(u),
        };
        let move_key = |(d, f, e, x): &Key| -> Option<Key> {
            let f2 = map(*d, *f, e - f)?;
            let mut others: Vec<(u32, u32)> = x.iter().map(|&(od, ou)| map(od, ou, w).map(|u| (od, u))).collect::<Option<_>>()?;
            others.sort_unstable();
            Some((*d, f2, f2 + (e - f), others))
        };
        prev.resize(ix.blocks.len(), Vec::new());
        let affected: std::collections::HashSet<u32> = ix.affected.iter().copied().collect();
        let (mut moved, mut copies, mut removed, mut added) = (0, 0, 0, 0);
        for d in 0..prev.len() {
            let was: Vec<Option<Key>> = prev[d].iter().map(move_key).collect();
            if !affected.contains(&(d as u32)) {
                prev[d] = was.into_iter().map(|k| k.expect("a finding outside the affected documents moves")).collect();
                continue;
            }
            let now = keyed_doc(&ix, d, &sources);
            let by_span: std::collections::HashMap<(u32, u32), &Vec<(u32, u32)>> = now.iter().map(|(_, f, e, x)| ((*f, *e), x)).collect();
            let mut matched = 0;
            for (k, old) in was.iter().zip(&prev[d]) {
                match k.as_ref().and_then(|k| by_span.get(&(k.1, k.2)).map(|y| (k, y))) {
                    Some((k, y)) => {
                        matched += 1;
                        if k.3 != **y {
                            copies += 1;
                        } else if k.1 != old.1 {
                            moved += 1;
                        }
                    }
                    None => removed += 1,
                }
            }
            added += now.len() - matched;
            prev[d] = now;
        }
        let total: usize = prev.iter().map(Vec::len).sum();
        let diff_ms = 1e3 * (cpu() - c5);
        let diff_i = instr() - i5;
        ix.last_changes.clear();
        let (mut rebuild_ms, mut check, mut rebuild_i) = (0.0, "-", 0);
        if o.check {
            let (c6, i6) = (cpu(), instr());
            // The rebuild, by the build asked for: SA-IS and Jeff's blocks, or doubling and the index's blocks.
            let full = if o.build == "sais" {
                rerail_dupsa_lab::report::blocks(&MultiSuffixArray::new(&ix.cur), w)
            } else {
                rerail_dupsa_lab::incr::Index::new(rerail_dupsa_lab::incr::Base::doubling(&ix.cur, w), ix.cur.clone()).findings()
            };
            rebuild_ms = 1e3 * (cpu() - c6);
            rebuild_i = instr() - i6;
            let after: Vec<Key> = prev.iter().flatten().cloned().collect();
            check = if keyed(&ix, full, &sources) == after { "same" } else { "DIFFERENT" };
        }
        let mut compact_ms = 0.0;
        let step_no: usize = name.parse().unwrap_or(0);
        if o.compact_every > 0 && step_no % o.compact_every == 0 {
            let c7 = cpu();
            ix.compact();
            compact_ms = 1e3 * (cpu() - c7);
        }
        let (od, oa) = ix.overlay_size();
        println!(
            "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{load_ms:.1}\t{update_ms:.2}\t{diff_ms:.2}\t{}\t{added}\t{removed}\t{copies}\t{moved}\t{rebuild_ms:.0}\t{check}\t{compact_ms:.0}\t{od}+{oa}\t{update_i}\t{diff_i}\t{rebuild_i}",
            st.docs, st.dead, st.added, st.fresh, st.touched, st.affected, st.recomputed_units, total
        );
    }
    eprintln!("peak {:.0} MB", peak_mb());
    eprintln!("process: {} instructions, {:.3} s CPU", instr(), cpu());
    Ok(())
}

fn o_clone(o: &Opts) -> Opts {
    Opts {
        pos: o.pos.clone(),
        include: o.include.clone(),
        ext: o.ext.clone(),
        min: o.min,
        min_lines: o.min_lines,
        cells: o.cells,
        key: o.key,
        out: o.out.clone(),
        build: o.build.clone(),
        whole_doc: o.whole_doc,
        check: o.check,
        compact_every: o.compact_every,
        save: o.save.clone(),
        load: o.load.clone(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("lines") => opts(&args[1..]).and_then(lines_command),
        Some("nodes") => opts(&args[1..]).and_then(nodes_command),
        Some("tlines") => opts(&args[1..]).and_then(tlines_command),
        Some("incr") => opts(&args[1..]).and_then(incr_command),
        Some("readcost") => opts(&args[1..]).and_then(readcost_command),
        Some("explain") => opts(&args[1..]).and_then(explain_command),
        Some("probe") => opts(&args[1..]).and_then(probe_command),
        _ => Err("usage: dupsa lines <root> … | dupsa nodes <capture> …".into()),
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
