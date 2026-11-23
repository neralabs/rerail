//! A join step's CPU by the relation it reads (Q36): placement prices a stratum's CPU by its planned work, one unit
//! a binding into a step (decision 0304), but a view's row can cost far more than a unit (a node's text, its line).
//! The engine times each step of a profiled walk (`setfix::StepClock`); here those times are summed by the step's
//! relation, how it is read and what holds it, with the rows into and out of it, so a view's price can be read off.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::State;

/// A step's relation: its literal's name, a negation's without the `!` (a filter's literal is kept whole).
fn relation(literal: &str) -> &str {
    let l = literal.trim_start_matches('!');
    l.split_once('(').map_or(literal, |(name, _)| name.trim())
}

#[derive(Default)]
struct Total {
    steps: u64,
    rows_in: u64,
    rows_out: u64,
    cpu: f64,
}

/// The report's section, and `sources.tsv`: per (relation, access, source), the plans' steps that read it, their
/// rows in and out and CPU, by CPU; and the head's rows made, the part of a walk no step holds. `unit` is
/// placement's CPU price of a unit of planned work, in seconds (`placement::CPU_PER_WORK`).
pub fn by_source(r: &mut String, state: &State, unit: f64) -> String {
    let mut by: BTreeMap<(String, String, String), Total> = BTreeMap::new();
    let (mut head, mut timed) = (0.0, 0.0);
    for t in state.plans.values() {
        if t.cpu.is_empty() {
            continue;
        }
        for (i, st) in t.plan.steps.iter().enumerate() {
            let (Some(&c), Some(&(a, b))) = (t.cpu.get(i), t.actual.get(i)) else { continue };
            let e = by.entry((relation(&st.literal).to_owned(), st.access.clone(), st.source.clone())).or_default();
            e.steps += 1;
            e.rows_in += a;
            e.rows_out += b;
            e.cpu += c;
            timed += c;
        }
        if t.cpu.len() == t.actual.len() + 1 {
            head += t.cpu.last().copied().unwrap_or(0.0);
        }
    }
    let mut rows: Vec<_> = by.into_iter().collect();
    rows.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
    let unit = unit * 1e9;
    // Per row in, a lookup's or a test's price; per row out, a scan's (one row in, its relation's rows out).
    let per = |cpu: f64, n: u64| if n > 0 { cpu * 1e9 / n as f64 } else { 0.0 };
    let mut tsv = String::from("relation\taccess\tsource\tsteps\trows_in\trows_out\tcpu_s\tns_per_row_in\tns_per_row_out\tunits_per_row_in\n");
    for ((rel, access, source), t) in &rows {
        let (i, o) = (per(t.cpu, t.rows_in), per(t.cpu, t.rows_out));
        let _ = writeln!(tsv, "{rel}\t{access}\t{source}\t{}\t{}\t{}\t{:.6}\t{i:.1}\t{o:.1}\t{:.2}", t.steps, t.rows_in, t.rows_out, t.cpu, i / unit);
    }
    let _ = writeln!(
        r,
        "\n== Join steps' CPU by relation read ({timed:.3} s in steps, {head:.3} s making the head's rows); a unit of planned work is priced at {unit:.0} ns (decision 0304)"
    );
    if rows.is_empty() {
        r.push_str("(no step was timed)\n");
        return tsv;
    }
    let _ = writeln!(r, "{:>9} {:>11} {:>11} {:>9} {:>9} {:>7}  relation / access / source", "cpu ms", "rows in", "rows out", "ns in", "ns out", "units in");
    for ((rel, access, source), t) in rows.iter().take(40) {
        let (i, o) = (per(t.cpu, t.rows_in), per(t.cpu, t.rows_out));
        let _ = writeln!(r, "{:>9.1} {:>11} {:>11} {i:>9.0} {o:>9.0} {:>7.1}  {rel} / {access} / {source}", t.cpu * 1e3, t.rows_in, t.rows_out, i / unit);
    }
    tsv
}

#[cfg(test)]
mod tests {
    use super::relation;

    #[test]
    fn a_steps_relation_is_its_literals_name() {
        assert_eq!(relation("rr_nearest_scope($f, $s)"), "rr_nearest_scope");
        assert_eq!(relation("!kind($s, #k1132)"), "kind");
        assert_eq!(relation("$a = $b"), "$a = $b");
    }
}
