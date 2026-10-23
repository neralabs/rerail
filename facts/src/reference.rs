//! The reference evaluator (Phase 2 deliverable 5; owner S5): a naive, from-scratch evaluation of
//! the IR, written from `docs/facts/IR.md` and nothing else. No indexes, no plans, no rewrites:
//! every join is a nested loop, every subquery is re-evaluated for every outer row, every fixpoint
//! runs level by level. The gate compares the backends against it:
//!
//! - the cross-file dataflow (S3) against [`evaluate_plan`] over one worktree's members:
//!   `member(w) ⋈ exported facts`, after every edit of the replay;
//! - S3's dataflow also evaluates scalars through [`eval_scalar`], so both sides share SQLite's
//!   expression semantics (`sql::value`) and the comparison tests the relational and delta
//!   machinery.
//!
//! Errors are run-time failures of the clause (SQLite raises them too: malformed JSON, a bad
//! regex, a fixpoint over the cap) or constructs the reference does not evaluate, each named;
//! nothing is approximated.
use crate::ir::*;
use crate::sql::value::{self as sv, Aff, DKey, V};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub type Row = Vec<V>;
pub type Error = String;

/// The backends' cap on a fixpoint's rows (IR.md § Recursion): over it the clause fails.
pub const FIXPOINT_CAP: usize = 50_000_000;

pub fn lit(l: &Lit) -> V {
    match l {
        Lit::Null => V::Null,
        Lit::Int(i) => V::Int(*i),
        Lit::Real(f) => V::Real(*f),
        Lit::Text(s) => V::text(s),
    }
}

/// Truth of a value in a boolean context (`None` for NULL).
pub fn truth(v: &V) -> Option<bool> {
    sv::truth(v)
}

/// A scalar expression over `row` (depth 0) and the enclosing rows `outer` (`outer[0]` is depth
/// 1). Subqueries (`Exists`, `InRel`, `Scalar`) are refused: a caller that evaluates relations
/// decorrelates them first, or uses [`Evaluator`].
pub fn eval_scalar<S>(e: &Expr<S>, row: &[V], outer: &[&[V]]) -> Result<V, Error> {
    let mut stack: Vec<&[V]> = Vec::with_capacity(outer.len() + 1);
    stack.push(row);
    stack.extend_from_slice(outer);
    let ev: Evaluator<'_, S> = Evaluator::new(&|_| Err("eval_scalar: a subquery needs an evaluator with sources".into()));
    ev.expr(e, &stack)
}

/// Evaluates relations over a source resolver: `source(src)` yields a scanned source's rows, in
/// source order.
pub struct Evaluator<'a, S> {
    source: &'a dyn Fn(&S) -> Result<Vec<Row>, Error>,
    locals: RefCell<Vec<(LocalId, Rc<Vec<Row>>)>>,
    /// The content a per-file plan runs over (`path`, source text), for `source_text`.
    content: Option<(&'a str, &'a str)>,
}

thread_local! {
    static REGEX: RefCell<HashMap<String, Rc<regex::Regex>>> = RefCell::new(HashMap::new());
}

fn regex(p: &str) -> Result<Rc<regex::Regex>, Error> {
    REGEX.with(|c| {
        if let Some(r) = c.borrow().get(p) {
            return Ok(r.clone());
        }
        let r = Rc::new(regex::Regex::new(p).map_err(|e| format!("regex: {e}"))?);
        c.borrow_mut().insert(p.to_owned(), r.clone());
        Ok(r)
    })
}

fn dkeys(row: &[V]) -> Vec<DKey> {
    row.iter().map(sv::dkey).collect()
}

fn row_cmp(a: &[V], b: &[V], keys: &[(usize, bool)]) -> Ordering {
    for &(c, desc) in keys {
        let o = sv::cmp(&a[c], &b[c]);
        if o != Ordering::Equal {
            return if desc { o.reverse() } else { o };
        }
    }
    Ordering::Equal
}

fn int_arg(v: &V) -> Option<i64> {
    match sv::cast(v, Aff::Integer) {
        V::Int(i) => Some(i),
        _ => None,
    }
}

fn rx_text(v: V) -> Result<Rc<str>, Error> {
    match v {
        V::Null => Ok(Rc::from("")),
        V::Text(s) => Ok(s),
        _ => Err("rx_* text argument must be text".into()),
    }
}

/// `x IN (values)` with SQLite's NULL rules: an empty set is false; a NULL left side, or no match
/// with a NULL in the set, is NULL.
fn in_values(x: &V, values: impl Iterator<Item = (V, (Aff, Aff))>, negated: bool) -> V {
    let mut any = false;
    let mut null = false;
    let mut hit = false;
    for (v, aff) in values {
        any = true;
        if v.is_null() {
            null = true;
            continue;
        }
        if !x.is_null() && sv::compare(x, &v, aff) == Some(Ordering::Equal) {
            hit = true;
            break;
        }
    }
    let r = if !any {
        Some(false)
    } else if x.is_null() {
        None
    } else if hit {
        Some(true)
    } else if null {
        None
    } else {
        Some(false)
    };
    match r {
        None => V::Null,
        Some(b) => V::bool(b != negated),
    }
}

fn json_to_sql(j: &serde_json::Value) -> V {
    sv::json_value_to_sql(j)
}

fn json_type(j: &serde_json::Value) -> &'static str {
    match j {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(true) => "true",
        serde_json::Value::Bool(false) => "false",
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        serde_json::Value::Number(_) => "real",
        serde_json::Value::String(_) => "text",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// `json_each(text)`'s rows: `key, value, type, atom, id, parent, fullkey, path`. Array elements
/// in order, object members in key order (IR.md § Relations); a scalar document is one row with a
/// NULL key. `id` and `parent` are NULL (the rules never read them).
fn json_each_rows(text: &str) -> Result<Vec<Row>, Error> {
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|_| "malformed JSON".to_string())?;
    let elem = |key: V, v: &serde_json::Value, fullkey: String| -> Row {
        let value = json_to_sql(v);
        let atom = if matches!(v, serde_json::Value::Array(_) | serde_json::Value::Object(_)) { V::Null } else { value.clone() };
        vec![key, value, V::text(json_type(v)), atom, V::Null, V::Null, V::text(&fullkey), V::text("$")]
    };
    Ok(match &doc {
        serde_json::Value::Array(a) => a.iter().enumerate().map(|(i, v)| elem(V::Int(i as i64), v, format!("$[{i}]"))).collect(),
        serde_json::Value::Object(o) => o
            .iter()
            .map(|(k, v)| {
                let quoted = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.is_empty() { k.clone() } else { format!("\"{k}\"") };
                elem(V::text(k), v, format!("$.{quoted}"))
            })
            .collect(),
        v => vec![{
            let mut r = elem(V::Null, v, "$".into());
            r[7] = V::text("$");
            r
        }],
    })
}

