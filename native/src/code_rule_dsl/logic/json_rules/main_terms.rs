//! `main_terms.py`: main's Rerail-extension matcher terms (`arguments`, `value`, `resolves`, `layout`) lowered to
//! plain Datalog over the tape. Each returns body literals for one JSON term at node v and adds the relations it
//! needs to the program's library part (`Program::lib`).

use super::lower::{Lit, match_, opt_alts};
use super::program::{Program, Rule, Vars};
use super::patterns::Patterns;
use super::pyre::re;
use super::value::{Obj, Value, dumps, py_str, q};

fn lib(prog: &mut Program, name: &str, decl: &str, note: &[&str], rules: Vec<Rule>) -> String {
    if !prog.lib.contains_key(name) {
        let notes = note.iter().enumerate().map(|(i, l)| if i == 0 { format!("// {name}: {l}") } else { format!("//   {l}") }).collect();
        prog.lib.insert(name.to_owned(), (format!(".decl {name}{decl}"), notes, rules));
    }
    name.to_owned()
}

fn r(head: &str, lits: &[&str]) -> Rule {
    (head.to_owned(), lits.iter().map(|s| (*s).to_owned()).collect())
}

fn rs(head: &str, lits: Vec<String>) -> Rule {
    (head.to_owned(), lits)
}

// ---- arguments ----------------------------------------------------------------------------------------------------

const SPLATS: [&str; 2] = ["list_splat", "parenthesized_list_splat"];
const NOT_PLAIN: [&str; 4] = ["keyword_argument", "dictionary_splat", "list_splat", "parenthesized_list_splat"];

fn args_shared(prog: &mut Program) {
    let mut plain = vec!["kind(l, \"argument_list\")".to_owned(), "child(l, a, i)".to_owned()];
    plain.extend(NOT_PLAIN.iter().chain(["comment"].iter()).map(|k| format!("!kind(a, {})", q(k))));
    lib(prog, "args_plain", "(l: node, a: node, i: number)", &["a is child i of argument list l and a positional argument (not `k=v`, `*`, `**`, a comment)"], vec![rs("args_plain(l, a, i)", plain)]);
    lib(
        prog,
        "args_splat",
        "(l: node, i: number)",
        &["child i of argument list l is `*xs`"],
        vec![rs("args_splat(l, i)", vec!["kind(l, \"argument_list\")".into(), "child(l, s, i)".into(), format!("(kind(s, {}) ; kind(s, {}))", q(SPLATS[0]), q(SPLATS[1]))])],
    );
    lib(
        prog,
        "args_pos",
        "(l: node, a: node, j: number)",
        &["a is the positional argument at position j of argument list l, before any `*xs`"],
        vec![r("args_pos(l, a, j)", &["args_plain(l, a, i)", "count : { args_splat(l, k), k < i } = 0", "j = count : { args_plain(l, _, m), m < i }"])],
    );
    lib(
        prog,
        "args_star",
        "(l: node, j: number)",
        &["argument list l has `*xs`, after j positional arguments: positional slots from j on are unknown"],
        vec![r("args_star(l, j)", &["args_splat(l, i)", "count : { args_splat(l, k), k < i } = 0", "j = count : { args_plain(l, _, m), m < i }"])],
    );
    lib(
        prog,
        "args_kwp",
        "(l: node, k: symbol, v: node)",
        &["`k=v` in argument list l"],
        vec![r("args_kwp(l, k, v)", &["kind(l, \"argument_list\")", "child(l, a, _)", "kind(a, \"keyword_argument\")", "field(a, \"name\", x)", "text(x, k)", "field(a, \"value\", v)"])],
    );
    lib(
        prog,
        "args_lit_dict",
        "(s: node, d: node)",
        &["`**{…}` s spreads the dict display d, whose every entry is a pair with a string key: its pairs are keywords"],
        vec![r(
            "args_lit_dict(s, d)",
            &["kind(s, \"dictionary_splat\")", "child(s, d, _)", "kind(d, \"dictionary\")", "count : { child(s, o, _), !kind(o, \"comment\") } = 1", "count : { child(d, p, _), !kind(p, \"comment\"), !args_str_pair(p) } = 0"],
        )],
    );
    lib(
        prog,
        "args_str_pair",
        "(p: node)",
        &["p is a `\"k\": v` pair whose key is a string with a constant value"],
        vec![r("args_str_pair(p)", &["kind(p, \"pair\")", "field(p, \"key\", k)", "(kind(k, \"string\") ; kind(k, \"concatenated_string\"))", "str(k, _)", "field(p, \"value\", _)"])],
    );
    lib(
        prog,
        "args_kw",
        "(l: node, k: symbol, v: node)",
        &["keyword k passed with value v in argument list l, `**{…}` included"],
        vec![
            r("args_kw(l, k, v)", &["args_kwp(l, k, v)"]),
            r("args_kw(l, k, v)", &["kind(l, \"argument_list\")", "child(l, s, _)", "args_lit_dict(s, d)", "child(d, p, _)", "kind(p, \"pair\")", "field(p, \"key\", x)", "str(x, k)", "field(p, \"value\", v)"]),
        ],
    );
    lib(
        prog,
        "args_kw_spread",
        "(l: node)",
        &["argument list l has a `**kw` whose keywords are not known"],
        vec![r("args_kw_spread(l)", &["kind(l, \"argument_list\")", "child(l, s, _)", "kind(s, \"dictionary_splat\")", "!args_lit_dict(s, _)"])],
    );
}

