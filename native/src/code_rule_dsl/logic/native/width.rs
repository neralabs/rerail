//! Decision 0462: the most rows a derived relation may hold, the most memory a run may take, and what a run does
//! past them. Everything here is a run's own (decision 0441): its settings, the relations it cut, the widest relations
//! it held, and its memory watch, in the `Width` of its `run::Run`. Two runs in one process, on any threads, each have
//! their own; nothing here is process state.
//!
//! A rule whose head really is huge (a product of two node ranges no join connects, 0461) fills memory until the
//! machine gives out: Ruff's SLF001 as written passed 56 GB on 1,644 PostHog files. Replanning cannot help, since no
//! order shrinks a head. So each derived relation has a cap on its rows (`Settings::cap`: `RERAIL_WIDEST_ROWS` as the
//! run starts; default `DEFAULT_ROWS`; `off` or 0 for none). A join walk checks it as it makes each head row
//! (`setfix`'s `enter`, through the walk's `Limits`).
//!
//! - `RERAIL_WIDEST=stop` (the default): the run stops at once, with an error that names the relation and the rules
//!   deriving it (`TooWide`).
//! - `RERAIL_WIDEST=cut`: the relation keeps the rows it has. The probe that reached the cap derives no more for the
//!   rest of the run, and the run goes on. Its results are incomplete: the run's `Record` names each relation cut, and
//!   the command fails at its end (`tools::cli_in`).
//!
//! The memory ceiling (`Settings::ceiling`: `RERAIL_MEMORY_MAX_MB`; default three quarters of the machine's memory;
//! `off` for none) is watched by a thread of the run's own while the run lasts. Past it, the run is told to stop: its
//! walks and its driver stop with an error (`OverMemory`) naming the widest relations it held (`Width::note_rows`).
//! Resident memory is the process's, so a run beside another stops when the two together pass its ceiling.

use std::sync::{Arc, OnceLock};

use super::super::run;

/// The default cap: 50 M rows, about 0.8 GB for a relation of two columns before its indexes.
pub const DEFAULT_ROWS: usize = 50_000_000;

/// How many of the widest relations a run keeps for its messages.
const WIDEST_KEPT: usize = 8;

/// A run's limits.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// The most rows a derived relation may hold (`usize::MAX`: none).
    pub cap: usize,
    /// At the cap, cut the relation and go on, not stop.
    pub cut: bool,
    /// The most resident memory, in bytes, before the run is stopped (`None`: not watched).
    pub ceiling: Option<u64>,
}

impl Settings {
    /// The settings a run starts with: `RERAIL_WIDEST_ROWS`, `RERAIL_WIDEST` and `RERAIL_MEMORY_MAX_MB`, read now.
    pub fn from_env() -> Self {
        Settings {
            cap: parse_cap(std::env::var("RERAIL_WIDEST_ROWS").ok().as_deref()),
            cut: std::env::var("RERAIL_WIDEST").is_ok_and(|v| v == "cut"),
            ceiling: crate::code_rule_dsl::logic::profile::watch::memory_ceiling(),
        }
    }
}

fn parse_cap(v: Option<&str>) -> usize {
    match v.map(str::trim) {
        None | Some("") => DEFAULT_ROWS,
        Some("off") | Some("0") => usize::MAX,
        Some(s) => s.replace(['_', ','], "").parse::<usize>().ok().filter(|&n| n > 0).unwrap_or(DEFAULT_ROWS),
    }
}

/// What derived a relation cut: a component's probe, until its driver names its rule (`Width::name_cuts`), or the rule.
#[derive(Clone, PartialEq, Eq, Debug)]
enum By {
    Probe(String, usize),
    Rule(String),
}

/// A run's limits and what it met of them.
pub struct Width {
    settings: Settings,
    /// The relations cut at the cap, in the order cut, each with what derived it.
    cuts: parking_lot::Mutex<Vec<(String, Vec<By>)>>,
    /// The widest relations seen (`WIDEST_KEPT` of them), most rows first.
    widest: parking_lot::Mutex<Vec<(String, usize)>>,
    /// Set by the memory watch: why the run must stop.
    stop: OnceLock<String>,
}