/// The output width of a relation (IR.md § Relations' schemas).
pub fn width<S>(r: &Rel<S>) -> usize {
    match r {
        Rel::Scan { schema, .. } | Rel::Local { schema, .. } | Rel::Project { schema, .. } => schema.len(),
        Rel::Unit => 0,
        Rel::JsonEach { input, .. } => width(input) + 8,
        Rel::Join { left, right, .. } => width(left) + width(right),
        Rel::Filter { input, .. } | Rel::Distinct { input, .. } | Rel::Sort { input, .. } => width(input),
        Rel::Aggregate { input, aggs, .. } => width(input) + aggs.len(),
        Rel::Window { input, funcs } => width(input) + funcs.len(),
        Rel::Union { left, .. } => width(left),
        Rel::With { body, .. } => width(body),
    }
}

/// Refuses (`tie: …`, IR.md § Order) when two distinct rows of `rows`, sorted on `keys`, have equal
/// keys: an order marked `Total::Checked` would otherwise break the tie by input order. Rows are
/// compared whole, so a sort on a row's only column never ties.
fn tie_free(rows: &[Row], keys: &[(usize, bool)], what: &str) -> Result<(), Error> {
    for w in rows.windows(2) {
        if row_cmp(&w[0], &w[1], keys) == Ordering::Equal && dkeys(&w[0]) != dkeys(&w[1]) {
            return Err(format!("tie: {what} between distinct rows on equal keys"));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Agg {
    count: i64,
    best: Option<V>,
    concat: Option<String>,
    sum_i: Option<i64>,
    sum_f: Option<f64>,
    json: Vec<serde_json::Value>,
    seen: HashSet<DKey>,
}

fn json_of(v: &V) -> serde_json::Value {
    match v {
        V::Null => serde_json::Value::Null,
        V::Int(i) => serde_json::Value::from(*i),
        V::Real(f) => serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, serde_json::Value::Number),
        V::Text(s) => serde_json::Value::String(s.to_string()),
    }
}

impl Agg {
    /// Folds one row's value; true when a `min`/`max` took it (strictly better than before).
    fn add(&mut self, f: AggFn, distinct: bool, v: Option<V>, sep: Option<V>) -> Result<bool, Error> {
        if f == AggFn::CountStar {
            self.count += 1;
            return Ok(false);
        }
        let v = v.unwrap_or(V::Null);
        if f == AggFn::JsonGroupArray {
            // json_group_array keeps NULLs.
            if distinct && !self.seen.insert(sv::dkey(&v)) {
                return Ok(false);
            }
            self.json.push(json_of(&v));
            return Ok(false);
        }
        if v.is_null() {
            return Ok(false);
        }
        if distinct && !self.seen.insert(sv::dkey(&v)) {
            return Ok(false);
        }
        match f {
            AggFn::Count => self.count += 1,
            AggFn::Min | AggFn::Max => {
                let better = self.best.as_ref().is_none_or(|b| {
                    let o = sv::cmp(&v, b);
                    (f == AggFn::Max && o == Ordering::Greater) || (f == AggFn::Min && o == Ordering::Less)
                });
                if better {
                    self.best = Some(v);
                    return Ok(true);
                }
            }
            AggFn::GroupConcat => {
                let t = sv::to_text(&v).unwrap_or_else(|| Rc::from(""));
                match &mut self.concat {
                    None => self.concat = Some(t.to_string()),
                    Some(c) => {
                        match sep {
                            None => c.push(','),
                            Some(s) => c.push_str(&sv::to_text(&s).map(|t| t.to_string()).unwrap_or_default()),
                        }
                        c.push_str(&t);
                    }
                }
            }
            AggFn::Sum | AggFn::Total => match (&v, self.sum_f) {
                (V::Int(i), None) => {
                    let s = self.sum_i.unwrap_or(0).checked_add(*i).ok_or("integer overflow")?;
                    self.sum_i = Some(s);
                }
                _ => {
                    // Anything but an integer makes the sum a REAL (SQLite's sum()).
                    let x = match sv::to_numeric(&v) {
                        Some(V::Int(i)) => i as f64,
                        Some(V::Real(r)) => r,
                        _ => 0.0,
                    };
                    self.sum_f = Some(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64) + x);
                }
            },
            AggFn::CountStar | AggFn::JsonGroupArray => unreachable!(),
        }
        Ok(false)
    }

    fn finish(self, f: AggFn) -> V {
        match f {
            AggFn::Count | AggFn::CountStar => V::Int(self.count),
            AggFn::Min | AggFn::Max => self.best.unwrap_or(V::Null),
            AggFn::GroupConcat => self.concat.map_or(V::Null, |c| V::text(&c)),
            AggFn::Sum => match (self.sum_i, self.sum_f) {
                (_, Some(f)) => V::Real(f),
                (Some(i), None) => V::Int(i),
                (None, None) => V::Null,
            },
            AggFn::Total => V::Real(self.sum_f.unwrap_or(self.sum_i.unwrap_or(0) as f64)),
            AggFn::JsonGroupArray => V::text(&serde_json::Value::Array(self.json).to_string()),
        }
    }
}

impl<'a, S> Evaluator<'a, S> {
    pub fn new(source: &'a dyn Fn(&S) -> Result<Vec<Row>, Error>) -> Self {
        Self { source, locals: RefCell::new(Vec::new()), content: None }
    }

    /// An evaluator over one content: `source_text(p, s, e)` is the byte range `[s, e)` of
    /// `text` when `p` is `path` (as the per-file executor: NULL for another path or a range
    /// outside the text).
    pub fn with_content(source: &'a dyn Fn(&S) -> Result<Vec<Row>, Error>, path: &'a str, text: &'a str) -> Self {
        Self { source, locals: RefCell::new(Vec::new()), content: Some((path, text)) }
    }

    fn local(&self, id: LocalId) -> Result<Rc<Vec<Row>>, Error> {
        self.locals.borrow().iter().rev().find(|(i, _)| *i == id).map(|(_, r)| r.clone()).ok_or_else(|| format!("unbound local {id}"))
    }

    /// One relation's rows in IR order; `outer` holds the enclosing rows of a correlated subquery
    /// (`outer[0]` is depth 1 for this relation's expressions).
    pub fn rel(&self, r: &Rel<S>, outer: &[&[V]]) -> Result<Vec<Row>, Error> {
        let with = |row: &[V], f: &mut dyn FnMut(&[&[V]]) -> Result<(), Error>| -> Result<(), Error> {
            let mut stack: Vec<&[V]> = Vec::with_capacity(outer.len() + 1);
            stack.push(row);
            stack.extend_from_slice(outer);
            f(&stack)
        };
        match r {
            Rel::Scan { src, .. } => (self.source)(src),
            Rel::Local { id, .. } => Ok((*self.local(*id)?).clone()),
            Rel::Unit => Ok(vec![Vec::new()]),
            Rel::JsonEach { input, arg, on, outer: keep } => {
                let n = width(r) - width(input);
                let mut out = Vec::new();
                for row in self.rel(input, outer)? {
                    let mut a = V::Null;
                    with(&row, &mut |s| {
                        a = self.expr(arg, s)?;
                        Ok(())
                    })?;
                    let elems = match &a {
                        V::Null => Vec::new(),
                        v => json_each_rows(&sv::to_text(v).unwrap())?,
                    };
                    let mut matched = false;
                    for e in elems {
                        let mut joined = row.clone();
                        joined.extend(e);
                        let mut keep_row = true;
                        if let Some(on) = on {
                            with(&joined, &mut |s| {
                                keep_row = sv::truth(&self.expr(on, s)?) == Some(true);
                                Ok(())
                            })?;
                        }
                        if keep_row {
                            matched = true;
                            out.push(joined);
                        }
                    }
                    if *keep && !matched {
                        let mut joined = row.clone();
                        joined.extend(std::iter::repeat_n(V::Null, n));
                        out.push(joined);
                    }
                }
                Ok(out)
            }
            Rel::Join { kind, left, right, on } => {
                let l = self.rel(left, outer)?;
                let rw = width(right);
                let rr = self.rel(right, outer)?;
                let mut out = Vec::new();
                for a in &l {
                    let mut matched = false;
                    for b in &rr {
                        let mut joined = a.clone();
                        joined.extend(b.iter().cloned());
                        let mut keep = true;
                        if let Some(on) = on {
                            with(&joined, &mut |s| {
                                keep = sv::truth(&self.expr(on, s)?) == Some(true);
                                Ok(())
                            })?;
                        }
                        if keep {
                            matched = true;
                            out.push(joined);
                        }
                    }
                    if *kind == JoinKind::Left && !matched {
                        let mut joined = a.clone();
                        joined.extend(std::iter::repeat_n(V::Null, rw));
                        out.push(joined);
                    }
                }
                Ok(out)
            }
            Rel::Filter { input, pred } => {
                let mut out = Vec::new();
                for row in self.rel(input, outer)? {
                    let mut keep = false;
                    with(&row, &mut |s| {
                        keep = sv::truth(&self.expr(pred, s)?) == Some(true);
                        Ok(())
                    })?;
                    if keep {
                        out.push(row);
                    }
                }
                Ok(out)
            }
            Rel::Project { input, exprs, .. } => {
                let mut out = Vec::new();
                for row in self.rel(input, outer)? {
                    let mut p = Vec::with_capacity(exprs.len());
                    with(&row, &mut |s| {
                        for e in exprs {
                            p.push(self.expr(e, s)?);
                        }
                        Ok(())
                    })?;
                    out.push(p);
                }
                Ok(out)
            }
            Rel::Aggregate { input, keys, aggs } => {
                let w = width(input);
                // An ordered aggregate whose order is not proven total: its input rows paired with the
                // keys of the Sort they come from (as S3's generator orders it), tie-checked per group.
                let ordered = aggs.iter().any(|a| a.total == Total::Checked && matches!(a.f, AggFn::GroupConcat | AggFn::JsonGroupArray));
                let (rows, sort_keys) = if ordered {
                    let (rows, k) = self.keyed(input, outer)?;
                    (rows, Some(k))
                } else {
                    (self.rel(input, outer)?, None)
                };
                let mut tied: HashMap<(Vec<DKey>, Vec<DKey>), Vec<DKey>> = HashMap::new();
                // Groups in first-seen order of their keys.
                let mut order: Vec<Vec<DKey>> = Vec::new();
                let mut groups: HashMap<Vec<DKey>, (Vec<Agg>, Option<Row>, Option<Row>)> = HashMap::new();
                for (i, row) in rows.iter().enumerate() {
                    let mut k = Vec::with_capacity(keys.len());
                    let mut vals: Vec<(Option<V>, Option<V>)> = Vec::with_capacity(aggs.len());
                    with(row, &mut |s| {
                        for e in keys {
                            k.push(sv::dkey(&self.expr(e, s)?));
                        }
                        for a in aggs {
                            let v = a.arg.as_ref().map(|e| self.expr(e, s)).transpose()?;
                            let sep = a.sep.as_ref().map(|e| self.expr(e, s)).transpose()?;
                            vals.push((v, sep));
                        }
                        Ok(())
                    })?;
                    if let Some(sk) = &sort_keys {
                        let first = tied.entry((k.clone(), dkeys(&sk[i]))).or_insert_with(|| dkeys(row));
                        if *first != dkeys(row) {
                            return Err("tie: ordered aggregate (group_concat/json_group_array) between distinct rows of one group on equal sort keys".into());
                        }
                    }
                    let g = groups.entry(k.clone()).or_insert_with(|| {
                        order.push(k);
                        (aggs.iter().map(|_| Agg::default()).collect(), None, None)
                    });
                    let mut picked = false;
                    for ((state, a), (v, sep)) in g.0.iter_mut().zip(aggs).zip(vals) {
                        picked |= state.add(a.f, a.distinct, v, sep)?;
                    }
                    if picked {
                        g.1 = Some(row.clone());
                    }
                    g.2 = Some(row.clone());
                }
                if rows.is_empty() && keys.is_empty() {
                    order.push(Vec::new());
                    groups.insert(Vec::new(), (aggs.iter().map(|_| Agg::default()).collect(), None, None));
                }
                // Representative row: the row a lone min/max picked, else the group's last row.
                let lone = aggs.len() == 1 && matches!(aggs[0].f, AggFn::Min | AggFn::Max);
                let mut out = Vec::with_capacity(order.len());
                for k in order {
                    let (states, picked, last) = groups.remove(&k).unwrap();
                    let mut row = if lone { picked.or(last) } else { last }.unwrap_or_else(|| vec![V::Null; w]);
                    for (s, a) in states.into_iter().zip(aggs) {
                        row.push(s.finish(a.f));
                    }
                    out.push(row);
                }
                Ok(out)
            }
            Rel::Window { input, funcs } => {
                let mut rows = self.rel(input, outer)?;
                let mut cols: Vec<Vec<V>> = Vec::with_capacity(funcs.len());
                for f in funcs {
                    // Per row: partition values and order keys.
                    let mut part: Vec<Vec<V>> = Vec::with_capacity(rows.len());
                    let mut ord: Vec<Vec<V>> = Vec::with_capacity(rows.len());
                    let mut arg: Vec<V> = Vec::with_capacity(rows.len());
                    for row in &rows {
                        with(row, &mut |s| {
                            part.push(f.partition.iter().map(|e| self.expr(e, s)).collect::<Result<_, _>>()?);
                            ord.push(f.order.iter().map(|(e, _)| self.expr(e, s)).collect::<Result<_, _>>()?);
                            arg.push(match &f.kind {
                                WindowKind::Min(e) | WindowKind::Max(e) => self.expr(e, s)?,
                                WindowKind::RowNumber => V::Null,
                            });
                            Ok(())
                        })?;
                    }
                    let mut by_part: HashMap<Vec<DKey>, Vec<usize>> = HashMap::new();
                    for (i, p) in part.iter().enumerate() {
                        by_part.entry(dkeys(p)).or_default().push(i);
                    }
                    let mut col = vec![V::Null; rows.len()];
                    for members in by_part.values() {
                        match &f.kind {
                            WindowKind::RowNumber => {
                                let mut m = members.clone();
                                let keys: Vec<(usize, bool)> = f.order.iter().enumerate().map(|(i, (_, d))| (i, *d)).collect();
                                // Total order: order keys, then input position.
                                m.sort_by(|&a, &b| row_cmp(&ord[a], &ord[b], &keys).then(a.cmp(&b)));
                                if f.total == Total::Checked {
                                    // Not proven total: distinct rows on equal keys would be numbered by input order.
                                    for w in m.windows(2) {
                                        if row_cmp(&ord[w[0]], &ord[w[1]], &keys) == Ordering::Equal && dkeys(&rows[w[0]]) != dkeys(&rows[w[1]]) {
                                            return Err("tie: row_number() between distinct rows on equal partition and order keys".into());
                                        }
                                    }
                                }
                                for (n, i) in m.into_iter().enumerate() {
                                    col[i] = V::Int(n as i64 + 1);
                                }
                            }
                            WindowKind::Min(_) | WindowKind::Max(_) => {
                                let max = matches!(f.kind, WindowKind::Max(_));
                                let mut best: Option<&V> = None;
                                for &i in members {
                                    let v = &arg[i];
                                    if v.is_null() {
                                        continue;
                                    }
                                    if best.is_none_or(|b| {
                                        let o = sv::cmp(v, b);
                                        if max { o == Ordering::Greater } else { o == Ordering::Less }
                                    }) {
                                        best = Some(v);
                                    }
                                }
                                let b = best.cloned().unwrap_or(V::Null);
                                for &i in members {
                                    col[i] = b.clone();
                                }
                            }
                        }
                    }
                    cols.push(col);
                }
                for (i, row) in rows.iter_mut().enumerate() {
                    for c in &cols {
                        row.push(c[i].clone());
                    }
                }
                Ok(rows)
            }
            Rel::Distinct { input, prefix } => {
                let mut seen = HashSet::new();
                Ok(self.rel(input, outer)?.into_iter().filter(|row| seen.insert(dkeys(&row[..*prefix as usize]))).collect())
            }
            Rel::Union { all, left, right } => {
                let mut rows = self.rel(left, outer)?;
                rows.extend(self.rel(right, outer)?);
                if *all {
                    return Ok(rows);
                }
                let mut seen = HashSet::new();
                Ok(rows.into_iter().filter(|row| seen.insert(dkeys(row))).collect())
            }
            Rel::Sort { input, keys, limit, offset, total } => {
                let mut rows = self.rel(input, outer)?;
                let keys: Vec<(usize, bool)> = keys.iter().map(|(c, d)| (*c as usize, *d)).collect();
                rows.sort_by(|a, b| row_cmp(a, b, &keys));
                if *total == Total::Checked && (limit.is_some() || offset.is_some()) {
                    // As S3's dataflow: any two distinct rows on equal sort keys, wherever the cut falls.
                    tie_free(&rows, &keys, "ORDER BY … LIMIT/OFFSET")?;
                }
                let constant = |e: &Option<Expr<S>>| -> Result<Option<i64>, Error> {
                    let Some(e) = e else { return Ok(None) };
                    let mut v = V::Null;
                    with(&[], &mut |s| {
                        v = self.expr(e, s)?;
                        Ok(())
                    })?;
                    Ok(int_arg(&v))
                };
                let off = constant(offset)?.unwrap_or(0).max(0) as usize;
                let lim = constant(limit)?.filter(|l| *l >= 0).map(|l| l as usize);
                Ok(rows.into_iter().skip(off).take(lim.unwrap_or(usize::MAX)).collect())
            }
            Rel::With { bindings, body } => {
                let depth = self.locals.borrow().len();
                let result = (|| {
                    for (id, b) in bindings {
                        let rows = match b {
                            Binding::Plain(rel) => self.rel(rel, outer)?,
                            Binding::Fix { init, step, set, .. } => self.fixpoint(*id, init, step, *set, outer)?,
                        };
                        self.locals.borrow_mut().push((*id, Rc::new(rows)));
                    }
                    self.rel(body, outer)
                })();
                self.locals.borrow_mut().truncate(depth);
                result
            }
        }
    }

    /// `WITH RECURSIVE` level by level: each round evaluates every step arm with `Local(id)` bound
    /// to the previous round's new rows; `set` admits a row only once (init rows included).
    fn fixpoint(&self, id: LocalId, init: &[Rel<S>], step: &[Rel<S>], set: bool, outer: &[&[V]]) -> Result<Vec<Row>, Error> {
        let mut seen: HashSet<Vec<DKey>> = HashSet::new();
        let mut result: Vec<Row> = Vec::new();
        let mut level: Vec<Row> = Vec::new();
        for arm in init {
            for row in self.rel(arm, outer)? {
                if !set || seen.insert(dkeys(&row)) {
                    level.push(row.clone());
                    result.push(row);
                }
            }
        }
        while !level.is_empty() {
            if result.len() > FIXPOINT_CAP {
                return Err("recursive CTE exceeded 50M rows".into());
            }
            self.locals.borrow_mut().push((id, Rc::new(std::mem::take(&mut level))));
            let next = (|| {
                let mut next = Vec::new();
                for arm in step {
                    next.extend(self.rel(arm, outer)?);
                }
                Ok::<_, Error>(next)
            })();
            self.locals.borrow_mut().pop();
            for row in next? {
                if !set || seen.insert(dkeys(&row)) {
                    level.push(row.clone());
                    result.push(row);
                }
            }
        }
        Ok(result)
    }

    /// An ordered aggregate's input rows and, per row, the keys that order it: those of a `Sort`
    /// reached through `Project`/`Filter` only (each row keeps its source row's keys). With no such
    /// `Sort` the order is input order and every row's keys are empty, so any two distinct rows of
    /// a group tie.
    fn keyed(&self, r: &Rel<S>, outer: &[&[V]]) -> Result<(Vec<Row>, Vec<Vec<V>>), Error> {
        let frame = |row: &[V], f: &mut dyn FnMut(&[&[V]]) -> Result<(), Error>| -> Result<(), Error> {
            let mut stack: Vec<&[V]> = Vec::with_capacity(outer.len() + 1);
            stack.push(row);
            stack.extend_from_slice(outer);
            f(&stack)
        };
        match r {
            Rel::Sort { keys, .. } => {
                let rows = self.rel(r, outer)?;
                let k = rows.iter().map(|row| keys.iter().map(|(c, _)| row[*c as usize].clone()).collect()).collect();
                Ok((rows, k))
            }
            Rel::Filter { input, pred } => {
                let (rows, keys) = self.keyed(input, outer)?;
                let (mut out, mut out_keys) = (Vec::new(), Vec::new());
                for (row, k) in rows.into_iter().zip(keys) {
                    let mut keep = false;
                    frame(&row, &mut |s| {
                        keep = sv::truth(&self.expr(pred, s)?) == Some(true);
                        Ok(())
                    })?;
                    if keep {
                        out.push(row);
                        out_keys.push(k);
                    }
                }
                Ok((out, out_keys))
            }
            Rel::Project { input, exprs, .. } => {
                let (rows, keys) = self.keyed(input, outer)?;
                let mut out = Vec::with_capacity(rows.len());
                for row in &rows {
                    let mut p = Vec::with_capacity(exprs.len());
                    frame(row, &mut |s| {
                        for e in exprs {
                            p.push(self.expr(e, s)?);
                        }
                        Ok(())
                    })?;
                    out.push(p);
                }
                Ok((out, keys))
            }
            _ => {
                let rows = self.rel(r, outer)?;
                let n = rows.len();
                Ok((rows, vec![Vec::new(); n]))
            }
        }
    }

    /// A scalar expression; `stack[0]` is the current row, `stack[d]` the row `d` levels out.
    pub fn expr(&self, e: &Expr<S>, stack: &[&[V]]) -> Result<V, Error> {
        let ev = |x: &Expr<S>| self.expr(x, stack);
        Ok(match e {
            Expr::Lit(l) => lit(l),
            Expr::Col { depth, col } => stack
                .get(*depth as usize)
                .and_then(|r| r.get(*col as usize))
                .cloned()
                .ok_or_else(|| format!("column {col} at depth {depth} is out of range"))?,
            Expr::Not(x) => match sv::truth(&ev(x)?) {
                None => V::Null,
                Some(b) => V::bool(!b),
            },
            Expr::Neg(x) => match sv::to_numeric(&ev(x)?) {
                None => V::Null,
                Some(V::Int(i)) => i.checked_neg().map_or(V::Real(-(i as f64)), V::Int),
                Some(V::Real(f)) => V::Real(-f),
                Some(_) => V::Null,
            },
            Expr::Plus(x) => ev(x)?,
            Expr::Arith(op, a, b) => {
                let c = match op {
                    ArithOp::Add => '+',
                    ArithOp::Sub => '-',
                    ArithOp::Mul => '*',
                    ArithOp::Div => '/',
                    ArithOp::Rem => '%',
                };
                sv::arith(c, &ev(a)?, &ev(b)?)
            }
            Expr::Concat(a, b) => match (sv::to_text(&ev(a)?), sv::to_text(&ev(b)?)) {
                (Some(x), Some(y)) => V::text(&format!("{x}{y}")),
                _ => V::Null,
            },
            Expr::And(a, b) => {
                let l = sv::truth(&ev(a)?);
                if l == Some(false) {
                    V::bool(false)
                } else {
                    match (l, sv::truth(&ev(b)?)) {
                        (_, Some(false)) => V::bool(false),
                        (Some(true), Some(true)) => V::bool(true),
                        _ => V::Null,
                    }
                }
            }
            Expr::Or(a, b) => {
                let l = sv::truth(&ev(a)?);
                if l == Some(true) {
                    V::bool(true)
                } else {
                    match (l, sv::truth(&ev(b)?)) {
                        (_, Some(true)) => V::bool(true),
                        (Some(false), Some(false)) => V::bool(false),
                        _ => V::Null,
                    }
                }
            }
            Expr::Cmp(op, a, b, aff) => match sv::compare(&ev(a)?, &ev(b)?, *aff) {
                None => V::Null,
                Some(o) => V::bool(match op {
                    CmpOp::Eq => o == Ordering::Equal,
                    CmpOp::Ne => o != Ordering::Equal,
                    CmpOp::Lt => o == Ordering::Less,
                    CmpOp::Le => o != Ordering::Greater,
                    CmpOp::Gt => o == Ordering::Greater,
                    CmpOp::Ge => o != Ordering::Less,
                }),
            },
            Expr::Is { left, right, negated } => {
                let (a, b) = (ev(left)?, ev(right)?);
                let same = match (a.is_null(), b.is_null()) {
                    (true, true) => true,
                    (false, false) => sv::compare(&a, &b, (Aff::None, Aff::None)) == Some(Ordering::Equal),
                    _ => false,
                };
                V::bool(same != *negated)
            }
            Expr::IsNull { e, negated } => V::bool(ev(e)?.is_null() != *negated),
            Expr::Like { text, pattern, negated } => match (sv::to_text(&ev(text)?), sv::to_text(&ev(pattern)?)) {
                (Some(t), Some(p)) => V::bool(sv::like(&p, &t) != *negated),
                _ => V::Null,
            },
            Expr::Func(f, args) => self.func(*f, args, stack)?,
            Expr::Case { operand, whens, else_ } => {
                let op = operand.as_ref().map(|o| ev(o)).transpose()?;
                for (w, t, aff) in whens {
                    let hit = match &op {
                        Some(v) => sv::compare(v, &ev(w)?, *aff) == Some(Ordering::Equal),
                        None => sv::truth(&ev(w)?) == Some(true),
                    };
                    if hit {
                        return ev(t);
                    }
                }
                match else_ {
                    Some(x) => ev(x)?,
                    None => V::Null,
                }
            }
            Expr::Cast(x, aff) => sv::cast(&ev(x)?, *aff),
            Expr::InList { e, list, negated } => {
                let x = ev(e)?;
                let vals = list.iter().map(|(v, aff)| Ok((ev(v)?, *aff))).collect::<Result<Vec<_>, Error>>()?;
                in_values(&x, vals.into_iter(), *negated)
            }
            Expr::InRel { e, rel, negated, aff } => {
                let x = ev(e)?;
                let rows = self.rel(rel, stack)?;
                in_values(&x, rows.into_iter().map(|r| (r.into_iter().next().unwrap_or(V::Null), *aff)), *negated)
            }
            Expr::Exists { rel, negated } => V::bool(self.rel(rel, stack)?.is_empty() == *negated),
            Expr::Scalar(rel) => self.rel(rel, stack)?.into_iter().next().and_then(|r| r.into_iter().next()).unwrap_or(V::Null),
        })
    }

    fn func(&self, f: Func, args: &[Expr<S>], stack: &[&[V]]) -> Result<V, Error> {
        let a = |i: usize| -> Result<V, Error> { args.get(i).map_or(Ok(V::Null), |x| self.expr(x, stack)) };
        Ok(match f {
            Func::Coalesce | Func::Ifnull => {
                for x in args {
                    let v = self.expr(x, stack)?;
                    if !v.is_null() {
                        return Ok(v);
                    }
                }
                V::Null
            }
            Func::Length => match a(0)? {
                V::Null => V::Null,
                v => V::Int(sv::to_text(&v).map_or(0, |s| s.chars().count() as i64)),
            },
            Func::Lower | Func::Upper => match sv::to_text(&a(0)?) {
                None => V::Null,
                Some(s) => V::text(&if f == Func::Lower { s.to_ascii_lowercase() } else { s.to_ascii_uppercase() }),
            },
            Func::Trim | Func::Ltrim | Func::Rtrim => {
                let Some(s) = sv::to_text(&a(0)?) else { return Ok(V::Null) };
                let chars = if args.len() > 1 {
                    match sv::to_text(&a(1)?) {
                        Some(c) => Some(c),
                        None => return Ok(V::Null),
                    }
                } else {
                    None
                };
                V::text(&sv::trim(&s, chars.as_deref(), f != Func::Rtrim, f != Func::Ltrim))
            }
            Func::Substr => {
                let Some(s) = sv::to_text(&a(0)?) else { return Ok(V::Null) };
                let Some(start) = int_arg(&a(1)?) else { return Ok(V::Null) };
                let len = if args.len() > 2 {
                    match int_arg(&a(2)?) {
                        Some(l) => Some(l),
                        None => return Ok(V::Null),
                    }
                } else {
                    None
                };
                V::text(&sv::substr(&s, start, len))
            }
            Func::Instr => match (sv::to_text(&a(0)?), sv::to_text(&a(1)?)) {
                (Some(h), Some(n)) => V::Int(h.find(&*n).map_or(0, |b| h[..b].chars().count() as i64 + 1)),
                _ => V::Null,
            },
            Func::Replace => match (sv::to_text(&a(0)?), sv::to_text(&a(1)?), sv::to_text(&a(2)?)) {
                (Some(s), Some(from), Some(to)) => {
                    if from.is_empty() {
                        V::Text(s)
                    } else {
                        V::text(&s.replace(&*from, &to))
                    }
                }
                _ => V::Null,
            },
            Func::Char => {
                let mut s = String::new();
                for i in 0..args.len() {
                    if let Some(ch) = int_arg(&a(i)?).and_then(|c| char::from_u32(c as u32)) {
                        s.push(ch);
                    }
                }
                V::text(&s)
            }
            Func::Abs => match sv::to_numeric(&a(0)?) {
                Some(V::Int(i)) => V::Int(i.checked_abs().ok_or("integer overflow")?),
                Some(V::Real(r)) => V::Real(r.abs()),
                _ => V::Null,
            },
            Func::Typeof => V::text(match a(0)? {
                V::Null => "null",
                V::Int(_) => "integer",
                V::Real(_) => "real",
                V::Text(_) => "text",
            }),
            Func::Max | Func::Min => {
                let mut best: Option<V> = None;
                for i in 0..args.len() {
                    let v = a(i)?;
                    if v.is_null() {
                        return Ok(V::Null);
                    }
                    best = Some(match best {
                        None => v,
                        Some(b) => {
                            let o = sv::cmp(&v, &b);
                            if (f == Func::Max && o == Ordering::Greater) || (f == Func::Min && o == Ordering::Less) { v } else { b }
                        }
                    });
                }
                best.unwrap_or(V::Null)
            }
            Func::Iif => {
                if sv::truth(&a(0)?) == Some(true) {
                    a(1)?
                } else {
                    a(2)?
                }
            }
            Func::Nullif => {
                let (x, y) = (a(0)?, a(1)?);
                if sv::compare(&x, &y, (Aff::None, Aff::None)) == Some(Ordering::Equal) { V::Null } else { x }
            }
            Func::JsonQuote => sv::json_quote(&a(0)?),
            Func::JsonExtract => match (sv::to_text(&a(0)?), sv::to_text(&a(1)?)) {
                (Some(j), Some(p)) => sv::json_extract(&j, &p)?,
                _ => V::Null,
            },
            Func::RxMatches => {
                let t = rx_text(a(0)?)?;
                let V::Text(p) = a(1)? else { return Err("rx pattern must be text".into()) };
                V::bool(regex(&p)?.is_match(&t))
            }
            Func::RxExtract => {
                let t = rx_text(a(0)?)?;
                let V::Text(p) = a(1)? else { return Err("rx pattern must be text".into()) };
                let g = match a(2)? {
                    V::Int(g) if g >= 0 => g as usize,
                    _ => return Err("rx_extract group must be a non-negative integer".into()),
                };
                let r = regex(&p)?;
                if g >= r.captures_len() {
                    return Err("unknown regex group".into());
                }
                r.captures(&t).and_then(|c| c.get(g)).map_or(V::Null, |m| V::text(m.as_str()))
            }
            Func::RxReplace => {
                let t = rx_text(a(0)?)?;
                let (V::Text(p), V::Text(by)) = (a(1)?, a(2)?) else { return Err("rx_replace pattern and replacement must be text".into()) };
                V::text(&regex(&p)?.replace_all(&t, &*by))
            }
            Func::SourceText => {
                let Some((path, text)) = self.content else {
                    return Err("reference: source_text reads a content's text; repository plans have none (IR.md § 7)".into());
                };
                match (a(0)?, a(1)?, a(2)?) {
                    (V::Text(p), V::Int(s), V::Int(e)) if &*p == path && s >= 0 && e >= s => text.get(s as usize..e as usize).map_or(V::Null, V::text),
                    (p, _, _) if !matches!(p, V::Text(_) | V::Null) => return Err("source_text path must be text".into()),
                    _ => V::Null,
                }
            }
            Func::JsonArray => {
                let mut items = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    items.push(json_of(&a(i)?));
                }
                V::text(&serde_json::Value::Array(items).to_string())
            }
            Func::Printf => {
                let Some(fmt) = sv::to_text(&a(0)?) else { return Ok(V::Null) };
                let mut vals = Vec::with_capacity(args.len().saturating_sub(1));
                for i in 1..args.len() {
                    vals.push(a(i)?);
                }
                V::text(&sv::printf(&fmt, &vals)?)
            }
        })
    }
}