/// [(name, position or None, keyword-passable)], as compile/arguments.rs reads `/` and `*`.
fn signature(sig: &[String]) -> Vec<(String, Option<usize>, bool)> {
    let mut slots: Vec<(String, &str)> = Vec::new();
    let mut kind = "either";
    for s in sig {
        if s == "/" {
            slots = slots.into_iter().map(|(n, _)| (n, "positional")).collect();
        } else if s == "*" {
            kind = "keyword";
        } else {
            slots.push((s.clone(), kind));
        }
    }
    let mut out = Vec::new();
    let mut p = 0;
    for (n, k) in slots {
        out.push((n, if k == "keyword" { None } else { Some(p) }, k != "positional"));
        if k != "keyword" {
            p += 1;
        }
    }
    out
}

/// (value relation, absent relation) for one slot of one signature.
fn slot_rels(prog: &mut Program, sig: &[String], slot: &str, syntactic: bool) -> Result<(String, String), String> {
    let slots = signature(sig);
    let (name, pos, kwok) = slots.into_iter().find(|s| s.0 == slot).ok_or("StopIteration")?;
    let key = (sig.to_vec(), slot.to_owned(), syntactic);
    if let Some((_, v)) = prog.arg_rels.iter().find(|(k, _)| *k == key) {
        return Ok(v.clone());
    }
    args_shared(prog);
    let k = prog.arg_rels.len() + 1;
    let base = super::pyre::sub_str(&re("[^0-9a-z]+"), &slot.to_lowercase(), "_");
    let (val, absent) = (format!("arg_{base}_{k}"), format!("arg_no_{base}_{k}"));
    let shown = sig.join(", ");
    let (mut rules_v, mut rules_a) = (Vec::new(), Vec::new());
    let pre = || vec!["kind(n, \"call\")".to_owned(), "field(n, \"arguments\", l)".to_owned()];
    let qn = q(&name);
    let gen0 = || rs(&format!("{val}(n, v)"), vec!["kind(n, \"call\")".into(), "field(n, \"arguments\", v)".into(), "kind(v, \"generator_expression\")".into()]);
    let mode;
    if syntactic {
        if kwok {
            let mut l = pre();
            l.push(format!("args_kwp(l, {qn}, v)"));
            rules_v.push(rs(&format!("{val}(n, v)"), l));
        }
        if let Some(pos) = pos {
            let mut l = pre();
            l.push(format!("args_pos(l, v, {pos})"));
            if kwok {
                l.push(format!("!args_kwp(l, {qn}, _)"));
            }
            rules_v.push(rs(&format!("{val}(n, v)"), l));
            if pos == 0 {
                rules_v.push(gen0());
            }
        }
        let mut neg = Vec::new();
        if kwok {
            neg.push(format!("!args_kwp(l, {qn}, _)"));
        }
        if let Some(pos) = pos {
            neg.push(format!("!args_pos(l, _, {pos})"));
        }
        if pos == Some(0) {
            neg.push("!kind(l, \"generator_expression\")".into());
        }
        let mut l = pre();
        l.extend(neg);
        rules_a.push(rs(&format!("{absent}(n)"), l));
        mode = "syntactic: Ruff's find_argument";
    } else {
        if let Some(pos) = pos {
            let mut l = pre();
            l.push(format!("args_pos(l, v, {pos})"));
            if kwok {
                l.push(format!("!args_kw(l, {qn}, _)"));
            }
            rules_v.push(rs(&format!("{val}(n, v)"), l));
            if pos == 0 {
                rules_v.push(gen0());
            }
        }
        if kwok {
            let mut l = pre();
            l.push(format!("args_kw(l, {qn}, v)"));
            l.push(format!("count : {{ args_kw(l, {qn}, _) }} = 1"));
            if let Some(pos) = pos {
                l.push(format!("!args_pos(l, _, {pos})"));
            }
            rules_v.push(rs(&format!("{val}(n, v)"), l));
        }
        let mut neg = Vec::new();
        if let Some(pos) = pos {
            neg.push(format!("!args_pos(l, _, {pos})"));
            neg.push(format!("count : {{ args_star(l, j), j <= {pos} }} = 0"));
            if pos == 0 {
                neg.push("!kind(l, \"generator_expression\")".into());
            }
        }
        if kwok {
            neg.push(format!("!args_kw(l, {qn}, _)"));
            neg.push("!args_kw_spread(l)".into());
        }
        let mut l = pre();
        l.extend(neg);
        rules_a.push(rs(&format!("{absent}(n)"), l));
        mode = "as Python binds the call";
    }
    lib(prog, &val, "(n: node, v: node)", &[&format!("call n passes v for slot `{slot}` of ({shown}), {mode}")], rules_v);
    lib(prog, &absent, "(n: node)", &[&format!("call n does not pass slot `{slot}` of ({shown}), {mode}")], rules_a);
    prog.arg_rels.push((key, (val.clone(), absent.clone())));
    Ok((val, absent))
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_list).map(|l| l.iter().map(py_str).collect()).unwrap_or_default()
}

