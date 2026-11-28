//! Rows made and dropped: the rule plans whose walks make rows at one step that a later step drops, each with its cause
//! and the rewrite that removes it (decision 0714). Read from the plans the planner recorded (`State::plans`): each
//! step's literal, its variables, its source, the planner's estimate, and the actual rows and CPU of every walk. No
//! query is run.
//!
//! A drop is a step that keeps at most one in `DROP_RATIO` of the rows that reach it, with `DROP_ROWS` rows in or
//! more. Its source is the last earlier step after the first whose rows grew: the rows the drop throws away were made
//! there. The first step's rows are a scan's (a delta, a seed's relation), and a drop with no later source narrows a
//! scan, which is not waste (a check that finds little reads what it must). The CPU lost
//! is the share the drop throws away of the CPU spent from the source to the drop.
use std::collections::{BTreeMap, BTreeSet};

/// A drop keeps at most one in this many rows ...
pub const DROP_RATIO: u64 = 10;
/// ... of at least this many rows in, summed over the run.
pub const DROP_ROWS: u64 = 20_000;
/// A rule is reported when it lost at least this much CPU (seconds) to its drops.
pub const LOST_CPU: f64 = 0.02;
/// A step's estimate is a misprediction when its actual rows per loop are this many times more.
pub const MISPREDICTED: f64 = 10.0;

/// One recorded plan, as the analysis reads it.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// The stratum's key (its relations), the plan's kind (`seed r`, `join r (delta d)`), and its rule's number.
    pub stratum: String,
    pub kind: String,
    pub rule: Option<usize>,
    /// The head as written (`p($x, $y)`), its relation, and the variables it reads.
    pub head: String,
    pub head_rel: String,
    pub head_vars: Vec<String>,
    /// Where the head's authored rules are (`file:line`), when the run's sources are known.
    pub authored: Option<String>,
    pub loops: u64,
    pub steps: Vec<Step>,
}

/// One step of a plan: what the planner chose and what the walks did.
#[derive(Clone, Debug, Default)]
pub struct Step {
    pub literal: String,
    pub access: String,
    /// `input`, `derived`, `view of an input`, `tree`, `text test`, or, for a stratum's own relation, `delta (last
    /// wave)` and `this stratum`; empty for a test.
    pub source: String,
    pub vars: Vec<String>,
    /// The planner's rows after the step, per loop, and how it priced them.
    pub est: f64,
    pub basis: String,
    /// The joins the planner passed over at this step: the literal and the rows a binding it was priced at.
    pub passed: Vec<(String, f64)>,
    pub rows_in: u64,
    pub rows_out: u64,
    pub cpu: f64,
}

/// Why a plan made rows it dropped, and so what fixes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    /// The rows the source joins again are copies but for a variable no later step and not the head reads.
    DeadVariable,
    /// Inside a recursion, the source and the drop read nothing of their stratum: every round joins them again.
    InvariantInRecursion,
    /// The source's rows are many times what the planner expected, so a narrower join was passed over.
    Mispredicted,
    /// The drop is a test of a value computed per row, which no index can look up.
    TestAfterWalk,
    /// None of the above: the rows are made and dropped, and the plan shows where.
    Dropped,
    /// A join that shares no variable with the rows before it.
    CrossProduct,
}

impl Cause {
    pub fn code(self) -> &'static str {
        match self {
            Cause::DeadVariable => "DEAD_VARIABLE_REJOINED",
            Cause::InvariantInRecursion => "INVARIANT_IN_RECURSION",
            Cause::Mispredicted => "MISPREDICTED_ORDER",
            Cause::TestAfterWalk => "TEST_AFTER_WALK",
            Cause::Dropped => "ROWS_DROPPED",
            Cause::CrossProduct => "CROSS_PRODUCT",
        }
    }
}

