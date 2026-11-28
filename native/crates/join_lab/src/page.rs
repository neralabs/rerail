//! The page of a profile's results (`join-lab page`): the run table, each engine's launches by time, and the
//! shapes by the time of the launches that run them (a launch's time shared among its shapes by their work).

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
}

fn rows(tsv: &str) -> Result<Vec<Row>, String> {
    let mut out = Vec::new();
    for (n, line) in tsv.lines().enumerate().skip(1).filter(|(_, l)| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 13 {
            return Err(format!("line {}: {} fields, not 13", n + 1, f.len()));
        }
        let num = |i: usize| f[i].parse::<f64>().map_err(|e| format!("line {}: {}: {e}", n + 1, f[i]));
        let shapes = f[12].split(',').filter(|s| !s.is_empty()).map(|s| {
            let p: Vec<&str> = s.splitn(3, '|').collect();
            (p[0].to_owned(), p.get(1).copied().unwrap_or("").to_owned(), p.get(2).and_then(|w| w.parse().ok()).unwrap_or(0))
        }).collect();
        out.push(Row { launch: f[0].into(), engine: f[1].into(), dispatches: num(2)? as u64, groups: num(3)? as u64, repeat: num(4)? as u64, best: num(5)?, median: num(6)?, cpu: num(7)?, instructions: num(8)? as u64, cycles: num(9)? as u64, bytes: num(10)? as u64, same: f[11] == "true", shapes });
    }
    Ok(out)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

const STYLE: &str = r#"<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;500;600;700&display=swap">
<style>
:root{--bg:#f6f7f9;--panel:#fff;--fg:#1b2330;--muted:#5b6676;--rule:#dde2ea;--soft:#eef1f5;--accent:#3d6fb6;--good:#3f8a4e;--bad:#b4483a;
--sans:"IBM Plex Sans",system-ui,-apple-system,"Segoe UI",sans-serif;--mono:"IBM Plex Mono",ui-monospace,"SF Mono",Menlo,monospace}
@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){--bg:#12161c;--panel:#1a2029;--fg:#e3e7ee;--muted:#98a2b1;--rule:#2c3440;--soft:#222a35;--accent:#6b9be0;--good:#6cbf7b;--bad:#e07a6b;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#12161c;--panel:#1a2029;--fg:#e3e7ee;--muted:#98a2b1;--rule:#2c3440;--soft:#222a35;--accent:#6b9be0;--good:#6cbf7b;--bad:#e07a6b;color-scheme:dark}
body{background:var(--bg);color:var(--fg);font-family:var(--sans);font-size:15px;line-height:1.55}
.wrap{max-width:1040px;margin:0 auto;padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));padding-block:28px 72px;display:flex;flex-direction:column;gap:32px}
h1{font-size:1.8rem;line-height:1.2;margin:0;text-wrap:balance}h2{font-size:1.25rem;margin:0;text-wrap:balance}p{margin:0;max-width:72ch}
.eyebrow{font-family:var(--mono);font-size:.74rem;letter-spacing:.08em;text-transform:uppercase;color:var(--muted)}
section{display:flex;flex-direction:column;gap:12px;min-width:0}.scroll{overflow-x:auto;min-width:0}
table{border-collapse:collapse;width:100%;font-size:.86rem}th,td{padding:6px 9px;border-bottom:1px solid var(--rule);text-align:left;vertical-align:top}
th{font-weight:600;color:var(--muted);font-size:.74rem;text-transform:uppercase;letter-spacing:.05em}
td.n{font-family:var(--mono);font-variant-numeric:tabular-nums;text-align:right;white-space:nowrap}code{font-family:var(--mono);font-size:.84em}
.bar{height:10px;background:var(--accent);border-radius:2px;min-width:1px}.ok{color:var(--good)}.bad{color:var(--bad)}
</style>"#;

