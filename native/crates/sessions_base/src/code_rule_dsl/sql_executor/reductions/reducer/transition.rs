//! Reuse a scalar transition across gates only at the same input and maintenance cursor.
use super::*;
use crate::code_rule_dsl::{facts::vtab::Source, logic::analysis::scalars};

#[derive(Clone)]
enum Input {
    Version(versions::Version),
    Bound(Arc<dyn Source>, u64),
}
impl Input {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Version(a), Self::Version(b)) => a.same(b),
            (Self::Bound(a, ar), Self::Bound(b, br)) => ar == br && Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
struct Cursor {
    input: Input,
    groups: versions::Version,
    values: Option<versions::Version>,
    plus: versions::Version,
    minus: versions::Version,
}
pub struct Transition {
    owner: String,
    scope: usize,
    signature: Arc<str>,
    before: Cursor,
    groups: versions::Version,
    values: Option<versions::Version>,
    result: Option<(versions::Version, u64)>,
}
fn same_optional(a: &Option<versions::Version>, b: &Option<versions::Version>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.same(b),
        (None, None) => true,
        _ => false,
    }
}
pub fn descriptor(r: &Reducer) -> Option<Arc<str>> {
    if r.combined.as_ref().is_some_and(|s| !s.owner) {
        return None;
    }
    // Ordered joins keep their complete order expression in the signature.
    // Only the engine's string/aggregate intrinsics have this effect contract.
    if let Some(join) = &r.join {
        let mut pure = true;
        join.visit(&mut |e| match e {
            E::Call { name, .. } => {
                pure &= matches!(*name, "sym_atom" | "sym_text" | "coalesce" | "rr_join")
            }
            E::Function { .. } | E::Scalar(_) | E::Exists { .. } | E::Parameter(_) => pure = false,
            _ => {}
        });
        if !pure {
            return None;
        }
    }
    // Opaque callbacks and values without a totality proof stay separate.
    let alias = &r.projection.body.from[0].alias;
    let columns = r
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| ((alias.clone(), c.clone()), i))
        .collect();
    let property = |e| crate::code_rule_dsl::logic::sql::properties::expression(e, &columns);
    if !r
        .projection
        .body
        .predicates
        .iter()
        .all(|e| property(e).stable())
        || !r
            .projection
            .body
            .columns
            .iter()
            .all(|(e, _)| property(e).non_null())
    {
        return None;
    }
    Some(Arc::from(signature(r)))
}
fn input(r: &Reducer, w: &Work<'_>) -> Option<Input> {
    r.transition_key.as_ref()?;
    let store = w.versions?;
    if let Some(v) = store.available(&r.relation) {
        Some(Input::Version(v))
    } else {
        let source = crate::code_rule_dsl::facts::vtab::bound_source(w.db, &r.relation)?;
        let revision = source.revision()?;
        Some(Input::Bound(source, revision))
    }
}
fn cursor(r: &Reducer, w: &Work<'_>, changes: &BTreeMap<String, Delta>) -> Option<Cursor> {
    let store = w.versions?;
    let groups = store.available(&r.groups)?;
    let delta = changes
        .get(&r.relation)
        .filter(|d| d.inserted + d.deleted > 0)?;
    let input = input(r, w)?;
    Some(Cursor {
        input,
        groups,
        values: store.available(&r.values),
        plus: store.available(&delta.plus)?,
        minus: store.available(&delta.minus)?,
    })
}
pub struct Pending {
    signature: Arc<str>,
    before: Cursor,
}
impl Pending {
    pub fn equivalent(&self, other: &Self) -> bool {
        self.signature == other.signature && scalars::maintenance_equivalent(
            self.before.input.same(&other.before.input),
            self.before.groups.same(&other.before.groups)
                && same_optional(&self.before.values, &other.before.values)
                && self.before.plus.same(&other.before.plus)
                && self.before.minus.same(&other.before.minus),
        ).proven
    }
}
#[allow(clippy::large_enum_variant)]
pub enum Prepared {
    Reused(Option<(String, u64)>),
    Evaluate(Option<Pending>),
}

