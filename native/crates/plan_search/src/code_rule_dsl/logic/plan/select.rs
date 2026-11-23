//! The SQL a planned rule runs as, read by handles (decision 0173, t36 item 3): one SELECT from a rule (`Body`) and its
//! planned steps. The SELECT reads the rule's literals in plan order, binds each variable (by slot) to the column or
//! expression that first gives its value, and writes every later use as a condition on it.
use super::body::{Body, VarSet, binds_right, term_vars};
use super::{
    Error, Inputs, Mode, Model, Result, Step, TOP, bin, call, col, eq, integer, kinds, length, string, substr, symbol_of, text_of,
};
use super::fields::Fields;
use super::super::program_image::{AggRef, AtomId, AtomRef, CmpOp, Lit, Slot, TermRef};
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
        TermRef::Branded(a) => integer(a.value()),
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
                // A node's document (`@doc`): nodes are `d << 32 | index`.
                ("doc", [x]) => bin(Op::Divide, x.clone(), integer(1i64 << 32)),
                // A sibling's position (`logic::convert`: `precedes`, `follows`), and `.rerail`'s arithmetic.
                ("add", [x, y]) => bin(Op::Add, x.clone(), y.clone()),
                ("sub", [x, y]) => bin(Op::Subtract, x.clone(), y.clone()),
                ("mul", [x, y]) => bin(Op::Multiply, x.clone(), y.clone()),
                ("div", [x, y]) => bin(Op::Divide, x.clone(), y.clone()),
                // Soufflé's `max(a, b)` and `min(a, b)`: SQL's scalar `max` and `min`.
                ("max", [x, y]) => call("max", vec![x.clone(), y.clone()]),
                ("min", [x, y]) => call("min", vec![x.clone(), y.clone()]),
                // `.rerail`'s string functions (Soufflé's): `substr` counts from 0.
                ("strlen", [x]) => length(&text_of(atoms, x)),
                ("concat", [x, y]) => symbol_of(atoms, bin(Op::Concatenate, text_of(atoms, x), text_of(atoms, y))),
                ("to_string", [x]) => symbol_of(atoms, bin(Op::Concatenate, x.clone(), string(""))),
                // A named group of a regex's match (`.rerail`'s `match` with captures, `strings::group`); NULL without one.
                ("rx_group", [x, re, g]) => symbol_of(atoms, call("rx_group", vec![text_of(atoms, x), text_of(atoms, re), text_of(atoms, g)])),
                ("substr", [x, i, n]) => symbol_of(atoms, substr(vec![text_of(atoms, x), bin(Op::Add, i.clone(), integer(1)), n.clone()])),
                // `@dot` (`core.rerail`): `a.b`, and `"" . b` is `b`.
                ("cat", [x, y]) if atoms.is_some() => call("sym_cat", vec![x.clone(), y.clone()]),
                // `@dot("", y)` is `y` (`core.rerail`).
                ("cat", [x, y]) => Expr::Case {
                    branches: vec![(bin(Op::Equal, x.clone(), string("")), y.clone())],
                    fallback: Box::new(bin(Op::Concatenate, bin(Op::Concatenate, x.clone(), string(".")), y.clone())),
                },
                // core.rerail's functors whose meaning is W's (`register_functors`), ahead of `strings::functions`' below.
                ("replace_all", [s, re, by]) => symbol_of(atoms, call("rr_replace_all", vec![text_of(atoms, s), text_of(atoms, re), text_of(atoms, by)])),
                // A JSON string literal as W's signed-list stages write one (`json_group_array`'s elements): SQLite's
                // `json_quote` of the text.
                ("json_str", [x]) => symbol_of(atoms, call("json_quote", vec![bin(Op::Concatenate, text_of(atoms, x), string(""))])),
                ("nfkc", [x]) => symbol_of(atoms, call("rr_nfkc", vec![text_of(atoms, x)])),
                ("py_repr", [x]) => symbol_of(atoms, call("rr_py_repr", vec![text_of(atoms, x)])),
                ("shell_unquote", [x]) => symbol_of(atoms, call("rr_shell_unquote", vec![text_of(atoms, x)])),
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
                // `.rerail`'s `@place(path, line)` (`rr::lower`'s `PLACE`): `-2 - (path << 32 | line)`, the path's symbol
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
                // `core.rerail`'s text functors (`strings::functions`): symbols in as text, a symbol result back.
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
        pf: Fields<'_>,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<String> {
        Ok(self.checked_query(pf, b, steps, delta_at, tables)?.render())
    }

    /// `sql_select`'s query, checked, before it is rendered.
    pub fn checked_query(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        // A rule whose SQL cannot be built says which rule and body it is, not only the variable: the cause first
        // (what a one-line reader, the executor coverage ledger, keeps), the rule after.
        let query = self.sql_query(pf, b, steps, delta_at, tables).map_err(|e| match e {
            Error::Plan(m) => Error::Plan(format!("{m}, in the SQL for {} :- {}", b.head_text(), b.shown(steps).join(", "))),
            e => e,
        })?;
        query.check(&|relation| self.declared(pf, relation)).map_err(|e| Error::Plan(format!("the SQL for {}: {e}", b.head_text())))?;
        Ok(query)
    }

    /// `sql_select`'s query, before it is checked.
    pub fn sql_query(
        &self,
        pf: Fields<'_>,
        b: &Body<'_>,
        steps: &[Step],
        delta_at: Option<usize>,
        tables: Option<&HashMap<String, (String, String)>>,
    ) -> Result<Query> {
        let mut q = SelectBuilder {
            model: self,
            pf,
            b,
            steps,
            delta_at,
            tables,
            atoms: self.inputs.atoms,
            inputs: &self.inputs,
            env: Env(vec![None; b.rule.var_count()]),
            frm: Vec::new(),
            wh: Vec::new(),
            ctes: Vec::new(),
            alias: 0,
        };
        for (idx, s) in steps.iter().enumerate() {
            q.step(idx, s)?;
        }
        q.finish()
    }
}

