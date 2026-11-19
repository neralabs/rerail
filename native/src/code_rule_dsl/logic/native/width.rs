//! Decision 0394: the most rows a derived relation may hold, and what a run does past it.
//!
//! A rule whose head really is huge (a product of two node ranges no join connects, 0393) fills memory until the
//! machine gives out: Ruff's SLF001 as written passed 56 GB on 1,644 PostHog files. Replanning cannot help, since no
//! order shrinks a head. So each derived relation has a cap on its rows, `RERAIL_WIDEST_ROWS` (default
//! `DEFAULT_ROWS`; `off` or 0 for none). A join walk checks it as it makes each head row (`setfix`'s `enter`).
//!
//! - `RERAIL_WIDEST=stop` (the default): the run stops at once, with an error that names the relation and the rules
//!   deriving it (`TooWide`).
//! - `RERAIL_WIDEST=cut`: the relation keeps the rows it has. The probe that reached the cap derives no more for the
//!   rest of the run, and the run goes on. Its results are then incomplete, so the process fails at its end, naming
//!   each relation cut (`cut_report`).
//!
//! The cap bounds memory one relation at a time. `profile::watch`'s memory ceiling (`RERAIL_MEMORY_MAX_MB`) bounds
//! the whole process, on every path, and names the widest relations seen (`note_rows`, `widest`).

use std::collections::BTreeMap;
use std::sync::LazyLock;

/// The default cap: 50 M rows, about 0.8 GB for a relation of two columns before its indexes.
pub const DEFAULT_ROWS: usize = 50_000_000;

/// The cap on a derived relation's rows (`usize::MAX`: none).
pub fn cap() -> usize {
    static CAP: LazyLock<usize> = LazyLock::new(|| parse_cap(std::env::var("RERAIL_WIDEST_ROWS").ok().as_deref()));
    match CAP_SET.load(Relaxed) {
        0 => *CAP,
        n => n,
    }
}

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering::Relaxed};
static CAP_SET: AtomicUsize = AtomicUsize::new(0);
static CUT_SET: AtomicU8 = AtomicU8::new(0);

/// Tests: the cap and the policy in place of the environment's (`None`: the environment's again). Process-wide, so
/// a test that sets them runs alone in its process (nextest runs each test so).
#[doc(hidden)]
pub fn set_for_tests(cap: Option<usize>, cut: Option<bool>) {
    CAP_SET.store(cap.unwrap_or(0), Relaxed);
    CUT_SET.store(cut.map_or(0, |c| 1 + u8::from(c)), Relaxed);
}

fn parse_cap(v: Option<&str>) -> usize {
    match v.map(str::trim) {
        None | Some("") => DEFAULT_ROWS,
        Some("off") | Some("0") => usize::MAX,
        Some(s) => s.replace(['_', ','], "").parse::<usize>().ok().filter(|&n| n > 0).unwrap_or(DEFAULT_ROWS),
    }
}

/// Whether a relation at its cap is cut and the run goes on (`RERAIL_WIDEST=cut`), not stopped.
pub fn cutting() -> bool {
    static CUT: LazyLock<bool> = LazyLock::new(|| std::env::var("RERAIL_WIDEST").is_ok_and(|v| v == "cut"));
    match CUT_SET.load(Relaxed) {
        0 => *CUT,
        n => n == 2,
    }
}

/// A relation that reached the cap: its table, the cap, and the rules deriving it (where the driver knows them).
#[derive(Debug, Clone)]
pub struct TooWide {
    pub table: String,
    pub cap: usize,
    pub rules: Vec<String>,
}

impl std::fmt::Display for TooWide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "relation `{}` reached {} rows, the most a derived relation may hold (RERAIL_WIDEST_ROWS)", self.table, self.cap)?;
        if !self.rules.is_empty() {
            write!(f, "; derived by:")?;
            for r in &self.rules {
                write!(f, "\n  {r}")?;
            }
        }
        write!(
            f,
            "\nA rule whose head is this large is usually a product of rows its body never joins. Bound it by a join, \
             raise RERAIL_WIDEST_ROWS if the size is expected, or set RERAIL_WIDEST=cut to keep the rows so far and go on \
             (the run then fails at its end)."
        )
    }
}

impl std::error::Error for TooWide {}

/// The error a walk raises at the cap.
pub fn too_wide(table: &str, cap: usize) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(Box::new(TooWide { table: table.to_owned(), cap, rules: Vec::new() }))
}

/// The cap's error inside `e`, if it is one.
pub fn as_too_wide(e: &rusqlite::Error) -> Option<&TooWide> {
    match e {
        rusqlite::Error::UserFunctionError(b) => b.downcast_ref::<TooWide>(),
        _ => None,
    }
}

/// `e` with the rules deriving its relation (`rules`), if it is the cap's error: a caller that knows them better
/// (the driver: their text) replaces what a lower one said (the component: the probe's number).
pub fn with_rules(e: rusqlite::Error, rules: impl FnOnce(&str) -> Vec<String>) -> rusqlite::Error {
    match e {
        rusqlite::Error::UserFunctionError(b) if b.is::<TooWide>() => {
            let mut w = *b.downcast::<TooWide>().expect("checked");
            let mut named = rules(&w.table);
            let mut seen = std::collections::BTreeSet::new();
            named.retain(|r| seen.insert(r.clone()));
            if !named.is_empty() {
                w.rules = named;
            }
            rusqlite::Error::UserFunctionError(Box::new(w))
        }
        e => e,
    }
}