pub fn arguments(prog: &mut Program, spec: &Obj, v: &str, vs: &mut Vars) -> Result<Vec<String>, String> {
    let sig = str_list(spec.get("signature").filter(|s| s.truthy()));
    let syntactic = spec.get("mode").and_then(Value::as_str) == Some("syntactic");
    let mut lits = vec![format!("kind({v}, \"call\")")];
    if let Some(bind) = spec.get("bind").filter(|b| b.truthy()).and_then(Value::as_obj) {
        for (cap, slot) in bind.iter() {
            let (val, _) = slot_rels(prog, &sig, &py_str(slot), syntactic)?;
            let x = vs.fresh("a");
            lits.push(format!("{val}({v}, {x}), {cap} = {x}"));
        }
    }
    for slot in str_list(spec.get("absent").filter(|b| b.truthy())) {
        let (_, absent) = slot_rels(prog, &sig, &slot, syntactic)?;
        lits.push(format!("{absent}({v})"));
    }
    Ok(lits)
}

// ---- value ----------------------------------------------------------------------------------------------------------

pub const VALUE_OPS: [&str; 9] = ["truthy", "bitsAny", "constant", "type", "startsWith", "gt", "ge", "lt", "le"];

fn numeric(x: &str) -> String {
    format!("(value_type({x}, \"int\") ; value_type({x}, \"float\"))")
}

fn truthy_rel(prog: &mut Program) -> String {
    let zero = "(x = \"0\" ; x = \"0.0\" ; x = \"-0.0\")";
    let nonzero = "x != \"0\", x != \"0.0\", x != \"-0.0\"";
    let num = numeric("x");
    let text = "(value_type(x, \"str\") ; value_type(x, \"bytes\"))";
    lib(
        prog,
        "truthy_value",
        "(n: node, b: symbol)",
        &["b is \"true\" when n's value is truthy (Python's bool(v)), \"false\" when it is falsy"],
        vec![
            r("truthy_value(n, \"false\")", &["value(n, x)", "value_type(x, \"NoneType\")"]),
            r("truthy_value(n, b)", &["value(n, x)", "value_type(x, \"boolean\")", "b = x"]),
            r("truthy_value(n, \"false\")", &["value(n, x)", &num, zero]),
            r("truthy_value(n, \"true\")", &["value(n, x)", &num, nonzero]),
            r("truthy_value(n, \"false\")", &["value(n, x)", text, "x = \"\""]),
            r("truthy_value(n, \"true\")", &["value(n, x)", text, "x != \"\""]),
        ],
    )
}

fn bits(x: &str, mask: i128) -> String {
    let len = if mask > 0 { 128 - mask.leading_zeros() } else if mask < 0 { 128 - (-mask).leading_zeros() } else { 0 };
    let tests: Vec<String> = (0..len)
        .filter(|i| (mask >> i) & 1 == 1)
        .map(|i| if i > 0 { format!("(to_number({x}) / {}) % 2 = 1", 1i128 << i) } else { format!("to_number({x}) % 2 = 1") })
        .collect();
    if tests.len() == 1 { tests[0].clone() } else { format!("({})", tests.join(" ; ")) }
}

