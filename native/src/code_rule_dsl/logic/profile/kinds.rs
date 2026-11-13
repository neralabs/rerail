//! Where a run's time went, by kind of work (note t27): the report's summary of what t27 first worked out by hand
//! from `spans.tsv` and `native.tsv`.
//!
//! - **By kind**: each span's own CPU in one class, such as the joins of each part, opening the engine, moving rows
//!   to the cross-file part, the memos, planning or parsing. The classes add up to the run.
//! - **Not explained**: spans whose own CPU is large next to their children's, where the next span is missing.
//! - **Joins by head relation**: the native joins' CPU and work, summed by the relation they derive, with lowering's
//!   copies (`rr_in_annotation_0_r`, `fl12_r`) counted as `r`.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::SpanTotal;

/// The class of a span's own CPU, from its path.
pub(super) fn kind(path: &str) -> String {
    let top = path.split('/').next().unwrap_or(path);
    let has = |s: &str| path.contains(s);
    let part = match top {
        "cross" => "cross-file part",
        "local" => "file-local part",
        t if t.starts_with("stratum ") => "the cone of requests",
        _ => "",
    };
    let class = if top.starts_with("worker") {
        "other threads (parse, batches)"
    } else if has("engine run (joins)") {
        return format!("joins: {}", if part.is_empty() { top } else { part });
    } else if has("engine open") {
        "engine: open inputs, build indexes"
    } else if has("engine live statistics") || has("engine step counts") || has("engine reprobe") || has("engine close") || has("profile record") {
        "engine: live statistics, bookkeeping"
    } else if path.ends_with("/search") && planned(path).is_some() {
        "planning: join order search"
    } else if has("/replan") || has("plan #") || has("statistics and plan") || has("/re-price") {
        "planning strata"
    } else if has("export to cross") {
        "moving rows to the cross-file part"
    } else if has("store import components") {
        "cross-file memo (import components)"
    } else if has("memo") {
        "per-file memo"
    } else if has("guest") {
        "guests"
    } else if has("/measure ") {
        "measuring sources"
    } else if top == "stats" {
        "statistics phase (preparing, measuring, binding)"
    } else if has("tape") || has("parse") || has("tree-sitter") {
        "parsing"
    } else if top == "write" {
        "writing outputs"
    } else {
        return format!("other: {top}");
    };
    class.to_owned()
}

/// The rule a planning span plans, from its parent (`plan #0062 wbind, 15 literals`, `replan …`, `replan whole …`):
/// its head relation and body size.
fn planned(path: &str) -> Option<(String, usize)> {
    let mut parts = path.rsplit('/');
    parts.next();
    let parent = parts.next()?;
    let rest = ["plan #", "replan #", "replan whole #"].iter().find_map(|p| parent.strip_prefix(p))?;
    let (_, rest) = rest.split_once(' ')?;
    let (rel, lits) = rest.split_once(", ")?;
    Some((head(&format!("join {rel}")), lits.strip_suffix(" literals")?.parse().ok()?))
}

/// The join order searches summed by the rule's head relation: CPU, searches and the largest body. A search is
/// made when a rule is first seen or a size it reads moved (decision 0038), so many searches of one head mean its
/// sizes keep moving between waves.
pub fn searches(r: &mut String, spans: &BTreeMap<String, SpanTotal>) {
    let mut heads: BTreeMap<String, (f64, u64, usize)> = BTreeMap::new();
    for (p, t) in spans.iter().filter(|(p, _)| p.ends_with("/search")) {
        if let Some((h, lits)) = planned(p) {
            let e = heads.entry(h).or_default();
            e.0 += t.total;
            e.1 += t.calls;
            e.2 = e.2.max(lits);
        }
    }
    let all: f64 = heads.values().map(|e| e.0).sum();
    let mut rows: Vec<(&String, &(f64, u64, usize))> = heads.iter().collect();
    rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    let _ = writeln!(r, "\n== Join order searches by head relation ({all:.2} s in all; a search is made on first sight or when a size moved)");
    let _ = writeln!(r, "{:>9} {:>6} {:>9} {:>9}  head", "cpu s", "%", "searches", "literals");
    for (h, (c, n, lits)) in rows.iter().take(15) {
        let _ = writeln!(r, "{c:>9.2} {:>5.1}% {n:>9} {lits:>9}  {h}", 100.0 * c / all.max(1e-9));
    }
}

/// The own CPU of every span, summed by `kind`; written to the report and as `kinds.tsv`.
pub fn by_kind(r: &mut String, spans: &BTreeMap<String, SpanTotal>) -> String {
    let mut kinds: BTreeMap<String, f64> = BTreeMap::new();
    // An `other` class's largest spans, so that the next span to add is named.
    let mut others: BTreeMap<String, Vec<(f64, &String)>> = BTreeMap::new();
    for (p, t) in spans {
        let k = kind(p);
        if k.starts_with("other: ") {
            others.entry(k.clone()).or_default().push((t.own, p));
        }
        *kinds.entry(k).or_default() += t.own;
    }
    let all: f64 = kinds.values().sum();
    let mut rows: Vec<(&String, &f64)> = kinds.iter().collect();
    rows.sort_by(|a, b| b.1.total_cmp(a.1));
    let _ = writeln!(r, "\n== Where the time went, by kind of work (own CPU of every span, {all:.2} s in all; note t27)");
    let mut tsv = String::from("kind\tcpu_s\n");
    for (k, s) in rows {
        let _ = writeln!(tsv, "{k}\t{s:.4}");
        // Classes under 10 ms are in the tsv only.
        if *s < 0.01 {
            continue;
        }
        let _ = writeln!(r, "{s:>9.2} {:>5.1}%  {k}", 100.0 * s / all.max(1e-9));
        if let Some(top) = others.get_mut(k).filter(|_| *s >= 0.25) {
            top.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (own, p) in top.iter().take(3).filter(|(own, _)| *own >= 0.05) {
                let _ = writeln!(r, "{:>17.2} s  {}", own, super::short(p, 150));
            }
        }
    }
    tsv
}