/// What derived a relation cut: a component's probe, until its driver names its rule (`name_cuts`), or the rule.
#[derive(Clone, PartialEq, Eq)]
enum By {
    Probe(String, usize),
    Rule(String),
}

static CUT: LazyLock<parking_lot::Mutex<BTreeMap<String, Vec<By>>>> = LazyLock::new(Default::default);

fn note(table: &str, by: By) {
    let mut c = CUT.lock();
    let e = c.entry(table.to_owned()).or_default();
    if !e.contains(&by) {
        e.push(by);
    }
}

/// A relation cut at the cap (`RERAIL_WIDEST=cut`) by probe `probe` of component `component`.
pub fn note_cut(table: &str, component: &str, probe: usize) {
    note(table, By::Probe(component.to_owned(), probe));
}

/// A relation cut at the cap by a rule written out (`rule`).
pub fn note_cut_by(table: &str, rule: String) {
    note(table, By::Rule(rule));
}

/// Whether any relation was cut (a driver then names its probes' rules: `name_cuts`).
pub fn any_cut() -> bool {
    !CUT.lock().is_empty()
}

/// The probes of `component` noted as cutting, by the rules `rule` writes for them (a line's rule in its order).
pub fn name_cuts(component: &str, rule: impl Fn(usize) -> Option<String>) {
    let mut c = CUT.lock();
    for by in c.values_mut() {
        let mut named: Vec<By> = Vec::with_capacity(by.len());
        for b in by.drain(..) {
            let b = match &b {
                By::Probe(comp, p) if comp == component => rule(*p).map_or(b, By::Rule),
                _ => b,
            };
            if !named.contains(&b) {
                named.push(b);
            }
        }
        *by = named;
    }
}

/// The relations cut so far, as the message a failed run ends with; `None` where none was.
pub fn cut_report() -> Option<String> {
    let c = CUT.lock();
    if c.is_empty() {
        return None;
    }
    let mut out = format!("{} relation(s) reached {} rows (RERAIL_WIDEST_ROWS) and were cut there (RERAIL_WIDEST=cut); the results are incomplete:", c.len(), cap());
    for (table, by) in c.iter() {
        out.push_str(&format!("\n  `{table}`"));
        for b in by {
            match b {
                By::Rule(r) => out.push_str(&format!("\n    derived by {r}")),
                By::Probe(comp, p) => out.push_str(&format!("\n    derived by probe {p} of {comp}")),
            }
        }
    }
    Some(out)
}

/// Forgets the relations cut (tests, and a process that runs again).
pub fn forget_cuts() {
    CUT.lock().clear();
}

/// The widest relations seen, by table: their rows as last settled (`note_rows`), for the memory ceiling's message.
static WIDEST: LazyLock<parking_lot::Mutex<BTreeMap<String, usize>>> = LazyLock::new(Default::default);

/// A relation's rows as a wave settled them. Only a relation past `NOTED` rows is noted: the small ones say nothing
/// about a runaway, and a lock per wave per small relation would cost more than they are worth.
pub fn note_rows(table: &str, rows: usize) {
    const NOTED: usize = 1 << 20;
    if rows >= NOTED {
        let mut w = WIDEST.lock();
        match w.get_mut(table) {
            Some(n) => *n = (*n).max(rows),
            None => {
                w.insert(table.to_owned(), rows);
            }
        }
    }
}

/// The `n` widest relations seen, most rows first.
pub fn widest(n: usize) -> Vec<(String, usize)> {
    let w = WIDEST.lock();
    let mut v: Vec<(String, usize)> = w.iter().map(|(t, r)| (t.clone(), *r)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_reads_its_setting() {
        assert_eq!(parse_cap(None), DEFAULT_ROWS);
        assert_eq!(parse_cap(Some("off")), usize::MAX);
        assert_eq!(parse_cap(Some("0")), usize::MAX);
        assert_eq!(parse_cap(Some("1_000")), 1000);
        assert_eq!(parse_cap(Some("2,500")), 2500);
        assert_eq!(parse_cap(Some("x")), DEFAULT_ROWS, "an unreadable setting keeps the default, never no cap");
    }

    #[test]
    fn the_caps_error_names_its_relation_and_rules() {
        let e = with_rules(too_wide("pairs", 10), |t| vec![format!("{t}(x, y) :- a(x), b(y).")]);
        let w = as_too_wide(&e).expect("the cap's error");
        assert_eq!(w.table, "pairs");
        let text = w.to_string();
        assert!(text.contains("`pairs` reached 10 rows") && text.contains("pairs(x, y) :- a(x), b(y).") && text.contains("RERAIL_WIDEST=cut"), "{text}");
        assert!(as_too_wide(&rusqlite::Error::InvalidQuery).is_none());
        let twice = with_rules(too_wide("pairs", 10), |_| vec!["r.".to_owned(), "r.".to_owned()]);
        assert_eq!(as_too_wide(&twice).unwrap().rules, ["r."], "a rule run by two lines is named once");
    }
}