fn cmp_sym(op: &str) -> &'static str {
    match op {
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        "ne" => "!=",
        _ => "",
    }
}

pub fn value(px: &Patterns, prog: &mut Program, val: &Obj, v: &str, vs: &mut Vars) -> Result<Vec<String>, String> {
    let reader = if val.get("follow").is_some_and(Value::truthy) { "static_value" } else { "value" };
    let ops = val.without(&["follow"]);
    if ops.get("constant").is_some_and(Value::is_false) && ops.len() == 1 {
        return Ok(vec![format!("!{reader}({v}, _)")]);
    }
    let x = vs.fresh("v");
    let mut lits = vec![format!("{reader}({v}, {x})")];
    for (op, arg) in ops.iter() {
        match op {
            "constant" => {
                if arg.is_false() {
                    lits.push("false".into());
                }
            }
            "regex" => lits.push(match_(px, &Lit::Val(arg), &x)),
            "eq" => lits.push(opt_alts(arg, &|c: &Lit| format!("{x} = {}", c.dumps()))),
            "ne" => lits.push(opt_alts(arg, &|c: &Lit| format!("{x} != {}", c.dumps()))),
            "type" => {
                let types: Vec<Value> = if let Value::List(l) = arg { l.clone() } else { vec![arg.clone()] };
                let alts: Vec<String> = types
                    .iter()
                    .map(|t| {
                        let t2 = if prog.lang == "python" && t.as_str() == Some("bool") { Value::Str("boolean".into()) } else { t.clone() };
                        format!("value_type({x}, {})", super::value::q_val(&t2))
                    })
                    .collect();
                lits.push(if alts.len() == 1 { alts[0].clone() } else { format!("({})", alts.join(" ; ")) });
            }
            "gt" | "ge" | "lt" | "le" if matches!(arg, Value::Int(_) | Value::Float(_)) => {
                lits.push(numeric(&x));
                lits.push(format!("to_number({x}) {} {}", cmp_sym(op), py_str(arg)));
            }
            "gt" | "ge" | "lt" | "le" => {
                let sym = cmp_sym(op);
                lits.push(opt_alts(arg, &|c: &Lit| format!("{x} {sym} {}", c.dumps())));
            }
            "startsWith" => {
                let pre: Vec<Value> = if let Value::List(l) = arg { l.clone() } else { vec![arg.clone()] };
                let alts: Vec<String> = pre.iter().map(|p| format!("prefix({}, {x})", super::value::q_val(p))).collect();
                lits.push(if alts.len() == 1 { alts[0].clone() } else { format!("({})", alts.join(" ; ")) });
            }
            "bitsAny" if arg.is_int() => {
                lits.push(format!("value_type({x}, \"int\")"));
                lits.push(bits(&x, arg.int().expect("an int")));
            }
            "truthy" if matches!(arg, Value::Bool(_)) => {
                let rel = truthy_rel(prog);
                lits.push(format!("{rel}({v}, {})", q(if arg.is_true() { "true" } else { "false" })));
            }
            _ => {
                prog.untranslated += 1;
                let term = Value::obj(vec![("value", Value::obj(vec![(op, arg.clone())]))]);
                lits.push(format!("ast({v}, {})", dumps(&term, false, ", ", ": ", false)));
            }
        }
    }
    Ok(lits)
}

// ---- resolves: names in import statements, and modules seen before ----------------------------------------------

const FROM: &str = "(kind(s, \"import_from_statement\") ; kind(s, \"future_import_statement\"))";