/// `sql_query`'s SELECT as it is built, step by step.
struct SelectBuilder<'a, 'v> {
    model: &'a Model,
    pf: Fields<'a>,
    b: &'a Body<'v>,
    steps: &'a [Step],
    delta_at: Option<usize>,
    tables: Option<&'a HashMap<String, (String, String)>>,
    atoms: Option<[i64; 3]>,
    inputs: &'a Inputs,
    env: Env,
    frm: Vec<(FromItem, Vec<Expr>)>,
    wh: Vec<Expr>,
    ctes: Vec<Cte>,
    alias: usize,
}

impl<'a, 'v> SelectBuilder<'a, 'v> {
    #[inline]
    fn next_alias(&mut self) -> String {
        self.alias += 1;
        format!("t{}", self.alias)
    }

    /// Step `s`, the plan's `idx`th, read into the SELECT.
    #[inline]
    fn step(&mut self, idx: usize, s: &Step) -> Result<()> {
        let b = self.b;
        match b.view(s.at) {
            Lit::Atom(l) if !l.neg() => self.atom(idx, s, l),
            Lit::Atom(l) => self.negation(s, l),
            Lit::Cmp { l, op, r } => self.comparison(l, op, r),
            Lit::Agg { var, op, over, atom } => self.aggregate(s, var, op, over, atom),
            Lit::Nearest { n, a, kindset } if s.mode == Some(Mode::Climb) && self.inputs.native.contains("nearest") => self.native_nearest(n, a, kindset),
            Lit::Nearest { n, a, kindset } => self.walk_nearest(s, n, a, kindset),
            Lit::Ast { .. } => Err(Error::Plan(format!("no SQL for {}", b.show(s.at)))),
        }
    }

    /// The table `rel` is read from: its delta table or its full one.
    #[inline]
    fn read_relation(&self, rel: &str, delta: bool) -> Relation {
        match self.tables.and_then(|t| t.get(rel)) {
            Some((full, delta_table)) => Relation::Bound { rel: rel.to_owned(), table: if delta { delta_table.clone() } else { full.clone() } },
            None => self.model.relation(self.pf, rel, delta),
        }
    }

    /// A positive atom: a FROM item, which binds each variable it gives first and has a condition for each later one.
    fn atom(&mut self, idx: usize, s: &Step, l: AtomRef<'v>) -> Result<()> {
        let b = self.b;
        let al = self.next_alias();
        let rel = b.rel(l);
        let args = b.args(s.at);
        let relation = self.read_relation(rel, Some(idx) == self.delta_at);
        let mut conds = Vec::new();
        if s.mode == Some(Mode::Range) {
            // docof($v, $d) with $d bound: $v lies in $d's node-id interval.
            conds.push(self.range_condition(s, &al, rel, args)?);
        }
        self.bind_columns(&al, rel, args, &mut conds)?;
        self.frm.push((FromItem::cross(relation, al), conds));
        Ok(())
    }