/// A rule that made rows it dropped: where, how much, why, and the fix written out.
#[derive(Clone, Debug)]
pub struct Dropped {
    pub cause: Cause,
    pub head_rel: String,
    pub rule: Option<usize>,
    pub authored: Option<String>,
    pub stratum: String,
    pub kind: String,
    /// CPU seconds and rows thrown away, summed over the rule's plans with this cause.
    pub lost_cpu: f64,
    pub lost_rows: u64,
    /// The costliest plan: its steps marked, the source and the drop (0-based).
    pub plan: Vec<Step>,
    pub source: usize,
    pub drop: usize,
    pub loops: u64,
    /// What is wrong, in a sentence, and the rewrite (rule text), or the planner's side for a misprediction.
    pub why: String,
    pub fix: String,
    /// The CPU the shown plan lost: the costliest of the rule's plans is the one shown.
    pub shown_cpu: f64,
}

/// The id bounds the split adds to a chunk's walks (`$n < 6917529027641081856`, `$d < 1610612736`): the engine's
/// own tests, not the rule's, so left out of a rewrite.
fn engine_test(literal: &str) -> bool {
    let mut w = literal.split_whitespace();
    matches!((w.next(), w.next(), w.next(), w.next()), (Some(v), Some("<"), Some(n), None)
        if v.starts_with('$') && n.len() >= 9 && n.bytes().all(|b| b.is_ascii_digit()))
}

fn recursive(source: &str) -> bool {
    source == "delta (last wave)" || source == "this stratum"
}

/// A literal as rule text: the engine's `$` before each variable dropped.
fn plain(literal: &str) -> String {
    literal.replace('$', "")
}

/// A helper relation's name: the head's, and the variables it keeps.
fn helper_name(head_rel: &str, vars: &[String]) -> String {
    let base = head_rel.trim_start_matches("magic_").trim_start_matches("demand_");
    format!("{base}_{}", vars.iter().map(|v| v.trim_start_matches('$')).collect::<Vec<_>>().join("_"))
}

/// The variables in order of first appearance, over `steps`.
fn bound_by(steps: &[Step]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    steps.iter().flat_map(|s| s.vars.iter()).filter(|v| seen.insert((*v).clone())).cloned().collect()
}

/// The rule's literals as text, its id bounds left out.
fn body(steps: &[Step]) -> Vec<String> {
    steps.iter().filter(|s| !engine_test(&s.literal)).map(|s| plain(&s.literal)).collect()
}

fn rewrite(head: &str, helper: &str, keep: &[String], made_from: &[Step], before: &[Step], after: &[Step]) -> String {
    let vars = keep.iter().map(|v| plain(v)).collect::<Vec<_>>().join(", ");
    let mut rest = body(before);
    rest.push(format!("{helper}({vars})"));
    rest.extend(body(after));
    format!(
        ".decl {helper}({})\n{helper}({vars}) :- {}.\n{} :- {}.",
        keep.iter().map(|v| format!("{}: symbol", plain(v))).collect::<Vec<_>>().join(", "),
        body(made_from).join(", "),
        plain(head),
        rest.join(", ")
    )
}

/// The drops of one plan: (source, drop) pairs, each with the rows it threw away and the CPU that cost.
fn drops(p: &Plan) -> Vec<(usize, usize, u64, f64)> {
    let mut out: Vec<(usize, usize, u64, f64)> = Vec::new();
    for (j, s) in p.steps.iter().enumerate() {
        if s.rows_in < DROP_ROWS || s.rows_out.saturating_mul(DROP_RATIO) > s.rows_in {
            continue;
        }
        let Some(k) = (1..j).rev().find(|&i| p.steps[i].rows_out > p.steps[i].rows_in) else { continue };
        // A source's rows are one finding, at the first step that drops them: a later drop of the same rows has
        // the same cause, and moving the first fixes both.
        if out.iter().any(|&(source, ..)| source == k) {
            continue;
        }
        let share = 1.0 - s.rows_out as f64 / s.rows_in as f64;
        let cpu: f64 = p.steps[k..=j].iter().map(|s| s.cpu).sum();
        out.push((k, j, s.rows_in - s.rows_out, share * cpu));
    }
    out
}

