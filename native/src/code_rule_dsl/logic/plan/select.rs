//! The SQL a planned rule runs as, read by handles (decision 0173, t36 item 3): one SELECT from a rule (`Body`) and its
//! planned steps. The SELECT reads the rule's literals in plan order, binds each variable (by slot) to the column or
//! expression that first gives its value, and writes every later use as a condition on it.
use super::body::{Body, VarSet, binds_right, term_vars};
use super::{
    Error, Inputs, Mode, Model, Result, Step, TOP, bin, call, col, eq, integer, kinds, length, string, substr, symbol_of, text_of,
};
use super::super::front::Program;
use super::super::program_image::{AggRef, CmpOp, Lit, Slot, TermRef};
use super::super::sql::select::{Cte, FromItem, Join, Query, Relation, Select};
use super::super::sql::sql_ast::{SqlBinaryOperator as Op, SqlExpression as Expr, SqlUnaryOperator};
use std::collections::HashMap;

/// What each of a rule's variables is bound to so far, by slot.
struct Env(Vec<Option<Expr>>);

impl Env {
    fn get(&self, s: Slot) -> Option<&Expr> {
        self.0.get(s.index()).and_then(Option::as_ref)
    }
    fn has(&self, s: Slot) -> bool {
        self.get(s).is_some()
    }
    fn set(&mut self, s: Slot, e: Expr) {
        if self.0.len() <= s.index() {
            self.0.resize(s.index() + 1, None);
        }
        self.0[s.index()] = Some(e);
    }
    /// The bound variables.
    fn bound(&self) -> VarSet {
        VarSet::of(self.0.iter().enumerate().filter(|(_, e)| e.is_some()).map(|(i, _)| Slot(i as u32)))
    }
    /// The bound variables in their names' order (a walk's columns are named by them).
    fn by_name(&self, b: &Body<'_>) -> Vec<(String, Slot)> {
        let mut out: Vec<(String, Slot)> = self.bound().iter().map(|s| (b.var(s).to_owned(), s)).collect();
        out.sort();
        out
    }
}

/// Whether `t` calls a function that can have no value (`strings::functions`' NULL, as `to_number("a")`; a regex
/// group of a string the regex does not match, `rx_group`): a row that binds or derives its value needs it not NULL,
/// or a negation over it would hold (`NOT EXISTS (… = NULL)`). `x / 0` is SQL's NULL, a value the row keeps (the
/// executor's walks carry it as a nullable seed, `sql_executor::walks::nullable`).
fn partial(b: &Body<'_>, t: TermRef<'_>) -> bool {
    match t {
        TermRef::Call(f, args) => {
            let f = b.v.text(f);
            super::super::strings::functions::signature(f).is_some() || f == "rx_group" || args.clone().any(|a| partial(b, a))
        }
        _ => false,
    }
}

/// Whether `t` is a call of `f`.
fn is_call(b: &Body<'_>, t: TermRef<'_>, f: &str) -> bool {
    matches!(t, TermRef::Call(g, _) if b.v.text(g) == f)
}

