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
        .map(|(keys, next, _)| (keys, next))
}
pub(super) fn append_with(
    state: &Retained,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
    phase: &mut phase::Phase,
    extra: &BTreeMap<usize, Vec<String>>,
    pending: &mut Vec<reducer::combined::publication::Publication>,
) -> Result<(String, String, Option<(String, String)>)> {
    let store = w
        .versions
        .ok_or_else(|| error("fused cache requires native storage"))?;
    let mut weights = vec![];
    let mut unique_heads = vec![];
    let unique = state.lattice.is_none() && state.support_uniqueness().proven;
    let width = state.width;
    for rule in state.rules.iter() {
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
        let no_removed = removed.is_empty();
        let removed = if no_removed {
            phase.view(rule.width, &empty(rule.width))
        } else {
            phase.relation(rule.width, &union(removed, rule.width, false))
        };
        if let Some(queries) = extra.get(&rule.step) {
            // Each dirty-key probe selects complete contexts. A single probe is
            // unique by the retained context key; multiple probes need UNION.
            let rows = phase.view(rule.width, &union(queries.clone(), rule.width, false));
            added.push(if no_removed {
                format!("SELECT * FROM {rows}")
            } else {
                format!(
                    "SELECT a.* FROM {rows} a WHERE NOT EXISTS(SELECT 1 FROM {removed} r WHERE {})",
                    equal("a", "r", rule.width)
                )
            });
        }
        let no_added = added.is_empty();
        let added = phase.relation(rule.width, &union(added, rule.width, false));
        let dirty = if no_removed {
            added.clone()
        } else if no_added {
            removed.clone()
        } else {
            phase.relation(
                rule.width,
                &format!("SELECT * FROM {removed} UNION SELECT * FROM {added}"),
            )
        };
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
        if unique {
            let old = phase.relation(
                width,
                &format!(
                    "SELECT {heads} FROM {dirty} d CROSS JOIN {} c WHERE {}",
                    rule.contribution_source(),
                    equal("c", "d", rule.width)
                ),
            );
            let next = phase.view(
                width,
                &format!(
                    "SELECT {} FROM {evaluated} c",
                    fields("c", rule.width, width)
                ),
            );
            unique_heads.push((old,next));
        } else {
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
    }
    if !unique_heads.is_empty() {
        let (old,next) = if unique_heads.len()==1 { unique_heads.pop().unwrap() } else {
        let old = phase.view(width,&unique_heads.iter().map(|(old,_)|format!("SELECT * FROM {old}")).collect::<Vec<_>>().join(" UNION ALL "));
        let next = phase.view(width,&unique_heads.iter().map(|(_,next)|format!("SELECT * FROM {next}")).collect::<Vec<_>>().join(" UNION ALL "));
        (old,next)
        };
        // The shared witness proof establishes at most one producer per head
        // in each snapshot. Replace dirty supports of one without an aggregate.
        let removed = phase.relation(
            width,
            &format!(
                "SELECT o.* FROM {old} o WHERE NOT EXISTS(SELECT 1 FROM {next} n WHERE {})",
                equal("n", "o", width)
            ),
        );
        let added = phase.relation(
            width,
            &format!(
                "SELECT n.* FROM {next} n WHERE NOT EXISTS(SELECT 1 FROM {old} o WHERE {})",
                equal("n", "o", width)
            ),
        );
        let keys = phase.view(
            width,
            &format!("SELECT * FROM {removed} UNION ALL SELECT * FROM {added}"),
        );
        let heads = phase.view(
            width + 1,
            &format!("SELECT {},1 FROM {added}", names("", width)),
        );
        reducer::combined::publication::append(
            w,
            phase,
            pending,
            &state.heads,
            &keys,
            &heads,
            width,
            width + 1,
        )?;
        if w.profiler.is_some() {
            phase.analyses.push(json!({"operation":"unique incremental support","proof":state.support_uniqueness(),"strategy":"Unique old/new head sets replace supports of one without grouping"}));
        }
        if store.available(&state.output).is_some() {
            phase.patch_computed(store, &state.output, &keys, &added)?;
        }
        return Ok((keys, added.clone(), Some((added, removed))));
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
    Ok((keys, next, None))
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
    let (keys, next, difference) =
        append_with(state, w, changes, &mut phase, &BTreeMap::new(), &mut vec![])?;
    let key_width = state.lattice.as_ref().map_or(width, |l| l.keys);
    let (plus_rows, minus_rows) = if let Some(difference) = difference {
        difference
    } else {
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
        (plus_rows, minus_rows)
    };
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