    /// The range read's `$v` and its document's term, from the plan's `docof($v, $d)` with `$d` bound.
    fn range_document(&self, s: &Step) -> Result<(Slot, TermRef<'v>)> {
        let b = self.b;
        let lv = b.vars(s.at);
        let found = self.steps.iter().find_map(|o| {
            let d = b.plain_atom(o.at)?;
            let &(nc, dc) = self.model.docof.get(b.rel(d))?;
            let dargs = b.args(o.at);
            let TermRef::Var(v) = dargs[nc] else { return None };
            let dv = dargs[dc];
            (lv.contains(v) && (b.constant(dv).is_some() || matches!(dv, TermRef::Var(x) if self.env.has(x)))).then_some((v, dv))
        });
        found.ok_or_else(|| Error::Plan(format!("no document for the range read of {}", b.show(s.at))))
    }

    /// The range read's condition: its column of `$v` lies between the bounds of `$v`'s document.
    fn range_condition(&mut self, s: &Step, al: &str, rel: &str, args: &[TermRef<'v>]) -> Result<Expr> {
        let b = self.b;
        let (v, dv) = self.range_document(s)?;
        let rc = column_of(args, v).expect("the range column");
        let (low, high) = if self.model.docs_by_shift {
            // A document's nodes are `d << 32 | index`: the interval is computed, no table read or
            // made (decision 0104). T's facts number nodes in preorder over the corpus: there the
            // intervals are `stat_docrange`'s.
            let lo = bin(Op::Multiply, term(b, dv, &self.env, self.atoms)?, integer(1 << 32));
            (lo.clone(), bin(Op::Add, lo, integer(0xffff_ffff)))
        } else {
            let ra = self.next_alias();
            let range = FromItem::cross(Relation::DocRange { schema: self.inputs.stats.clone() }, ra.clone());
            self.frm.push((range, vec![eq(col(&ra, "d"), term(b, dv, &self.env, self.atoms)?)]));
            (col(&ra, "lo"), col(&ra, "hi"))
        };
        Ok(Expr::Between { input: Box::new(col(al, &self.model.col(rel, rc))), low: Box::new(low), high: Box::new(high) })
    }

    /// The atom read as `al`: a variable's first use binds it; a later use or a constant is a condition in `conds`.
    #[inline]
    fn bind_columns(&mut self, al: &str, rel: &str, args: &[TermRef<'v>], conds: &mut Vec<Expr>) -> Result<()> {
        for (c, &t) in args.iter().enumerate() {
            let column = col(al, &self.model.col(rel, c));
            match t {
                TermRef::Var(v) => match self.env.get(v) {
                    Some(e) => conds.push(eq(column, e.clone())),
                    None => self.env.set(v, column),
                },
                TermRef::Int(_) | TermRef::Branded(_) | TermRef::Str(_) | TermRef::Top => conds.push(eq(column, term(self.b, t, &self.env, self.atoms)?)),
                _ => {}
            }
        }
        Ok(())
    }

    /// A negated atom: a `NOT EXISTS` condition.
    fn negation(&mut self, s: &Step, l: AtomRef<'v>) -> Result<()> {
        let al = self.next_alias();
        let rel = self.b.rel(l);
        let conds = self.sub_conds(s.at, &al, &VarSet::new())?;
        // A negation of a component's own (gated) relation reads its full table there.
        let relation = self.read_relation(rel, false);
        let query = subquery(integer(1), relation, al, conds);
        self.wh.push(Expr::Exists { negated: true, query: Box::new(query) });
        Ok(())
    }

    /// The conditions an atom read in a subquery (a negation, an aggregate) puts on its columns: each argument but
    /// `_` and the variables `skip` names (the subquery's own).
    fn sub_conds(&self, i: usize, al: &str, skip: &VarSet) -> Result<Vec<Expr>> {
        let b = self.b;
        let rel = b.rel(b.atom(i).expect("an atom"));
        b.args(i)
            .iter()
            .enumerate()
            .filter(|(_, t)| !matches!(t, TermRef::Wild) && !matches!(t, TermRef::Var(v) if skip.contains(*v)))
            .map(|(c, &t)| Ok(eq(col(al, &self.model.col(rel, c)), term(b, t, &self.env, self.atoms)?)))
            .collect()
    }

    /// A comparison: it binds a variable (either side's, or the parts of a split), or it is a condition.
    fn comparison(&mut self, l: TermRef<'v>, op: CmpOp, r: TermRef<'v>) -> Result<()> {
        let lv = l.var();
        if op == CmpOp::Eq && lv.is_some_and(|v| !self.env.has(v)) {
            self.bind_left(lv, r)
        } else if let Some(y) = binds_right(l, op, r, &self.env.bound()) {
            let e = term(self.b, l, &self.env, self.atoms)?;
            self.env.set(y, e);
            Ok(())
        } else if let Some(lv) = self.split_whole(op, lv, r) {
            self.split_cat(lv, r)
        } else {
            self.condition(l, op, r)
        }
    }

    /// `$lv = r` with `$lv` unbound: `$lv` is `r`.
    fn bind_left(&mut self, lv: Option<Slot>, r: TermRef<'v>) -> Result<()> {
        let e = term(self.b, r, &self.env, self.atoms)?;
        // A partial scalar has no binding on failure. CTEs do not have the old
        // derived tables' NOT NULL constraints, including for discarded bindings.
        if partial(self.b, r) {
            self.wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
        }
        self.env.set(lv.expect("a variable"), e);
        Ok(())
    }

    /// `$lv` of `$lv = r` where `r` is a `@dot` whose variables are not all bound: the whole that splits into them.
    fn split_whole(&self, op: CmpOp, lv: Option<Slot>, r: TermRef<'v>) -> Option<Slot> {
        if op == CmpOp::Eq
            && let Some(lv) = lv
            && self.b.is_cat(r)
            && !{
                let mut rv = VarSet::new();
                term_vars(r, &mut rv);
                rv.is_subset(&self.env.bound())
            }
        {
            Some(lv)
        } else {
            None
        }
    }

    fn split_cat(&mut self, lv: Slot, r: TermRef<'v>) -> Result<()> {
        let inputs = self.inputs;
        // A split of the bound whole, as `@dot` makes it: `"" . w` is `w`, else at the "." a bound
        // part places, or at every "." with a part before it (`rdl_int`). In atoms mode the text
        // is `sym_text`'s, and the parts are interned (`sym_atom`).
        let w = inputs.text_of(self.env.get(lv).expect("the bound whole"));
        let TermRef::Call(_, mut parts) = r else { unreachable!() };
        let (Some(TermRef::Var(q)), Some(TermRef::Var(x))) = (parts.next(), parts.next()) else { unreachable!() };
        // A later equality can already constrain the prefix. Enumerating every
        // position first both amplifies work and loses prefixes beyond rdl_int's cap.
        // Only nonempty constants use this rewrite: an unconstrained reverse cat
        // and cat with an empty prefix have different enumeration contracts.
        self.bind_known_prefix(q, x)?;
        let case = |when: Expr, then: Expr, otherwise: Expr| Expr::Case { branches: vec![(when, then)], fallback: Box::new(otherwise) };
        if let Some(eq_) = self.env.get(q).map(|e| inputs.text_of(e)) {
            let empty = eq(eq_.clone(), string(""));
            let prefix = eq(substr(vec![w.clone(), integer(1), bin(Op::Add, length(&eq_), integer(1))]), bin(Op::Concatenate, eq_.clone(), string(".")));
            self.wh.push(bin(Op::Or, empty.clone(), prefix));
            self.env.set(x, inputs.symbol_of(case(empty, w.clone(), substr(vec![w, bin(Op::Add, length(&eq_), integer(2))]))));
        } else if let Some(ex) = self.env.get(x).map(|e| inputs.text_of(e)) {
            let whole = eq(w.clone(), ex.clone());
            let negated = Expr::Unary { operator: SqlUnaryOperator::Negate, input: Box::new(length(&ex)) };
            let suffix = bin(
                Op::And,
                eq(substr(vec![w.clone(), bin(Op::Subtract, negated, integer(1))]), bin(Op::Concatenate, string("."), ex.clone())),
                bin(Op::Greater, length(&w), bin(Op::Add, length(&ex), integer(1))),
            );
            self.wh.push(bin(Op::Or, whole.clone(), suffix));
            let end = bin(Op::Subtract, bin(Op::Subtract, length(&w), length(&ex)), integer(1));
            self.env.set(q, inputs.symbol_of(case(whole, string(""), substr(vec![w, integer(1), end]))));
        } else {
            // Position 1 stands for `"" . w`: a "." there has no part before it.
            let al = self.next_alias();
            let at = col(&al, "i");
            let first = eq(at.clone(), integer(1));
            let conds = vec![
                bin(Op::LessOrEqual, at.clone(), call("max", vec![length(&w), integer(1)])),
                bin(Op::Or, first.clone(), eq(substr(vec![w.clone(), at.clone(), integer(1)]), string("."))),
            ];
            self.frm.push((FromItem::cross(Relation::Integers, al), conds));
            self.env.set(q, inputs.symbol_of(substr(vec![w.clone(), integer(1), bin(Op::Subtract, at.clone(), integer(1))])));
            self.env.set(x, inputs.symbol_of(case(first, w.clone(), substr(vec![w, bin(Op::Add, at, integer(1))]))));
        }
        Ok(())
    }

    /// With neither part of a split bound, the prefix `$q` a plan's `$q = c` gives it.
    fn bind_known_prefix(&mut self, q: Slot, x: Slot) -> Result<()> {
        if !self.env.has(q) && !self.env.has(x) {
            let prefix = self.known_prefix(q);
            if let Some(prefix) = prefix {
                let e = term(self.b, prefix, &self.env, self.atoms)?;
                self.env.set(q, e);
            }
        }
        Ok(())
    }

    /// The constant `c` of a plan's `$q = c`, when it is a nonempty one.
    fn known_prefix(&self, q: Slot) -> Option<TermRef<'v>> {
        let b = self.b;
        self.steps.iter().find_map(|step| match b.view(step.at) {
            Lit::Cmp { l: TermRef::Var(v), op: CmpOp::Eq, r } if v == q && self.nonempty_constant(r) => Some(r),
            _ => None,
        })
    }

    /// Whether `r` is a nonempty string, or in atoms mode a symbol other than the empty one.
    #[inline]
    fn nonempty_constant(&self, r: TermRef<'v>) -> bool {
        matches!(r, TermRef::Str(s) if !self.b.v.text(s).is_empty())
            || self.inputs.atoms.is_some() && (matches!(r, TermRef::Int(n) if n > 0) || matches!(r, TermRef::Branded(a) if a.atom > 0))
    }

    /// A comparison that binds nothing: a condition.
    fn condition(&mut self, l: TermRef<'v>, op: CmpOp, r: TermRef<'v>) -> Result<()> {
        let operator = match op {
            CmpOp::Eq => Op::Equal,
            CmpOp::Ne => Op::NotEqual,
            CmpOp::Lt => Op::Less,
            CmpOp::Le => Op::LessOrEqual,
            CmpOp::Gt => Op::Greater,
            CmpOp::Ge => Op::GreaterOrEqual,
        };
        self.wh.push(bin(operator, term(self.b, l, &self.env, self.atoms)?, term(self.b, r, &self.env, self.atoms)?));
        Ok(())
    }

    /// An aggregate: a scalar subquery, which binds `var` or tests the binding planning gave it first.
    fn aggregate(&mut self, s: &Step, var: Slot, op: AggRef<'v>, over: Option<Slot>, atom: AtomRef<'v>) -> Result<()> {
        let b = self.b;
        let al = self.next_alias();
        let rel = b.rel(atom);
        let args = b.args(s.at);
        let conds = self.aggregate_conds(s.at, &al, rel, args, op, over)?;
        let what = self.aggregate_value(&al, rel, args, op, over);
        let query = subquery(what, self.model.relation(self.pf, rel, false), al, conds);
        let value = Expr::Scalar(Box::new(query));
        // Planning may bind the result first (`n = 0` before the count): the aggregate then tests that
        // binding, which replacing it would drop.
        if let Some(bound) = self.env.get(var) {
            self.wh.push(eq(bound.clone(), value));
        } else {
            if matches!(op, AggRef::Min | AggRef::Max | AggRef::Bounded { .. }) {
                self.wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(value.clone()) });
            }
            self.env.set(var, value);
        }
        Ok(())
    }

    /// The conditions of the aggregate's subquery over the atom at `at`, read as `al`.
    fn aggregate_conds(&self, at: usize, al: &str, rel: &str, args: &[TermRef<'v>], op: AggRef<'v>, over: Option<Slot>) -> Result<Vec<Expr>> {
        // The aggregated variable and a `join`'s ordering ones are the body's own (where the rule does not
        // bind them): not correlated.
        let mut local = VarSet::of(over);
        if let AggRef::Join { by, .. } = op {
            by.filter(|v| !self.env.has(*v)).for_each(|v| {
                local.insert(v);
            });
        }
        let mut conds = self.sub_conds(at, al, &local)?;
        // Repeated local value fields are equal within the bag, even when
        // the outer rule binds a variable with that name.
        if let Some(over) = over {
            let columns: Vec<usize> = args.iter().enumerate().filter_map(|(c, t)| (t.var() == Some(over)).then_some(c)).collect();
            for pair in columns.windows(2) {
                conds.push(eq(col(al, &self.model.col(rel, pair[0])), col(al, &self.model.col(rel, pair[1]))));
            }
        }
        // A bounded aggregate's values are those on its bound's side (decision 0511).
        if let (AggRef::Bounded { max, bound, strict }, Some(o)) = (op, over) {
            let b = self.b;
            let at = self.env.get(bound).ok_or_else(|| Error::Plan(format!("${} is unbound in SQL", b.var(bound))))?.clone();
            let side = match (max, strict) {
                (true, true) => Op::Less,
                (true, false) => Op::LessOrEqual,
                (false, true) => Op::Greater,
                (false, false) => Op::GreaterOrEqual,
            };
            let c = column_of(args, o).expect("the aggregated column");
            conds.push(Expr::Binary { operator: side, left: Box::new(col(al, &self.model.col(rel, c))), right: Box::new(at) });
        }
        Ok(conds)
    }

    /// The aggregate's value over the rows of its atom, read as `al`.
    fn aggregate_value(&self, al: &str, rel: &str, args: &[TermRef<'v>], op: AggRef<'v>, over: Option<Slot>) -> Expr {
        let (b, inputs) = (self.b, self.inputs);
        let column = |v: Slot| {
            let c = column_of(args, v).expect("the aggregated column");
            let symbol = self.pf.column_type(rel, c) == Some("sym");
            (col(al, &self.model.col(rel, c)), symbol)
        };
        match (op, over) {
            (AggRef::Count, _) | (_, None) => call("count", vec![Expr::All]),
            (AggRef::Bounded { max: false, .. }, Some(o)) => call("min", vec![column(o).0]),
            (AggRef::Bounded { max: true, .. }, Some(o)) => call("max", vec![column(o).0]),
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
                    args.push(match self.env.get(v) {
                        Some(e) => e.clone(),
                        None => text(column(v)),
                    });
                }
                inputs.symbol_of(call("coalesce", vec![call("rr_join", args), string("")]))
            }
        }
    }

    /// A climb to the nearest member of a kind set, as the native `tape_nearest`.
    fn native_nearest(&mut self, n: Slot, a: Slot, kindset: AtomId) -> Result<()> {
        let b = self.b;
        // The climb is a native function over the tape: no CTE, and its result is just a column expression.
        let from = self.env.get(n).ok_or_else(|| Error::Plan(format!("${} is unbound in SQL", b.var(n))))?.clone();
        let near = call("tape_nearest", vec![from, string(&kindset_texts(self.pf, b.v.text(kindset)).join(","))]);
        if let Some(e) = self.env.get(a) {
            self.wh.push(eq(near, e.clone()));
        } else {
            self.env.set(a, near.clone());
            self.wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(near) });
        }
        Ok(())
    }

    /// A walk to the nearest member of a kind set, in SQL: the seed, walk and result CTEs the rule reads on from.
    fn walk_nearest(&mut self, s: &Step, n: Slot, a: Slot, kindset: AtomId) -> Result<()> {
        let (b, inputs) = (self.b, self.inputs);
        let (n_name, a_var) = (b.var(n), b.var(a));
        // A lowered program's members are kind atoms.
        let ks = kinds(&kindset_texts(self.pf, b.v.text(kindset)));
        let member = |e: Expr| Expr::In { input: Box::new(e), values: ks.clone() };
        let input = |rel: &str| Relation::Input { schema: inputs.facts.clone(), rel: rel.to_owned() };
        let on = |rel: &str, alias: &str, on: Vec<Expr>| FromItem { relation: input(rel), alias: alias.to_owned(), join: Join::On(on) };
        // Everything joined so far becomes the seed CTE; the walk is a recursive CTE seeded only from it.
        let k = self.ctes.len();
        let (seed, walk, near) = (format!("seed{k}"), format!("walk{k}"), format!("near{k}"));
        let seed_vars = self.env.by_name(b);
        let seed_cols: Vec<String> = seed_vars.iter().map(|(v, _)| v.clone()).collect();
        if self.frm.is_empty() {
            return Err(Error::Plan(format!("the walk of {} reads nothing before it", b.show(s.at))));
        }
        self.push_seed(&seed, &seed_vars);
        // The seed's columns are threaded through the walk (SQL index prototype S050 `_path_threaded`), so the
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
            self.ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
            let mut predicates = vec![eq(col("k", "n"), col("w", "cur")), member(col("k", "kind"))];
            if self.env.has(a) {
                predicates.push(eq(col("w", "cur"), col("w", a_var)));
            }
            let arm = Select {
                columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w"), FromItem::cross(input("node"), "k")],
                predicates,
                ..Select::default()
            };
            self.ctes.push(Cte::walk(near.clone(), None, vec![arm]));
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
            self.ctes.push(Cte::walk(walk.clone(), Some(walk_columns), arms));
            let mut predicates = Vec::new();
            if self.env.has(n) {
                predicates.push(eq(col("w", "cur"), col("w", n_name)));
            }
            let arm = Select {
                columns: with("w", Vec::new()).into_iter().chain([(col("w", "cur"), Some("near_out".to_owned()))]).collect(),
                from: vec![FromItem::cross(Relation::Cte(walk.clone()), "w")],
                predicates,
                ..Select::default()
            };
            self.ctes.push(Cte::walk(near.clone(), None, vec![arm]));
            n
        };
        self.read_on_from_walk(near, &seed_vars, out_var);
        Ok(())
    }

    /// Everything joined so far, with every condition, as the walk's seed CTE `seed` of the bound variables.
    fn push_seed(&mut self, seed: &str, seed_vars: &[(String, Slot)]) {
        let (items, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = self.frm.drain(..).unzip();
        self.ctes.push(Cte::walk(
            seed.to_owned(),
            None,
            vec![Select {
                distinct: true,
                columns: seed_vars.iter().map(|(v, s)| (self.env.get(*s).expect("bound").clone(), Some(v.clone()))).collect(),
                from: items,
                predicates: conds.into_iter().flatten().chain(self.wh.drain(..)).collect(),
            }],
        ));
    }

    /// The rule reads on from walk result `near` alone: the seed's variables and `out_var` are its columns.
    fn read_on_from_walk(&mut self, near: String, seed_vars: &[(String, Slot)], out_var: Slot) {
        self.env = Env(vec![None; self.b.rule.var_count()]);
        for (v, s) in seed_vars {
            self.env.set(*s, col(&near, v));
        }
        if !self.env.has(out_var) {
            self.env.set(out_var, col(&near, "near_out"));
        }
        self.frm = vec![(FromItem::cross(Relation::Cte(near.clone()), near), Vec::new())];
    }

    /// The SELECT of the rule's head over everything read.
    fn finish(mut self) -> Result<Query> {
        let b = self.b;
        let mut columns = Vec::new();
        for (i, t) in b.head().args().enumerate() {
            let e = term(b, t, &self.env, self.atoms)?;
            if partial(b, t) {
                self.wh.push(Expr::Unary { operator: SqlUnaryOperator::IsNotNull, input: Box::new(e.clone()) });
            }
            columns.push((e, Some(format!("c{i}"))));
        }
        // A safe scalar rule (global aggregates, ground comparisons, or Boolean negation)
        // evaluates once. SQLite permits its SELECT and WHERE without an outer FROM.
        let (from, conds): (Vec<FromItem>, Vec<Vec<Expr>>) = self.frm.into_iter().unzip();
        let body = Select { distinct: true, columns, from, predicates: conds.into_iter().flatten().chain(self.wh).collect() };
        Ok(Query { ctes: self.ctes, body })
    }
}

/// A subquery of `what` over `relation`, read as `al`.
fn subquery(what: Expr, relation: Relation, al: String, predicates: Vec<Expr>) -> Select {
    Select { columns: vec![(what, None)], from: vec![FromItem::cross(relation, al)], predicates, ..Select::default() }
}

/// Kind set `set`'s kinds, as text (`Program::kindsets[set]`), failing where it has none.
fn kindset_texts(pf: Fields<'_>, set: &str) -> Vec<String> {
    pf.kindset(set).unwrap_or_else(|| panic!("no kind set {set}")).iter().map(|&k| pf.text(k).to_owned()).collect()
}