/// Why plan `p` made at step `k` the rows step `j` dropped, and the fix.
fn cause(p: &Plan, k: usize, j: usize) -> (Cause, String, String) {
    let st = &p.steps;
    let before: BTreeSet<&String> = st[..k].iter().flat_map(|s| s.vars.iter()).collect();
    let read_later: BTreeSet<&String> = st[k..].iter().flat_map(|s| s.vars.iter()).chain(p.head_vars.iter()).collect();
    // Inside a recursion: led by the stratum's delta, the source and drop reading nothing of the stratum.
    let led = st.first().is_some_and(|s| s.source == "delta (last wave)");
    let invariant = st[k..=j].iter().all(|s| !recursive(&s.source));
    if led && p.loops > 1 && invariant && st[k..=j].iter().any(|s| !s.source.is_empty()) {
        // The helper takes every literal from the source that reads nothing of the stratum, past the drop too: what
        // the rest of the rule tests of the rows made is as invariant as the rows.
        let end = (j + 1..st.len()).find(|&i| recursive(&st[i].source)).unwrap_or(st.len());
        let made_from = &st[k..end];
        let mine: BTreeSet<&String> = made_from.iter().flat_map(|s| s.vars.iter()).collect();
        let outside: BTreeSet<&String> = st[..k].iter().chain(&st[end..]).flat_map(|s| s.vars.iter()).chain(p.head_vars.iter()).collect();
        let keep: Vec<String> = bound_by(made_from).into_iter().filter(|v| mine.contains(v) && outside.contains(v)).collect();
        let helper = helper_name(&p.head_rel, &keep);
        let why = format!(
            "each round joins its new rows to {} and only then keeps the few that {} allows; neither reads {}, so the same rows are made again every round ({} loops)",
            plain(&st[k].literal),
            plain(&st[j].literal),
            p.stratum,
            p.loops
        );
        return (Cause::InvariantInRecursion, why, rewrite(&p.head, &helper, &keep, made_from, &st[..k], &st[end..]));
    }
    // A variable bound before the source that nothing from the source on reads.
    let dead: Vec<&String> = before.iter().copied().filter(|v| !read_later.contains(v) && !v.starts_with("$rr_")).collect();
    if !dead.is_empty() {
        let keep: Vec<String> = bound_by(&st[..k]).into_iter().filter(|v| !dead.contains(&v)).collect();
        let helper = helper_name(&p.head_rel, &keep);
        let why = format!(
            "{} is read by no step from {} on and not by the head, so each of its values joins {} again",
            dead.iter().map(|v| plain(v)).collect::<Vec<_>>().join(", "),
            plain(&st[k].literal),
            plain(&st[k].literal)
        );
        return (Cause::DeadVariable, why, rewrite(&p.head, &helper, &keep, &st[..k], &[], &st[k..]));
    }
    // The source's rows per loop against the planner's estimate.
    let loops = p.loops.max(1) as f64;
    let actual = st[k].rows_out as f64 / loops;
    if st[k].est > 0.0 && actual >= st[k].est * MISPREDICTED {
        let narrower = st[k].passed.iter().min_by(|a, b| a.1.total_cmp(&b.1));
        let why = format!(
            "the planner expected {:.0} rows after {} and it made {:.0} a loop, priced by {}",
            st[k].est,
            plain(&st[k].literal),
            actual,
            if st[k].basis.is_empty() { "an unnamed basis" } else { &st[k].basis }
        );
        let fix = match narrower {
            Some((lit, rows)) => format!(
                "The planner's estimate is the fault: {} was passed over here at {rows:.0} rows a binding. Joined first, it bounds {} by its own rows. Fix the estimate's basis ({}); a rule can only reorder by its bindings.",
                plain(lit),
                plain(&st[k].literal),
                st[k].basis
            ),
            None => format!("The planner's estimate is the fault; its basis is {}.", st[k].basis),
        };
        return (Cause::Mispredicted, why, fix);
    }
    if st[j].source.is_empty() && st[j].literal.contains('@') {
        let why = format!("{} tests a value made for each row, which no index can look up, after {} made the rows", plain(&st[j].literal), plain(&st[k].literal));
        let mut fix = "Bind the value through a relation that holds it, looked up by the value, or join first a literal that bounds the rows: a function's value is only tested.".to_owned();
        if let Some((lit, rows)) = st[k].passed.iter().min_by(|a, b| a.1.total_cmp(&b.1)) {
            fix.push_str(&format!(" The planner passed over {} at step {} ({rows:.0} rows a binding): led by it, {} is a lookup of each of its rows.", plain(lit), k + 1, plain(&st[k].literal)));
        }
        return (Cause::TestAfterWalk, why, fix);
    }
    let why = format!("{} makes rows that {} drops", plain(&st[k].literal), plain(&st[j].literal));
    // A join that drops, sharing a variable with the rows before the source, could have been joined there.
    let shared: Vec<&String> = st[j].vars.iter().filter(|v| before.contains(v) && !v.starts_with("$rr_")).collect();
    let joinable = !st[j].source.is_empty() && !st[j].access.starts_with("nearest");
    let mut fix = if joinable && !shared.is_empty() {
        format!(
            "{} shares {} with the rows before step {}, so it could be joined there, before {} makes its rows. The planner priced {} at {:.0} rows a loop{}, and it made {:.0}.",
            plain(&st[j].literal),
            shared.iter().map(|v| plain(v)).collect::<Vec<_>>().join(", "),
            k + 1,
            plain(&st[k].literal),
            plain(&st[k].literal),
            st[k].est,
            if st[k].basis.is_empty() { String::new() } else { format!(" ({})", st[k].basis) },
            actual
        )
    } else {
        format!("The rows {} makes are tested by {} alone: bind what the test reads before the walk.", plain(&st[k].literal), plain(&st[j].literal))
    };
    if !st[k].passed.is_empty() {
        let passed = st[k].passed.iter().map(|(l, r)| format!("{} at {r:.0}", plain(l))).collect::<Vec<_>>().join(", ");
        fix.push_str(&format!(" Passed over at step {}, in rows a binding: {passed}.", k + 1));
    }
    (Cause::Dropped, why, fix)
}