fn statement_names(prog: &mut Program) -> String {
    lib(
        prog,
        "from_module",
        "(s: node, m: symbol)",
        &["the module text of from-import s, read as `relative: \"text\"` does (dots dropped)"],
        vec![
            r("from_module(s, m)", &["kind(s, \"future_import_statement\")", "m = \"__future__\""]),
            r("from_module(s, m)", &["kind(s, \"import_from_statement\")", "field(s, \"module_name\", x)", "kind(x, \"dotted_name\")", "name(x, m)"]),
            r("from_module(s, m)", &["kind(s, \"import_from_statement\")", "field(s, \"module_name\", x)", "kind(x, \"relative_import\")", "child(x, y, _)", "kind(y, \"dotted_name\")", "name(y, m)"]),
            r("from_module(s, \"\")", &["kind(s, \"import_from_statement\")", "field(s, \"module_name\", x)", "kind(x, \"relative_import\")", "count : { child(x, y, _), kind(y, \"dotted_name\") } = 0"]),
        ],
    );
    lib(
        prog,
        "statement_name",
        "(n: node, q: symbol)",
        &["dotted name n in an import statement imports q (relative modules by their text)"],
        vec![
            r("statement_name(n, q)", &["kind(n, \"dotted_name\")", "child(s, n, _)", "kind(s, \"import_statement\")", "name(n, q)"]),
            r("statement_name(n, q)", &["kind(n, \"dotted_name\")", "child(s, n, _)", "kind(s, \"relative_import\")", "name(n, q)"]),
            r("statement_name(n, q)", &["kind(n, \"dotted_name\")", "field(a, \"name\", n)", "kind(a, \"aliased_import\")", "child(s, a, _)", "kind(s, \"import_statement\")", "name(n, q)"]),
            r("statement_name(n, q)", &["kind(n, \"dotted_name\")", "field(a, \"name\", n)", "kind(a, \"aliased_import\")", "child(s, a, _)", FROM, "from_module(s, m)", "name(n, x)", "q = @dot(m, x)"]),
            r("statement_name(n, m)", &["kind(n, \"dotted_name\")", "field(s, \"module_name\", n)", FROM, "from_module(s, m)"]),
            r("statement_name(n, q)", &["kind(n, \"dotted_name\")", "child(s, n, _)", FROM, "!field(s, \"module_name\", n)", "from_module(s, m)", "name(n, x)", "q = @dot(m, x)"]),
        ],
    )
}

pub fn statement_imported(prog: &mut Program, names: &[Value], v: &str, vs: &mut Vars, prefix: bool) -> Vec<String> {
    let rel = statement_names(prog);
    let qv = vs.fresh("q");
    let alts: Vec<String> = names
        .iter()
        .map(|n| {
            let ns = super::value::q_val(n);
            if prefix { format!("({qv} = {ns} ; prefix({}, {qv}))", q(&(py_str(n) + "."))) } else { format!("{qv} = {ns}") }
        })
        .collect();
    vec![format!("{rel}({v}, {qv})"), if alts.len() == 1 { alts[0].clone() } else { format!("({})", alts.join(" ; ")) }]
}

fn fn_depth(n: &str, k: &str) -> String {
    format!("count : {{ within({n}, b{k}), field(f{k}, \"body\", b{k}), kind(f{k}, \"function_definition\") }}")
}

pub fn seen_before(prog: &mut Program, modules: &[Value], v: &str, vs: &mut Vars) -> Vec<String> {
    let rel = lib(
        prog,
        "seen_import",
        "(s: node, h: symbol, k: number)",
        &["import statement s imports absolute module h (its first segment), inside k function bodies"],
        vec![
            rs(
                "seen_import(s, h, k)",
                vec![
                    "kind(s, \"import_statement\")".into(),
                    "child(s, c, _)".into(),
                    "(kind(c, \"dotted_name\"), x = c ; kind(c, \"aliased_import\"), field(c, \"name\", x))".into(),
                    "child(x, i, 0)".into(),
                    "kind(i, \"identifier\")".into(),
                    "text(i, h)".into(),
                    format!("k = {}", fn_depth("s", "")),
                ],
            ),
            rs(
                "seen_import(s, h, k)",
                vec![
                    "kind(s, \"import_from_statement\")".into(),
                    "field(s, \"module_name\", x)".into(),
                    "kind(x, \"dotted_name\")".into(),
                    "child(x, i, 0)".into(),
                    "kind(i, \"identifier\")".into(),
                    "text(i, h)".into(),
                    format!("k = {}", fn_depth("s", "")),
                ],
            ),
        ],
    );
    let names: Vec<String> = ["d", "st", "s", "so", "h", "ks", "kn"].iter().map(|p| vs.fresh(p)).collect();
    let (d, st, s, so, h, ks, kn) = (&names[0], &names[1], &names[2], &names[3], &names[4], &names[5], &names[6]);
    let mods: Vec<String> = modules.iter().map(|m| format!("{h} = {}", super::value::q_val(m))).collect();
    let k = vs.fresh("");
    vec![
        format!("node({d}, {v}, _, {st}, _)"),
        format!("{rel}({s}, {h}, {ks})"),
        format!("({})", mods.join(" ; ")),
        format!("node({d}, {s}, _, {so}, _)"),
        format!("{kn} = {}", fn_depth(v, &k)),
        format!("({ks} < {kn} ; {ks} = {kn}, {so} < {st} ; count : {{ within({v}, l{k}), field(m{k}, \"body\", l{k}), kind(m{k}, \"lambda\") }} > 0)"),
    ]
}

// ---- resolves.unresolved -------------------------------------------------------------------------------------------

