//! The page of a profile's results (`join-lab page`), in the form of the join program's shape profile:
//!
//! - the run table: each engine's time, CPU, instructions and cycles over the replayed launches, and against the
//!   program's over the same launches;
//! - the shapes by work on each engine (a launch's time shared among its shapes by the rows into them), each with its
//!   class or the part it lacks (the catalogue's, `--shapes`);
//! - a card a shape: its stages as chips in their inputs' colours, each stage's rows in and out, its ms by engine, the
//!   joins it ran for, the indexes it probes;
//! - the program's time by item kind (`join_run_timed`), its cycles in that share, and the parts by the work of the
//!   shapes that need them (no class runs yet: the work queue the parts' library takes);
//! - a flame graph for each engine sampled (`profile --sample`'s folded stacks);
//! - the launches, slowest first.

use crate::catalogue::Entry;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A row of `results.tsv` (`Profile::tsv`).
struct Row {
    launch: String,
    engine: String,
    dispatches: u64,
    groups: u64,
    repeat: u64,
    best: f64,
    median: f64,
    cpu: f64,
    instructions: u64,
    cycles: u64,
    bytes: u64,
    same: bool,
    shapes: Vec<(String, String, u64)>,
    /// Its items' time by kind: name, nanoseconds, items.
    kinds: Vec<(String, u64, u64)>,
}