/// Spans whose own CPU is not under a span of their own: at least `MIN_OWN` seconds and a quarter of the span's.
/// The join order search is its own work (`searches`), not a missing span.
pub fn unexplained(r: &mut String, spans: &BTreeMap<String, SpanTotal>) {
    const MIN_OWN: f64 = 0.5;
    let parents: std::collections::BTreeSet<&str> = spans.keys().filter_map(|p| p.rsplit_once('/').map(|(parent, _)| parent)).collect();
    let mut rows: Vec<(&String, &SpanTotal)> = spans.iter().filter(|(p, t)| parents.contains(p.as_str()) && kind(p) != "planning: join order search" && t.own >= MIN_OWN && t.own >= 0.25 * t.total).collect();
    rows.sort_by(|a, b| b.1.own.total_cmp(&a.1.own));
    let _ = writeln!(r, "\n== Own CPU no span inside names (at least {MIN_OWN} s and a quarter of the span): where a span is missing");
    if rows.is_empty() {
        let _ = writeln!(r, "  (none)");
    }
    for (p, t) in rows.iter().take(15) {
        let _ = writeln!(r, "{:>9.2} of {:>9.2} s  {}", t.own, t.total, super::short(p, 160));
    }
}

/// A native join plan's head relation: `seed r: …` and `join r(…) (delta at i): …` derive `r`. Lowering's copies
/// count as the relation they copy.
fn head(plan: &str) -> String {
    let rest = plan.strip_prefix("seed ").or_else(|| plan.strip_prefix("join ")).unwrap_or(plan);
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    let mut name = name.as_str();
    loop {
        let stripped = name
            .strip_prefix("rr_in_annotation_")
            .and_then(|s| s.split_once('_').filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit())).map(|(_, r)| r))
            .or_else(|| name.strip_prefix("fl").and_then(|s| s.split_once('_').filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(_, r)| r)));
        match stripped {
            Some(s) if !s.is_empty() => name = s,
            _ => break,
        }
    }
    name.to_owned()
}

/// The native joins summed by part and head relation: CPU (while profiling, per probe) and work (rows into every
/// step); written to the report and as `heads.tsv`.
pub fn by_head(r: &mut String, native: &[(&(String, String), &Vec<(u64, u64)>)], cpu: &BTreeMap<(String, String), f64>) -> String {
    let mut heads: BTreeMap<(String, String), (f64, u64, usize)> = BTreeMap::new();
    for ((p, plan), steps) in native {
        let part = kind(&format!("{p}/engine run (joins)"));
        let e = heads.entry((part, head(plan))).or_default();
        e.0 += cpu.get(&(p.clone(), plan.clone())).copied().unwrap_or(0.0);
        e.1 += super::plan_work(steps);
        e.2 += 1;
    }
    let (all_cpu, all_work) = heads.values().fold((0.0, 0u64), |(c, w), e| (c + e.0, w + e.1));
    let mut rows: Vec<(&(String, String), &(f64, u64, usize))> = heads.iter().collect();
    rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)));
    let _ = writeln!(r, "\n== Native joins by head relation ({all_cpu:.2} s CPU in the probes, {all_work} work in all; a copy lowering made counts as its relation)");
    let _ = writeln!(r, "{:>9} {:>6} {:>13} {:>6}  part / head", "cpu s", "%", "work", "plans");
    let mut tsv = String::from("part\thead\tcpu_s\twork\tplans\n");
    for ((part, h), (c, w, n)) in &rows {
        let _ = writeln!(tsv, "{part}\t{h}\t{c:.4}\t{w}\t{n}");
    }
    for ((part, h), (c, w, n)) in rows.iter().take(30) {
        let _ = writeln!(r, "{c:>9.2} {:>5.1}% {w:>13} {n:>6}  {part} / {h}", 100.0 * c / all_cpu.max(1e-9));
    }
    tsv
}

#[cfg(test)]
pub mod tests {
    #[test]
    fn a_plan_is_counted_as_the_relation_it_derives() {
        assert_eq!(super::head("join rr_in_annotation_0_fret($f, $k) (delta at 1): pv($r, $k, $v)"), "fret");
        assert_eq!(super::head("seed fl12_bv: kind($n, 3)"), "bv");
        assert_eq!(super::head("join rr_in_annotation_0_annotation_3_fcall($c) (delta at 0): x"), "annotation_3_fcall");
        assert_eq!(super::head("join flow($a) (delta at 0): x"), "flow");
        assert_eq!(super::kind("cross/stratum a/b/native/engine run (joins)"), "joins: cross-file part");
        assert_eq!(super::kind("local/local/export to cross/totals"), "moving rows to the cross-file part");
        assert_eq!(super::kind("cross/stratum a/native/spec/replan #0062 rr_in_annotation_0_wbind, 15 literals/search"), "planning: join order search");
        assert_eq!(super::planned("x/plan #0004 fl3_bv, 1 literals/search"), Some(("bv".to_owned(), 1)));
    }
}
