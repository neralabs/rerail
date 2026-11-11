//! Lower the emitted rule gates and lattice folds to SQLite expressions.
use super::*;

const PARENT: &str = "__rr_parent.state";

fn source(table: &str, slot: usize, width: usize) -> String {
    format!(
        "(SELECT {} FROM {table} WHERE rr_state={PARENT} AND rr_slot={slot})",
        names("", width)
    )
}

fn evaluate(id: usize, width: usize, query: &str, filter_null: bool) -> String {
    let fields = names("", width);
    let present = (0..width)
        .map(|c| format!("c{c} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let select = if filter_null {
        format!("SELECT 1,{fields} FROM __rr_feed WHERE {present}")
    } else {
        format!("SELECT 1,{fields} FROM __rr_feed")
    };
    let sentinel = (0..width)
        .map(|i| format!("NULL AS c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "(WITH __rr_feed({fields}) AS MATERIALIZED ({query}) SELECT {COLLECT}({PARENT},{id},rr_present,{fields}) FROM (SELECT 0 AS rr_present, {sentinel} UNION ALL {select}))",
    )
}

fn empty(width: usize, keys: usize, indexes: &[usize]) -> versions::Version {
    let mut empty = versions::Version::empty(Arc::new(versions::Schema::integers(width, keys)));
    empty.restrict_indexes(indexes);
    empty
}

pub(super) fn plan(trace: &Trace, db: &rusqlite::Connection) -> Result<Option<Plan>> {
    let positions = trace
        .relations
        .keys()
        .enumerate()
        .map(|(id, r)| (r.as_str(), id))
        .collect::<BTreeMap<_, _>>();
    let width = trace
        .relations
        .values()
        .map(|r| r.width.max(r.keys + usize::from(r.widening())))
        .max()
        .unwrap_or(0);
    if width == 0 || width > 120 || trace.steps.len() > 400 {
        return Ok(None);
    }
    let strata = trace.steps.iter().map(|(s, _)| s.stratum).collect();
    let mut rules = vec![];
    let mut expressions = vec![];
    let mut fact_probe_anchors = 0;
    for (id, (step, checked)) in trace.steps.iter().enumerate() {
        let relation = positions[step.output.as_str()];
        let spec = &trace.relations[&step.output];
        let mut reads = BTreeSet::new();
        let mut external = BTreeSet::new();
        let (query, reusable) = if let Some(original) = checked {
            let mut query = original.clone();
            let derived = query
                .body
                .from
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.relation
                        .program_relation()
                        .is_some_and(|r| positions.contains_key(r))
                })
                .collect::<Vec<_>>();
            let delta = derived
                .iter()
                .find(|(_, item)| matches!(item.relation, Rel::Derived { delta: true, .. }))
                .map(|(i, _)| *i);
            let anchor = delta.or_else(|| bound_fact_anchor(&query, &derived, trace, db));
            if delta.is_none() && anchor.is_some() {
                fact_probe_anchors += 1;
            }
            if let Some(at) = anchor {
                maintenance::anchor(&mut query, at);
            } else {
                query.body.sqlite_order();
            }
            let local = |r: &Rel| {
                let name = r.program_relation()?;
                let at = *positions.get(name)?;
                let delta = matches!(r, Rel::Derived { delta: true, .. });
                Some((at * 3 + usize::from(delta), trace.relations[name].width))
            };
            for r in query.reads() {
                if let Some((slot, _)) = local(r) {
                    reads.insert(slot);
                } else {
                    match r {
                        Rel::Input { rel, .. } => {
                            external.insert(rel.clone());
                        }
                        Rel::Cte(_) | Rel::Integers => {}
                        _ => {
                            external.insert(String::new());
                        }
                    }
                }
            }
            for block in query
                .ctes
                .iter_mut()
                .flat_map(|c| &mut c.arms)
                .chain([&mut query.body])
            {
                block.relations_mut(&mut |r| {
                    if let Some((slot, width)) = local(r) {
                        *r = Rel::Bound {
                            rel: r.program_relation().unwrap().into(),
                            table: source(if slot % 3 == 1 { FRONTIER } else { ROWS }, slot, width),
                        };
                    }
                });
            }
            let reusable = plan::acyclic::total(&query.body)
                && query
                    .ctes
                    .iter()
                    .all(|c| c.arms.iter().all(plan::acyclic::total));
            (query.render(), reusable)
        } else {
            let prefix = format!("INSERT OR IGNORE INTO new_{} ", step.output);
            let Some(select) = step.sql.strip_prefix(&prefix) else {
                return Ok(None);
            };
            if trace.producers.contains_key(&step.output) {
                // Opaque producers with derived reads need a public owner gate.
                // A literal SELECT has no reads or callback effects.
                let producer_query = select
                    .strip_prefix("WITH q(c0) AS (")
                    .and_then(|q| q.strip_suffix(") SELECT c0 FROM q"))
                    .unwrap_or("");
                let mut tokens = producer_query.split_whitespace();
                let literal = tokens
                    .next()
                    .is_some_and(|t| t.eq_ignore_ascii_case("select"))
                    && tokens.next().is_some_and(|v| v.parse::<i64>().is_ok())
                    && tokens.next().is_none();
                if !literal || !step.reads.is_empty() || !trace.producers[&step.output].is_empty() {
                    return Ok(None);
                }
            } else if !trace.emitted.constant_rows.contains_key(&step.output) {
                return Ok(None);
            }
            (select.to_owned(), true)
        };
        expressions.push(evaluate(id, spec.width, &query, true));
        rules.push(Rule {
            relation,
            reads: reads.into_iter().collect(),
            external,
            reusable,
            candidate: empty(spec.width, spec.width, &[]),
        });
    }
    let mut outputs = vec![];
    for (id, (name, spec)) in trace.relations.iter().enumerate() {
        // A registered opaque aggregate can require a particular evaluation order.
        // Built-in lattice reductions agree with a distinct bag of rule candidates.
        if spec.lattice
            && spec
                .ops
                .as_ref()
                .is_some_and(|o| matches!(o.reduction, Reduction::Opaque))
        {
            return Ok(None);
        }
        let mut input = rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.relation == id)
            .map(|(r, _)| {
                format!(
                    "SELECT * FROM {}",
                    source(CANDIDATES, positions.len() * 3 + r, spec.width)
                )
            })
            .collect::<Vec<_>>();
        if input.is_empty() {
            input.push(format!(
                "SELECT {} WHERE 0",
                (0..spec.width)
                    .map(|c| format!("NULL AS c{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let candidates = input.join(" UNION ");
        let previous = source(ROWS, id * 3, spec.width);
        let (query, output_width) = fold(spec, &candidates, &previous, id);
        expressions.push(evaluate(rules.len() + id, output_width, &query, false));
        let readers = trace
            .steps
            .iter()
            .enumerate()
            .filter(|(_, (s, _))| s.reads.contains(name))
            .map(|(r, _)| r)
            .collect();
        outputs.push(Output {
            name: name.clone(),
            spec: spec.clone(),
            readers,
            empty: state::Roots {
                rows: empty(spec.width, spec.keys, &spec.indexes),
                frontier: empty(spec.width, spec.keys, &spec.indexes),
                counts: empty(spec.keys + 1, spec.keys, &[]),
            },
        });
    }
    let reuse_id = expressions.len();
    expressions.push(format!("{REUSE}({PARENT})"));
    let branches = expressions
        .iter()
        .enumerate()
        .map(|(id, sql)| format!("WHEN {id} THEN {sql}"))
        .collect::<Vec<_>>()
        .join(" ");
    let sql = format!(
        "WITH RECURSIVE __rr_states(state) AS (VALUES(0) UNION ALL SELECT CASE {NEXT}({PARENT}) {branches} END FROM __rr_states __rr_parent WHERE {NEXT}({PARENT})>=0) SELECT count(*) FROM __rr_states"
    );
    if sql.len() > 2_000_000 {
        return Ok(None);
    }
    Ok(Some(Plan {
        rules,
        outputs,
        after: trace.after.clone(),
        strata,
        width,
        sql,
        reuse_id,
        fact_probe_anchors,
    }))
}

/// Keep repeated fact lookups from joining each other on their unbound values.
/// Choose an anchor only when it binds an indexed probe of every other input.
fn bound_fact_anchor(
    query: &Query,
    derived: &[(usize, &select::FromItem)],
    trace: &Trace,
    db: &rusqlite::Connection,
) -> Option<usize> {
    use crate::code_rule_dsl::{facts::vtab::bound_source, logic::sql::properties};
    if derived.is_empty() || !query.ctes.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut repeated = false;
    let mut columns = vec![];
    let mut probes = vec![];
    for item in &query.body.from {
        let relation = item.relation.program_relation()?;
        if let Some(spec) = trace.relations.get(relation) {
            columns.push((0..spec.width).map(|c| format!("c{c}")).collect());
            probes.push(
                std::iter::once(0)
                    .chain(spec.indexes.iter().copied())
                    .collect(),
            );
            continue;
        }
        let bound = bound_source(db, relation)?;
        let fields = bound.columns();
        let (projected, positional) = match &item.relation {
            Rel::Batch { source, .. } if !source.batched => (&source.columns, source.positional),
            Rel::Input { .. } => (&fields, false),
            _ => return None,
        };
        let selective = projected
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let c = fields.iter().position(|f| f == name)?;
                (bound.indexed(c) && bound.estimated_matches(c, false) <= 4).then_some(i)
            })
            .collect::<Vec<_>>();
        if selective.is_empty() {
            return None;
        }
        repeated |= !seen.insert(relation);
        columns.push(if positional {
            (0..projected.len()).map(|c| format!("c{c}")).collect()
        } else {
            projected.clone()
        });
        probes.push(selective);
    }
    if !repeated {
        return None;
    }
    let mut projection =
        properties::projection(query, &columns, &trace.emitted.analysis.relation_keys);
    derived.iter().find_map(|&(anchor, _)| {
        projection.outputs = projection.inputs[anchor]
            .columns
            .iter()
            .cloned()
            .map(Some)
            .collect();
        let proof = projection.unique();
        projection
            .inputs
            .iter()
            .enumerate()
            .all(|(id, input)| {
                id == anchor
                    || probes[id]
                        .iter()
                        .any(|&c| proof.determined.contains(&input.columns[c]))
            })
            .then_some(anchor)
    })
}

fn fold(spec: &Relation, candidates: &str, previous: &str, id: usize) -> (String, usize) {
    if !spec.lattice {
        let same = same("p", "c", spec.width);
        return (
            format!(
                "SELECT c.* FROM ({candidates}) c WHERE NOT EXISTS(SELECT 1 FROM {previous} p WHERE {same})"
            ),
            spec.width,
        );
    }
    let value = format!("c{}", spec.width - 1);
    let keys = (0..spec.keys).map(|i| format!("c{i}")).collect::<Vec<_>>();
    let group = if keys.is_empty() {
        String::new()
    } else {
        format!(" GROUP BY {}", keys.join(","))
    };
    let aggregate = spec.aggregate(&value);
    let aggregate_fields = keys
        .iter()
        .cloned()
        .chain([format!("{aggregate} AS {value}")])
        .collect::<Vec<_>>()
        .join(",");
    let equality = |a, b| {
        if spec.keys == 0 {
            "1".into()
        } else {
            same(a, b, spec.keys)
        }
    };
    let merge = spec.merge(&format!("p.{value}"), &format!("c.{value}"));
    let candidate_keys = keys.iter().map(|k| format!("c.{k}")).collect::<Vec<_>>();
    let mut merged = candidate_keys.clone();
    merged.push(format!("p.{value} AS old"));
    merged.push(format!(
        "CASE WHEN p.{value} IS NULL THEN c.{value} ELSE {merge} END AS merged"
    ));
    if spec.widening() {
        let counts = source(ROWS, id * 3 + 2, spec.keys + 1);
        let count_eq = equality("q", "c");
        merged.push(format!(
            "coalesce((SELECT q.c{} FROM {counts} q WHERE {count_eq}),0) AS changes",
            spec.keys
        ));
    }
    let mut fields = keys.clone();
    let value = if let Some(ops) = spec.ops.as_ref().filter(|_| spec.widening()) {
        let after = ops.after.unwrap();
        let widen = ops.widen.as_ref().unwrap();
        format!(
            "CASE WHEN old IS NOT NULL AND merged<>old AND changes>={after} THEN {widen}(old,merged) ELSE merged END"
        )
    } else {
        "merged".to_owned()
    };
    fields.push(format!("{value} AS value"));
    fields.push("old".into());
    if spec.widening() {
        fields.push("changes+1 AS next_count".into());
    }
    let output = keys
        .into_iter()
        .chain(["value".into()])
        .chain(spec.widening().then_some("next_count".into()))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH c AS MATERIALIZED (SELECT {aggregate_fields} FROM ({candidates}){group}), m AS MATERIALIZED (SELECT {} FROM c LEFT JOIN {previous} p ON {} WHERE c.c{} IS NOT NULL), n AS MATERIALIZED (SELECT {} FROM m) SELECT {output} FROM n WHERE old IS NULL OR value<>old",
        merged.join(","),
        equality("p", "c"),
        spec.width - 1,
        fields.join(","),
    );
    (sql, spec.width + usize::from(spec.widening()))
}