/// A positive join after the first step that shares no variable with the rows before it, over at least `DROP_ROWS`.
fn cross_products(p: &Plan) -> Option<(usize, u64, f64)> {
    let mut seen: BTreeSet<&String> = BTreeSet::new();
    for (i, s) in p.steps.iter().enumerate() {
        let join = !s.source.is_empty() && !s.access.starts_with("anti-join") && !s.access.starts_with("nearest");
        if i > 0 && join && !s.vars.is_empty() && s.vars.iter().all(|v| !seen.contains(v)) && s.rows_out >= DROP_ROWS && s.rows_out > s.rows_in {
            return Some((i, s.rows_out - s.rows_in, s.cpu));
        }
        seen.extend(s.vars.iter());
    }
    None
}

/// The rules that made rows they dropped, costliest first.
pub fn analyze(plans: &[Plan]) -> Vec<Dropped> {
    let mut by: BTreeMap<(String, Option<usize>, Cause), Dropped> = BTreeMap::new();
    let mut add = |p: &Plan, cause: Cause, k: usize, j: usize, rows: u64, cpu: f64, why: String, fix: String| {
        let e = by.entry((p.head_rel.clone(), p.rule, cause)).or_insert_with(|| Dropped {
            cause,
            head_rel: p.head_rel.clone(),
            rule: p.rule,
            authored: p.authored.clone(),
            stratum: p.stratum.clone(),
            kind: p.kind.clone(),
            lost_cpu: 0.0,
            lost_rows: 0,
            plan: p.steps.clone(),
            source: k,
            drop: j,
            loops: p.loops,
            why: why.clone(),
            fix: fix.clone(),
            shown_cpu: 0.0,
        });
        let costliest = cpu > e.shown_cpu;
        e.lost_cpu += cpu;
        e.lost_rows += rows;
        if costliest {
            (e.plan, e.source, e.drop, e.loops, e.why, e.fix, e.stratum, e.kind, e.shown_cpu) = (p.steps.clone(), k, j, p.loops, why, fix, p.stratum.clone(), p.kind.clone(), cpu);
        }
    };
    for p in plans {
        if let Some((i, rows, cpu)) = cross_products(p) {
            let why = format!("{} shares no variable with the rows before it, so it pairs each with every row of its relation", plain(&p.steps[i].literal));
            add(p, Cause::CrossProduct, i, i, rows, cpu, why, "Bind one of its variables before it, through a literal that joins it to the rows so far; a product the rule means is written with its bound first.".into());
        }
        for (k, j, rows, cpu) in drops(p) {
            let (c, why, fix) = cause(p, k, j);
            add(p, c, k, j, rows, cpu, why, fix);
        }
    }
    let mut out: Vec<Dropped> = by.into_values().filter(|d| d.lost_cpu >= LOST_CPU || d.cause == Cause::CrossProduct).collect();
    out.sort_by(|a, b| b.lost_cpu.total_cmp(&a.lost_cpu).then(a.head_rel.cmp(&b.head_rel)));
    out
}