/// The page of `tsv` (a profile's results).
pub fn write(tsv: &str) -> Result<String, String> {
    let rows = rows(tsv)?;
    let mut h = String::new();
    let _ = write!(h, "<title>Join Lab Replays</title>\n<meta name=\"claude:viewport-fit-sides\" content=\"cover\">\n{STYLE}\n<div class=\"wrap\">");
    let engines: Vec<String> = rows.iter().map(|r| r.engine.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    let same = rows.iter().filter(|r| r.same).count();
    let _ = write!(h, "<header style=\"display:flex;flex-direction:column;gap:10px\"><div class=\"eyebrow\">rerail · join lab · {} launches · {}</div><h1>Captured join launches, replayed apart from the engine</h1><p>Each launch the engine captured (<code>RERAIL_JOIN_CAPTURE</code>) placed again in the lab, its addresses bound where its regions lie now, and run warm on each engine. A replay that leaves every region it writes as the engine's launch left it is <b>same</b>. {same} of {} replays are.</p></header>", rows.len(), esc(&engines.join(", ")), rows.len());
    // The run table, by engine.
    let _ = write!(h, "<section><h2>The run</h2><div class=\"scroll\"><table><thead><tr><th>Engine</th><th>Launches</th><th>Same</th><th>Best ms, summed</th><th>Median ms, summed</th><th>CPU ms, summed</th><th>M instructions</th><th>M cycles</th><th>IPC</th><th>Groups</th></tr></thead><tbody>");
    for e in &engines {
        let of: Vec<&Row> = rows.iter().filter(|r| &r.engine == e).collect();
        let (ins, cyc) = (of.iter().map(|r| r.instructions).sum::<u64>(), of.iter().map(|r| r.cycles).sum::<u64>());
        let _ = write!(h, "<tr><td><code>{}</code></td><td class=n>{}</td><td class=n>{}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{:.1}</td><td class=n>{}</td><td class=n>{}</td></tr>", esc(e), of.len(), of.iter().filter(|r| r.same).count(), of.iter().map(|r| r.best).sum::<f64>(), of.iter().map(|r| r.median).sum::<f64>(), of.iter().map(|r| r.cpu).sum::<f64>(), ins as f64 / 1e6, cyc as f64 / 1e6, if cyc > 0 { format!("{:.2}", ins as f64 / cyc as f64) } else { "—".into() }, of.iter().map(|r| r.groups).sum::<u64>());
    }
    let _ = write!(h, "</tbody></table></div></section>");
    // Shapes by time: a launch's best time shared among its shapes by their work.
    let mut by_shape: BTreeMap<(String, String), (f64, u64, Vec<String>)> = BTreeMap::new();
    for r in &rows {
        let total: u64 = r.shapes.iter().map(|s| s.2).sum::<u64>().max(1);
        for (s, rule, w) in &r.shapes {
            let e = by_shape.entry((r.engine.clone(), s.clone())).or_default();
            e.0 += r.best * *w as f64 / total as f64;
            e.1 += w;
            if !e.2.contains(rule) {
                e.2.push(rule.clone());
            }
        }
    }
    let mut shapes: Vec<_> = by_shape.into_iter().collect();
    shapes.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let top = shapes.first().map_or(1.0, |s| s.1.0).max(1e-9);
    let _ = write!(h, "<section><h2>Shapes by time</h2><p>Each launch's best time shared among the shapes its runs encoded, by the rows into their inputs; the forty largest.</p><div class=\"scroll\"><table><thead><tr><th>Engine</th><th>Shape</th><th>ms</th><th style=\"width:30%\"></th><th>Rows in</th><th>Rules</th></tr></thead><tbody>");
    for ((e, s), (ms, work, rules)) in shapes.iter().take(40) {
        let _ = write!(h, "<tr><td><code>{}</code></td><td><code>{}</code></td><td class=n>{:.3}</td><td><div class=bar style=\"width:{:.1}%\"></div></td><td class=n>{}</td><td><code>{}</code></td></tr>", esc(e), esc(s), ms, 100.0 * ms / top, work, esc(&rules.join(" ")));
    }
    let _ = write!(h, "</tbody></table></div></section>");
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
