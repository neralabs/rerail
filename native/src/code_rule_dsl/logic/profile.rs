//! A run's profile (`RERAIL_PROFILE=<dir>`): where a rule's CPU goes, in every phase, down to the SQL statement and
//! the native join step, so a slow rule can be read and fixed.
//!
//! - **Spans.** `span("name")` opens a named span on this thread until the guard drops. Spans nest into paths
//!   (`local/stratum violation/sql #12`), summed over every call: calls, CPU (user + system of the thread, never
//!   wall) in total and in the span itself. The worker thread's spans (`worker/…`) are its own.
//! - **SQL.** Each statement SQLite runs on a traced connection is charged to the span running it, by text: runs,
//!   SQLite's time, VM steps, full-scan steps, sorts, automatic-index rows (`sql`). VM steps do not move with load.
//! - **Native joins.** Each native component's plans with rows in and out of every step, summed over the chunks
//!   (`native`).
//! - **Notes.** `note(key, text)`: one text per key (a rule's text, a statement's query plan).
//!
//! `write(dir)` writes `profile.txt` (the report: spans by CPU, statements by time with their query plans, native
//! steps by work, and warnings) and the tables `spans.tsv`, `sql.tsv` and `native.tsv`. Off, a span costs one load.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

static ON: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(std::env::var_os("RERAIL_PROFILE").is_some()));

/// Whether this run is profiled.
pub(in crate::code_rule_dsl) fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns profiling on or off (a test that profiles without the environment variable).
#[allow(dead_code)]
pub(in crate::code_rule_dsl) fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

static DIR: LazyLock<Mutex<Option<std::path::PathBuf>>> = LazyLock::new(|| Mutex::new(std::env::var_os("RERAIL_PROFILE").map(Into::into)));

/// The directory the next report is written to: `RERAIL_PROFILE`'s, unless set (one per rule in a measurement).
pub(in crate::code_rule_dsl) fn directory() -> Option<std::path::PathBuf> {
    DIR.lock().unwrap().clone()
}

/// Sets the directory the next report is written to.
#[allow(dead_code)]
pub(in crate::code_rule_dsl) fn set_directory(dir: &Path) {
    *DIR.lock().unwrap() = Some(dir.to_path_buf());
}

fn cpu() -> f64 {
    let (u, s) = super::plan::corpus::thread_usage();
    u + s
}

#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct SpanTotal {
    pub(in crate::code_rule_dsl) calls: u64,
    pub(in crate::code_rule_dsl) total: f64,
    pub(in crate::code_rule_dsl) own: f64,
}

#[derive(Default, Clone)]
pub(in crate::code_rule_dsl) struct SqlTotal {
    pub(in crate::code_rule_dsl) runs: u64,
    pub(in crate::code_rule_dsl) ns: u64,
    pub(in crate::code_rule_dsl) vm: u64,
    pub(in crate::code_rule_dsl) scan: u64,
    pub(in crate::code_rule_dsl) sort: u64,
    pub(in crate::code_rule_dsl) autoindex: u64,
}

#[derive(Default)]
struct State {
    spans: BTreeMap<String, SpanTotal>,
    /// By (span path, statement text).
    sql: BTreeMap<(String, String), SqlTotal>,
    /// By (span path, plan text): per step, rows in and out.
    native: BTreeMap<(String, String), Vec<(u64, u64)>>,
    notes: BTreeMap<String, String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

struct Open {
    path: String,
    start: f64,
    children: f64,
}

thread_local! {
    static STACK: RefCell<Vec<Open>> = const { RefCell::new(Vec::new()) };
}

/// An open span; it closes when dropped.
pub(in crate::code_rule_dsl) struct Span(bool);

/// Opens span `name` (made only when profiling) under this thread's open span.
pub(in crate::code_rule_dsl) fn span(name: impl FnOnce() -> String) -> Span {
    if !on() {
        return Span(false);
    }
    let name = name();
    STACK.with(|s| {
        let mut s = s.borrow_mut();
        let path = match s.last() {
            Some(top) => format!("{}/{name}", top.path),
            None => name,
        };
        s.push(Open { path, start: cpu(), children: 0.0 });
    });
    Span(true)
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        let now = cpu();
        STACK.with(|s| {
            let mut s = s.borrow_mut();
            let Some(open) = s.pop() else { return };
            let total = now - open.start;
            if let Some(parent) = s.last_mut() {
                parent.children += total;
            }
            let mut state = STATE.lock().unwrap();
            let t = state.spans.entry(open.path).or_default();
            t.calls += 1;
            t.total += total;
            t.own += total - open.children;
        });
    }
}