fn rows(tsv: &str) -> Result<Vec<Row>, String> {
    let mut out = Vec::new();
    for (n, line) in tsv.lines().enumerate().skip(1).filter(|(_, l)| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 14 {
            return Err(format!("line {}: {} fields, not 14", n + 1, f.len()));
        }
        let num = |i: usize| f[i].parse::<f64>().map_err(|e| format!("line {}: {}: {e}", n + 1, f[i]));
        let shapes = f[12].split(',').filter(|s| !s.is_empty()).map(|s| {
            let p: Vec<&str> = s.splitn(3, '|').collect();
            (p[0].to_owned(), p.get(1).copied().unwrap_or("").to_owned(), p.get(2).and_then(|w| w.parse().ok()).unwrap_or(0))
        }).collect();
        let kinds = f[13].split(',').filter(|s| !s.is_empty()).map(|s| {
            let p: Vec<&str> = s.splitn(3, '|').collect();
            (p[0].to_owned(), p.get(1).and_then(|w| w.parse().ok()).unwrap_or(0), p.get(2).and_then(|w| w.parse().ok()).unwrap_or(0))
        }).collect();
        out.push(Row { launch: f[0].into(), engine: f[1].into(), dispatches: num(2)? as u64, groups: num(3)? as u64, repeat: num(4)? as u64, best: num(5)?, median: num(6)?, cpu: num(7)?, instructions: num(8)? as u64, cycles: num(9)? as u64, bytes: num(10)? as u64, same: f[11] == "true", shapes, kinds });
    }
    Ok(out)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const STYLE: &str = r#"<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;500;600;700&display=swap">
<style>
:root{--bg:#f6f7f9;--panel:#fff;--fg:#1b2330;--muted:#5b6676;--rule:#dde2ea;--soft:#eef1f5;--accent:#3d6fb6;--good:#3f8a4e;--bad:#b4483a;--warn:#c07a1b;
--l-view:#e4eefb;--l-rel:#f6ead9;--l-cmp:#eceef1;--l-fn:#e6f3e6;--l-neg:#f7e1de;--l-axis:#efe6f7;
--f1:#e9b872;--f2:#e39a5b;--f3:#d9785a;--f4:#efcf8a;
--sans:"IBM Plex Sans",system-ui,-apple-system,"Segoe UI",sans-serif;--mono:"IBM Plex Mono",ui-monospace,"SF Mono",Menlo,monospace}
@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){--bg:#12161c;--panel:#1a2029;--fg:#e3e7ee;--muted:#98a2b1;--rule:#2c3440;--soft:#222a35;--accent:#6b9be0;--good:#6cbf7b;--bad:#e07a6b;--warn:#e3a24a;
--l-view:#1f3047;--l-rel:#3a2f1f;--l-cmp:#262c35;--l-fn:#1f3524;--l-neg:#3f2523;--l-axis:#2e2440;--f1:#8a6a33;--f2:#8c5733;--f3:#83432f;--f4:#7d6a3c;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#12161c;--panel:#1a2029;--fg:#e3e7ee;--muted:#98a2b1;--rule:#2c3440;--soft:#222a35;--accent:#6b9be0;--good:#6cbf7b;--bad:#e07a6b;--warn:#e3a24a;
--l-view:#1f3047;--l-rel:#3a2f1f;--l-cmp:#262c35;--l-fn:#1f3524;--l-neg:#3f2523;--l-axis:#2e2440;--f1:#8a6a33;--f2:#8c5733;--f3:#83432f;--f4:#7d6a3c;color-scheme:dark}
body{background:var(--bg);color:var(--fg);font-family:var(--sans);font-size:15px;line-height:1.55}
body::before{content:"";position:fixed;inset:0 0 auto;height:env(safe-area-inset-top,0px);z-index:10;background:var(--bg)}
.wrap{max-width:1080px;margin:0 auto;padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));padding-block:28px 72px;display:flex;flex-direction:column;gap:36px}
h1{font-size:1.8rem;line-height:1.2;margin:0;text-wrap:balance}h2{font-size:1.25rem;margin:0;text-wrap:balance}h3{font-size:1rem;margin:0}p{margin:0;max-width:74ch}
.eyebrow{font-family:var(--mono);font-size:.74rem;letter-spacing:.08em;text-transform:uppercase;color:var(--muted)}.muted{color:var(--muted)}
section{display:flex;flex-direction:column;gap:12px;min-width:0}.scroll{overflow-x:auto;min-width:0}
table{border-collapse:collapse;width:100%;font-size:.86rem}th,td{padding:6px 9px;border-bottom:1px solid var(--rule);text-align:left;vertical-align:top}
th{font-weight:600;color:var(--muted);font-size:.74rem;text-transform:uppercase;letter-spacing:.05em}
td.n{font-family:var(--mono);font-variant-numeric:tabular-nums;text-align:right;white-space:nowrap}code{font-family:var(--mono);font-size:.84em}
.bar{height:10px;background:var(--accent);border-radius:2px;min-width:1px}.ok{color:var(--good)}.bad{color:var(--bad)}.warn{color:var(--warn)}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(min(100%,480px),1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--rule);border-radius:8px;padding:14px 16px;display:flex;flex-direction:column;gap:10px;min-width:0}
.steps{display:flex;flex-wrap:wrap;gap:5px;align-items:stretch}
.step{border:1px solid var(--rule);border-radius:5px;padding:4px 7px;font-family:var(--mono);font-size:.72rem;display:flex;flex-direction:column;gap:1px;min-width:0;max-width:220px}
.step .io{color:var(--muted)}
.l-view{background:var(--l-view)}.l-rel{background:var(--l-rel)}.l-cmp{background:var(--l-cmp)}.l-fn{background:var(--l-fn)}.l-neg{background:var(--l-neg)}.l-axis{background:var(--l-axis)}
.legend{display:flex;flex-wrap:wrap;gap:6px 14px;font-size:.8rem;color:var(--muted)}.legend span{display:inline-flex;align-items:center;gap:6px}
.sw{width:11px;height:11px;border-radius:2px;display:inline-block;border:1px solid var(--rule)}
.pill{display:inline-block;font-family:var(--mono);font-size:.72rem;padding:1px 8px;border-radius:999px;background:var(--soft);color:var(--muted)}
.flame{overflow-x:auto;min-width:0;border:1px solid var(--rule);border-radius:6px;background:var(--panel)}
.flame svg{display:block;width:100%;min-width:640px;height:auto}.flame text{font-family:var(--mono);font-size:10px;fill:var(--fg);pointer-events:none}
.flame rect{stroke:var(--panel);stroke-width:.5}
</style>"#;

/// A stage's chip class (its input's colour, as the design's legend has them).
fn chip(s: &crate::catalogue::Stage) -> &'static str {
    match (s.kind.as_str(), s.mode.as_str(), s.input_kind()) {
        (_, "anti", _) => "l-neg",
        ("filter", ..) => "l-cmp",
        ("map" | "rowtext" | "names" | "textop" | "verdict", ..) => "l-fn",
        (_, _, "view") => "l-view",
        (_, _, "axis") => "l-axis",
        _ => "l-rel",
    }
}