/// Term `t` of `b` as SQL, its variables read from `env`.
fn term(b: &Body<'_>, t: TermRef<'_>, env: &Env, atoms: Option<[i64; 3]>) -> Result<Expr> {
    use super::super::strings;
    Ok(match t {
        TermRef::Var(v) => env.get(v).cloned().ok_or_else(|| Error::Plan(format!("${} is unbound in SQL", b.var(v))))?,
        TermRef::Int(i) => integer(i),
        TermRef::Str(s) => string(b.v.text(s)),
        TermRef::Top => integer(TOP),
        TermRef::Call(f, args) => {
            let args: Vec<TermRef<'_>> = args.collect();
            let a = args.iter().map(|&x| term(b, x, env, atoms)).collect::<Result<Vec<_>>>()?;
            let no_sql = || Error::Plan(format!("no SQL for {}", b.v.show_term(b.rule, t)));
            match (b.v.text(f), a.as_slice()) {
                // Membership of a kind or field among atoms (`plan::lower`): 1 or 0.
                ("in", [x, _]) => {
                    let items: Vec<TermRef<'_>> = match args[1] {
                        TermRef::Call(g, items) if b.v.text(g) == strings::SET => items.collect(),
                        one => vec![one],
                    };
                    let mut out = None;
                    for item in items {
                        let e = eq(x.clone(), term(b, item, env, atoms)?);
                        out = Some(match out {
                            None => e,
                            Some(o) => bin(Op::Or, o, e),
                        });
                    }
                    out.unwrap_or_else(|| integer(0))
                }
                // A literal test on a name or path (`logic::strings`): the atoms' text in atoms mode.
                (f, [x, y]) if strings::is_test(f) => {
                    let op = strings::TESTS.iter().find(|t| **t == f).expect("a test");
                    // A pattern set is its patterns' text already (`set` below).
                    let set = is_call(b, args[1], strings::SET);
                    call("str_test", vec![string(op), text_of(atoms, x), if set { y.clone() } else { text_of(atoms, y) }])
                }
                // A pattern set (decision 0037): its patterns' text joined by `strings::SET_SEP`.
                (f, items) if f == strings::SET => {
                    let sep = string(&strings::SET_SEP.to_string());
                    let mut out = items.first().map_or_else(|| string(""), |e| text_of(atoms, e));
                    for item in items.iter().skip(1) {
                        out = bin(Op::Concatenate, bin(Op::Concatenate, out, sep.clone()), text_of(atoms, item));
                    }
                    out
                }
                // A node's document (`typing::cross_rounds`): nodes are `d << 32 | index`.
                ("doc", [x]) => bin(Op::Divide, x.clone(), integer(1i64 << 32)),
                // A sibling's position (`logic::convert`: `precedes`, `follows`), and `.rr`'s arithmetic.
                ("add", [x, y]) => bin(Op::Add, x.clone(), y.clone()),
                ("sub", [x, y]) => bin(Op::Subtract, x.clone(), y.clone()),
                ("mul", [x, y]) => bin(Op::Multiply, x.clone(), y.clone()),
                ("div", [x, y]) => bin(Op::Divide, x.clone(), y.clone()),
                // Soufflé's `max(a, b)` and `min(a, b)`: SQL's scalar `max` and `min`.
                ("max", [x, y]) => call("max", vec![x.clone(), y.clone()]),
                ("min", [x, y]) => call("min", vec![x.clone(), y.clone()]),
                // `.rr`'s string functions (Soufflé's): `substr` counts from 0.
                ("strlen", [x]) => length(&text_of(atoms, x)),
                ("concat", [x, y]) => symbol_of(atoms, bin(Op::Concatenate, text_of(atoms, x), text_of(atoms, y))),
                ("to_string", [x]) => symbol_of(atoms, bin(Op::Concatenate, x.clone(), string(""))),
                // A named group of a regex's match (`.rr`'s `match` with captures, `strings::group`); NULL without one.
                ("rx_group", [x, re, g]) => symbol_of(atoms, call("rx_group", vec![text_of(atoms, x), text_of(atoms, re), text_of(atoms, g)])),
                ("substr", [x, i, n]) => symbol_of(atoms, substr(vec![text_of(atoms, x), bin(Op::Add, i.clone(), integer(1)), n.clone()])),
                // `@dot` (`core.rr`): `a.b`, and `"" . b` is `b`.
                ("cat", [x, y]) if atoms.is_some() => call("sym_cat", vec![x.clone(), y.clone()]),
                // `@dot("", y)` is `y` (`core.rr`).
                ("cat", [x, y]) => Expr::Case {
                    branches: vec![(bin(Op::Equal, x.clone(), string("")), y.clone())],
                    fallback: Box::new(bin(Op::Concatenate, bin(Op::Concatenate, x.clone(), string(".")), y.clone())),
                },
                // core.rr's functors whose meaning is W's (`register_functors`), ahead of `strings::functions`' below.
                ("replace_all", [s, re, by]) => symbol_of(atoms, call("rr_replace_all", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                // A JSON string literal as W's signed-list stages write one (`json_group_array`'s elements): SQLite's
                // `json_quote` of the text.
                ("json_str", [x]) => symbol_of(atoms, call("json_quote", vec![bin(Op::Concatenate, text_of(atoms, x), string(""))])),
                ("nfkc", [x]) => symbol_of(atoms, call("rr_nfkc", vec![text_of(atoms, x)])),
                ("py_repr", [x]) => symbol_of(atoms, call("rr_py_repr", vec![text_of(atoms, x)])),
                ("ed25519_ok", [k, m, sig]) => call("rr_ed25519_ok", vec![text_of(atoms, k), text_of(atoms, m), text_of(atoms, sig)]),
                ("rx_replace", [s, re, by]) => symbol_of(atoms, call("rr_rx_replace", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                ("flat_bin", [o, x, y]) => {
                    let big = integer(1i64 << 40);
                    let [plus, minus, times] = match atoms {
                        Some(ops) => ops.map(integer),
                        None => ["+", "-", "*"].map(string),
                    };
                    let small = |e: &Expr| bin(Op::Less, call("abs", vec![e.clone()]), big.clone());
                    Expr::Case {
                        branches: vec![
                            (bin(Op::Or, eq(x.clone(), integer(TOP)), eq(y.clone(), integer(TOP))), integer(TOP)),
                            (eq(o.clone(), plus), bin(Op::Add, x.clone(), y.clone())),
                            (eq(o.clone(), minus), bin(Op::Subtract, x.clone(), y.clone())),
                            (bin(Op::And, bin(Op::And, eq(o.clone(), times), small(x)), small(y)), bin(Op::Multiply, x.clone(), y.clone())),
                        ],
                        fallback: Box::new(integer(TOP)),
                    }
                }
                // `.rr`'s `@place(path, line)` (`rr::lower`'s `PLACE`): `-2 - (path << 32 | line)`, the path's symbol
                // atom and the line, below `unset` (-1) and every node id; `unset` for a line outside `0 .. 2^32`.
                // `@place_path` and `@place_line` take it apart.
                ("place", [p, l]) if atoms.is_some() => {
                    let valid = bin(Op::And, bin(Op::GreaterOrEqual, l.clone(), integer(0)), bin(Op::Less, l.clone(), integer(1 << 32)));
                    let place = bin(Op::Subtract, integer(-2), bin(Op::Add, bin(Op::Multiply, p.clone(), integer(1 << 32)), l.clone()));
                    Expr::Case { branches: vec![(valid, place)], fallback: Box::new(integer(-1)) }
                }
                ("place_path", [x]) if atoms.is_some() => bin(Op::Divide, bin(Op::Subtract, integer(-2), x.clone()), integer(1 << 32)),
                ("place_line", [x]) if atoms.is_some() => {
                    let n = bin(Op::Subtract, integer(-2), x.clone());
                    bin(Op::Subtract, n.clone(), bin(Op::Multiply, bin(Op::Divide, n, integer(1 << 32)), integer(1 << 32)))
                }
                ("place" | "place_path" | "place_line", _) => {
                    return Err(Error::Plan(format!("no SQL for {}: a place is made of a symbol atom", b.v.show_term(b.rule, t))));
                }
                // `core.rr`'s text functors (`strings::functions`): symbols in as text, a symbol result back.
                (f, items) if let Some((f, kinds, ret)) = strings::functions::signature(f) && kinds.len() == items.len() => {
                    use strings::functions::Kind;
                    let items = items.iter().zip(kinds).map(|(e, k)| if *k == Kind::Text { text_of(atoms, e) } else { e.clone() }).collect();
                    let value = call(f, items);
                    if ret == Kind::Text { symbol_of(atoms, value) } else { value }
                }
                _ => return Err(no_sql()),
            }
        }
        TermRef::Wild => return Err(Error::Plan("no SQL for _".to_owned())),
    })
}

/// The column of `args` holding variable `v`.
fn column_of(args: &[TermRef<'_>], v: Slot) -> Option<usize> {
    args.iter().position(|t| t.var() == Some(v))
}

impl Model {
    /// One SELECT for rule `b` planned as `steps`. `delta_at` is the plan position of the recursive atom that reads
    /// its delta table. `tables` maps a relation to the (full, delta) tables to read it from instead (the native set
    /// fixpoint's vtabs).
    pub fn sql_select(
        &self,
        prog: &Program,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<String> {
        Ok(self.checked_query(prog, b, steps, delta_at, tables)?.render())
    }

    /// `sql_select`'s query, checked, before it is rendered.
    pub fn checked_query(
        &self,
        prog: &Program,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        // A rule whose SQL cannot be built says which rule and body it is, not only the variable: the cause first
        // (what a one-line reader, the executor coverage ledger, keeps), the rule after.
        let query = self.sql_query(prog, b, steps, delta_at, tables).map_err(|e| match e {
            Error::Plan(m) => Error::Plan(format!("{m}, in the SQL for {} :- {}", b.head_text(), b.shown(steps).join(", "))),
            e => e,
        })?;
        query.check(&|relation| self.declared(prog, relation)).map_err(|e| Error::Plan(format!("the SQL for {}: {e}", b.head_text())))?;
        Ok(query)
    }

    /// `sql_select`'s query, before it is checked.
    pub fn sql_query(
        &self,
        prog: &Program,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        let atoms = self.inputs.atoms;
        let inputs: &Inputs = &self.inputs;
        let mut env = Env(vec![None; b.rule.var_count()]);
        let (mut frm, mut wh, mut ctes) = (Vec::<(FromItem, Vec<Expr>)>::new(), Vec::<Expr>::new(), Vec::<Cte>::new());
        let mut alias = 0;
        let mut a_name = || {
            alias += 1;
            format!("t{alias}")
        };
        // The conditions an atom read in a subquery (a negation, an aggregate) puts on its columns: each argument but
        // `_` and the variables `skip` names (the subquery's own).
        let sub_conds = |i: usize, al: &str, env: &Env, skip: &VarSet| -> Result<Vec<Expr>> {
            let rel = b.rel(b.atom(i).expect("an atom"));
            b.args(i)
                .iter()
                .enumerate()
                .filter(|(_, t)| !matches!(t, TermRef::Wild) && !matches!(t, TermRef::Var(v) if skip.contains(*v)))
                .map(|(c, &t)| Ok(eq(col(al, &self.col(rel, c)), term(b, t, env, atoms)?)))
                .collect()
        };
        let subquery = |what: Expr, relation: Relation, al: String, predicates: Vec<Expr>| Select {
            columns: vec![(what, None)],
            from: vec![FromItem::cross(relation, al)],
            predicates,
            ..Select::default()
        };
        for (idx, s) in steps.iter().enumerate() {
            match b.view(s.at) {
                Lit::Atom(l) if !l.neg() => {
                    let al = a_name();
                    let rel = b.rel(l);
                    let args = b.args(s.at);
                    let relation = match tables.and_then(|t| t.get(rel)) {
                        Some((full, delta)) => Relation::Bound { rel: rel.to_owned(), table: if Some(idx) == delta_at { delta.clone() } else { full.clone() } },
                        None => self.relation(prog, rel, Some(idx) == delta_at),
                    };
                    let mut conds = Vec::new();
                    if s.mode == Some(Mode::Range) {
                        // docof($v, $d) with $d bound: $v lies in $d's node-id interval.
                        let lv = b.vars(s.at);
                        let found = steps.iter().find_map(|o| {
                            let d = b.plain_atom(o.at)?;
                            let &(nc, dc) = self.docof.get(b.rel(d))?;
                            let dargs = b.args(o.at);
                            let TermRef::Var(v) = dargs[nc] else { return None };
                            let dv = dargs[dc];
                            (lv.contains(v) && (b.constant(dv).is_some() || matches!(dv, TermRef::Var(x) if env.has(x)))).then_some((v, dv))
                        });
                        let (v, dv) = found.ok_or_else(|| Error::Plan(format!("no document for the range read of {}", b.show(s.at))))?;
                        let rc = column_of(args, v).expect("the range column");
                        let (low, high) = if self.docs_by_shift {
                            // A document's nodes are `d << 32 | index`: the interval is computed, no table read or
                            // made (decision 0104). T's facts number nodes in preorder over the corpus: there the
                            // intervals are `stat_docrange`'s.
                            let lo = bin(Op::Multiply, term(b, dv, &env, atoms)?, integer(1 << 32));
                            (lo.clone(), bin(Op::Add, lo, integer(0xffff_ffff)))
                        } else {
                            let ra = a_name();
                            let range = FromItem::cross(Relation::DocRange { schema: inputs.stats.clone() }, ra.clone());
                            frm.push((range, vec![eq(col(&ra, "d"), term(b, dv, &env, atoms)?)]));
                            (col(&ra, "lo"), col(&ra, "hi"))
                        };
                        conds.push(Expr::Between { input: Box::new(col(&al, &self.col(rel, rc))), low: Box::new(low), high: Box::new(high) });
                    }
                    for (c, &t) in args.iter().enumerate() {
                        let column = col(&al, &self.col(rel, c));
                        match t {
                            TermRef::Var(v) => match env.get(v) {
                                Some(e) => conds.push(eq(column, e.clone())),
                                None => env.set(v, column),
                            },
                            TermRef::Int(_) | TermRef::Str(_) | TermRef::Top => conds.push(eq(column, term(b, t, &env, atoms)?)),
                            _ => {}
                        }
                    }
                    frm.push((FromItem::cross(relation, al), conds));
                }
                Lit::Atom(l) => {
                    let al = a_name();
                    let rel = b.rel(l);
                    let conds = sub_conds(s.at, &al, &env, &VarSet::new())?;
                    // A negation of a component's own (gated) relation reads its full table there.
                    let relation = match tables.and_then(|t| t.get(rel)) {
                        Some((full, _)) => Relation::Bound { rel: rel.to_owned(), table: full.clone() },
                        None => self.relation(prog, rel, false),
                    };
                    let query = subquery(integer(1), relation, al, conds);
                    wh.push(Expr::Exists { negated: true, query: Box::new(query) });
                }
                Lit::Cmp { l, op, r } => {
                    let lv = l.var();
                    if op == CmpOp::Eq && lv.is_some_and(|v| !env.has(v)) {
                        let e = term(b, r, &env, atoms)?;
                        // A partial scalar has no binding on failure. CTEs do not have the old
                        // derived tables' NOT NULL constraints, including for discarded bindings.
                        if partial(b, r) {
                            wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
                        }
                        env.set(lv.expect("a variable"), e);
                    } else if let Some(y) = binds_right(l, op, r, &env.bound()) {
                        let e = term(b, l, &env, atoms)?;
                        env.set(y, e);
                    } else if op == CmpOp::Eq
                        && let Some(lv) = lv
                        && b.is_cat(r)
                        && !{
                            let mut rv = VarSet::new();
                            term_vars(r, &mut rv);
                            rv.is_subset(&env.bound())
                        }
                    {
                        // A split of the bound whole, as `@dot` makes it: `"" . w` is `w`, else at the "." a bound
                        // part places, or at every "." with a part before it (`rdl_int`). In atoms mode the text
                        // is `sym_text`'s, and the parts are interned (`sym_atom`).
                        let w = inputs.text_of(env.get(lv).expect("the bound whole"));
                        let TermRef::Call(_, mut parts) = r else { unreachable!() };
                        let (Some(TermRef::Var(q)), Some(TermRef::Var(x))) = (parts.next(), parts.next()) else { unreachable!() };
                        // A later equality can already constrain the prefix. Enumerating every
                        // position first both amplifies work and loses prefixes beyond rdl_int's cap.
                        // Only nonempty constants use this rewrite: an unconstrained reverse cat
                        // and cat with an empty prefix have different enumeration contracts.
                        if !env.has(q) && !env.has(x) {
                            let prefix = steps.iter().find_map(|step| match b.view(step.at) {
                                Lit::Cmp { l: TermRef::Var(v), op: CmpOp::Eq, r }
                                    if v == q
                                        && (matches!(r, TermRef::Str(s) if !b.v.text(s).is_empty())
                                            || inputs.atoms.is_some() && matches!(r, TermRef::Int(n) if n > 0)) =>
                                {
                                    Some(r)
                                }
                                _ => None,
                            });
                            if let Some(prefix) = prefix {
                                let e = term(b, prefix, &env, atoms)?;
                                env.set(q, e);
                            }
                        }
                        let case = |when: Expr, then: Expr, otherwise: Expr| Expr::Case { branches: vec![(when, then)], fallback: Box::new(otherwise) };
                        if let Some(eq_) = env.get(q).map(|e| inputs.text_of(e)) {
                            let empty = eq(eq_.clone(), string(""));
                            let prefix = eq(substr(vec![w.clone(), integer(1), bin(Op::Add, length(&eq_), integer(1))]), bin(Op::Concatenate, eq_.clone(), string(".")));
                            wh.push(bin(Op::Or, empty.clone(), prefix));
                            env.set(x, inputs.symbol_of(case(empty, w.clone(), substr(vec![w, bin(Op::Add, length(&eq_), integer(2))]))));
                        } else if let Some(ex) = env.get(x).map(|e| inputs.text_of(e)) {
                            let whole = eq(w.clone(), ex.clone());
                            let negated = Expr::Unary { operator: SqlUnaryOperator::Negate, input: Box::new(length(&ex)) };
                            let suffix = bin(
                                Op::And,
                                eq(substr(vec![w.clone(), bin(Op::Subtract, negated, integer(1))]), bin(Op::Concatenate, string("."), ex.clone())),
                                bin(Op::Greater, length(&w), bin(Op::Add, length(&ex), integer(1))),
                            );
                            wh.push(bin(Op::Or, whole.clone(), suffix));
                            let end = bin(Op::Subtract, bin(Op::Subtract, length(&w), length(&ex)), integer(1));
                            env.set(q, inputs.symbol_of(case(whole, string(""), substr(vec![w, integer(1), end]))));
                        } else {
                            // Position 1 stands for `"" . w`: a "." there has no part before it.
                            let al = a_name();
                            let at = col(&al, "i");
                            let first = eq(at.clone(), integer(1));
                            let conds = vec![
                                bin(Op::LessOrEqual, at.clone(), call("max", vec![length(&w), integer(1)])),
                                bin(Op::Or, first.clone(), eq(substr(vec![w.clone(), at.clone(), integer(1)]), string("."))),
                            ];
                            frm.push((FromItem::cross(Relation::Integers, al), conds));
                            env.set(q, inputs.symbol_of(substr(vec![w.clone(), integer(1), bin(Op::Subtract, at.clone(), integer(1))])));
                            env.set(x, inputs.symbol_of(case(first, w.clone(), substr(vec![w, bin(Op::Add, at, integer(1))]))));
                        }
                    } else {
                        let operator = match op {
                            CmpOp::Eq => Op::Equal,
                            CmpOp::Ne => Op::NotEqual,
                            CmpOp::Lt => Op::Less,
                            CmpOp::Le => Op::LessOrEqual,
                            CmpOp::Gt => Op::Greater,
                            CmpOp::Ge => Op::GreaterOrEqual,
                        };
                        wh.push(bin(operator, term(b, l, &env, atoms)?, term(b, r, &env, atoms)?));
                    }
                }
                Lit::Agg { var, op, over, atom } => {
                    let al = a_name();
                    let rel = b.rel(atom);
                    let args = b.args(s.at);
                    // The aggregated variable and a `join`'s ordering ones are the body's own (where the rule does not
                    // bind them): not correlated.
                    let mut local = VarSet::of(over);
                    if let AggRef::Join { by, .. } = op {
                        by.filter(|v| !env.has(*v)).for_each(|v| {
                            local.insert(v);
                        });
                    }
                    let mut conds = sub_conds(s.at, &al, &env, &local)?;
                    // Repeated local value fields are equal within the bag, even when
                    // the outer rule binds a variable with that name.
                    if let Some(over) = over {
                        let columns: Vec<usize> = args.iter().enumerate().filter_map(|(c, t)| (t.var() == Some(over)).then_some(c)).collect();
                        for pair in columns.windows(2) {
                            conds.push(eq(col(&al, &self.col(rel, pair[0])), col(&al, &self.col(rel, pair[1]))));
                        }
                    }
                    let column = |v: Slot| {
                        let c = column_of(args, v).expect("the aggregated column");
                        let symbol = prog.decls.get(rel).and_then(|cols| cols.get(c)).is_some_and(|(_, t)| t == "sym");
                        (col(&al, &self.col(rel, c)), symbol)
                    };
                    let what = match (op, over) {
                        (AggRef::Count, _) | (_, None) => call("count", vec![Expr::All]),
                        (AggRef::Min, Some(o)) => call("min", vec![column(o).0]),
                        (AggRef::Max, Some(o)) => call("max", vec![column(o).0]),
                        // Soufflé's sum is 0 over no rows.
                        (AggRef::Sum, Some(o)) => call("coalesce", vec![call("rr_exact_sum", vec![column(o).0]), integer(0)]),
                        // 1 over no rows (`strings::functions::Product`).
                        (AggRef::Product, Some(o)) => call("coalesce", vec![call("rr_product", vec![column(o).0]), integer(1)]),
                        // `rr_join(sep, value, key, …)` (`register_functors`) orders and joins the distinct rows' text;
                        // symbols in as text.
                        (AggRef::Join { by, sep }, Some(o)) => {
                            let text = |(e, symbol): (Expr, bool)| if symbol { inputs.text_of(&e) } else { e };
                            let mut args = vec![string(b.v.text(sep)), text(column(o))];
                            for v in by {
                                args.push(match env.get(v) {
                                    Some(e) => e.clone(),
                                    None => text(column(v)),
                                });
                            }
                            inputs.symbol_of(call("coalesce", vec![call("rr_join", args), string("")]))
                        }
                    };
                    let query = subquery(what, self.relation(prog, rel, false), al, conds);
                    let value = Expr::Scalar(Box::new(query));
                    // Planning may bind the result first (`n = 0` before the count): the aggregate then tests that
                    // binding, which replacing it would drop.
                    if let Some(bound) = env.get(var) {
                        wh.push(eq(bound.clone(), value));
                    } else {
                        if matches!(op, AggRef::Min | AggRef::Max) {
                            wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(value.clone()) });
                        }
                        env.set(var, value);
                    }
                }
                Lit::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) && inputs.native.contains("nearest") => {
                    // The climb is a native function over the tape: no CTE, and its result is just a column expression.
                    let from = env.get(n).ok_or_else(|| Error::Plan(format!("${} is unbound in SQL", b.var(n))))?.clone();
                    let near = call("tape_nearest", vec![from, string(&prog.kindsets[b.v.text(kindset)].join(","))]);
                    if let Some(e) = env.get(a) {
                        wh.push(eq(near, e.clone()));
                    } else {
                        env.set(a, near.clone());
                        wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(near) });
                    }
                }
                Lit::Nearest { n, a, kindset } => {
                    let (n_name, a_var) = (b.var(n), b.var(a));
                    // A lowered program's members are kind atoms.
                    let ks = kinds(&prog.kindsets[b.v.text(kindset)]);
                    let member = |e: Expr| Expr::In { input: Box::new(e), values: ks.clone() };
                    let input = |rel: &str| Relation::Input { schema: inputs.facts.clone(), rel: rel.to_owned() };
                    let on = |rel: &str, alias: &str, on: Vec<Expr>| FromItem { relation: input(rel), alias: alias.to_owned(), join: Join::On(on) };
                    // Everything joined so far becomes the seed CTE; the walk is a recursive CTE seeded only from it.
                    let k = ctes.len();
                    let (seed, walk, near) = (format!("seed{k}"), format!("walk{k}"), format!("near{k}"));
                    let seed_vars = env.by_name(b);
                    let seed_cols: Vec<String> = seed_vars.iter().map(|(v, _)| v.clone()).collect();
                    if frm.is_empty() {
                        return Err(Error::Plan(format!("the walk of {} reads nothing before it", b.show(s.at))));
                    }
                    let (items, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = frm.drain(..).unzip();
                    ctes.push(Cte::walk(
                        seed.clone(),
                        None,
                        vec![Select {
                            distinct: true,
                            columns: seed_vars.iter().map(|(v, s)| (env.get(*s).expect("bound").clone(), Some(v.clone()))).collect(),
                            from: items,
                            predicates: conds.into_iter().flatten().chain(wh.drain(..)).collect(),
                        }],
                    ));
                    // The seed's columns are threaded through the walk (ast-index-spike S050 `_path_threaded`), so the
                    // walk's output is the extended row and nothing joins back to the seed.
                    let threaded = |alias: &str| seed_cols.iter().map(|v| (col(alias, v), None)).collect::<Vec<_>>();
                    let with = |alias: &str, extra: Vec<Expr>| threaded(alias).into_iter().chain(extra.into_iter().map(|e| (e, None))).collect::<Vec<_>>();
                    let mut walk_columns = seed_cols.clone();
                    let out_var = if s.mode == Some(Mode::Climb) {
                        walk_columns.push("cur".to_owned());
                        let above = bin(Op::GreaterOrEqual, col("t", "par"), integer(0));
                        let arms = vec![
                            Select {
                                columns: with("s", vec![col("t", "par")]),
                                from: vec![FromItem::cross(Relation::Cte(seed.clone()), "s"), on("tape", "t", vec![eq(col("t", "n"), col("s", n_name))])],
                                predicates: vec![above.clone()],
                                ..Select::default()
                            },
                            Select {
                                columns: with("w", vec![col("t", "par")]),
                                from: vec![
                                    FromItem::cross(Relation::Cte(walk.clone()), "w"),
                                    on("node", "k", vec![eq(col("k", "n"), col("w", "cur")), Expr::Unary { operator: SqlUnaryOperator::Not, input: Box::new(member(col("k", "kind"))) }]),
                                    on("tape", "t", vec![eq(col("t", "n"), col("w", "cur"))]),
                                ],
                                predicates: vec![above],
                                ..Select::default()
                            },
                        ];
                        ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
                        let mut predicates = vec![eq(col("k", "n"), col("w", "cur")), member(col("k", "kind"))];
                        if env.has(a) {
                            predicates.push(eq(col("w", "cur"), col("w", a_var)));
                        }
                        let arm = Select {
                            columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                            from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w"), FromItem::cross(input("node"), "k")],
                            predicates,
                            ..Select::default()
                        };
                        ctes.push(Cte::walk(near.clone(), None, vec![arm]));
                        a
                    } else {
                        // Order keys preserve comparisons, not distances (decision 0121).
                        // Follow child edges so traversal works across granule boundaries
                        // and stops below a nested member of the nearest-kind set.
                        walk_columns.push("cur".to_owned());
                        let child = col("c", "c");
                        let arms = vec![
                            Select {
                                columns: with("s", vec![child.clone()]),
                                from: vec![
                                    FromItem::cross(Relation::Cte(seed.clone()), "s"),
                                    on("node", "k", vec![eq(col("k", "n"), col("s", a_var)), member(col("k", "kind"))]),
                                    on("child", "c", vec![eq(col("c", "p"), col("s", a_var))]),
                                ],
                                ..Select::default()
                            },
                            Select {
                                columns: with("w", vec![child]),
                                from: vec![
                                    FromItem::cross(Relation::Cte(walk.clone()), "w"),
                                    on("node", "k", vec![eq(col("k", "n"), col("w", "cur")), Expr::Unary { operator: SqlUnaryOperator::Not, input: Box::new(member(col("k", "kind"))) }]),
                                    on("child", "c", vec![eq(col("c", "p"), col("w", "cur"))]),
                                ],
                                ..Select::default()
                            },
                        ];
                        ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
                        let mut predicates = Vec::new();
                        if env.has(n) {
                            predicates.push(eq(col("w", "cur"), col("w", n_name)));
                        }
                        let arm = Select {
                            columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                            from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w")],
                            predicates,
                            ..Select::default()
                        };
                        ctes.push(Cte::walk(near.clone(), None, vec![arm]));
                        n
                    };
                    env = Env(vec![None; b.rule.var_count()]);
                    for (v, s) in &seed_vars {
                        env.set(*s, col(&near, v));
                    }
                    if !env.has(out_var) {
                        env.set(out_var, col(&near, "near_out"));
                    }
                    frm = vec![(FromItem::cross(Relation::Cte(near.clone()), near), Vec::new())];
                }
                Lit::Ast { .. } => return Err(Error::Plan(format!("no SQL for {}", b.show(s.at)))),
            }
        }
        let mut columns = Vec::new();
        for (i, t) in b.head().args().enumerate() {
            let e = term(b, t, &env, atoms)?;
            if partial(b, t) {
                wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
            }
            columns.push((e, Some(format!("c{i}"))));
        }
        // A safe scalar rule (global aggregates, ground comparisons, or Boolean negation)
        // evaluates once. SQLite permits its SELECT and WHERE without an outer FROM.
        let (from, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = frm.into_iter().unzip();
        let body = Select { distinct: true, columns, from, predicates: conds.into_iter().flatten().chain(wh).collect() };
        Ok(Query { ctes, body })
    }
}