fn dunder_all(prog: &mut Program) -> String {
    lib(
        prog,
        "all_expr",
        "(x: node)",
        &[
            "x is a module-level `__all__ =`, `+=` or `: T =` value, or a part of one that lists names",
            "(Ruff's extract_dunder_all_names: `+` chains, parentheses, walrus values, list(...) and tuple(...))",
        ],
        vec![
            r(
                "all_expr(x)",
                &[
                    "(kind(st, \"assignment\") ; kind(st, \"augmented_assignment\"))",
                    "field(st, \"left\", l)",
                    "kind(l, \"identifier\")",
                    "text(l, \"__all__\")",
                    "child(es, st, _)",
                    "kind(es, \"expression_statement\")",
                    "child(m, es, _)",
                    "kind(m, \"module\")",
                    "field(st, \"right\", x)",
                ],
            ),
            r("all_expr(y)", &["all_expr(x)", "kind(x, \"assignment\")", "field(x, \"right\", y)"]),
            r("all_expr(y)", &["all_expr(x)", "kind(x, \"binary_operator\")", "(field(x, \"left\", y) ; field(x, \"right\", y))"]),
            r("all_expr(y)", &["all_expr(x)", "kind(x, \"parenthesized_expression\")", "child(x, y, _)", "!kind(y, \"comment\")"]),
            r("all_expr(y)", &["all_expr(x)", "kind(x, \"named_expression\")", "field(x, \"value\", y)"]),
            r("all_expr(y)", &["all_expr(x)", "kind(x, \"call\")", "field(x, \"function\", f)", "kind(f, \"identifier\")", "(text(f, \"list\") ; text(f, \"tuple\"))", "field(x, \"arguments\", a)", "child(a, y, _)"]),
        ],
    );
    lib(
        prog,
        "all_entry",
        "(e: node, x: symbol)",
        &["string e is an entry of a module `__all__` definition, exporting the name x"],
        vec![r(
            "all_entry(e, x)",
            &["all_expr(l)", "(kind(l, \"list\") ; kind(l, \"tuple\"))", "child(l, e, _)", "(kind(e, \"string\") ; kind(e, \"concatenated_string\"))", "count : { within(i, e), kind(i, \"interpolation\") } = 0", "str(e, x)"],
        )],
    )
}

pub fn unresolved(prog: &mut Program, mode: &Value, v: &str, vs: &mut Vars) -> Vec<String> {
    let rel = dunder_all(prog);
    let (x, d, m) = (vs.fresh("x"), vs.fresh("d"), vs.fresh("m"));
    let star = if mode.as_str() == Some("star") { format!("star({m})") } else { format!("!star({m})") };
    vec![
        format!("docof({v}, {d})"),
        format!("modnode({d}, {m})"),
        format!("( kind({v}, \"identifier\"), !bind_only({v}), miss({v}, {x}) ; (kind({v}, \"string\") ; kind({v}, \"concatenated_string\")), {rel}({v}, {x}), !bound_in({m}, {x}) )"),
        format!("!builtin({x})"),
        star,
    ]
}

// ---- layout (matcher/layout.rs) -----------------------------------------------------------------------------------

fn bounds(c: &Value, x: &str) -> Vec<String> {
    let Some(o) = c.as_obj() else { return Vec::new() };
    o.iter()
        .filter_map(|(k, n)| {
            let op = match k {
                "min" => ">=",
                "max" => "<=",
                "eq" => "=",
                _ => return None,
            };
            let int = match n {
                Value::Int(i) => i.to_string(),
                Value::Bool(b) => (*b as i32).to_string(),
                Value::Float(f) => (f.trunc() as i128).to_string(),
                Value::Str(s) => s.trim().parse::<i128>().map(|i| i.to_string()).unwrap_or_else(|_| s.clone()),
                other => py_str(other),
            };
            Some(format!("{x} {op} {int}"))
        })
        .collect()
}