/// A stage's chip text: its kind and mode, its input.
fn chip_text(s: &crate::catalogue::Stage) -> String {
    let input = match s.input_kind() {
        "view" => rerail_join_shapes::parts::view_name(s.input_field("view").and_then(|v| v.parse().ok()).unwrap_or(0)).to_owned(),
        "derived" => format!("R/{}", s.input_field("arity").unwrap_or("?")),
        "none" => String::new(),
        k => k.to_owned(),
    };
    let mode = if s.mode == "join" || s.mode == s.kind { String::new() } else { format!(" {}", s.mode) };
    format!("{}{mode} {input}{}", s.kind, if s.keyed { " by key" } else { "" })
}

/// The page of `tsv` (a profile's results), with the catalogue's shapes where given, and each engine's folded stacks
/// (`folded`, by engine name) where its profile sampled them.
pub fn write(tsv: &str, catalogue: Option<&BTreeMap<String, Entry>>, folded: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let rows = rows(tsv)?;
    let peak: Option<u64> = tsv.lines().find_map(|l| l.strip_prefix("# peak_rss ")).and_then(|v| v.trim().parse().ok());
    let mut h = String::new();
    let _ = write!(h, "<title>Join Lab Replays</title>\n<meta name=\"claude:viewport-fit-sides\" content=\"cover\">\n{STYLE}\n<div class=\"wrap\">");
    let engines: Vec<String> = rows.iter().map(|r| r.engine.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let launches: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.launch.as_str()).collect();
    let same = rows.iter().filter(|r| r.same).count();
    let _ = write!(h, "<header style=\"display:flex;flex-direction:column;gap:10px\"><div class=\"eyebrow\">rerail · join lab · {} launches · {}</div><h1>Captured join launches, replayed apart from the engine</h1><p>Each launch the engine captured (<code>RERAIL_JOIN_CAPTURE</code>) is placed again in the lab, its addresses bound where its regions lie now, and run warm on each engine: <code>program</code> is the join program's Rust as the CPU device runs it, <code>metal</code> its kernel on the device, <code>class</code> the shapes' generated classes. A replay is <b>same</b> when every region it writes is byte for byte what the engine's launch left. {same} of {} replays are.</p></header>", launches.len(), esc(&engines.join(", ")), rows.len());

    // The run table, by engine, and each against the program over the launches both ran.
    let program: BTreeMap<&str, &Row> = rows.iter().filter(|r| r.engine == "program").map(|r| (r.launch.as_str(), r)).collect();
    let _ = write!(h, "<section><h2>The run</h2><p class=muted>Times summed over the launches. A <code>metal</code> time is the device's (its command buffer's GPU time); its CPU is the host thread's, which waits. Instructions and cycles are the replaying thread's own counters (the process's on macOS, which runs it alone). Against the program: the engine's best time over the program's on the launches both ran.</p><div class=\"scroll\"><table><thead><tr><th>Engine</th><th>Launches</th><th>Same</th><th>Best ms</th><th>Median ms</th><th>CPU ms</th><th>M instr</th><th>M cycles</th><th>IPC</th><th>Groups</th><th>vs program</th></tr></thead><tbody>");
    for e in &engines {
        let of: Vec<&Row> = rows.iter().filter(|r| &r.engine == e).collect();
        let (ins, cyc) = (of.iter().map(|r| r.instructions).sum::<u64>(), of.iter().map(|r| r.cycles).sum::<u64>());
        let both: Vec<(&Row, &Row)> = of.iter().filter_map(|r| program.get(r.launch.as_str()).map(|p| (*r, *p))).collect();
        let (eb, pb) = (both.iter().map(|(r, _)| r.best).sum::<f64>(), both.iter().map(|(_, p)| p.best).sum::<f64>());
        let vs = if e == "program" { "1.00×".to_owned() } else if pb > 0.0 && !both.is_empty() { format!("{:.2}× ({} launches)", eb / pb, both.len()) } else { "—".into() };
        let _ = write!(h, "<tr><td><code>{}</code></td><td class=n>{}</td><td class=n>{}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{vs}</td></tr>", esc(e), of.len(), of.iter().filter(|r| r.same).count(), of.iter().map(|r| r.best).sum::<f64>(), of.iter().map(|r| r.median).sum::<f64>(), of.iter().map(|r| r.cpu).sum::<f64>(), ins as f64 / 1e6, cyc as f64 / 1e6, if cyc > 0 { format!("{:.2}", ins as f64 / cyc as f64) } else { "—".into() }, of.iter().map(|r| r.groups).sum::<u64>());
    }
    let _ = write!(h, "</tbody></table></div>");
    if let Some(p) = peak {
        let _ = write!(h, "<p class=muted>The profile's process peaked at {:.0} MB resident (every engine's replays in one process).</p>", p as f64 / 1048576.0);
    }
    let _ = write!(h, "</section>");

    // Shapes by work on each engine: a launch's best time shared among its shapes by the rows into them.
    let mut by_shape: BTreeMap<String, (BTreeMap<String, f64>, u64, Vec<String>, usize)> = BTreeMap::new();
    for r in &rows {
        let total: u64 = r.shapes.iter().map(|s| s.2).sum::<u64>().max(1);
        for (s, rule, w) in &r.shapes {
            let e = by_shape.entry(s.clone()).or_default();
            *e.0.entry(r.engine.clone()).or_default() += r.best * *w as f64 / total as f64;
            if r.engine == engines[0] {
                e.1 += w;
                e.3 += 1;
            }
            if !e.2.contains(rule) {
                e.2.push(rule.clone());
            }
        }
    }
    let lead = if engines.iter().any(|e| e == "program") { "program".to_owned() } else { engines[0].clone() };
    let mut shapes: Vec<_> = by_shape.iter().collect();
    shapes.sort_by(|a, b| b.1.0.get(&lead).copied().unwrap_or(0.0).total_cmp(&a.1.0.get(&lead).copied().unwrap_or(0.0)));
    let top = shapes.iter().flat_map(|s| s.1.0.values()).fold(1e-9f64, |a, &b| a.max(b));
    let class_of = |name: &str| -> String {
        match catalogue.and_then(|c| c.get(name)) {
            Some(e) => match e.class() {
                Ok(c) => format!("<span class=ok>{}</span>", esc(c)),
                Err(part) => format!("<span class=warn>missing <code>{}</code></span>", esc(&part)),
            },
            None => "<span class=muted>not in the catalogue</span>".into(),
        }
    };
    let _ = write!(h, "<section><h2>Shapes by work</h2><p class=muted>Each launch's best time shared among the shapes its runs encoded, by the rows into their inputs, on each engine; the forty largest on <code>{}</code>. A shape with no class names the first part its class lacks.</p><div class=\"scroll\"><table><thead><tr><th>Shape</th><th>Engine</th><th>ms</th><th style=\"width:28%\"></th><th>Rows in</th><th>Class</th></tr></thead><tbody>", esc(&lead));
    for (s, (ms, work, _, _)) in shapes.iter().take(40) {
        for (k, e) in engines.iter().enumerate() {
            let m = ms.get(e).copied().unwrap_or(0.0);
            let lead_cells = if k == 0 { format!("<td rowspan={}><code>{}</code></td>", engines.len(), esc(s)) } else { String::new() };
            let tail = if k == 0 { format!("<td class=n rowspan={n}>{work}</td><td rowspan={n}>{}</td>", class_of(s), n = engines.len()) } else { String::new() };
            let _ = write!(h, "<tr>{lead_cells}<td><code>{}</code></td><td class=n>{m:.3}</td><td><div class=bar style=\"width:{:.1}%\"></div></td>{tail}</tr>", esc(e), 100.0 * m / top);
        }
    }
    let _ = write!(h, "</tbody></table></div></section>");

    // A card a shape, the largest first.
    if let Some(cat) = catalogue {
        let _ = write!(h, "<section><h2>The shapes</h2><div class=legend><span><i class=\"sw l-rel\"></i>derived relation</span><span><i class=\"sw l-view\"></i>view of the tape</span><span><i class=\"sw l-axis\"></i>tape axis</span><span><i class=\"sw l-fn\"></i>text function or computed column</span><span><i class=\"sw l-cmp\"></i>comparison</span><span><i class=\"sw l-neg\"></i>negation</span></div><p class=muted>Each shape's stages as the program encodes them, with the rows into and out of each over the catalogue's whole run (<code>shapes.yaml</code>), its replayed ms by engine, the joins it ran for, and the indexes it probes (made by the level's build ahead).</p><div class=cards>");
        for (s, (ms, work, rules, launches)) in shapes.iter().take(24) {
            let Some(e) = cat.get(s.as_str()) else { continue };
            let _ = write!(h, "<div class=card><div style=\"display:flex;flex-wrap:wrap;gap:8px;align-items:baseline;justify-content:space-between\"><h3><code>{}</code></h3><span class=pill>{} runs · {} captured</span></div><div class=steps role=list>", esc(s), e.runs, launches);
            for (i, st) in e.stages.iter().enumerate() {
                let io = e.rows.get(i).map_or(String::new(), |[a, b]| format!("{a} → {b}"));
                let _ = write!(h, "<div class=\"step {}\" role=listitem><span>{}</span><span class=io>{io}</span></div>", chip(st), esc(&chip_text(st)));
            }
            let _ = write!(h, "<div class=\"step l-rel\" role=listitem><span>head {}</span><span class=io>emit</span></div></div>", esc(&e.head));
            let by: Vec<String> = engines.iter().map(|en| format!("<code>{}</code> {:.3} ms", esc(en), ms.get(en).copied().unwrap_or(0.0))).collect();
            let _ = write!(h, "<p style=\"font-size:.86rem\">{} · {} rows into it replayed · {}</p>", by.join(" · "), work, class_of(s));
            let mut heads: Vec<String> = e.instances.iter().map(|(hd, n)| format!("{} ×{n}", hd.trim_start_matches("rule "))).collect();
            heads.truncate(6);
            let captured: Vec<String> = rules.iter().map(|r| r.trim_start_matches("rule ").to_owned()).collect();
            let indexes: Vec<String> = e.stages.iter().enumerate().filter(|(_, st)| st.input_field("indexed") == Some("true")).map(|(i, st)| format!("stage {} ({})", i + 1, chip_text(st))).collect();
            let _ = write!(h, "<p class=muted style=\"font-size:.82rem\">Joins: <code>{}</code>{}. Captured: <code>{}</code>. Indexes probed: {}. Parts: <code>{}</code>.</p></div>", esc(&heads.join(", ")), if e.instances.len() > 6 { format!(" and {} more", e.instances.len() - 6) } else { String::new() }, esc(&captured.join(", ")), if indexes.is_empty() { "none".to_owned() } else { esc(&indexes.join(", ")) }, esc(&e.parts().join(" ")));
        }
        let _ = write!(h, "</div></section>");
    }

    // The program's time by item kind, and its cycles in that share.
    let mut kinds: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.engine == "program") {
        for (k, ns, n) in &r.kinds {
            let e = kinds.entry(k.as_str()).or_default();
            e.0 += ns;
            e.1 += n;
        }
    }
    if !kinds.is_empty() {
        let total: u64 = kinds.values().map(|v| v.0).sum::<u64>().max(1);
        let cycles: u64 = rows.iter().filter(|r| r.engine == "program").map(|r| r.cycles).sum();
        let mut ks: Vec<_> = kinds.into_iter().collect();
        ks.sort_by_key(|k| std::cmp::Reverse(k.1.0));
        let _ = write!(h, "<section><h2>The program by item kind</h2><p class=muted>Each item's time in a run of its own (<code>join_run_timed</code>: a clock read before and after each item), summed over the launches; its cycles are the median runs' in that share.</p><div class=\"scroll\"><table><thead><tr><th>Item kind</th><th>ms</th><th>Share</th><th style=\"width:30%\"></th><th>M cycles</th><th>Items</th><th>µs an item</th></tr></thead><tbody>");
        for (k, (ns, n)) in &ks {
            let share = *ns as f64 / total as f64;
            let _ = write!(h, "<tr><td><code>{}</code></td><td class=n>{:.3}</td><td class=n>{:.1}%</td><td><div class=bar style=\"width:{:.1}%\"></div></td><td class=n>{:.2}</td><td class=n>{n}</td><td class=n>{:.2}</td></tr>", esc(k), *ns as f64 / 1e6, 100.0 * share, 100.0 * *ns as f64 / ks[0].1.0.max(1) as f64, share * cycles as f64 / 1e6, *ns as f64 / 1e3 / (*n).max(1) as f64);
        }
        let _ = write!(h, "</tbody></table></div></section>");
    }

    // The parts by the work of the shapes that need them.
    if let Some(cat) = catalogue {
        let mut parts: BTreeMap<String, (f64, u64, usize)> = BTreeMap::new();
        for (s, (ms, work, _, _)) in &shapes {
            let Some(e) = cat.get(s.as_str()) else { continue };
            for p in e.parts() {
                let v = parts.entry(p).or_default();
                v.0 += ms.get(&lead).copied().unwrap_or(0.0);
                v.1 += work;
                v.2 += 1;
            }
        }
        let mut ps: Vec<_> = parts.into_iter().collect();
        ps.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        let top = ps.first().map_or(1e-9, |p| p.1.0).max(1e-9);
        let built = rerail_join_shapes::parts::BUILT.len();
        let _ = write!(h, "<section><h2>By part</h2><p class=muted>The parts the replayed shapes' classes are made of, by the <code>{}</code> time of the shapes that need each (a shape's time counted once for every part it needs). {}</p><div class=\"scroll\"><table><thead><tr><th>Part</th><th>Built</th><th>ms of its shapes</th><th style=\"width:30%\"></th><th>Shapes</th><th>Rows in</th></tr></thead><tbody>", esc(&lead), if built == 0 { "No part is built yet, so no class runs and no cycles are spent in one: this is the order the parts' library takes them in.".to_owned() } else { format!("{built} parts built.") });
        for (p, (ms, work, n)) in &ps {
            let b = rerail_join_shapes::parts::built(p);
            let _ = write!(h, "<tr><td><code>{}</code></td><td class=\"{}\">{}</td><td class=n>{ms:.3}</td><td><div class=bar style=\"width:{:.1}%\"></div></td><td class=n>{n}</td><td class=n>{work}</td></tr>", esc(p), if b { "ok" } else { "warn" }, if b { "yes" } else { "no" }, 100.0 * ms / top);
        }
        let _ = write!(h, "</tbody></table></div></section>");
    }

    // A flame graph for each engine sampled.
    let mut flames = String::new();
    for e in &engines {
        if let Some(f) = folded(e).filter(|f| !f.trim().is_empty()) {
            let _ = write!(flames, "<h3><code>{}</code></h3>{}", esc(e), flame(&f));
        }
    }
    if !flames.is_empty() {
        let _ = write!(h, "<section><h2>Flame graphs</h2><p class=muted>The replaying thread's stacks, sampled each millisecond of its measured runs (<code>profile --sample</code>), root at the top; a frame's width is its samples. On <code>metal</code> they are the host thread's, which encodes and waits: the device's work has no stacks.</p>{flames}</section>");
    }

    // The launches, slowest first.
    let mut by_time: Vec<&Row> = rows.iter().collect();
    by_time.sort_by(|a, b| b.best.total_cmp(&a.best));
    let _ = write!(h, "<section><h2>Launches</h2><div class=\"scroll\"><table><thead><tr><th>Launch</th><th>Engine</th><th>Best ms</th><th>Median ms</th><th>CPU ms</th><th>M instr</th><th>M cycles</th><th>Dispatches</th><th>Groups</th><th>MB</th><th>Runs</th><th></th><th>Shapes</th></tr></thead><tbody>");
    for r in by_time {
        let shapes: Vec<&str> = r.shapes.iter().map(|s| s.0.as_str()).collect();
        let _ = write!(h, "<tr><td><code>{}</code></td><td><code>{}</code></td><td class=n>{:.3}</td><td class=n>{:.3}</td><td class=n>{:.3}</td><td class=n>{:.2}</td><td class=n>{:.2}</td><td class=n>{}</td><td class=n>{}</td><td class=n>{:.1}</td><td class=n>{}</td><td class=\"{}\">{}</td><td><code>{}</code></td></tr>", esc(&r.launch), esc(&r.engine), r.best, r.median, r.cpu, r.instructions as f64 / 1e6, r.cycles as f64 / 1e6, r.dispatches, r.groups, r.bytes as f64 / 1048576.0, r.repeat, if r.same { "ok" } else { "bad" }, if r.same { "same" } else { "differs" }, esc(&shapes.join(" ")));
    }
    let _ = write!(h, "</tbody></table></div></section></div>");
    Ok(h)
}