/// A plan's stages in order, then its sink's relation: the rows a plan produces. `scan` resolves
/// the plan's non-stage sources (for a repository plan: an exported relation's rows over the
/// worktree's members). Stage rows are stored with their columns' affinities (CTAS).
pub fn evaluate_plan<S: Clone>(plan: &Plan<S>, stage_of: impl Fn(&S) -> Option<StageId>, scan: &dyn Fn(&S) -> Result<Vec<Row>, Error>) -> Result<Vec<Row>, Error> {
    let stages: RefCell<Vec<Rc<Vec<Row>>>> = RefCell::new(Vec::new());
    let source = |s: &S| -> Result<Vec<Row>, Error> {
        match stage_of(s) {
            Some(id) => stages.borrow().get(id as usize).map(|r| (**r).clone()).ok_or_else(|| format!("stage {id} read before it is computed")),
            None => scan(s),
        }
    };
    let ev = Evaluator::new(&source);
    for st in &plan.stages {
        let rows = match &st.def {
            StageDef::Query { rel } => {
                let mut rows = ev.rel(rel, &[])?;
                for row in rows.iter_mut() {
                    for (v, col) in row.iter_mut().zip(&st.schema) {
                        *v = sv::apply_affinity(std::mem::replace(v, V::Null), col.aff);
                    }
                }
                rows
            }
            StageDef::Reach { roots, edges, graph } => reach(&ev.rel(roots, &[])?, edges.as_ref().map(|e| ev.rel(e, &[])).transpose()?.as_deref(), graph)?,
        };
        stages.borrow_mut().push(Rc::new(rows));
    }
    match &plan.sink {
        Sink::Findings { rel, .. } => ev.rel(rel, &[]),
        Sink::Export(_) => Err("reference: an Export sink is the per-file side of a clause, not a repository result".into()),
        Sink::Witnesses { .. } => Err("reference: a forbid sink reads one content's input".into()),
        Sink::Budget { .. } => Err("reference: budget sinks (closures, conditions) are not evaluated by the reference yet".into()),
    }
}