impl Width {
    pub(in crate::code_rule_dsl) fn new(settings: Settings) -> Width {
        Width { settings, cuts: Default::default(), widest: Default::default(), stop: OnceLock::new() }
    }
    pub fn cap(&self) -> usize {
        self.settings.cap
    }
    pub fn cutting(&self) -> bool {
        self.settings.cut
    }
    /// Why the run must stop now (its memory watch said so), if it must.
    pub fn stopped(&self) -> Option<&str> {
        self.stop.get().map(String::as_str)
    }
    /// The error the run stops with now (`stopped`), if it must.
    pub fn stop_error(&self) -> Option<rusqlite::Error> {
        self.stopped().map(|m| rusqlite::Error::UserFunctionError(Box::new(OverMemory(m.to_owned()))))
    }

    fn note(&self, table: &str, by: By) {
        let mut c = self.cuts.lock();
        match c.iter_mut().find(|(t, _)| t == table) {
            Some((_, bys)) => {
                if !bys.contains(&by) {
                    bys.push(by);
                }
            }
            None => c.push((table.to_owned(), vec![by])),
        }
    }
    /// A relation cut at the cap by probe `probe` of component `component`.
    pub fn note_cut(&self, table: &str, component: &str, probe: usize) {
        self.note(table, By::Probe(component.to_owned(), probe));
    }
    /// A relation cut at the cap by a rule written out (`rule`).
    pub fn note_cut_by(&self, table: &str, rule: String) {
        self.note(table, By::Rule(rule));
    }
    /// Whether any relation was cut (a driver then names its probes' rules: `name_cuts`).
    pub fn any_cut(&self) -> bool {
        !self.cuts.lock().is_empty()
    }
    /// The probes of `component` noted as cutting, named by the rules `rule` writes for them (a line's rule in its
    /// order).
    pub fn name_cuts(&self, component: &str, rule: impl Fn(usize) -> Option<String>) {
        let mut c = self.cuts.lock();
        for (_, by) in c.iter_mut() {
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

    /// A relation's rows as a wave settled them. Only a relation past `NOTED` rows is noted: the small ones say
    /// nothing about a runaway, and a lock per wave per small relation would cost more than they are worth.
    pub fn note_rows(&self, table: &str, rows: usize) {
        const NOTED: usize = 1 << 20;
        if rows < NOTED {
            return;
        }
        let mut w = self.widest.lock();
        match w.iter_mut().find(|(t, _)| t == table) {
            Some(e) => e.1 = e.1.max(rows),
            None => w.push((table.to_owned(), rows)),
        }
        w.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        w.truncate(WIDEST_KEPT);
    }

    /// What the run met: the relations it cut and the widest it held.
    pub fn record(&self) -> Record {
        let cuts = self
            .cuts
            .lock()
            .iter()
            .map(|(t, by)| {
                let by = by
                    .iter()
                    .map(|b| match b {
                        By::Rule(r) => r.clone(),
                        By::Probe(comp, p) => format!("probe {p} of {comp}"),
                    })
                    .collect();
                (t.clone(), by)
            })
            .collect();
        Record { cap: self.settings.cap, cuts, widest: self.widest.lock().clone() }
    }

    /// The memory watch's message: the widest relations the run held, and what to do.
    fn ceiling_message(&self, max: u64, now: u64) -> String {
        let mut out = format!("the run stopped at the memory ceiling (RERAIL_MEMORY_MAX_MB={}: {} MB resident).", max >> 20, now >> 20);
        let widest = self.widest.lock().clone();
        if widest.is_empty() {
            out.push_str(" No derived relation had passed a million rows: the memory went elsewhere (inputs, a cache, the device).");
        } else {
            out.push_str(" The widest derived relations, by rows:");
            for (table, rows) in &widest {
                out.push_str(&format!("\n  `{table}`: {rows}"));
            }
        }
        out.push_str(&format!(
            "\nA relation's rows are capped at {} (RERAIL_WIDEST_ROWS); lower it to stop at the relation, naming its rules, \
             before the run reaches the ceiling. Raise RERAIL_MEMORY_MAX_MB if the memory is expected.",
            self.settings.cap
        ));
        out
    }
}

/// What a run met of its limits, returned with its result.
#[derive(Clone, Debug, Default)]
pub struct Record {
    pub cap: usize,
    /// Each relation cut at the cap, with the rules (or probes) that derived it.
    pub cuts: Vec<(String, Vec<String>)>,
    /// The widest relations it held, most rows first.
    pub widest: Vec<(String, usize)>,
}

impl Record {
    /// The message a run with cuts ends with; `None` where it cut none.
    pub fn incomplete(&self) -> Option<String> {
        if self.cuts.is_empty() {
            return None;
        }
        let mut out = format!(
            "{} relation(s) reached {} rows (RERAIL_WIDEST_ROWS) and were cut there (RERAIL_WIDEST=cut); the results are incomplete:",
            self.cuts.len(),
            self.cap
        );
        for (table, by) in &self.cuts {
            out.push_str(&format!("\n  `{table}`"));
            for b in by {
                out.push_str(&format!("\n    derived by {b}"));
            }
        }
        Some(out)
    }
}

/// The run's memory watch, where its settings have a ceiling: the resident memory every 50 ms; past it, the run is told
/// to stop. It ends with the run.
pub(in crate::code_rule_dsl) fn watch(run: &Arc<run::Run>) {
    let Some(max) = run.width.settings.ceiling else { return };
    let run = Arc::downgrade(run);
    let _ = rerail_accel::exec::service("rerail-run-memory", None, move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let Some(r) = run.upgrade() else { return };
            if r.ended() || r.width.stop.get().is_some() {
                return;
            }
            let Some(now) = crate::code_rule_dsl::logic::profile::watch::resident_bytes() else { return };
            if now > max {
                let _ = r.width.stop.set(r.width.ceiling_message(max, now));
                return;
            }
        }
    });
}

