//! Scalar Boolean projection over explicitly supplied utility/search boundaries.
//! This renderer never executes a matcher, charges work, publishes memo entries,
//! or allocates continuations. Boundary owners retain those responsibilities.
use super::super::ir::Set;
use super::utility_projection_lower::{Expr, Id, Program};

#[derive(Clone, Debug)]
pub(super) struct Summary {
    pub truth: String,
    pub cost: String,
    pub peak: String,
}
impl Summary {
    pub fn atom(truth: String) -> Self {
        Self {
            truth,
            cost: "0".into(),
            peak: "0".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub(super) struct Reached {
    pub expression: Id,
    /// SQL Boolean in the caller's demand scope. Includes ordered short circuit.
    pub guard: String,
}
pub(super) struct Projection {
    pub summary: Summary,
    /// Ordered direct boundaries. A boundary's own recipe owns its descendants.
    pub reached: Vec<Reached>,
    pub scalar_bytes: usize,
}
/// Render one body, with prejoined scalar answers supplied at every Call/Search.
/// A complete summary is observationally usable only when every reached boundary
/// has performed its native entry/return protocol. In particular, completed child
/// publications cannot be replaced by the parent's aggregate cost/peak.
pub(super) fn render(
    program: &Program,
    entry: Id,
    atom: impl FnMut(&Set) -> Result<String, String>,
    boundary: impl FnMut(Id, &Expr) -> Result<Summary, String>,
) -> Result<Projection, String> {
    render_mode(program, entry, atom, boundary, false)
}
pub(super) fn render_mode(
    program: &Program,
    entry: Id,
    mut atom: impl FnMut(&Set) -> Result<String, String>,
    mut boundary: impl FnMut(Id, &Expr) -> Result<Summary, String>,
    complete: bool,
) -> Result<Projection, String> {
    fn walk(
        p: &Program,
        id: Id,
        guard: &str,
        atom: &mut impl FnMut(&Set) -> Result<String, String>,
        boundary: &mut impl FnMut(Id, &Expr) -> Result<Summary, String>,
        reached: &mut Vec<Reached>,
        complete: bool,
    ) -> Result<Summary, String> {
        let expr = p
            .expressions
            .get(id)
            .ok_or("projection expression out of range")?;
        match expr {
            Expr::Atom(set) => Ok(Summary::atom(atom(set)?)),
            Expr::Call(_) | Expr::Search { .. } => {
                reached.push(Reached {
                    expression: id,
                    guard: guard.into(),
                });
                boundary(id, expr)
            }
            Expr::Not(child) => {
                let s = walk(p, *child, guard, atom, boundary, reached, complete)?;
                Ok(Summary {
                    truth: if complete {
                        format!("NOT ({})", s.truth)
                    } else {
                        format!(
                            "CASE ({}) WHEN 0 THEN 1 WHEN 1 THEN 0 ELSE ({}) END",
                            s.truth, s.truth
                        )
                    },
                    ..s
                })
            }
            Expr::All(children) | Expr::Any(children) => {
                let all = matches!(expr, Expr::All(_));
                let decisive = if all { 0 } else { 1 };
                let identity = if all { 1 } else { 0 };
                let mut prefix = "1".to_owned();
                let mut truth = String::from("CASE");
                let mut costs = Vec::new();
                let mut peaks = vec!["0".to_owned()];
                let mut unknown = Vec::new();
                let mut simple_truth = Vec::new();
                for child in children {
                    let child_guard = format!("({guard}) AND ({prefix})");
                    let s = walk(p, *child, &child_guard, atom, boundary, reached, complete)?;
                    truth.push_str(&format!(
                        " WHEN ({prefix}) AND (({})<0 OR ({})={decisive}) THEN ({})",
                        s.truth, s.truth, s.truth
                    ));
                    if s.cost != "0" {
                        costs.push(format!("CASE WHEN ({prefix}) THEN ({}) ELSE 0 END", s.cost));
                    }
                    if s.peak != "0" {
                        peaks.push(format!("CASE WHEN ({prefix}) THEN ({}) ELSE 0 END", s.peak));
                    }
                    unknown.push(format!("(({prefix}) AND ({})=2)", s.truth));
                    simple_truth.push(format!("({})", s.truth));
                    prefix = if complete {
                        format!("({prefix}) AND ({})={identity}", s.truth)
                    } else {
                        format!("({prefix}) AND ({}) IN ({identity},2)", s.truth)
                    };
                }
                if children.is_empty() {
                    return Ok(Summary::atom(identity.to_string()));
                }
                truth.push_str(&format!(
                    " ELSE CASE WHEN {} THEN 2 ELSE {identity} END END",
                    unknown.join(" OR ")
                ));
                Ok(Summary {
                    truth: if complete {
                        simple_truth.join(if all { " AND " } else { " OR " })
                    } else {
                        truth
                    },
                    cost: if costs.is_empty() {
                        "0".into()
                    } else {
                        costs.join("+")
                    },
                    peak: if peaks.len() == 1 {
                        "0".into()
                    } else {
                        format!("max({})", peaks.join(","))
                    },
                })
            }
        }
    }
    let mut reached = Vec::new();
    let summary = walk(
        program,
        entry,
        "1",
        &mut atom,
        &mut boundary,
        &mut reached,
        complete,
    )?;
    let scalar_bytes = summary.truth.len()
        + summary.cost.len()
        + summary.peak.len()
        + reached.iter().map(|r| r.guard.len()).sum::<usize>();
    Ok(Projection {
        summary,
        reached,
        scalar_bytes,
    })
}
