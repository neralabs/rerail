//! A positive cache transition: contexts, contributions, head counts, and results in one SQL phase.
use super::*;

impl Retained {
    pub(in crate::code_rule_dsl::sql_executor) fn can_fuse(&self) -> bool {
        self.rebuildable()
            && self.rules.len() <= 64
            && self.rules.iter().all(|rule| {
                if rule.width + self.width >= 2000 {
                    return false;
                }
                let query = rule.evaluation("rr_probe");
                let mut calls = false;
                for e in plan::expressions(&query.body)
                    .into_iter()
                    .chain(plan::expressions(&rule.context_query.body))
                {
                    e.visit(&mut |e| {
                        calls |= matches!(
                            e,
                            E::Call { .. } | E::Function { .. } | E::Scalar(_) | E::Exists { .. }
                        )
                    });
                }
                !calls
            })
    }
}

fn empty(width: usize) -> String {
    format!("SELECT {} WHERE 0", vec!["NULL"; width].join(","))
}
fn union(queries: Vec<String>, width: usize, all: bool) -> String {
    if queries.is_empty() {
        empty(width)
    } else {
        queries.join(if all { " UNION ALL " } else { " UNION " })
    }
}
fn fields(alias: &str, offset: usize, width: usize) -> String {
    (offset..offset + width)
        .map(|c| format!("{alias}.c{c}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn extreme(state: &Retained, changed: &str, keys: usize, function: &str) -> String {
    let value = state.width - 1;
    let order = if function == "min" { "ASC" } else { "DESC" };
    let previous = format!(
        "SELECT h.c{value} AS v FROM {} h WHERE {} AND NOT EXISTS(SELECT 1 FROM {changed} t WHERE {}) ORDER BY h.c{value} {order} LIMIT 1",
        quote(&state.heads),
        equal("h", "k", keys),
        equal("t", "h", state.width)
    );
    format!(
        "(SELECT {function}(v) FROM(SELECT v FROM ({previous}) UNION ALL SELECT t.c{value} AS v FROM {changed} t WHERE {} AND t.c{}>0))",
        equal("t", "k", keys),
        state.width
    )
}

pub(super) fn append(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
    phase: &mut phase::Phase,
) -> Result<(String, String)> {
    append_with(state, w, changes, phase, &BTreeMap::new(), &mut vec![])
}
pub(super) fn append_with(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
    phase: &mut phase::Phase,
    extra: &BTreeMap<usize, Vec<String>>,
    pending: &mut Vec<reducer::combined::publication::Publication>,
) -> Result<(String, String)> {
    let store = w
        .versions
        .ok_or_else(|| error("fused cache requires native storage"))?;
    let mut weights = vec![];
    let width = state.width;
    for rule in &state.rules {
        let mut removed = vec![];
        let mut added = vec![];
        for atom in &rule.atoms {
            let Some(delta) = changes.get(&atom.relation) else {
                continue;
            };
            if delta.deleted > 0 {
                let predicate = (0..atom.columns.len())
                    .map(|c| format!("c.c{}=d.c{c}", atom.offset + c))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                removed.push(format!(
                    "SELECT c.* FROM {} d CROSS JOIN {} c WHERE {predicate}",
                    quote(&delta.minus),
                    quote(&rule.contexts)
                ));
            }
            if delta.inserted > 0 {
                let mut query = rule.context_query.clone();
                query.body.from[atom.at].relation = source(&delta.plus, &atom.columns);
                maintenance::anchor(&mut query, atom.at);
                added.push(query.render());
            }
        }
        let removed = phase.relation(rule.width, &union(removed, rule.width, false));
        if let Some(queries) = extra.get(&rule.step) {
            let rows = phase.view(rule.width, &union(queries.clone(), rule.width, false));
            added.push(format!(
                "SELECT * FROM {rows} EXCEPT SELECT * FROM {removed}"
            ));
        }
        let added = phase.relation(rule.width, &union(added, rule.width, false));
        let dirty = phase.relation(
            rule.width,
            &format!("SELECT * FROM {removed} UNION SELECT * FROM {added}"),
        );
        let evaluated = phase.relation(rule.width + width, &rule.evaluation(&added).render());
        if store.available(&rule.contexts).is_some()
            || rule.atoms.iter().any(|a| {
                changes
                    .get(&a.relation)
                    .is_some_and(|d| d.inserted + d.deleted > 0)
            })
        {
            reducer::combined::publication::append(
                w,
                phase,
                pending,
                &rule.contexts,
                &dirty,
                &added,
                rule.width,
                rule.width,
            )?;
        }
        if !rule.contribution_view {
            reducer::combined::publication::append(
                w,
                phase,
                pending,
                &rule.contributions,
                &dirty,
                &evaluated,
                rule.width,
                rule.width + width,
            )?;
        }
        let heads = (0..width)
            .map(|c| format!("c.h{c}"))
            .collect::<Vec<_>>()
            .join(",");
        weights.push(format!(
            "SELECT {heads},-1 FROM {dirty} d CROSS JOIN {} c WHERE {}",
            rule.contribution_source(),
            equal("c", "d", rule.width)
        ));
        weights.push(format!(
            "SELECT {},1 FROM {evaluated} c",
            fields("c", rule.width, width)
        ));
    }
    let weights = phase.relation(width + 1, &union(weights, width + 1, true));
    let cols = names("", width);
    let grouped = phase.relation(
        width + 1,
        &format!(
            "SELECT {cols},sum(c{width}) FROM {weights} GROUP BY {cols} HAVING sum(c{width})<>0"
        ),
    );
    let totals=phase.relation(width+2,&format!("SELECT {},rr_nonnegative_count(coalesce(h.n,0)+d.c{width}),coalesce(h.n,0) FROM {grouped} d LEFT JOIN {} h ON {}",names("d",width),quote(&state.heads),equal("d","h",width)));
    let head_keys = phase.relation(width, &format!("SELECT {cols} FROM {totals}"));
    let head_rows = phase.relation(
        width + 1,
        &format!("SELECT {cols},c{width} FROM {totals} WHERE c{width}>0"),
    );
    reducer::combined::publication::append(
        w,
        phase,
        pending,
        &state.heads,
        &head_keys,
        &head_rows,
        width,
        width + 1,
    )?;
    let (keys, next) = if let Some(lattice) = &state.lattice {
        let k = lattice.keys;
        let keys = phase.relation(
            k,
            &format!("SELECT DISTINCT {} FROM {totals}", names("", k)),
        );
        let next = if let Some(aggregate) = &lattice.aggregate {
            let values = phase.relation(
                k + 1,
                &format!(
                    "SELECT {},{} FROM {keys} k",
                    names("k", k),
                    extreme(state, &totals, k, aggregate)
                ),
            );
            phase.relation(
                width,
                &format!("SELECT * FROM {values} WHERE c{k} IS NOT NULL"),
            )
        } else {
            let values = phase.relation(
                k + 2,
                &format!(
                    "SELECT {},{},{} FROM {keys} k",
                    names("k", k),
                    extreme(state, &totals, k, "min"),
                    extreme(state, &totals, k, "max")
                ),
            );
            phase.relation(width,&format!("SELECT {},CASE WHEN c{k}=c{} THEN c{k} ELSE {} END FROM {values} WHERE c{k} IS NOT NULL",names("",k),k+1,lattice.top))
        };
        (keys, next)
    } else {
        let next = phase.relation(
            width,
            &format!("SELECT {cols} FROM {totals} WHERE c{width}>0"),
        );
        (head_keys, next)
    };
    if store.available(&state.output).is_some() {
        phase.patch_computed(store, &state.output, &keys, &next)?;
    }
    Ok((keys, next))
}

pub(super) fn apply(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<(Delta, u64, u64)> {
    let store = w
        .versions
        .ok_or_else(|| error("fused cache requires native storage"))?;
    let width = state.width;
    let mut phase = phase::Phase::default();
    let (keys, next) = append(state, w, changes, &mut phase)?;
    let key_width = state.lattice.as_ref().map_or(width, |l| l.keys);
    let old = phase.relation(
        width,
        &format!(
            "SELECT r.* FROM {keys} k CROSS JOIN {} r WHERE {}",
            quote(&state.output),
            equal("r", "k", key_width)
        ),
    );
    let plus_rows = phase.relation(
        width,
        &format!("SELECT * FROM {next} EXCEPT SELECT * FROM {old}"),
    );
    let minus_rows = phase.relation(
        width,
        &format!("SELECT * FROM {old} EXCEPT SELECT * FROM {next}"),
    );
    let plus = w.output_table(width)?;
    let minus = w.output_table(width)?;
    phase.load_empty(store, &plus, &plus_rows)?;
    phase.load_empty(store, &minus, &minus_rows)?;
    let counts = phase.execute(w)?;
    let contexts = state
        .rules
        .iter()
        .map(|r| counts[&r.contexts].keys as u64)
        .sum();
    let groups = if state.lattice.is_some() {
        counts[&state.output].keys as u64
    } else {
        0
    };
    Ok((
        Delta {
            inserted: counts[&plus].rows as u64,
            deleted: counts[&minus].rows as u64,
            plus,
            minus,
        },
        contexts,
        groups,
    ))
}
