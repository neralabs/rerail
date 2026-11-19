//! The planner's estimates from relations' layouts (decision 0361 step 4): a column's distinct values from its
//! relation's facts, declared or inferred (`program_image::layouts`), where they answer it, and no statistics made for
//! a relation whose every column they answer.
use crate::code_rule_dsl::logic::front::{FEW, Fan, LayoutFact, LayoutSize, LayoutValues};
use std::collections::HashMap;

/// A `few` probe's rows, as the planner prices it: the class bounds it at `FEW` (16), and the trees' parents have
/// about three children (the batch estimate this replaced priced a parent column at a third of its rows).
pub const FEW_ROWS: f64 = 3.0;

/// Each relation's declared columns' names and its facts, by its name: every form of it (its deltas, its `__old`) is
/// named apart and takes none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shapes(pub HashMap<String, (Vec<String>, Vec<LayoutFact>)>);

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

/// The run's exact counts a `values` fact is priced by (decision 0361 item 5): the documents and the grammar's kinds
/// its statistics hold (`Stats::counts`); `None` where they are not known, and a fact of them answers nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub docs: Option<f64>,
    pub kinds: Option<f64>,
}

/// An analysis of the version (`manager::analysis`): made once a version, and shared by every plan made of it (a
/// session's runs make tens of thousands; made per plan, it was a third of `make_plan`'s own CPU).
impl crate::code_rule_dsl::logic::program_image::manager::Analysis for Shapes {
    const NAME: &'static str = "shapes";
    const READS: crate::code_rule_dsl::logic::program_image::manager::Reads = crate::code_rule_dsl::logic::program_image::manager::Reads::RULES.or(crate::code_rule_dsl::logic::program_image::manager::Reads::ABOUT);
    fn compute(cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>) -> Result<Shapes, String> {
        let l = cx.get::<crate::code_rule_dsl::logic::program_image::layouts::Layouts>()?;
        Ok(Shapes::of_layouts(cx.v(), &l))
    }
}

impl Shapes {
    /// The facts of `v`'s relations (`Layouts`), by name: the version's analysis, shared.
    pub fn of_version(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> std::sync::Arc<Shapes> {
        // `RERAIL_LAYOUT_ESTIMATES=off` plans from statistics alone, as before (a measurement aid).
        if std::env::var("RERAIL_LAYOUT_ESTIMATES").is_ok_and(|x| x == "off") {
            return Default::default();
        }
        // A program the analysis refuses (its strata) has no shapes: it plans from statistics, as before.
        crate::code_rule_dsl::logic::program_image::manager::analysis::<Shapes>(v).unwrap_or_default()
    }

    fn of_layouts(v: &crate::code_rule_dsl::logic::program_image::Version, l: &crate::code_rule_dsl::logic::program_image::layouts::Layouts) -> Shapes {
        Shapes(
            l.0.iter()
                .map(|(rel, facts)| {
                    let name = v.rel_name(*rel);
                    let cols = v.decl_of(name).map_or_else(Vec::new, |d| d.cols().map(|(c, _)| v.text(c).to_owned()).collect());
                    (name.to_owned(), (cols, facts.iter().map(|(f, _)| f.clone()).collect()))
                })
                .collect(),
        )
    }

    /// Column `c` of `rel`'s distinct values, of `rows` rows, from its facts: a key's column has one a row; a
    /// size `per c` one per `most` rows; a fan-out's source one per probe's rows; a dense order within a fan-out's
    /// source as many as a probe returns at most; a `values` fact by its class, of the run's `counts`. `None` where no
    /// fact answers it.
    pub fn ndv(&self, rel: &str, c: usize, rows: f64, counts: Counts) -> Option<f64> {
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
                LayoutFact::Values { col, values } if *col == c => match values {
                    LayoutValues::Documents => counts.docs.map(|d| rows.min(d.max(1.0))),
                    LayoutValues::Kinds => counts.kinds.map(|k| rows.min(k.max(1.0))),
                    LayoutValues::WithinDocument => counts.docs.map(|d| (rows / d.max(1.0)).max(1.0)),
                    LayoutValues::Many => Some(rows),
                    LayoutValues::About(n) => Some(rows.min(f64::from((*n).max(1)))),
                    LayoutValues::OneIn(n) => Some((rows / f64::from((*n).max(1))).max(1.0)),
                },
                _ => None,
            }
        };
        // The most distinct values any fact gives (a key and a fan-out of one column: the key's).
        facts.iter().filter_map(by).reduce(f64::max)
    }

    /// Whether every one of `rel`'s columns is answered (`ndv`): its statistics are not made.
    pub fn answers(&self, rel: &str, counts: Counts) -> bool {
        self.0.get(rel).is_some_and(|(cols, _)| !cols.is_empty() && (0..cols.len()).all(|c| self.ndv(rel, c, 2.0, counts).is_some()))
    }

    /// The place of `rel`'s declared column `name` (a source's spelling: `end_` is `end`).
    pub fn place(&self, rel: &str, name: &str) -> Option<usize> {
        let (cols, _) = self.0.get(rel)?;
        cols.iter().position(|c| c == name).or_else(|| cols.iter().position(|c| c == name.trim_end_matches('_')))
    }

    /// A column's statistic as `syntax::measured` gives it, `(rows, ndv, sq, sqd)`, from its facts where they answer
    /// it: postings taken even (`sq = rows² / ndv`), and within documents a document's share of them (one document's
    /// alone where the column is the documents).
    pub fn statistic(&self, rel: &str, name: &str, rows: i64, counts: Counts) -> Option<(i64, i64, i64, i64)> {
        let c = self.place(rel, name)?;
        let r = rows.max(0) as f64;
        let ndv = self.ndv(rel, c, r, counts)?.max(1.0);
        let sq = (r * r / ndv).max(r);
        let documents = self.0.get(rel).is_some_and(|(_, fs)| fs.iter().any(|f| matches!(f, LayoutFact::Values { col, values: LayoutValues::Documents } if *col == c)));
        let sqd = if documents { sq } else { counts.docs.map_or(sq, |d| (sq / d.max(1.0)).max(r)) };
        Some((rows, ndv.round() as i64, sq.round() as i64, sqd.round() as i64))
    }
}