fn code_line(prog: &mut Program, side: &str, comments: bool) -> String {
    let name = format!("code_{side}_line{}", if comments { "_c" } else { "" });
    if prog.lib.contains_key(&name) {
        return name;
    }
    let nl = |w: &str| format!("strlen({w}) - strlen(@replace_all({w}, \"\\n\", \"\"))");
    let mut rules: Vec<Rule>;
    if side == "before" {
        let (code, blank) = ("\"(?s).*\\\\S\\\\s*\"", "\"\\\\s*\"");
        rules = vec![
            rs("code_before_line(x, l)", vec!["structural_before(x, g)".into(), format!("match({code}, g)"), "ws = @replace_all(g, \"(?s)^.*\\\\S\", \"\")".into(), format!("k = {}", nl("ws")), "line(x, lx)".into(), "l = lx - k".into()]),
            rs(
                "code_before_line(x, l)",
                vec![
                    "structural_before(x, g)".into(),
                    format!("match({blank}, g)"),
                    "child(p, x, _)".into(),
                    "node(_, x, _, s, _)".into(),
                    "o = s - strlen(g)".into(),
                    "token(p, _, _, _, o)".into(),
                    "!code_comment_end(p, o)".into(),
                    format!("k = {}", nl("g")),
                    "line(x, lx)".into(),
                    "l = lx - k".into(),
                ],
            ),
            rs(
                "code_before_line(x, l)",
                vec!["structural_before(x, g)".into(), format!("match({blank}, g)"), "child(p, x, _)".into(), "node(_, x, _, s, _)".into(), "node(_, p, _, ps, _)".into(), "ps = s - strlen(g)".into(), "code_before_line(p, l)".into()],
            ),
            rs(
                "code_before_line(x, l)",
                vec![
                    "structural_before(x, g)".into(),
                    format!("match({blank}, g)"),
                    "child(p, x, _)".into(),
                    "node(_, x, _, s, _)".into(),
                    "o = s - strlen(g)".into(),
                    "child(p, c, _)".into(),
                    "node(_, c, _, _, o)".into(),
                    "comment(c, _)".into(),
                    "code_before_line(c, l)".into(),
                ],
            ),
        ];
        prog.lib.insert(
            "code_comment_end".into(),
            (".decl code_comment_end(p: node, o: number)".into(), vec!["// code_comment_end(p, o): a comment child of p ends at offset o".into()], vec![r("code_comment_end(p, o)", &["child(p, c, _)", "comment(c, _)", "node(_, c, _, _, o)"])]),
        );
    } else {
        let (code, blank) = ("\"(?s)\\\\s*\\\\S.*\"", "\"\\\\s*\"");
        rules = vec![
            rs("code_after_line(x, l)", vec!["structural_after(x, g)".into(), format!("match({code}, g)"), "ws = @replace_all(g, \"(?s)\\\\S.*\", \"\")".into(), format!("k = {}", nl("ws")), "end_line(x, lx)".into(), "l = lx + k".into()]),
            rs(
                "code_after_line(x, l)",
                vec![
                    "structural_after(x, g)".into(),
                    format!("match({blank}, g)"),
                    "child(p, x, _)".into(),
                    "node(_, x, _, _, e)".into(),
                    "o = e + strlen(g)".into(),
                    "token(p, _, _, o, _)".into(),
                    "!code_comment_start(p, o)".into(),
                    format!("k = {}", nl("g")),
                    "end_line(x, lx)".into(),
                    "l = lx + k".into(),
                ],
            ),
            rs(
                "code_after_line(x, l)",
                vec!["structural_after(x, g)".into(), format!("match({blank}, g)"), "child(p, x, _)".into(), "node(_, x, _, _, e)".into(), "node(_, p, _, _, pe)".into(), "pe = e + strlen(g)".into(), "code_after_line(p, l)".into()],
            ),
            rs(
                "code_after_line(x, l)",
                vec![
                    "structural_after(x, g)".into(),
                    format!("match({blank}, g)"),
                    "child(p, x, _)".into(),
                    "node(_, x, _, _, e)".into(),
                    "o = e + strlen(g)".into(),
                    "child(p, c, _)".into(),
                    "node(_, c, _, o, _)".into(),
                    "comment(c, _)".into(),
                    "code_after_line(c, l)".into(),
                ],
            ),
        ];
        prog.lib.insert(
            "code_comment_start".into(),
            (".decl code_comment_start(p: node, o: number)".into(), vec!["// code_comment_start(p, o): a comment child of p starts at offset o".into()], vec![r("code_comment_start(p, o)", &["child(p, c, _)", "comment(c, _)", "node(_, c, _, o, _)"])]),
        );
    }
    if comments {
        let plain = format!("code_{side}_line");
        rules = rules
            .into_iter()
            .filter(|(_, b)| !b.iter().any(|l| l == "comment(c, _)"))
            .map(|(h, b)| (h.replace(&plain, &name), b.into_iter().filter(|l| !l.starts_with("!code_comment_")).map(|l| l.replace(&format!("{plain}("), &format!("{name}("))).collect()))
            .collect();
    }
    prog.lib.insert(
        name.clone(),
        (
            format!(".decl {name}(x: node, l: number)"),
            vec![format!(
                "// {name}(x, l): the line of the code {side} x (main's token_{side}{}), read from the source gap: the tape leaves out the tokens its grammar implies",
                if comments { ", comments counted as code" } else { ", comments passed over" }
            )],
            rules,
        ),
    );
    name
}