/// Reachability `(__path, root, node)`: reflexive, set semantics, per path (IR.md § Recursion).
fn reach(roots: &[Row], edges: Option<&[Row]>, graph: &Graph) -> Result<Vec<Row>, Error> {
    if *graph == Graph::Syntax {
        return Err("reference: syntax reachability reads one content's facts (a per-file plan)".into());
    }
    let edges = edges.ok_or("reachability without edges")?;
    let mut adj: HashMap<(DKey, DKey), Vec<V>> = HashMap::new();
    let mut seen_edge = HashSet::new();
    let mut parent: HashMap<(DKey, DKey), DKey> = HashMap::new();
    for e in edges {
        if e.len() < 3 || e[0].is_null() || e[1].is_null() || e[2].is_null() {
            continue;
        }
        if !seen_edge.insert(dkeys(&e[..3])) {
            continue;
        }
        if *graph == Graph::Forest && parent.insert((sv::dkey(&e[0]), sv::dkey(&e[2])), sv::dkey(&e[1])).is_some() {
            return Err("forest node has multiple parents".into());
        }
        adj.entry((sv::dkey(&e[0]), sv::dkey(&e[1]))).or_default().push(e[2].clone());
    }
    let mut out = Vec::new();
    let mut seen_root = HashSet::new();
    for r in roots {
        if r.len() < 2 || r[0].is_null() || r[1].is_null() || !seen_root.insert(dkeys(&r[..2])) {
            continue;
        }
        let pk = sv::dkey(&r[0]);
        let mut visited: HashSet<DKey> = HashSet::from([sv::dkey(&r[1])]);
        let mut frontier = vec![r[1].clone()];
        out.push(vec![r[0].clone(), r[1].clone(), r[1].clone()]);
        while let Some(n) = frontier.pop() {
            for c in adj.get(&(pk.clone(), sv::dkey(&n))).map(Vec::as_slice).unwrap_or(&[]) {
                if visited.insert(sv::dkey(c)) {
                    out.push(vec![r[0].clone(), r[1].clone(), c.clone()]);
                    frontier.push(c.clone());
                } else if *graph == Graph::Forest {
                    return Err("forest contains a cycle".into());
                }
            }
        }
    }
    Ok(out)
}