pub fn prepare(
    r: &Reducer,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<Prepared> {
    if r.combined.as_ref().is_some_and(|s| s.owner) && r.pool_consumers > 1
        && let Some(result) = super::super::pool::recalled(w, &r.groups)? {
            return Ok(Prepared::Reused(result));
        }
    let Some(before) = cursor(r, w, changes) else {
        return Ok(Prepared::Evaluate(None));
    };
    let signature = Arc::clone(r.transition_key.as_ref().unwrap());
    let found = w.summary_transitions.iter().find(|entry| {
        entry.signature == signature
            && scalars::maintenance_equivalent(
                before.input.same(&entry.before.input),
                before.groups.same(&entry.before.groups)
                    && same_optional(&before.values, &entry.before.values)
                    && before.plus.same(&entry.before.plus)
                    && before.minus.same(&entry.before.minus),
            )
            .proven
    });
    if let Some(entry) = found {
        let _context = w.profile_scope(||json!({"stage":"shared summary transition","source_group":entry.owner,"source_scope":entry.scope,"target_group":r.groups,"target_scope":w.versions.unwrap().current_scope(),"logicalAnalysis":scalars::maintenance_equivalent(true,true)}));
        let operation = w
            .profiler
            .map(|p| p.operation("shared summary transition", ("sql-extension", 0), 0, vec![]));
        let store = w.versions.unwrap();
        store.replace_available(&r.groups, entry.groups.clone())?;
        if let Some(values) = &entry.values {
            store.replace_available(&r.values, values.clone())?;
        }
        let cached = entry.result.clone();
        w.summary_reuses += 1;
        let result = cached
            .map(|(version, touched)| {
                let table = w.output_table(version.columns().len())?;
                if !store.clone_scratch(&table, &version)? {
                    return Err(error("shared transition keys changed shape"));
                }
                Ok((table, touched))
            })
            .transpose()?;
        if r.pool_consumers > 1 {
            super::super::pool::remember(w, &r.groups, &result, r.keys())?;
        }
        if let Some(operation) = operation {
            operation.finish(1);
        }
        return Ok(Prepared::Reused(result));
    }
    Ok(Prepared::Evaluate(Some(Pending { signature, before })))
}

pub fn remember(
    r: &Reducer,
    w: &mut Work<'_>,
    pending: Option<Pending>,
    result: &Option<(String, u64)>,
) -> Result<()> {
    if r.pool_consumers > 1 {
        super::super::pool::remember(w, &r.groups, result, r.keys())?;
    }
    let Some(Pending { signature, before }) = pending else { return Ok(()); };
    let store = w.versions.unwrap();
    let keys = result
        .as_ref()
        .map(|(table, n)| store.available(table).map(|v| (v, *n)));
    if keys.as_ref().is_some_and(Option::is_none) {
        return Ok(());
    }
    // A bounded operation-local cache retains no history across failed or later batches.
    if w.summary_transitions.len() == 128 {
        w.summary_transitions.remove(0);
    }
    w.summary_transitions.push(Transition {
        owner: r.groups.clone(),
        scope: store.current_scope(),
        signature,
        before,
        groups: store.available(&r.groups).unwrap(),
        values: store.available(&r.values),
        result: keys.flatten(),
    });
    Ok(())
}

pub fn apply(
    r: &Reducer,
    w: &mut Work<'_>,
    changes: &BTreeMap<String, Delta>,
) -> Result<Option<(String, u64)>> {
    match prepare(r, w, changes)? {
        Prepared::Reused(result) => Ok(result),
        Prepared::Evaluate(pending) => {
            let result = r.apply_uncached(w, changes)?;
            remember(r, w, pending, &result)?;
            Ok(result)
        }
    }
}

fn signature(r: &Reducer) -> String {
    let mut projection = r.projection.clone();
    let alias = projection.body.from[0].alias.clone();
    projection.body.from[0].alias = "rr_bag_source".into();
    plan::replace(&mut projection.body, &mut |e| {
        if let E::Qualified { alias: a, .. } = e
            && *a == alias
        {
            *a = "rr_bag_source".into();
        }
    });
    let kind = if r.combined.is_some() {
        "combined"
    } else {
        match r.kind {
            Kind::Count => "count",
            Kind::Sum => "sum",
            Kind::Min => "min",
            Kind::Max => "max",
            Kind::Join => "ordered join",
            Kind::Matches(true) => "not exists",
            Kind::Matches(false) => "exists",
        }
    };
    format!(
        "{kind}:{}:{}:{}",
        projection.render(),
        r.join.as_ref().map(E::render).unwrap_or_default(),
        r.combined
            .as_ref()
            .map(|s| serde_json::to_string(&s.layout).unwrap())
            .unwrap_or_default()
    )
}

pub struct Initial {
    owner: String,
    scope: usize,
    before: InitialCursor,
    groups: versions::Version,
    values: Option<versions::Version>,
}

pub struct InitialCursor {
    signature: Arc<str>,
    input: Input,
}
impl InitialCursor {
    pub fn equivalent(&self, other: &Self) -> bool {
        self.signature==other.signature && self.input.same(&other.input)
    }
}
pub enum InitialPrepared {
    Reused,
    Evaluate(Option<InitialCursor>),
}

pub fn prepare_initial(
    r: &Reducer,
    w: &mut Work<'_>,
    objects: &mut Vec<(String,bool)>,
) -> Result<InitialPrepared> {
    let Some(input) = input(r,w) else { return Ok(InitialPrepared::Evaluate(None)); };
    let signature = Arc::clone(r.transition_key.as_ref().unwrap());
    let before = InitialCursor { signature, input };
    let store = w.versions.unwrap();
    let found = w.summary_initializations.iter().find(|i|before.equivalent(&i.before));
    if let Some(entry) = found {
        let _context=w.profile_scope(||json!({"stage":"shared summary initialization","source_group":entry.owner,"source_scope":entry.scope,"target_group":r.groups,"target_scope":store.current_scope(),"logicalAnalysis":crate::code_rule_dsl::logic::analysis::scalars::maintenance_equivalent(true,true)}));
        let operation = w.profiler.map(|p|p.operation("shared summary initialization",("sql-extension",0),0,vec![]));
        for (name,version) in std::iter::once((&r.groups,&entry.groups))
            .chain(entry.values.as_ref().map(|v|(&r.values,v)))
        {
            if store.available(name).is_some() {
                store.replace_available(name,version.clone())?;
            } else if w.versions.is_some() {
                store.fork_readonly_version(w.db,name,version.clone())?;
            } else {
                store.fork_version(w.db,name,version.clone())?;
            }
            objects.push((name.clone(),true));
        }
        w.summary_reuses+=1;
        if let Some(operation)=operation { operation.finish(1); }
        return Ok(InitialPrepared::Reused);
    }
    Ok(InitialPrepared::Evaluate(Some(before)))
}

pub fn remember_initial(r: &Reducer, w: &mut Work<'_>, before: Option<InitialCursor>) {
    let Some(before)=before else { return; };
    let store=w.versions.unwrap();
    if w.summary_initializations.len()==128 { w.summary_initializations.remove(0); }
    w.summary_initializations.push(Initial {
        owner:r.groups.clone(),scope:store.current_scope(),before,
        groups:store.available(&r.groups).unwrap(),values:store.available(&r.values),
    });
}

pub fn initialize(
    r: &Reducer,
    w: &mut Work<'_>,
    objects: &mut Vec<(String, bool)>,
    storage: Option<&versions::Store>,
) -> Result<()> {
    if storage.is_none() || objects.iter().any(|(n,_)|n==&r.groups) || !r.shared_owner() {
        return r.initialize(w.db,objects,storage,w.versions.is_some());
    }
    match prepare_initial(r,w,objects)? {
        InitialPrepared::Reused => Ok(()),
        InitialPrepared::Evaluate(before) => {
            r.initialize(w.db,objects,storage,w.versions.is_some())?;
            remember_initial(r,w,before);
            Ok(())
        }
    }
}