/// What a walk reads of its run's limits, taken once as it starts.
#[derive(Clone)]
pub struct Limits {
    pub cap: usize,
    run: Option<Arc<run::Run>>,
}

impl Limits {
    /// The limits of the run under way here; with none, the default cap, stopping.
    pub fn here() -> Limits {
        let run = run::current();
        Limits { cap: run.as_ref().map_or(DEFAULT_ROWS, |r| r.width.cap()), run }
    }
    fn width(&self) -> Option<&Width> {
        self.run.as_deref().map(|r| &r.width)
    }
    /// Whether the run cuts a relation at the cap (else it stops). A walk with no run stops: a cut needs a record.
    pub fn cutting(&self) -> bool {
        self.width().is_some_and(|w| w.cutting())
    }
    /// The error a walk stops with now: the run's memory stop, else the cap's for `table`, if `rows` reached it.
    #[inline]
    pub fn check(&self, table: &str, rows: usize) -> Option<rusqlite::Error> {
        if let Some(e) = self.stop_error() {
            return Some(e);
        }
        (rows >= self.cap).then(|| too_wide(table, self.cap))
    }
    /// The run's memory stop, if it must stop.
    #[inline]
    pub fn stop_error(&self) -> Option<rusqlite::Error> {
        self.width().and_then(|w| w.stop_error())
    }
    pub fn note_cut(&self, table: &str, component: &str, probe: usize) {
        if let Some(w) = self.width() {
            w.note_cut(table, component, probe);
        }
    }
    pub fn note_cut_by(&self, table: &str, rule: String) {
        if let Some(w) = self.width() {
            w.note_cut_by(table, rule);
        }
    }
    pub fn note_rows(&self, table: &str, rows: usize) {
        if let Some(w) = self.width() {
            w.note_rows(table, rows);
        }
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

/// The run's memory watch stopped it: the message names the widest relations it held.
#[derive(Debug, Clone)]
pub struct OverMemory(pub String);

impl std::fmt::Display for OverMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OverMemory {}

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

    /// The memory watch stops a run past its ceiling, naming the widest relations it held.
    #[test]
    fn the_memory_watch_stops_a_run_past_its_ceiling() {
        if crate::code_rule_dsl::logic::profile::watch::resident_bytes().is_none() {
            return;
        }
        let run = run::scope(|| Settings { cap: DEFAULT_ROWS, cut: false, ceiling: Some(1) });
        run.run().width.note_rows("big", 2 << 20);
        let started = std::time::Instant::now();
        while run.run().width.stopped().is_none() && started.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let why = run.run().width.stopped().expect("stopped within 5 s").to_owned();
        assert!(why.contains("memory ceiling") && why.contains("`big`"), "{why}");
        assert!(Limits::here().check("t", 0).is_some(), "a walk stops once its run must");
    }
}