/// A repository plan over one worktree: every `Exported(id)` scan is the union over the member
/// contents of their exported rows (`exported(id)`), every `Stage(id)` the plan's own stage.
pub fn evaluate_repository(plan: &Plan<RepoSource>, exported: &dyn Fn(ExportId) -> Result<Vec<Row>, Error>) -> Result<Vec<Row>, Error> {
    evaluate_plan(
        plan,
        |s| match s {
            RepoSource::Stage(id) => Some(*id),
            RepoSource::Exported(_) => None,
        },
        &|s| match s {
            RepoSource::Exported(id) => exported(*id),
            RepoSource::Stage(_) => unreachable!("stages resolve first"),
        },
    )
}

/// Rows as a multiset of their distinct-keys (for comparing a backend's output with the
/// reference's, order aside).
pub fn multiset(rows: &[Row]) -> HashMap<Vec<DKey>, usize> {
    let mut m = HashMap::new();
    for r in rows {
        *m.entry(dkeys(r)).or_default() += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(c: u16) -> Expr<RepoSource> {
        Expr::Col { depth: 0, col: c }
    }
    fn scan(id: ExportId, w: usize) -> Rel<RepoSource> {
        Rel::Scan { src: RepoSource::Exported(id), schema: (0..w).map(|i| Column::new(&format!("c{i}"), Aff::None)).collect() }
    }
    fn t(s: &str) -> V {
        V::text(s)
    }
    fn rows(v: Vec<Vec<V>>) -> Result<Vec<Row>, Error> {
        Ok(v)
    }
    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.iter().map(|v| sv::to_text(v).map_or("NULL".into(), |s| s.to_string())).collect::<Vec<_>>().join("|")).collect()
    }

    /// Dead code: definitions with no reference anywhere (antijoin by NOT EXISTS, correlated).
    #[test]
    fn antijoin_by_correlated_not_exists() {
        let defs = || rows(vec![vec![t("a.py"), t("f")], vec![t("a.py"), t("g")], vec![t("b.py"), t("h")]]);
        let refs = || rows(vec![vec![t("g")], vec![t("x")]]);
        let src = move |s: &RepoSource| match s {
            RepoSource::Exported(0) => defs(),
            RepoSource::Exported(1) => refs(),
            _ => unreachable!(),
        };
        let ev = Evaluator::new(&src);
        let q = Rel::Filter {
            input: Box::new(scan(0, 2)),
            pred: Expr::Exists {
                negated: true,
                rel: Box::new(Rel::Filter { input: Box::new(scan(1, 1)), pred: Expr::Cmp(CmpOp::Eq, Box::new(col(0)), Box::new(Expr::Col { depth: 1, col: 1 }), (Aff::None, Aff::None)) }),
            },
        };
        assert_eq!(texts(&ev.rel(&q, &[]).unwrap()), ["a.py|f", "b.py|h"]);
    }

    /// Transitive callers of a seed, set fixpoint (terminates on a cycle).
    #[test]
    fn set_fixpoint_over_a_cycle() {
        let edges = || rows(vec![vec![t("a"), t("b")], vec![t("b"), t("c")], vec![t("c"), t("b")], vec![t("d"), t("a")]]);
        let src = move |s: &RepoSource| match s {
            RepoSource::Exported(0) => edges(),
            _ => unreachable!(),
        };
        let ev = Evaluator::new(&src);
        let local = || Rel::Local { id: 7, schema: vec![Column::new("n", Aff::None)] };
        // reach(n) = {c} ∪ {caller | caller -> n, n ∈ reach}
        let step = Rel::Project {
            input: Box::new(Rel::Join { kind: JoinKind::Inner, left: Box::new(local()), right: Box::new(scan(0, 2)), on: Some(Expr::Cmp(CmpOp::Eq, Box::new(col(0)), Box::new(col(2)), (Aff::None, Aff::None))) }),
            exprs: vec![col(1)],
            schema: vec![Column::new("n", Aff::None)],
        };
        let q = Rel::With {
            bindings: vec![(7, Binding::Fix { init: vec![Rel::Project { input: Box::new(Rel::Unit), exprs: vec![Expr::Lit(Lit::Text("c".into()))], schema: vec![Column::new("n", Aff::None)] }], step: vec![step], set: true, schema: vec![Column::new("n", Aff::None)] })],
            body: Box::new(local()),
        };
        assert_eq!(texts(&ev.rel(&q, &[]).unwrap()), ["c", "b", "a", "d"]);
    }

    /// Group count with first-seen group order and the last row as representative.
    #[test]
    fn aggregate_count_and_representative() {
        let src = |_: &RepoSource| rows(vec![vec![t("m"), V::Int(1)], vec![t("n"), V::Int(2)], vec![t("m"), V::Int(3)]]);
        let ev = Evaluator::new(&src);
        let q = Rel::Aggregate { input: Box::new(scan(0, 2)), keys: vec![col(0)], aggs: vec![AggCall { f: AggFn::CountStar, arg: None, sep: None, distinct: false, total: Total::InputOrder }] };
        assert_eq!(texts(&ev.rel(&q, &[]).unwrap()), ["m|3|2", "n|2|1"]);
        let empty = |_: &RepoSource| rows(vec![]);
        let ev = Evaluator::new(&empty);
        let q = Rel::Aggregate { input: Box::new(scan(0, 2)), keys: vec![], aggs: vec![AggCall { f: AggFn::Count, arg: Some(col(1)), sep: None, distinct: false, total: Total::InputOrder }] };
        assert_eq!(texts(&ev.rel(&q, &[]).unwrap()), ["NULL|NULL|0"]);
    }

    /// row_number() ties resolve by input position; NOT IN with a NULL on the right is NULL.
    #[test]
    fn row_number_ties_and_not_in_null() {
        let src = |_: &RepoSource| rows(vec![vec![t("p"), V::Int(2)], vec![t("p"), V::Int(1)], vec![t("p"), V::Int(1)]]);
        let ev = Evaluator::new(&src);
        let q = Rel::Window { input: Box::new(scan(0, 2)), funcs: vec![WindowFn { kind: WindowKind::RowNumber, partition: vec![col(0)], order: vec![(col(1), false)], total: Total::InputOrder }] };
        assert_eq!(texts(&ev.rel(&q, &[]).unwrap()), ["p|2|3", "p|1|1", "p|1|2"]);
        let e: Expr<RepoSource> = Expr::InList { e: Box::new(Expr::Lit(Lit::Int(3))), list: vec![(Expr::Lit(Lit::Int(1)), (Aff::None, Aff::None)), (Expr::Lit(Lit::Null), (Aff::None, Aff::None))], negated: true };
        assert!(eval_scalar(&e, &[], &[]).unwrap().is_null());
    }

    /// `Total::Checked` orders refuse a tie between distinct rows (`tie: …`) and accept a tie
    /// between identical rows; `InputOrder` and an unlimited `Sort` never check.
    #[test]
    fn checked_orders_refuse_ties() {
        let run = |data: Vec<Vec<V>>, q: &Rel<RepoSource>| {
            let src = move |_: &RepoSource| rows(data.clone());
            let ev = Evaluator::new(&src);
            ev.rel(q, &[])
        };
        let tie = |r: Result<Vec<Row>, Error>| matches!(r, Err(e) if e.starts_with("tie:"));
        // row_number() over (partition c0, order c1).
        let rn = |total| Rel::Window { input: Box::new(scan(0, 3)), funcs: vec![WindowFn { kind: WindowKind::RowNumber, partition: vec![col(0)], order: vec![(col(1), false)], total }] };
        assert!(tie(run(vec![vec![t("p"), V::Int(1), t("a")], vec![t("p"), V::Int(1), t("b")]], &rn(Total::Checked))));
        assert!(run(vec![vec![t("p"), V::Int(1), t("a")], vec![t("p"), V::Int(1), t("a")]], &rn(Total::Checked)).is_ok());
        assert!(run(vec![vec![t("p"), V::Int(1), t("a")], vec![t("q"), V::Int(1), t("b")]], &rn(Total::Checked)).is_ok());
        assert!(run(vec![vec![t("p"), V::Int(1), t("a")], vec![t("p"), V::Int(1), t("b")]], &rn(Total::InputOrder)).is_ok());
        // ORDER BY c0 LIMIT 1: any tie between distinct rows, wherever the cut falls.
        let sort = |limit: Option<i64>| Rel::Sort { input: Box::new(scan(0, 2)), keys: vec![(0, false)], limit: limit.map(|l| Expr::Lit(Lit::Int(l))), offset: None, total: Total::Checked };
        assert!(tie(run(vec![vec![V::Int(2), t("a")], vec![V::Int(2), t("b")], vec![V::Int(1), t("c")]], &sort(Some(1)))));
        assert!(run(vec![vec![V::Int(1), t("a")], vec![V::Int(1), t("a")], vec![V::Int(2), t("b")]], &sort(Some(1))).is_ok());
        assert!(run(vec![vec![V::Int(1), t("a")], vec![V::Int(1), t("b")]], &sort(None)).is_ok());
        // group_concat(v) GROUP BY g over (SELECT g, v FROM s ORDER BY k): ties within a group on k.
        let concat = |input: Rel<RepoSource>| Rel::Aggregate { input: Box::new(input), keys: vec![col(0)], aggs: vec![AggCall { f: AggFn::GroupConcat, arg: Some(col(1)), sep: None, distinct: false, total: Total::Checked }] };
        let sorted = || Rel::Project {
            input: Box::new(Rel::Sort { input: Box::new(scan(0, 3)), keys: vec![(1, false)], limit: None, offset: None, total: Total::Checked }),
            exprs: vec![col(0), col(2)],
            schema: vec![Column::new("g", Aff::None), Column::new("v", Aff::None)],
        };
        assert!(tie(run(vec![vec![t("g"), V::Int(1), t("x")], vec![t("g"), V::Int(1), t("y")]], &concat(sorted()))));
        let ok = run(vec![vec![t("g"), V::Int(2), t("x")], vec![t("g"), V::Int(1), t("y")], vec![t("h"), V::Int(1), t("z")]], &concat(sorted())).unwrap();
        assert_eq!(texts(&ok), ["g|x|y,x", "h|z|z"]);
        // No Sort below: input order, so any two distinct rows of a group tie.
        assert!(tie(run(vec![vec![t("g"), t("x")], vec![t("g"), t("y")]], &concat(scan(0, 2)))));
        assert!(run(vec![vec![t("g"), t("x")], vec![t("g"), t("x")], vec![t("h"), t("y")]], &concat(scan(0, 2))).is_ok());
    }

    #[test]
    fn printf_and_json_array() {
        assert_eq!(sv::printf("%012d.", &[V::Int(42)]).unwrap(), "000000000042.");
        assert_eq!(sv::printf("%-4s|%5.2f|%%|%+d", &[t("ab"), V::Real(3.14159), V::Int(7)]).unwrap(), "ab  | 3.14|%|+7");
        assert!(sv::printf("%q", &[]).is_err());
        let e: Expr<RepoSource> = Expr::Func(Func::JsonArray, vec![Expr::Lit(Lit::Text("a".into())), Expr::Lit(Lit::Int(1)), Expr::Lit(Lit::Null)]);
        assert_eq!(sv::to_text(&eval_scalar(&e, &[], &[]).unwrap()).unwrap().as_ref(), r#"["a",1,null]"#);
    }
}