/// A frame of a flame graph: its samples, its children by name.
#[derive(Default)]
struct Frame {
    n: u64,
    kids: BTreeMap<String, Frame>,
}

/// `folded` (`a;b;c n` lines) as an icicle SVG: the root at the top, each frame as wide as its samples, frames under
/// a thousandth of the whole left out.
fn flame(folded: &str) -> String {
    let mut root = Frame::default();
    for line in folded.lines() {
        let Some((path, n)) = line.rsplit_once(' ') else { continue };
        let Ok(n) = n.parse::<u64>() else { continue };
        root.n += n;
        let mut f = &mut root;
        for name in path.split(';').filter(|s| !s.is_empty()) {
            f = f.kids.entry(name.to_owned()).or_default();
            f.n += n;
        }
    }
    let (width, row) = (1200.0f64, 15.0f64);
    let mut rects = String::new();
    let mut depth = 0usize;
    fn walk(f: &Frame, x: f64, d: usize, scale: f64, total: u64, row: f64, out: &mut String, depth: &mut usize) {
        let mut at = x;
        for (name, k) in &f.kids {
            let w = k.n as f64 * scale;
            if (k.n as f64) < total as f64 / 1000.0 {
                at += w;
                continue;
            }
            *depth = (*depth).max(d + 1);
            let colour = ["--f1", "--f2", "--f3", "--f4"][name.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) % 4];
            let short = name.rsplit("::").take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("::");
            let fits = (w / 6.2) as usize;
            let label = if fits >= 4 { if short.chars().count() > fits { format!("{}…", short.chars().take(fits.saturating_sub(1)).collect::<String>()) } else { short.clone() } } else { String::new() };
            let _ = write!(out, "<g><title>{} — {} samples ({:.1}%)</title><rect x=\"{at:.1}\" y=\"{:.1}\" width=\"{w:.1}\" height=\"{:.1}\" fill=\"var({colour})\"/>", esc(name), k.n, 100.0 * k.n as f64 / total as f64, d as f64 * row, row - 1.0);
            if !label.is_empty() {
                let _ = write!(out, "<text x=\"{:.1}\" y=\"{:.1}\">{}</text>", at + 3.0, d as f64 * row + row - 4.0, esc(&label));
            }
            let _ = write!(out, "</g>");
            walk(k, at, d + 1, scale, total, row, out, depth);
            at += w;
        }
    }
    let scale = width / root.n.max(1) as f64;
    walk(&root, 0.0, 0, scale, root.n, row, &mut rects, &mut depth);
    format!("<div class=flame><svg viewBox=\"0 0 {width} {:.0}\" role=img aria-label=\"flame graph, {} samples\">{rects}</svg></div><p class=muted style=\"font-size:.8rem\">{} samples.</p>", depth as f64 * row, root.n, root.n)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_flame_graph_has_a_frame_for_each_path_in_its_samples_share() {
        let svg = super::flame("main;run;item 3\nmain;run;scan 1\nmain;load 4\n");
        assert_eq!(svg.matches("<rect").count(), 5, "{svg}");
        assert!(svg.contains("main — 8 samples (100.0%)") && svg.contains("item — 3 samples (37.5%)"), "{svg}");
    }

    #[test]
    fn a_results_row_reads_its_kinds() {
        let tsv = format!("{}\nw1\tprogram\t3\t9\t5\t1.0\t1.2\t1.1\t100\t50\t4096\ttrue\ts-1|rule 1|7\tstage_count|1500|3,size|20|2\n# peak_rss 1048576\n", crate::Profile::header());
        let rows = super::rows(&tsv).unwrap();
        assert_eq!(rows[0].kinds, vec![("stage_count".to_owned(), 1500, 3), ("size".to_owned(), 20, 2)]);
        let page = super::write(&tsv, None, &|_| None).unwrap();
        assert!(page.contains("The program by item kind") && page.contains("peaked at 1 MB resident"), "{page}");
    }
}