/// This thread's open span's path, `-` outside any.
pub(in crate::code_rule_dsl) fn current() -> String {
    STACK.with(|s| s.borrow().last().map_or_else(|| "-".to_owned(), |o| o.path.clone()))
}

/// Charges one run of statement `text` to the current span.
pub(in crate::code_rule_dsl) fn sql(text: String, ns: u64, vm: u64, scan: u64, sort: u64, autoindex: u64) {
    let path = current();
    let mut state = STATE.lock().unwrap();
    let t = state.sql.entry((path, text)).or_default();
    t.runs += 1;
    t.ns += ns;
    t.vm += vm;
    t.scan += scan;
    t.sort += sort;
    t.autoindex += autoindex;
}

/// Adds a native plan's rows in and out per step to the current span.
pub(in crate::code_rule_dsl) fn native(plan: String, steps: &[(u64, u64)]) {
    let path = current();
    let mut state = STATE.lock().unwrap();
    let t = state.native.entry((path, plan)).or_default();
    if t.len() < steps.len() {
        t.resize(steps.len(), (0, 0));
    }
    for (a, b) in t.iter_mut().zip(steps) {
        a.0 += b.0;
        a.1 += b.1;
    }
}

/// Keeps `text` under `key` (the first one given).
pub(in crate::code_rule_dsl) fn note(key: String, text: impl FnOnce() -> String) {
    let mut state = STATE.lock().unwrap();
    state.notes.entry(key).or_insert_with(text);
}