/// A finding's plan, its source and drop marked.
pub fn shown(d: &Dropped) -> String {
    let mut r = String::new();
    for (i, s) in d.plan.iter().enumerate() {
        let mark = if i == d.drop && i == d.source { "  <- here" } else if i == d.drop { "  <- drops" } else if i == d.source { "  <- makes" } else { "" };
        r.push_str(&format!("      {:>2}. {:<44} {:>10} -> {:<10} est {:>9.0}{mark}\n", i + 1, plain(&s.literal), s.rows_in, s.rows_out, s.est));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(literal: &str, source: &str, rows: (u64, u64), est: f64, cpu_ms: f64) -> Step {
        let vars = literal.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).filter(|w| w.starts_with('$')).map(str::to_owned).collect();
        let access = if source.is_empty() { "filter" } else if literal.starts_with('!') { "anti-join on (c0)" } else { "lookup on (c0)" };
        Step { literal: literal.into(), access: access.into(), source: source.into(), vars, est, rows_in: rows.0, rows_out: rows.1, cpu: cpu_ms / 1e3, ..Default::default() }
    }

    fn plan(kind: &str, head: &str, loops: u64, steps: Vec<Step>) -> Plan {
        let head_rel = head.split('(').next().unwrap().to_owned();
        let head_vars = step(head, "x", (0, 0), 0.0, 0.0).vars;
        Plan { stratum: "s".into(), kind: kind.into(), rule: Some(1), head: head.into(), head_rel, head_vars, loops, steps, ..Default::default() }
    }

    fn only(plans: &[Plan]) -> Dropped {
        let found = analyze(plans);
        assert_eq!(found.len(), 1, "{found:#?}");
        found.into_iter().next().unwrap()
    }

    /// 0691 fix 19's rule before the fix, as this branch's points_to ran it over 291 PostHog files: each round's new
    /// scope terms walked every node under the scope, and only then were the definitions kept. The rewrite is fix
    /// 19's `pt_tm_child`, made once outside the recursion.
    #[test]
    fn an_invariant_join_inside_a_recursion_is_made_once_outside_it() {
        let p = plan("join pt_scope_tm (delta pt_scope_tm)", "pt_scope_tm($f, @dot($q, $x))", 3, vec![
            step("pt_scope_tm($s, $q)", "delta (last wave)", (21, 4460), 20.0, 0.3),
            step("rr_nearest_pt_scope($f, $s)", "input", (4460, 560364), 2497.0, 62.7),
            step("$f < 6917529027641081856", "", (560364, 559787), 2400.0, 0.0),
            step("kind($f, $k)", "derived", (559787, 559782), 2400.0, 124.5),
            step("pt_tm_def($k)", "derived", (559782, 4168), 30.0, 51.4),
            step("field($f, #f65555, $nm)", "input", (4168, 4168), 30.0, 2.4),
            step("$x = @rr_node_text_any($nm)", "", (4168, 4168), 30.0, 2.5),
            step("pt_tm_ord($f, 0)", "derived", (4168, 4167), 30.0, 1.5),
        ]);
        let d = only(&[p]);
        assert_eq!(d.cause, Cause::InvariantInRecursion);
        assert_eq!((d.source, d.drop), (1, 4));
        assert!((d.lost_cpu - 0.2373).abs() < 1e-3, "{}", d.lost_cpu);
        assert_eq!(
            d.fix,
            ".decl pt_scope_tm_f_s_x(f: symbol, s: symbol, x: symbol)\n\
             pt_scope_tm_f_s_x(f, s, x) :- rr_nearest_pt_scope(f, s), kind(f, k), pt_tm_def(k), field(f, #f65555, nm), x = @rr_node_text_any(nm), pt_tm_ord(f, 0).\n\
             pt_scope_tm(f, @dot(q, x)) :- pt_scope_tm(s, q), pt_scope_tm_f_s_x(f, s, x)."
        );
        // Fix 19's rule: the round's new terms join their definitions by the scope; nothing is dropped.
        let fixed = plan("join pt_scope_tm (delta pt_scope_tm)", "pt_scope_tm($f, @dot($q, $x))", 3, vec![
            step("pt_scope_tm($s, $q)", "delta (last wave)", (21, 4460), 20.0, 0.3),
            step("pt_tm_child($f, $s, $x, 0)", "derived", (4460, 4167), 30.0, 1.0),
        ]);
        assert!(analyze(&[fixed]).is_empty());
    }

    /// pt_type_miss: `t0` only tests that `n` has a type `u` covers, but each of its values joins every type of `n`
    /// again (1.15 M rows for 30,412 kept, 0.53 s).
    #[test]
    fn a_dead_variable_that_multiplies_rows_is_projected_away_first() {
        let p = plan("seed pt_type_miss", "pt_type_miss($n, $u)", 1, vec![
            step("type_of($n, $t0)", "derived", (1, 45318), 45318.0, 0.0),
            step("pt_tsub($n, $t0, $u)", "derived", (45318, 132413), 100000.0, 0.0),
            step("type_of($n, $t)", "derived", (132413, 1148767), 1000000.0, 43.5),
            step("!pt_tsub($n, $t, $u)", "derived", (1148767, 30412), 30000.0, 531.5),
        ]);
        let d = only(&[p]);
        assert_eq!(d.cause, Cause::DeadVariable);
        assert!(d.why.starts_with("t0 is read by no step"), "{}", d.why);
        assert_eq!(
            d.fix,
            ".decl pt_type_miss_n_u(n: symbol, u: symbol)\n\
             pt_type_miss_n_u(n, u) :- type_of(n, t0), pt_tsub(n, t0, u).\n\
             pt_type_miss(n, u) :- pt_type_miss_n_u(n, u), type_of(n, t), !pt_tsub(n, t, u)."
        );
    }

    /// pt_rbound's seed: each demanded name walks every node under its scope and tests its text (7.3 M rows for
    /// 32,172, 1.9 s). The fix names the join the planner passed over.
    #[test]
    fn a_test_after_a_walk_names_the_narrower_join_passed_over() {
        let mut walk = step("rr_nearest_ks8($t, $s)", "input", (10947, 7303978), 709649.0, 423.7);
        walk.passed = vec![("pt_tgt($t)".into(), 2050.0), ("kind($t, #k1025)".into(), 69183.0)];
        let p = plan("seed pt_rbound", "pt_rbound($s, $x)", 2, vec![
            step("demand_pt_rbound($s, $x)", "derived", (9, 10947), 5449.0, 1.2),
            walk,
            step("$x = @rr_node_text_any($t)", "", (7303978, 32172), 283860.0, 1457.0),
            step("pt_tgt($t)", "derived", (32172, 216), 8396.0, 8.2),
        ]);
        let d = only(&[p]);
        assert_eq!(d.cause, Cause::TestAfterWalk);
        assert!(d.fix.contains("passed over pt_tgt(t) at step 2 (2050 rows a binding)"), "{}", d.fix);
    }

    /// Ten times the rows the planner expected at the source is its estimate's fault, and says so.
    #[test]
    fn a_misprediction_names_the_estimate_and_its_basis() {
        let mut walk = step("node($rr_shown1, $n, _, $b, $e)", "input", (183070, 2520250), 2000.0, 115.4);
        walk.basis = "postings".into();
        walk.passed = vec![("child($p, $n, _)".into(), 1.0)];
        let p = plan("join pt_tm_same (delta pt_tm_same)", "pt_tm_same($n, $c)", 1, vec![
            step("pt_tm_same($p, $c)", "delta (last wave)", (10, 183070), 100.0, 7.0),
            step("node($rr_shown0, $p, _, $b, $e)", "input", (183070, 183070), 180000.0, 77.0),
            walk,
            step("child($p, $n, _)", "input", (2520250, 15590), 100.0, 67.1),
        ]);
        let d = only(&[p]);
        assert_eq!(d.cause, Cause::Mispredicted);
        assert!(d.why.contains("expected 2000 rows") && d.why.contains("priced by postings"), "{}", d.why);
        assert!(d.fix.contains("child(p, n, _) was passed over here at 1 rows a binding"), "{}", d.fix);
    }

    /// Rows a step makes and keeps are no waste, however many: pt_tm's term for every node.
    #[test]
    fn a_fan_out_whose_rows_are_kept_is_not_reported() {
        let p = plan("seed pt_tm", "pt_tm($n, $q)", 1, vec![
            step("pt_scope_tm($s, $q)", "derived", (1, 2158), 2000.0, 0.1),
            step("rr_nearest_ks9($n, $s)", "input", (2158, 286331), 280000.0, 70.0),
            step("!host_node($n, _)", "input", (286331, 286330), 280000.0, 50.0),
        ]);
        assert!(analyze(&[p]).is_empty());
    }

    /// pt_tm_same: a node looked up by its span grows the rows, and `child(p, n)` then keeps one in sixteen. `child`
    /// shares `p` with the rows before the lookup, so the fix says it could be joined there, with the planner's price.
    #[test]
    fn a_drop_that_could_be_joined_before_its_source_says_so() {
        let mut span = step("node($rr_shown1, $n, _, $b, $e)", "input", (183070, 252025), 49231.0, 115.4);
        span.basis = "postings".into();
        let p = plan("join pt_tm_same (delta pt_tm_same)", "pt_tm_same($n, $c)", 1, vec![
            step("pt_tm_same($p, $c)", "delta (last wave)", (10, 186381), 51261.0, 7.0),
            step("node($rr_shown0, $p, _, $b, $e)", "input", (186381, 183071), 50236.0, 77.0),
            span,
            step("child($p, $n, _)", "input", (251987, 15590), 47282.0, 67.1),
        ]);
        let d = only(&[p]);
        assert_eq!(d.cause, Cause::Dropped);
        assert!(
            d.fix.starts_with("child(p, n, _) shares p with the rows before step 3, so it could be joined there, before node(rr_shown1, n, _, b, e) makes its rows. The planner priced node(rr_shown1, n, _, b, e) at 49231 rows a loop (postings), and it made 252025."),
            "{}",
            d.fix
        );
    }

    /// A delta's scan that a test narrows reads what it must: magic_pt_tm_same_bf's leaves (926,766 rows, 582 kept).
    #[test]
    fn a_scan_narrowed_by_a_test_is_not_reported() {
        let p = plan("join pt_tm_same (delta magic_pt_tm_same_bf)", "pt_tm_same($n, $c)", 6, vec![
            step("magic_pt_tm_same_bf($n)", "delta (last wave)", (29, 926766), 56592.0, 13.3),
            step("!child(_, $n, _)", "input", (926766, 582), 53762.0, 137.0),
        ]);
        assert!(analyze(&[p]).is_empty());
    }

    /// A join that shares no variable with the rows before it pairs each row with all of its relation's.
    #[test]
    fn a_cross_product_is_reported_whatever_its_cost() {
        let p = plan("seed pha_latest", "pha_latest($x, $v)", 1, vec![
            step("pha_write($x, $k)", "derived", (1, 300), 300.0, 0.1),
            step("visit_index($v, $j)", "input", (300, 60000), 300.0, 1.0),
            step("$j = $k", "", (60000, 300), 300.0, 0.2),
        ]);
        let found = analyze(&[p]);
        assert!(found.iter().any(|d| d.cause == Cause::CrossProduct && d.source == 1), "{found:#?}");
    }
}
