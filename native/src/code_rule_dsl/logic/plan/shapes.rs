//! The planner's estimates from relations' layouts (decision 0361 step 4): a column's distinct values from its
//! relation's facts, declared or inferred (`program_image::layouts`), where they answer it, and no statistics made for
//! a relation whose every column they answer.
use crate::code_rule_dsl::logic::front::{FEW, Fan, LayoutFact, LayoutSize};
use std::collections::HashMap;

/// A `few` probe's rows, as the planner prices it: the class bounds it at `FEW` (16), and the trees' parents have
/// about three children (the batch estimate this replaced priced a parent column at a third of its rows).
pub const FEW_ROWS: f64 = 3.0;

/// Each relation's declared width and facts, by its name: every form of it (its deltas, its `__old`) is named apart
/// and takes none.
#[derive(Clone, Debug, Default)]
pub struct Shapes(pub HashMap<String, (usize, Vec<LayoutFact>)>);

/// The rows a probe of a fan-out's columns returns, as priced; `None` for `many` (statistics price it).
fn probe_rows(fan: Fan) -> Option<f64> {
    match fan {
        Fan::One => Some(1.0),
        Fan::Few => Some(FEW_ROWS),
        Fan::About(n) => Some(f64::from(n.max(1))),
        Fan::Many => None,
    }
}

/// The most rows a probe of a fan-out's columns returns, by its class; `None` for `many`.
fn probe_bound(fan: Fan) -> Option<f64> {
    match fan.class() {
        Fan::One => Some(1.0),
        Fan::Few => Some(f64::from(FEW)),
        _ => None,
    }
}

impl Shapes {
    /// The facts of `v`'s relations (`Layouts`), by name.
    pub fn of_version(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Shapes {
        use crate::code_rule_dsl::logic::program_image::{layouts::Layouts, manager::analysis};
        // `RERAIL_LAYOUT_ESTIMATES=off` plans from statistics alone, as before (a measurement aid).
        if std::env::var("RERAIL_LAYOUT_ESTIMATES").is_ok_and(|x| x == "off") {
            return Shapes::default();
        }
        // A program the analysis refuses (its strata) has no shapes: it plans from statistics, as before.
        let Ok(l) = analysis::<Layouts>(v) else { return Shapes::default() };
        Shapes(
            l.0.iter()
                .map(|(rel, facts)| {
                    let name = v.rel_name(*rel);
                    let width = v.decl_of(name).map_or(0, |d| d.cols().count());
                    (name.to_owned(), (width, facts.iter().map(|(f, _)| f.clone()).collect()))
                })
                .collect(),
        )
    }

    /// Column `c` of `rel`'s distinct values, of `rows` rows, from its facts: a key's column has one a row; a
    /// size `per c` one per `most` rows; a fan-out's source one per probe's rows; a dense order within a fan-out's
    /// source as many as a probe returns at most. `None` where no fact answers it.
    pub fn ndv(&self, rel: &str, c: usize, rows: f64) -> Option<f64> {
        let (_, facts) = self.0.get(rel)?;
        let rows = rows.max(1.0);
        let by = |f: &LayoutFact| -> Option<f64> {
            match f {
                LayoutFact::Key(k) if k.as_slice() == [c] => Some(rows),
                LayoutFact::Size(LayoutSize::Per { most, col }) if *col == c => Some((rows / f64::from((*most).max(1))).max(1.0)),
                LayoutFact::Fanout { from, fan, .. } if from.as_slice() == [c] => probe_rows(*fan).map(|n| (rows / n).max(1.0)),
                LayoutFact::Order { col, within, dense: true } if *col == c => facts.iter().find_map(|g| match g {
                    LayoutFact::Fanout { from, fan, .. } if from == within => probe_bound(*fan).map(|n| rows.min(n)),
                    _ => None,
                }),
                _ => None,
            }
        };
        // The most distinct values any fact gives (a key and a fan-out of one column: the key's).
        facts.iter().filter_map(by).reduce(f64::max)
    }

    /// Whether every one of `rel`'s columns is answered (`ndv`): its statistics are not made.
    pub fn answers(&self, rel: &str) -> bool {
        self.0.get(rel).is_some_and(|(width, _)| *width > 0 && (0..*width).all(|c| self.ndv(rel, c, 2.0).is_some()))
    }
}