/// Literals for a `layout` term, or None when it asks for something not lowered.
pub fn layout(prog: &mut Program, spec: &Obj, v: &str, vs: &mut Vars) -> Option<Vec<String>> {
    if !spec.keys_within(&["span", "start", "before", "after"]) {
        return None;
    }
    let mut lits = Vec::new();
    if let Some(span) = spec.get("span") {
        let (a, b, x) = (vs.fresh("l"), vs.fresh("l"), vs.fresh("k"));
        lits.extend([format!("line({v}, {a})"), format!("end_line({v}, {b})"), format!("{x} = {b} - {a}")]);
        lits.extend(bounds(span, &x));
    }
    if let Some(start) = spec.get("start") {
        let s = vs.fresh("st");
        lits.push(format!("node(_, {v}, _, {s}, _)"));
        lits.extend(bounds(start, &s));
    }
    for side in ["before", "after"] {
        let Some(gap) = spec.get(side) else { continue };
        if gap.is_null() {
            continue;
        }
        let gap_o = gap.as_obj()?;
        let to_v = gap_o.get("to").cloned().unwrap_or(Value::Str("token".into()));
        let comments = gap_o.get("comments").is_some_and(Value::truthy);
        if !gap_o.keys_within(&["to", "breaks", "edge", "comments"]) || (comments && !matches!(to_v.as_str(), Some("token" | "sibling"))) {
            return None;
        }
        let (x, nb) = (vs.fresh("k"), vs.fresh("t"));
        let (a, b) = (vs.fresh("l"), vs.fresh("l"));
        let (here, mut there) = if side == "before" { (format!("line({v}, {b})"), Some(format!("end_line({nb}, {a})"))) } else { (format!("end_line({v}, {a})"), Some(format!("line({nb}, {b})"))) };
        let find: Vec<String>;
        match to_v.as_str() {
            Some("token") => {
                let rel = code_line(prog, side, comments);
                find = vec![format!("{rel}({v}, {})", if side == "before" { &a } else { &b })];
                there = None;
            }
            Some("sibling") if comments => {
                find = vec![if side == "before" { format!("next_sibling({nb}, {v})") } else { format!("next_sibling({v}, {nb})") }];
            }
            Some("sibling") => {
                let m = vs.fresh("m");
                let (a_, b_) = if side == "before" { (nb.clone(), v.to_owned()) } else { (v.to_owned(), nb.clone()) };
                find = vec![format!("precedes({a_}, {b_})"), format!("!comment({nb}, _)"), format!("count : {{ precedes({a_}, {m}), precedes({m}, {b_}), !comment({m}, _) }} = 0")];
            }
            Some(t) if t.starts_with("capture.") => {
                let c = t.split_once('.').expect("a dot").1.to_owned();
                let (s1, e1, s2, e2) = (vs.fresh("s"), vs.fresh("e"), vs.fresh("s"), vs.fresh("e"));
                find = vec![
                    format!("{nb} = {c}"),
                    format!("node(_, {v}, _, {s1}, {e1})"),
                    format!("node(_, {c}, _, {s2}, {e2})"),
                    if side == "before" { format!("{e2} <= {s1}") } else { format!("{s2} >= {e1}") },
                ];
            }
            _ => return None,
        }
        let breaks = gap_o.get("breaks").filter(|b| b.truthy()).cloned().unwrap_or(Value::Obj(Obj::default()));
        let bnds = bounds(&breaks, &x);
        let mut found = find.clone();
        found.push(here);
        if let Some(t) = there {
            found.push(t);
        }
        found.push(format!("{x} = {b} - {a}"));
        found.extend(bnds.iter().cloned());
        if gap_o.get("edge").is_some_and(Value::truthy) {
            let mut edge;
            if side == "before" {
                edge = vec![format!("count : {{ {} }} = 0", find.join(", ")), format!("line({v}, {b})"), format!("{x} = {b} - 1")];
            } else {
                let (d, k) = (vs.fresh("d"), vs.fresh("k"));
                edge = vec![format!("count : {{ {} }} = 0", find.join(", ")), format!("end_line({v}, {a})"), format!("docof({v}, {d})"), format!("line_count({d}, {k})"), format!("{x} = {k} - {a}")];
            }
            edge.extend(bnds);
            lits.push(format!("( {} ; {} )", found.join(", "), edge.join(", ")));
        } else {
            lits.extend(found);
        }
    }
    Some(lits)
}