/// A statement's text as the SQL trace keys it: its words, single-spaced.
pub(in crate::code_rule_dsl) fn normal(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Notes `statement`'s query plan (`EXPLAIN QUERY PLAN`, once per text) when profiling.
pub(in crate::code_rule_dsl) fn plan(db: &rusqlite::Connection, statement: &str) {
    if !on() {
        return;
    }
    let key = format!("plan:{}", normal(statement));
    if STATE.lock().unwrap().notes.contains_key(&key) {
        return;
    }
    let text = (|| -> rusqlite::Result<String> {
        let mut q = db.prepare(&format!("EXPLAIN QUERY PLAN {statement}"))?;
        let rows: Vec<(i64, i64, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        // Each step indented under its parent.
        let mut depth: BTreeMap<i64, usize> = BTreeMap::new();
        let mut out = String::new();
        for (id, parent, detail) in rows {
            let d = depth.get(&parent).map_or(0, |d| d + 1);
            depth.insert(id, d);
            let _ = writeln!(out, "{}{detail}", "  ".repeat(d));
        }
        Ok(out)
    })()
    .unwrap_or_else(|e| format!("(no plan: {e})"));
    note(key, || text);
}

/// Forgets everything recorded (a run's start).
pub(in crate::code_rule_dsl) fn reset() {
    *STATE.lock().unwrap() = State::default();
}

/// The statements recorded, by SQLite's time, most first: (span, text, totals).
pub(in crate::code_rule_dsl) fn statements() -> Vec<(String, String, SqlTotal)> {
    let state = STATE.lock().unwrap();
    let mut all: Vec<_> = state.sql.iter().map(|((p, t), s)| (p.clone(), t.clone(), s.clone())).collect();
    all.sort_by_key(|(_, _, s)| std::cmp::Reverse(s.ns));
    all
}

/// The spans recorded: path and totals.
pub(in crate::code_rule_dsl) fn spans() -> BTreeMap<String, SpanTotal> {
    STATE.lock().unwrap().spans.clone()
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_owned() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

/// The report, and the tables, into `dir`. `header`: what was run (lines put first).
pub(in crate::code_rule_dsl) fn write(dir: &Path, header: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let state = STATE.lock().unwrap();
    let mut spans_tsv = String::from("path\tcalls\tcpu_s\town_s\n");
    for (p, t) in &state.spans {
        let _ = writeln!(spans_tsv, "{p}\t{}\t{:.4}\t{:.4}", t.calls, t.total, t.own);
    }
    std::fs::write(dir.join("spans.tsv"), spans_tsv)?;
    let mut sql: Vec<_> = state.sql.iter().collect();
    sql.sort_by_key(|(_, s)| std::cmp::Reverse(s.ns));
    let mut sql_tsv = String::from("span\truns\tms\tvm_steps\tfullscan_steps\tsorts\tautoindex_rows\tsql\n");
    for ((p, text), s) in &sql {
        let _ = writeln!(sql_tsv, "{p}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{text}", s.runs, s.ns as f64 / 1e6, s.vm, s.scan, s.sort, s.autoindex);
    }
    std::fs::write(dir.join("sql.tsv"), sql_tsv)?;
    let work = |steps: &Vec<(u64, u64)>| steps.iter().map(|(a, _)| a).sum::<u64>();
    let mut native: Vec<_> = state.native.iter().collect();
    native.sort_by_key(|(_, s)| std::cmp::Reverse(work(s)));
    let mut native_tsv = String::from("span\twork\tsteps_in_out\tplan\n");
    for ((p, plan), steps) in &native {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let _ = writeln!(native_tsv, "{p}\t{}\t{}\t{plan}", work(steps), s.join(","));
    }
    std::fs::write(dir.join("native.tsv"), native_tsv)?;

    // The report.
    let mut r = String::new();
    r.push_str(header);
    let roots: f64 = state.spans.iter().filter(|(p, _)| !p.contains('/')).map(|(_, t)| t.total).sum();
    let _ = writeln!(r, "\n== Spans: CPU (user + system), summed over calls; own = less the spans inside it ({roots:.2} s in all)");
    let _ = writeln!(r, "{:>9} {:>9} {:>6} {:>9}  span", "cpu s", "own s", "%", "calls");
    // A tree: each span under its parent, children by CPU.
    fn walk(r: &mut String, spans: &BTreeMap<String, SpanTotal>, parent: Option<&str>, depth: usize, all: f64) {
        let mut kids: Vec<(&String, &SpanTotal)> = spans
            .iter()
            .filter(|(p, _)| match parent {
                None => !p.contains('/'),
                Some(par) => p.strip_prefix(par).and_then(|rest| rest.strip_prefix('/')).is_some_and(|rest| !rest.contains('/')),
            })
            .collect();
        kids.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
        for (p, t) in kids {
            // Spans under 0.1% of the run are left out of the tree (the tables keep them).
            if t.total < all * 0.001 && depth > 0 {
                continue;
            }
            let name = p.rsplit('/').next().unwrap_or(p);
            let _ = writeln!(r, "{:>9.3} {:>9.3} {:>5.1}% {:>9}  {}{}", t.total, t.own, 100.0 * t.total / all.max(1e-9), t.calls, "  ".repeat(depth), short(name, 100));
            walk(r, spans, Some(p), depth + 1, all);
        }
    }
    walk(&mut r, &state.spans, None, 0, roots);

    let sql_ns: u64 = sql.iter().map(|(_, s)| s.ns).sum();
    let sql_vm: u64 = sql.iter().map(|(_, s)| s.vm).sum();
    let _ = writeln!(
        r,
        "\n== SQL statements by SQLite's time ({} distinct, {:.2} s, {sql_vm} VM steps in all); plans for the first 15",
        sql.len(),
        sql_ns as f64 / 1e9
    );
    let _ = writeln!(r, "{:>9} {:>7} {:>12} {:>11} {:>6} {:>9}  span / statement", "ms", "runs", "vm steps", "full scan", "sorts", "autoidx");
    for (i, ((p, text), s)) in sql.iter().take(40).enumerate() {
        let _ = writeln!(r, "{:>9.1} {:>7} {:>12} {:>11} {:>6} {:>9}  {p}", s.ns as f64 / 1e6, s.runs, s.vm, s.scan, s.sort, s.autoindex);
        let _ = writeln!(r, "{:>58}{}", "", short(text, 400));
        if i < 15
            && let Some(plan) = state.notes.get(&format!("plan:{text}"))
        {
            for line in plan.lines() {
                let _ = writeln!(r, "{:>60}{line}", "");
            }
        }
    }

    let _ = writeln!(r, "\n== Native joins by work (rows into every step, summed over chunks); steps in:out");
    for ((p, plan), steps) in native.iter().take(30) {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let _ = writeln!(r, "{:>12}  {p}\n{:>14}steps {}\n{:>14}{}", work(steps), "", s.join(" "), "", short(plan, 600));
    }

    // Warnings: the shapes that usually explain a slow rule.
    let _ = writeln!(r, "\n== Warnings");
    let mut warned = 0;
    for ((p, text), s) in &sql {
        if s.autoindex > 0 {
            let _ = writeln!(r, "- automatic index ({} rows) built for a statement in {p}: an index is missing.\n    {}", s.autoindex, short(text, 300));
            warned += 1;
        }
        if s.vm > 0 && s.scan * 2 > s.vm && s.vm > 1_000_000 {
            let _ = writeln!(r, "- full scans are {:.0}% of {} VM steps in {p}: a join reads a whole table.\n    {}", 100.0 * s.scan as f64 / s.vm as f64, s.vm, short(text, 300));
            warned += 1;
        }
        if s.runs > 1000 && s.ns / s.runs.max(1) < 20_000 {
            let _ = writeln!(r, "- {} runs of one statement in {p}: per-row or per-chunk statement overhead.\n    {}", s.runs, short(text, 200));
            warned += 1;
        }
    }
    for ((p, plan), steps) in &native {
        // Step 0 is the seed (one binding a chunk), so its step is the first atom's scan, not a fan-out.
        for (i, w) in steps.windows(2).enumerate().skip(1) {
            if w[1].0 > 0 && w[0].0 > 0 && w[1].0 > 20 * w[0].0 && w[1].0 > 1_000_000 {
                let _ = writeln!(r, "- native step {} fans out {}x ({} -> {} rows) in {p}:\n    {}", i + 1, w[1].0 / w[0].0, w[0].0, w[1].0, short(plan, 300));
                warned += 1;
            }
        }
    }
    if warned == 0 {
        r.push_str("(none)\n");
    }
    let _ = writeln!(r, "\n== Notes");
    for (k, v) in state.notes.iter().filter(|(k, _)| !k.starts_with("plan:")) {
        let _ = writeln!(r, "{k}: {}", short(v, 600));
    }
    std::fs::write(dir.join("profile.txt"), r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_nest_and_charge_their_own_cpu() {
        set(true);
        reset();
        {
            let _a = span(|| "a".into());
            {
                let _b = span(|| "b".into());
                let mut x = 0u64;
                for i in 0..20_000_000u64 {
                    x = x.wrapping_mul(31).wrapping_add(i);
                }
                std::hint::black_box(x);
                sql("SELECT 1".into(), 10, 5, 0, 0, 0);
            }
        }
        let s = spans();
        set(false);
        assert!(s["a/b"].total > 0.0 && s["a"].total >= s["a/b"].total);
        assert!(s["a"].own < s["a"].total);
        assert_eq!(statements()[0].0, "a/b");
    }
}
