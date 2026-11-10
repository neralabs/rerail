//! One summary store per proven bag, with independent context lookups for its consumers.
use super::*;
use crate::code_rule_dsl::logic::analysis::{bags, scalars};
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl::sql_executor) struct Plans {
    pub states: BTreeMap<String, Retained>,
    pub storage: Vec<Report>,
}
#[derive(Clone, serde::Serialize)]
pub(in crate::code_rule_dsl::sql_executor) struct Report {
    #[serde(serialize_with = "serialize_analysis")]
    pub analysis: Arc<bags::Analysis>,
    pub groups: Vec<String>,
    pub evaluation_boundary: &'static str,
}
impl std::ops::Deref for Plans {
    type Target = BTreeMap<String, Retained>;
    fn deref(&self) -> &Self::Target {
        &self.states
    }
}
impl std::ops::DerefMut for Plans {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.states
    }
}

pub(super) fn combine(plans: &mut Plans, queries: &[Option<Query>]) {
    let mut paths = vec![];
    let mut inputs = vec![];
    for (output, state) in &plans.states {
        for (rule, r) in state.rules.iter().enumerate() {
            for (scalar, input) in r.scalar_analysis.inputs.iter().enumerate() {
                paths.push((output.clone(), rule, scalar));
                inputs.push((
                    bags::Site {
                        rule: r.step,
                        head: output.clone(),
                        scalar,
                    },
                    input.clone(),
                ));
            }
        }
    }
    if paths.is_empty() {
        return;
    }
    let analysis = Arc::new(bags::analyze(inputs));
    let mut groups = vec![];
    for family in &analysis.bags.families {
        let selected = family
            .members
            .iter()
            .find(|&&i| analysis.bags.inputs[i].kind != scalars::Kind::Count)
            .copied()
            .unwrap_or(family.members[0]);
        let (out, rule, site) = &paths[selected];
        let (_, _, projection, _) =
            reducer::combined::storage(&plans.states[out].rules[*rule].reducers[*site]);
        let (out, rule, site) = &paths[family.members[0]];
        let (group, values, _, _) =
            reducer::combined::storage(&plans.states[out].rules[*rule].reducers[*site]);
        groups.push(group.clone());
        if !family.lookup.proven {
            continue;
        }
        let mut locals = BTreeMap::<(String, usize), Vec<usize>>::new();
        for &i in &family.members {
            let (out, rule, _) = &paths[i];
            locals.entry((out.clone(), *rule)).or_default().push(i);
        }
        let consumers = locals
            .values()
            .map(|indices| {
                scalars::analyze(
                    indices
                        .iter()
                        .map(|&i| analysis.bags.inputs[i].clone())
                        .collect(),
                )
                .families
                .len()
            })
            .sum();
        for ((output, rule), indices) in locals {
            let local = scalars::analyze(
                indices
                    .iter()
                    .map(|&i| analysis.bags.inputs[i].clone())
                    .collect(),
            );
            for lookup in local.families {
                let members = lookup
                    .members
                    .iter()
                    .map(|&i| paths[indices[i]].2)
                    .collect::<Vec<_>>();
                for &site in &members {
                    reducer::combined::share(
                        &mut Arc::make_mut(&mut plans.states.get_mut(&output).unwrap().rules)[rule].reducers[site],
                        &group,
                        &values,
                        &projection,
                        &family.layout,
                        &members,
                        site == members[0],
                        lookup.lookup.proven,
                        consumers,
                    );
                }
            }
        }
    }
    for state in plans.states.values_mut() {
        for rule in Arc::make_mut(&mut state.rules) {
            rule.rewrite(queries[rule.step].as_ref().unwrap());
        }
    }
    plans.storage.push(Report { analysis, groups, evaluation_boundary: "Bag storage ignores outer lookup dependencies; each consumer keeps its own lookup and dependency index. Cross-gate transitions require equal input snapshots and maintenance cursors" });
}

/// Wave gates can share a layout, but their applied input cursors can diverge.
/// Keep distinct names for their immutable roots; transitions share only after
/// the input-version and maintenance-cursor proof succeeds.
pub(in crate::code_rule_dsl::sql_executor) fn combine_cursors(plans: &mut Plans, queries: &[Option<Query>]) {
    let names = plans.states.iter().flat_map(|(out,state)|state.rules.iter().enumerate().flat_map(move |(rule,r)|
        r.reducers.iter().enumerate().map(move |(scalar,s)|{
            let (groups,values,_,_)=reducer::combined::storage(s);
            ((out.clone(),rule,scalar),(groups,values))
        })))
        .collect::<BTreeMap<_,_>>();
    combine(plans,queries);
    for (out,state) in &mut plans.states {
        for (rule,r) in Arc::make_mut(&mut state.rules).iter_mut().enumerate() {
            for (scalar,s) in r.reducers.iter_mut().enumerate() {
                let (groups,values) = &names[&(out.clone(),rule,scalar)];
                reducer::combined::cursor_names(s,groups,values);
            }
            r.rewrite(queries[r.step].as_ref().unwrap());
        }
    }
    for report in &mut plans.storage {
        report.evaluation_boundary = "Independent wave gates retain separate cursor roots. Equal input versions, summary roots, and pending journals permit shared initialization or transition evaluation";
    }
}

/// A maintenance pass keeps immutable dirty-key snapshots across scratch rewinds.
/// Nested passes get their own cache; a later batch always applies its delta again.
pub(in crate::code_rule_dsl::sql_executor) type Cache = BTreeMap<(usize, String), Cached>;
#[derive(Clone)]
pub(in crate::code_rule_dsl::sql_executor) enum Cached {
    Native(versions::Version),
    Rows(usize, Vec<Vec<i64>>),
    Empty,
}
pub(super) fn recalled(w: &mut Work<'_>, group: &str) -> Result<Option<Option<(String, u64)>>> {
    let scope = w.versions.map_or(0, versions::Store::current_scope);
    let Some(cached) = w.summaries.get(&(scope, group.into())).cloned() else {
        return Ok(None);
    };
    let result = match cached {
        Cached::Empty => None,
        Cached::Native(version) => {
            let keys = w.output_table(version.columns().len())?;
            if !w.versions.unwrap().clone_scratch(&keys, &version)? {
                return Err(error("shared summary keys changed shape"));
            }
            Some((keys, 0))
        }
        Cached::Rows(width, rows) => Some((w.rows(width, &rows)?, 0)),
    };
    Ok(Some(result))
}
pub(super) fn remember(
    w: &mut Work<'_>,
    group: &str,
    result: &Option<(String, u64)>,
    width: usize,
) -> Result<()> {
    let cached = if let Some((keys, _)) = result {
        if let Some(version) = w.versions.and_then(|s| s.available(keys)) {
            Cached::Native(version)
        } else {
            let mut statement =
                w.db.prepare(&format!("SELECT * FROM {}", quote(keys)))
                    .map_err(error)?;
            let rows = statement
                .query_map([], |r| {
                    (0..width)
                        .map(|c| r.get::<_, i64>(c))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_err(error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(error)?;
            Cached::Rows(width, rows)
        }
    } else {
        Cached::Empty
    };
    w.summaries.insert(
        (
            w.versions.map_or(0, versions::Store::current_scope),
            group.into(),
        ),
        cached,
    );
    Ok(())
}

fn serialize_analysis<S: serde::Serializer>(
    value: &Arc<bags::Analysis>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serde::Serialize::serialize(value.as_ref(), serializer)
}
