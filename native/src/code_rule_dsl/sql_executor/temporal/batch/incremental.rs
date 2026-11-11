//! SQL finds affected heads and rechecks their support against current inputs.
use super::*;
use crate::code_rule_dsl::logic::analysis::{Decision, deltas};
use crate::code_rule_dsl::logic::plan::estimates::{InputChange, ProjectionCost};
use crate::code_rule_dsl::logic::sql::sql_ast::SqlExpression as E;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum Input {
    Derived(usize),
    External(String),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum VersionKind {
    Added,
    Removed,
    Superset,
}

pub(super) struct Rule {
    pub expression: usize,
    pub inputs: Vec<Input>,
}

struct Atom {
    input: Input,
    current: Rel,
    fields: Vec<String>,
    positions: Vec<usize>,
    plus: usize,
    minus: usize,
    superset: usize,
}

impl Atom {
    fn changed(&self, negative: bool) -> String {
        let slot = if negative { self.minus } else { self.plus };
        let columns = self
            .fields
            .iter()
            .zip(&self.positions)
            .map(|(field, at)| format!("c{at} AS {}", quote(field)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "SELECT {columns} FROM {CHANGES} WHERE rr_state={} AND rr_slot={slot}",
            compile::PARENT
        )
    }

    fn relation(&self, pivot: bool) -> Rel {
        let removed = self.changed(true);
        let query = if pivot {
            format!("{} UNION ALL {removed}", self.changed(false))
        } else {
            let columns = self
                .fields
                .iter()
                .zip(&self.positions)
                .map(|(field, at)| format!("c{at} AS {}", quote(field)))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "SELECT {columns} FROM {ROWS} WHERE rr_state={} AND rr_slot={}",
                compile::PARENT,
                self.superset
            )
        };
        Rel::Bound {
            rel: self.current.program_relation().unwrap().into(),
            table: format!("({query})"),
        }
    }
}

fn change_slot(
    changes: &mut Vec<(Input, VersionKind)>,
    base: usize,
    input: &Input,
    kind: VersionKind,
) -> usize {
    let key = (input.clone(), kind);
    let index = changes
        .iter()
        .position(|entry| entry == &key)
        .unwrap_or_else(|| {
            changes.push(key);
            changes.len() - 1
        });
    base + index
}

pub(super) fn compile(
    original: &Query,
    current: &Query,
    trace: &Trace,
    db: &rusqlite::Connection,
    positions: &BTreeMap<&str, usize>,
    changes: &mut Vec<(Input, VersionKind)>,
    id: usize,
) -> (Decision, Option<(Rule, String)>) {
    let mut positive = original.ctes.is_empty() && !original.body.from.is_empty();
    let mut complete_dependencies = true;
    for expr in original
        .body
        .columns
        .iter()
        .map(|(e, _)| e)
        .chain(&original.body.predicates)
        .chain(original.body.from.iter().flat_map(|f| match &f.join {
            select::Join::Cross => &[][..],
            select::Join::On(on) => on.as_slice(),
        }))
    {
        expr.visit(&mut |e| match e {
            E::Call {
                name: "tape_nearest",
                ..
            } => complete_dependencies = false,
            E::Exists { .. } | E::Scalar(_) => positive = false,
            E::Call { name, arguments }
                if matches!(*name, "count" | "sum" | "avg" | "total")
                    || matches!(*name, "min" | "max") && arguments.len() == 1 =>
            {
                positive = false
            }
            _ => {}
        });
    }
    let total = plan::acyclic::total(&original.body);
    let proof = deltas::reconcile_projection(total, positive, complete_dependencies);
    // Bound expression size before generating a branch for every join atom.
    if !proof.proven || original.body.from.len() > 8 {
        return (
            if proof.proven {
                Decision::new(
                    false,
                    "Signed projection exceeds the eight-input SQL expansion bound",
                )
            } else {
                proof
            },
            None,
        );
    }
    let base = trace.relations.len() * 3 + trace.steps.len();
    let mut current = current.clone();
    current.body.from.sort_by_key(|item| {
        original
            .body
            .from
            .iter()
            .position(|old| old.alias == item.alias)
            .unwrap()
    });
    let atoms = original
        .body
        .from
        .iter()
        .zip(&current.body.from)
        .map(|(old, now)| {
            let name = old.relation.program_relation()?;
            let (input, fields, projected) = if let Some(&at) = positions.get(name) {
                let slot =
                    at * 3 + usize::from(matches!(old.relation, Rel::Derived { delta: true, .. }));
                let width = trace.relations[name].width;
                (
                    Input::Derived(slot),
                    (0..width).map(|c| format!("c{c}")).collect(),
                    (0..width).collect(),
                )
            } else {
                let source = crate::code_rule_dsl::facts::vtab::bound_source(db, name)?;
                if source.made_on_lookup() {
                    return None;
                }
                let full = source.columns();
                if full.len() > 120 {
                    return None;
                }
                let (visible, projected) = match &old.relation {
                    Rel::Input { .. } => (full.clone(), (0..full.len()).collect()),
                    Rel::Batch { source, .. } if !source.batched => {
                        let projected = source
                            .columns
                            .iter()
                            .map(|c| full.iter().position(|field| field == c))
                            .collect::<Option<Vec<_>>>()?;
                        let visible = if source.positional {
                            (0..projected.len()).map(|c| format!("c{c}")).collect()
                        } else {
                            source.columns.clone()
                        };
                        (visible, projected)
                    }
                    _ => return None,
                };
                (Input::External(name.into()), visible, projected)
            };
            Some(Atom {
                plus: change_slot(changes, base, &input, VersionKind::Added),
                minus: change_slot(changes, base, &input, VersionKind::Removed),
                superset: change_slot(changes, base, &input, VersionKind::Superset),
                input,
                current: now.relation.clone(),
                fields,
                positions: projected,
            })
        })
        .collect::<Option<Vec<_>>>();
    let Some(atoms) = atoms else {
        return (deltas::reconcile_projection(total, positive, false), None);
    };
    // Both queries use the original atom order. Anchoring can then move a pivot
    // without changing which columns its signed input supplies.
    let mut affected = Vec::new();
    for pivot in 0..atoms.len() {
        let mut query = current.clone();
        for (at, item) in query.body.from.iter_mut().enumerate() {
            item.relation = atoms[at].relation(at == pivot);
        }
        maintenance::anchor(&mut query, pivot);
        query.body.distinct = true;
        affected.push(query.render());
    }
    let width = original.body.columns.len();
    let fields = names("", width);
    let supported = maintenance::targeted(current.clone(), "__rr_affected").render();
    let present = (0..width)
        .map(|c| format!("c{c} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sentinel = (0..width)
        .map(|c| format!("NULL AS c{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "(WITH __rr_affected({fields}) AS MATERIALIZED ({}) \
         SELECT {COLLECT}({},{id},rr_present,{fields}) FROM (\
           SELECT 0 AS rr_present,{sentinel} \
           UNION ALL SELECT 2,{fields} FROM __rr_affected WHERE {present} \
           UNION ALL SELECT 1,{fields} FROM ({supported}) WHERE {present}))",
        affected.join(" UNION "),
        compile::PARENT,
    );
    let inputs = atoms.into_iter().map(|a| a.input).collect();
    (
        proof,
        Some((
            Rule {
                expression: 0,
                inputs,
            },
            sql,
        )),
    )
}

pub(super) struct External {
    pub statistics: InputChange,
    pub plus: versions::Version,
    pub minus: versions::Version,
    pub source: Arc<dyn Source>,
}

pub(super) fn external(
    w: &Work<'_>,
    plan: &Plan,
    changes: &BTreeMap<String, Delta>,
) -> Result<BTreeMap<String, External>> {
    let mut sources = BTreeMap::new();
    for (input, _) in &plan.changes {
        let Input::External(name) = input else {
            continue;
        };
        if sources.contains_key(name) {
            continue;
        }
        let source = crate::code_rule_dsl::facts::vtab::bound_source(w.db, name)
            .ok_or_else(|| error(format!("missing signed batch input {name}")))?;
        let width = source.columns().len();
        let empty = versions::Version::empty(Arc::new(versions::Schema::integers(width, width)));
        let (plus, minus) = if let Some(delta) = changes.get(name) {
            let store = w
                .versions
                .ok_or_else(|| error("signed SQL batch requires versioned inputs"))?;
            let read = |name: &str| {
                store
                    .available(name)
                    .ok_or_else(|| error("signed SQL batch requires immutable delta roots"))
            };
            (read(&delta.plus)?, read(&delta.minus)?)
        } else {
            (empty.clone(), empty)
        };
        sources.insert(
            name.clone(),
            External {
                statistics: InputChange {
                    rows: source.estimated_len(),
                    inserted: plus.len(),
                    deleted: minus.len(),
                    distinct_keys: None,
                    probe_rows: source.estimated_matches(0, false),
                },
                plus,
                minus,
                source,
            },
        );
    }
    Ok(sources)
}

pub(super) struct Differences {
    pub plus: versions::Version,
    pub minus: versions::Version,
    pub statistics: InputChange,
}

impl Arena {
    pub(super) fn differences(&mut self, slot: usize) -> rusqlite::Result<&Differences> {
        if !self.differences.contains_key(&slot) {
            let old = if self.waves.is_empty() {
                &self.initial
            } else {
                &self
                    .history
                    .get(self.waves.len() - 1)
                    .ok_or_else(|| sql_error("missing preceding-wave history"))?
                    .roots
            };
            let before = old[slot / 3].read(slot % 3);
            let after = self.current()[slot / 3].read(slot % 3);
            let rows = after.len();
            let distinct_keys = after.distinct_values(0);
            let probe_rows = after.estimated_matches(0, false);
            let (plus, minus, visited) = before.difference_roots(after).map_err(sql_error)?;
            self.version_nodes += visited;
            let statistics = InputChange {
                rows,
                inserted: plus.len(),
                deleted: minus.len(),
                distinct_keys,
                probe_rows,
            };
            self.differences.insert(
                slot,
                Differences {
                    plus,
                    minus,
                    statistics,
                },
            );
        }
        Ok(&self.differences[&slot])
    }

    pub(super) fn incremental_stage(&mut self, id: usize) -> rusqlite::Result<Option<usize>> {
        let Some(rule) = self.plan.rules.get(id).and_then(|r| r.incremental.as_ref()) else {
            return Ok(None);
        };
        if self.previous_candidate(id).is_none() {
            return Ok(None);
        }
        let expression = rule.expression;
        let inputs = rule.inputs.clone();
        let mut statistics = Vec::with_capacity(inputs.len());
        for input in &inputs {
            statistics.push(match input {
                Input::Derived(slot) => self.differences(*slot)?.statistics.clone(),
                Input::External(name) => self.external[name].statistics.clone(),
            });
        }
        let cost = ProjectionCost::estimate(&statistics);
        let selected = cost.sparse && !force_full();
        let reuse = cost.changed_rows == 0 && !force_full();
        self.decisions.insert(id, json!({"inputs":statistics,"cost":cost,"selected":selected,"reused":reuse,
            "proof":"Same rule and wave; signed differences use exact preceding-wave roots and the operation's external journals"}));
        if reuse {
            self.signed_reuses.insert(id);
            return Ok(Some(self.plan.reuse_id));
        }
        Ok(selected.then_some(expression))
    }
}

#[cfg(test)]
thread_local! {pub(super) static FORCE_FULL:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

fn force_full() -> bool {
    #[cfg(test)]
    {
        FORCE_FULL.get()
    }
    #[cfg(not(test))]
    {
        false
    }
}
