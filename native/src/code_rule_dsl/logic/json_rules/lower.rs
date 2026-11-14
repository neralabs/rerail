//! The translator's lowering of one JSON rule object at a time: `lower` gives the body literals that hold where a
//! node matches the object, adding the helper relations it needs (alternatives, stop sets, `$option` cases) to
//! the program. Also the clause-level pieces: utils, transforms, fixes and the violation's lines.

use super::main_terms;
use super::program::{Program, Vars};
use super::patterns::Patterns;
use super::pyre::{captures, esc, findall, fullmatch, g, has, py_escape, re, sub};
use super::souffle::whole;
use super::textglob::text_test;
use super::value::{Obj, Value, compact, dumps, py_str, q, q_val, truthy};
use std::collections::BTreeSet;

pub type R<T> = Result<T, String>;

const INLINE_MAX: usize = 4;

/// A literal an operand renders to: a JSON constant, or an option's variable (Python's `OptVar`).
pub enum Lit<'a> {
    Opt(String),
    Val(&'a Value),
}

impl Lit<'_> {
    /// `q(s)`: an option's variable as it is, a constant as JSON.
    pub fn q(&self) -> String {
        match self {
            Lit::Opt(s) => s.clone(),
            Lit::Val(v) => q_val(v),
        }
    }

    /// main_terms' `_q(s)`, `json.dumps`: an option's variable quoted too (it is a `str`), as the Python writes it.
    pub fn dumps(&self) -> String {
        match self {
            Lit::Opt(s) => super::value::q(s),
            Lit::Val(v) => q_val(v),
        }
    }
}

/// A backtick literal: fenced with one more backtick than the longest run inside. Newlines stay
/// real newlines: the reader takes the body as written, so an escaped `\n` would be two characters
/// of code and a multi-line pattern would never match.
pub fn tick(s: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for ch in s.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    if longest == 0 {
        return format!("`{s}`");
    }
    let f = "`".repeat(longest + 1);
    format!("{f} {s} {f}")
}

/// `re.sub(r"[^0-9A-Za-z]+", "_", s)`.
fn underscores(s: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

/// A util's relation: its name in snake case, and `_util` after a name core.rr declares (a util `field` is not the
/// core `field(p, f, c)` the rules beside it read).
pub fn util_name(name: &str) -> String {
    let s = snake(name);
    if crate::code_rule_dsl::logic::rr::lower::is_core(&s) { format!("{s}_util") } else { s }
}

pub fn snake(name: &str) -> String {
    let cs: Vec<char> = name.chars().collect();
    let mut s = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if i > 0 && c.is_ascii_uppercase() && (cs[i - 1].is_ascii_lowercase() || cs[i - 1].is_ascii_digit()) {
            s.push('_');
        }
        s.push(c);
    }
    let mut s = underscores(&s).trim_matches('_').to_owned();
    if s.is_empty() {
        s = "r".into();
    }
    if s.chars().next().is_some_and(char::is_numeric) {
        s = format!("r_{s}");
    }
    s.to_lowercase()
}

pub fn ident(code: &str) -> String {
    let s = underscores(code).trim_matches('_').to_owned();
    if s.is_empty() || s.chars().next().is_some_and(char::is_numeric) {
        return format!("V{s}");
    }
    s
}

/// A JSON metavariable reference ("$X", "X", "$$$X") as a Datalog variable.
pub fn mv(name: &str) -> String {
    name.trim_start_matches('$').to_owned()
}

pub fn mv_v(name: &Value) -> String {
    mv(&py_str(name))
}

/// A capture name as a variable: in capitals.
pub fn cap(name: &str) -> String {
    mv(name).to_uppercase()
}

/// A helper's column for a metavariable: its name in lower case, unless that is `n`.
pub fn col(name: &str) -> String {
    let c = mv(name).to_lowercase();
    if c == "n" { format!("{c}_mv") } else { c }
}

/// Named groups renamed to capitals.
pub fn caps(pattern: &str) -> String {
    sub(&re(r"\(\?(P?)<([A-Za-z_]\w*)>"), pattern, |c| format!("(?{}<{}>", g(c, 1), g(c, 2).to_uppercase()))
}

fn ast_atom(prog: &mut Program, v: &str, rule: Value) -> String {
    prog.untranslated += 1;
    format!("ast({v}, {})", compact(&Value::obj(vec![("language", Value::Str(prog.lang.clone())), ("rule", rule)])))
}

fn is_option(v: &Value) -> bool {
    v.has("$option")
}

fn cases_of(v: &Value) -> Vec<(String, Value)> {
    match v.get("cases") {
        Some(Value::Obj(o)) => o.0.clone(),
        _ => Vec::new(),
    }
}

/// A JSON operand that may be `{"$option": name, "cases": {…}, "default": …}`: one guarded alternative per
/// configured value.
pub fn opt_alts(arg: &Value, render: &dyn Fn(&Lit) -> String) -> String {
    if !is_option(arg) {
        return render(&Lit::Val(arg));
    }
    let o = arg.as_obj().expect("an object");
    if !(truthy(o.get("cases")) || o.has("default")) {
        return render_option(o, render);
    }
    let name = q_val(o.get("$option").expect("$option"));
    let cases = cases_of(arg);
    let mut parts: Vec<String> = cases.iter().map(|(c, b)| format!("option({name}, {}), {}", q(c), opt_alts(b, render))).collect();
    let guard: Vec<String> = cases.iter().map(|(c, _)| format!("!option({name}, {})", q(c))).collect();
    if let Some(d) = o.get("default") {
        let mut all = guard;
        all.push(opt_alts(d, render));
        parts.push(all.join(", "));
    }
    if parts.len() > 1 { format!("({})", parts.join(" ; ")) } else { parts.remove(0) }
}

/// An option's value as an operand. A list option has no `option` row, only `option_item` rows (core.rr), so
/// `{"$option": L, "join": "|", "template": T}` (the regex T around the items joined by `|`) is read per item: the
/// joined regex matches where the regex T around some one item does. A `template` with no `join` puts a scalar
/// option's value where the template's first `{}` is (`"^.{0,{}}$"`), as main does.
fn render_option(opt: &Obj, render: &dyn Fn(&Lit) -> String) -> String {
    let name = opt.get("$option").expect("$option");
    let o = format!("o_{}", snake(&py_str(name)));
    let template = opt.get("template").and_then(Value::as_str);
    let around = |v: &str| -> Option<(String, String)> {
        let (pre, post) = template?.split_once("{}")?;
        let r = format!("{v}_t");
        Some((format!("{r} = cat({}, cat({v}, {}))", q(pre), q(post)), r))
    };
    if opt.get("join").and_then(Value::as_str) == Some("|") {
        let item = format!("option_item({}, _, {o})", q_val(name));
        return match around(&o) {
            Some((lit, r)) => format!("{item}, {lit}, {}", render(&Lit::Opt(r))),
            None => format!("{item}, {}", render(&Lit::Opt(o.clone()))),
        };
    }
    match around(&o) {
        Some((lit, r)) if !opt.has("join") => format!("option({}, {o}), {lit}, {}", q_val(name), render(&Lit::Opt(r))),
        _ => format!("option({}, {o}), {}", q_val(name), render(&Lit::Opt(o.clone()))),
    }
}

/// `match(pattern, subject)`: a glob-family test where it is exactly the regex, `match` otherwise.
pub fn match_(px: &Patterns, pattern: &Lit, subject: &str) -> String {
    match pattern {
        Lit::Opt(o) => format!("match({o}, {subject})"),
        Lit::Val(v) if is_option(v) => opt_alts(v, &|p: &Lit| match_(px, p, subject)),
        Lit::Val(Value::Str(p)) => {
            let rx = whole(px, &caps(p));
            text_test(&rx, subject).ok().flatten().unwrap_or_else(|| format!("match({}, {subject})", q(&rx)))
        }
        Lit::Val(v) => format!("ast({subject}, {})", compact(&Value::obj(vec![("regex", (*v).clone())]))),
    }
}

fn match_str(px: &Patterns, p: &str, subject: &str) -> String {
    match_(px, &Lit::Val(&Value::Str(p.to_owned())), subject)
}

/// `!(…)` in Soufflé form: one atom, a disjunction of atoms (De Morgan), or an aggregate.
pub fn negate(px: &Patterns, lits: Vec<String>) -> Vec<String> {
    if lits.len() == 1 && super::text::is_atom(px, &lits[0]) {
        return vec![format!("!{}", lits[0])];
    }
    if lits.len() == 1
        && let Some(atoms) = super::text::disj_atoms(px, &lits[0])
    {
        return atoms.into_iter().map(|a| format!("!{a}")).collect();
    }
    vec![format!("count : {{ {} }} = 0", lits.join(", "))]
}

/// `fmt.format(x)` for a format with one `{}`.
fn fmt1(fmt: &str, x: &str) -> String {
    fmt.replacen("{}", x, 1)
}

/// One value: the atom; a few: a disjunction; many: a relation of facts. A `{"$option": name, "spread": true}`
/// entry splices in the option's configured list.
fn one_of(prog: &mut Program, vs: &mut Vars, fmt: &str, values: &[Value], p: &str, note: &str) -> String {
    let spread: Vec<Value> = values.iter().filter(|x| is_option(x) && x.get("spread").is_some_and(Value::is_true)).map(|x| x.get("$option").expect("$option").clone()).collect();
    if !spread.is_empty() {
        let kept: Vec<Value> = values.iter().filter(|x| !x.is_obj()).cloned().collect();
        let name = prog.facts("set", "v", kept, note);
        for o in spread {
            prog.add_rule(format!("{name}(x)"), vec![format!("option_item({}, _, x)", q_val(&o))]);
        }
        let x = vs.fresh(p);
        return format!("{}, {name}({x})", fmt1(fmt, &x));
    }
    if values.len() == 1 {
        return fmt1(fmt, &q_val(&values[0]));
    }
    if values.len() <= INLINE_MAX {
        return format!("({})", values.iter().map(|x| fmt1(fmt, &q_val(x))).collect::<Vec<_>>().join(" ; "));
    }
    let name = prog.facts("set", "v", values.to_vec(), note);
    let x = vs.fresh(p);
    format!("{}, {name}({x})", fmt1(fmt, &x))
}

fn sym_of(op: &str) -> &str {
    match op {
        "ne" => "!=",
        "lt" => "<",
        "le" => "<=",
        "gt" => ">",
        "ge" => ">=",
        other => other,
    }
}

/// Literals that hold when node v matches the rule object.
/// A rule whose subject is an anonymous token that opens its parent: `{regex: "^except$", inside: {stopBy: neighbor,
/// kind: except_clause, ..}, precedes: {stopBy: neighbor, regex: "^:$"}}` (E722's bare `except:`), or a capture of
/// one of several (`regex: "^(?<BRANCH>elif|else)$"` inside an elif or else clause, RET505). A token the tape stores
/// is the parent's first `token` child, the next one the rule names its second; one the tape leaves out (RER-131) is
/// read at the start of the source before the parent's first stored child (the parent's whole text where it has
/// none). The subject is the parent, which starts where the token does, so a finding sits on the token's line and
/// column, and a capture is the token's text. Uncaptured, a named node the regex matches (`self`) is the first
/// alternative; a capture regex over a node of no named kind is refused by the reader, so a captured token has none.
/// None when the rule is not of this shape (more keys, a regex that is not anchored literal tokens, a field, a
/// parent of no named kind).
fn leading_token(px: &Patterns, prog: &mut Program, rule: &Obj, v: &str, vs: &mut Vars) -> R<Option<Vec<String>>> {
    if !rule.keys_within(&["regex", "inside", "precedes"]) || !rule.has("inside") {
        return Ok(None);
    }
    let literal = re(r"^(?:[A-Za-z_]+|[^\w\s\\.^$*+?()\[\]{}|]+)$");
    let group = re(r"^\(\?P?<([A-Za-z_]\w*)>(.*)\)$");
    // an anchored regex of literal tokens: (its capture, the tokens)
    let tokens = |rx: Option<&Value>| -> Option<(Option<String>, Vec<String>)> {
        let rx = rx?.as_str()?;
        if !(rx.starts_with('^') && rx.ends_with('$')) || rx.chars().count() < 3 {
            return None;
        }
        let body = super::text::chars_slice(rx, 1, 1);
        let (cap, inner) = match super::pyre::find(&group, &body) {
            Some(c) => (Some(g(&c, 1).to_owned()), g(&c, 2).to_owned()),
            None => (None, body.clone()),
        };
        let alts: Vec<String> = inner.split('|').map(str::to_owned).collect();
        alts.iter().all(|a| literal.is_match(a).unwrap_or(false)).then_some((cap, alts))
    };
    let neighbor = |o: &Obj| o.get("stopBy").is_none_or(|s| s.as_str() == Some("neighbor"));
    let Some((cap, toks)) = tokens(rule.get("regex")) else { return Ok(None) };
    let Some(inside) = rule.get("inside").and_then(Value::as_obj) else { return Ok(None) };
    let kind_alt = |a: &Value| a.as_obj().is_some_and(|o| o.key_set_is(&["kind"]) && o.get("kind").is_some_and(Value::is_str));
    let named_kind = inside.get("kind").is_some_and(Value::is_str)
        || inside.get("any").and_then(Value::as_list).is_some_and(|l| !l.is_empty() && l.iter().all(kind_alt));
    if !neighbor(inside) || !named_kind || inside.has("field") {
        return Ok(None);
    }
    let after = match rule.get("precedes") {
        None => None,
        Some(nxt) => {
            let a = nxt.as_obj().filter(|o| o.without(&["stopBy"]).key_set_is(&["regex"]) && neighbor(o)).and_then(|o| tokens(o.get("regex")));
            match a {
                Some((None, a)) if a.len() == 1 => Some(a[0].clone()),
                _ => return Ok(None),
            }
        }
    };
    let named = if cap.is_none() { Some(lower(px, prog, &Value::Obj(rule.clone()), v, vs, &["__lead__"])?) } else { None };
    let (c, gv) = (vs.fresh("c"), vs.fresh("g"));
    let word = toks.iter().all(|t| t.chars().last().is_some_and(|l| l.is_alphanumeric() || l == '_'));
    let tail = match &after {
        Some(a) => format!(r"\s*{}.*", py_escape(a)),
        None if word => r"(?:\W.*)?".to_owned(),
        None => ".*".to_owned(),
    };
    let alts = toks.iter().map(|t| py_escape(t)).collect::<Vec<_>>().join("|");
    let head = match (&cap, toks.len()) {
        (Some(x), _) => format!("(?<{x}>{alts})"),
        (None, 1) => alts.clone(),
        (None, _) => format!("(?:{alts})"),
    };
    let gap = q(&format!(r"(?s)^\s*{head}{tail}$"));
    let mut stored = match (&cap, toks.len()) {
        (None, 1) => vec![format!("token({v}, 0, {}, _, _)", q(&toks[0]))],
        _ => {
            let t = vs.fresh("t");
            vec![format!("token({v}, 0, {t}, _, _)"), format!("match({}, {t})", q(&format!("^{head}$")))]
        }
    };
    if let Some(a) = &after {
        stored.push(format!("token({v}, 1, {}, _, _)", q(a)));
    }
    // One relation of the parent (and the capture), a rule per form, so the stored-token read carries its reason.
    let ren_re = re(&format!(r"(?<![\w$]){}\b", esc(v)));
    let ren = |ls: &[String]| -> Vec<String> { ls.iter().map(|l| sub(&ren_re, l, |_| "n".to_owned())).collect() };
    let rules = [
        ren(&stored),
        ren(&[format!("child({v}, {c}, 0)"), format!("structural_before({c}, {gv})"), format!("match({gap}, {gv})")]),
        ren(&[format!("!child({v}, _, _)"), format!("node_text({v}, {gv})"), format!("match({gap}, {gv})")]),
    ];
    let head_args = match &cap {
        Some(x) => format!("n, {x}"),
        None => "n".to_owned(),
    };
    let sig = format!("({head_args})\u{2}{}", rules.iter().map(|r| r.join("\u{1}")).collect::<Vec<_>>().join("\u{2}"));
    let name = match prog.token_rels.iter().find(|(s, _)| *s == sig) {
        Some((_, n)) => n.clone(),
        None => {
            let name = format!("tok_lead_{}", prog.token_rels.len() + 1);
            prog.token_rels.push((sig, name.clone()));
            let cols = match &cap {
                Some(x) => format!("n: node, {}: symbol", x.to_lowercase()),
                None => "n: node".to_owned(),
            };
            prog.lib.insert(
                name.clone(),
                (
                    format!(".decl {name}({cols})\n.stored {name} \"its token read is of the parent's first token where the tape stores it; one the tape leaves out is read from the source before the parent's first stored child, in its own rule\""),
                    vec![format!(
                        "// {name}(n): n starts with the token {}: a token the tape stores, or one it leaves out, read from the source before n's first stored child",
                        dumps(rule.get("regex").expect("regex"), true, ", ", ": ", false)
                    )],
                    rules.into_iter().map(|b| (format!("{name}({head_args})"), b)).collect(),
                ),
            );
            name
        }
    };
    // the capture is the token's text, a symbol (`branch: symbol`), not a node or span
    if let Some(x) = &cap {
        prog.symbolic.insert(x.clone());
    }
    let mut token = lower(px, prog, &Value::Obj(inside.clone()), v, vs, &["stopBy"])?;
    token.push(match &cap {
        Some(x) => format!("{name}({v}, {x})"),
        None => format!("{name}({v})"),
    });
    Ok(Some(match named {
        Some(named) => vec![format!("( {} ; {} )", named.join(", "), token.join(", "))],
        None => token,
    }))
}

pub fn lower(px: &Patterns, prog: &mut Program, rule: &Value, v: &str, vs: &mut Vars, skip: &[&str]) -> R<Vec<String>> {
    let mut lits: Vec<String> = Vec::new();
    if let Value::List(rules) = rule {
        for s in rules {
            lits.extend(lower(px, prog, s, v, vs, &[])?);
        }
        if lits.is_empty() {
            lits.push(format!("node(_, {v}, _, _, _)"));
        }
        return Ok(lits);
    }
    let Value::Obj(ro) = rule else {
        prog.untranslated += 1;
        return Ok(vec![format!("ast({v}, {})", compact(rule))]);
    };
    if skip.is_empty()
        && let Some(lead) = leading_token(px, prog, ro, v, vs)?
    {
        return Ok(lead);
    }
    let mut rest = Obj::default();
    for (k, val) in ro.iter() {
        if skip.contains(&k) {
            continue;
        }
        let vo = val.as_obj();
        if k == "kind" {
            lits.push(format!("kind({v}, {})", q_val(val)));
        } else if k == "regex" && is_option(val) && !(truthy(val.get("cases")) || val.has("default")) {
            let o = vs.fresh("o");
            lits.push(format!("option({}, {o}), match({o}, {v})", q_val(val.get("$option").expect("$option"))));
        } else if k == "regex" {
            lits.push(match_(px, &Lit::Val(val), v));
        } else if k == "spread" && ro.has("$option") {
            continue;
        } else if k == "spread" && matches!(val, Value::Bool(_)) {
            lits.push(if val.is_true() { format!("spread({v})") } else { format!("!spread({v})") });
        } else if k == "capture" && val.is_str() {
            lits.push(format!("{} = {v}", cap(val.as_str().expect("a string"))));
        } else if k == "compare" && vo.is_some_and(|o| o.has("left") && o.has("op") && o.has("right")) {
            lits.extend(compare(vo.expect("an object"), vs));
        } else if k == "count" && vo.is_some() {
            lits.extend(count(px, prog, vo.expect("an object"), v, vs)?);
        } else if k == "layout" && vo.is_some() && main_terms::layout(prog, vo.expect("an object"), v, &mut Vars::default()).is_some() {
            lits.extend(main_terms::layout(prog, vo.expect("an object"), v, vs).expect("layout"));
        } else if k == "nthChild" && vo.is_some() && nth_position(vo.and_then(|o| o.get("position"))).is_some() {
            lits.extend(nth_child(px, prog, vo.expect("an object"), v, vs)?);
        } else if k == "nthChild" && val.is_str() && nth_position(Some(val)).is_some() {
            let c = Obj(vec![("position".to_owned(), val.clone())]);
            lits.extend(nth_child(px, prog, &c, v, vs)?);
        } else if k == "binds" && vo.is_some_and(Obj::is_empty) {
            // `binds: {}`: v is some binding's name node (names.rr's `bind_name`, in every language).
            prog.use_("names");
            lits.push(format!("bind_name({v}, _)"));
        } else if k == "binds" && vo.is_some() {
            // Main's `binds` holds at the binding's name node, and its filters are about the binding's site.
            prog.use_("names");
            let s = vs.fresh("b");
            lits.push(format!("bind_name({v}, {s})"));
            lits.extend(site_filters(px, prog, vo.expect("an object"), &s, vs)?);
        } else if k == "parse" && vo.is_some_and(|o| o.has("grammar")) {
            let o = vo.expect("an object");
            let r = vs.fresh("r");
            lits.push(format!("parsed({v}, {}, {r})", q_val(o.get("grammar").expect("grammar"))));
            lits.extend(lower(px, prog, &Value::Obj(o.without(&["grammar"])), &r, vs, &[])?);
        } else if k == "pattern" && vo.is_some_and(|o| o.has("context")) {
            let o = vo.expect("an object");
            let sel = o.get("selector").map_or(String::new(), |s| format!(" at {}", q_val(s)));
            lits.push(format!("{v} ~ {}{sel}", tick(&py_str(o.get("context").expect("context")))));
        } else if k == "matches" && vo.is_some_and(|o| o.len() == 1) {
            let (name, args) = vo.expect("an object").0[0].clone();
            let rel = prog.util_rel(&name);
            prog.names.insert(rel.clone());
            let args: Vec<String> = args.as_obj().map(|a| a.iter().map(|(_, x)| mv_v(x)).collect()).ok_or("matches arguments are not an object")?;
            lits.push(format!("{rel}({v}, {})", args.join(", ")));
        } else if k == "notRegex" {
            lits.push(format!("!{}", match_(px, &Lit::Val(val), v)));
        } else if k == "pattern" && val.is_str() {
            lits.push(format!("{v} ~ {}", tick(val.as_str().expect("a string"))));
        } else if k == "value" && vo.is_some_and(|o| o.len() > 1 || o.has("type") || o.keys().next().is_some_and(|k| main_terms::VALUE_OPS.contains(&k))) {
            lits.extend(main_terms::value(px, prog, vo.expect("an object"), v, vs)?);
        } else if k == "$option" && val.is_str() && (ro.has("cases") || ro.has("default")) {
            lits.extend(option_cases(px, prog, val, ro, v, false)?);
        } else if (k == "cases" || k == "default") && ro.has("$option") {
            continue;
        } else if k == "stored" && ro.has("regex") {
            // Main's note on why the regex reads a stored token's text: the `regex` beside it is the test.
            continue;
        } else if k == "value" && vo.is_some_and(|o| o.len() == 1) {
            let (op, arg) = vo.expect("an object").0[0].clone();
            match op.as_str() {
                "eq" => {
                    let v2 = v.to_owned();
                    lits.push(opt_alts(&arg, &|c: &Lit| {
                        let lit = match c {
                            Lit::Val(x @ (Value::Int(_) | Value::Float(_))) => q(&py_str(x)),
                            other => other.q(),
                        };
                        format!("value({v2}, {lit})")
                    }));
                }
                "ne" | "lt" | "le" | "gt" | "ge" => {
                    let x = vs.fresh("v");
                    let sym = sym_of(&op);
                    lits.push(format!("value({v}, {x}), {}", opt_alts(&arg, &|c: &Lit| format!("{x} {sym} {}", c.q()))));
                }
                "in" if arg.is_list() => {
                    lits.push(one_of(prog, vs, &format!("value({v}, {{}})"), arg.as_list().expect("a list"), "v", "the JSON `value.in` list"));
                }
                "regex" => {
                    let x = vs.fresh("v");
                    lits.push(format!("value({v}, {x}), {}", match_(px, &Lit::Val(&arg), &x)));
                }
                "startsWith" | "endsWith" | "contains" if arg.is_str() => {
                    let x = vs.fresh("v");
                    let a = arg.as_str().expect("a string");
                    let pat = match op.as_str() {
                        "startsWith" => format!("^{}", py_escape(a)),
                        "endsWith" => format!("{}$", py_escape(a)),
                        _ => py_escape(a),
                    };
                    lits.push(format!("value({v}, {x}), {}", match_str(px, &pat, &x)));
                }
                _ => rest.insert(k, val.clone()),
            }
        } else if matches!(k, "has" | "inside" | "precedes" | "follows") && vo.is_some() {
            lits.extend(relation(px, prog, k, vo.expect("an object"), v, vs)?);
        } else if k == "all" && val.is_list() {
            for s in val.as_list().expect("a list") {
                if is_option(s) && s.get("spread").is_some_and(Value::is_true) {
                    lits.extend(option_cases(px, prog, s.get("$option").expect("$option"), s.as_obj().expect("an object"), v, true)?);
                } else {
                    lits.extend(lower(px, prog, s, v, vs, &[])?);
                }
            }
        } else if k == "any" && val.is_list() {
            lits.extend(alternatives(px, prog, val.as_list().expect("a list"), v, vs)?);
        } else if k == "not" {
            let inner = lower(px, prog, val, v, vs, &[])?;
            lits.extend(negate(px, inner));
            if let Some(fa) = vo.and_then(|o| o.get("formatArgs")).filter(|fa| is_format_args(prog, fa)) {
                lits.extend(format_args_decided(fa.as_obj().expect("an object"), v));
            }
        } else if k == "matches" && val.is_str() {
            let rel = prog.util_rel(val.as_str().expect("a string"));
            prog.names.insert(rel.clone());
            lits.push(format!("{rel}({v})"));
        } else if k == "nthChild" && val.is_int() {
            lits.push(format!("child(_, {v}, {})", val.int().expect("an int") - 1));
        } else if k == "arguments" && vo.is_some() && prog.lang == "python" {
            lits.extend(main_terms::arguments(prog, vo.expect("an object"), v, vs)?);
        } else if k == "resolves" && vo.is_some() {
            lits.extend(resolves(px, prog, vo.expect("an object"), v, vs)?);
        } else if k == "host" && vo.is_some() {
            // inside a `parse`: the host node the parse holds, matched by the host language's rule; a guest capture
            // it reads is the guest node, compared by its text
            let (d, h) = (vs.fresh("d"), vs.fresh("h"));
            lits.push(format!("docof({v}, {d}), guest({d}, _, {h}, _)"));
            lits.extend(lower(px, prog, val, &h, vs, &[])?);
            prog.host_reads.extend(host_reads(px, val));
        } else if k == "formatArgs" && is_format_args(prog, val) {
            lits.extend(format_args(px, prog, val.as_obj().expect("an object"), v, vs)?);
        } else {
            rest.insert(k, val.clone());
        }
    }
    if !rest.is_empty() {
        lits.push(ast_atom(prog, v, Value::Obj(rest)));
    }
    if lits.is_empty() {
        lits.push(format!("node(_, {v}, _, _, _)"));
    }
    Ok(lits)
}

fn compare(c: &Obj, vs: &mut Vars) -> Vec<String> {
    let op = c.get("op").expect("op");
    if !op.is_str() {
        return vec![format!("ast({}, {})", mv_v(c.get("left").expect("left")), compact(&Value::obj(vec![("compare", Value::Obj(c.clone()))])))];
    }
    let (l, r, op) = (mv_v(c.get("left").expect("left")), mv_v(c.get("right").expect("right")), py_str(op));
    let (a, b) = (vs.fresh("t"), vs.fresh("t"));
    let text = c.get("text").cloned().unwrap_or(Value::Str("node".into()));
    let reader = if matches!(text.as_str(), Some("node" | "text")) { "text" } else { "value" };
    if op == "eq" {
        return vec![format!("{reader}({l}, {a}), {reader}({r}, {a})")];
    }
    vec![format!("{reader}({l}, {a}), {reader}({r}, {b}), {a} {} {b}", sym_of(&op))]
}

fn count(px: &Patterns, prog: &mut Program, c: &Obj, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let inner = c.without(&["min", "max", "eq", "as", "offset"]);
    let body = lower(px, prog, &Value::Obj(inner), v, vs, &[])?;
    let k = vs.fresh("k");
    let mut lits = vec![format!("{k} = count : {{ {} }}", body.join(", "))];
    if let Some(Value::Str(as_)) = c.get("as") {
        let off = c.get("offset").cloned().unwrap_or(Value::Int(0));
        lits.push(format!("{} = {k}{}", mv(as_), if off.truthy() { format!(" + {}", py_str(&off)) } else { String::new() }));
        prog.numeric.insert(mv(as_));
    }
    for (key, op) in [("eq", "="), ("min", ">="), ("max", "<=")] {
        let Some(bound) = c.get(key) else { continue };
        if bound.is_number() {
            lits.push(format!("{k} {op} {}", py_str(bound)));
        } else if is_option(bound) {
            let k2 = k.clone();
            lits.push(opt_alts(bound, &|o: &Lit| match o {
                Lit::Opt(s) => format!("{k2} {op} to_number({s})"),
                Lit::Val(x) => format!("{k2} {op} {}", py_str(x)),
            }));
        } else {
            prog.untranslated += 1;
            lits.push(format!("ast({v}, {})", compact(&Value::obj(vec![("count", Value::obj(vec![(key, bound.clone())]))]))));
        }
    }
    Ok(lits)
}

/// A JSON `nthChild` position as the tests on k.
#[derive(Clone, Copy)]
enum Nth {
    Eq(i128),
    Le(i128),
    Ge(i128),
}

impl Nth {
    fn at(self, k: &str) -> Vec<String> {
        vec![match self {
            Nth::Eq(b) => format!("{k} = {b}"),
            Nth::Le(b) => format!("{k} <= {b}"),
            Nth::Ge(b) => format!("{k} >= {b}"),
        }]
    }
}

fn nth_position(pos: Option<&Value>) -> Option<Nth> {
    match pos {
        Some(Value::Int(i)) => Some(Nth::Eq(i - 1)),
        Some(Value::Str(s)) => {
            let c = fullmatch(r"\s*(-?)n\s*\+\s*(\d+)\s*", s)?;
            let b: i128 = g(&c, 2).parse::<i128>().ok()? - 1;
            Some(if !g(&c, 1).is_empty() { Nth::Le(b) } else { Nth::Ge(b.max(0)) })
        }
        _ => None,
    }
}

/// `host_reads`: the guest captures a `host` rule reads (the metavariables it names, `$K` passed to a util, a
/// comparison's sides) that it does not capture itself (a pattern's `$KN`, `capture`).
fn host_reads(px: &Patterns, val: &Value) -> BTreeSet<String> {
    fn walk(px: &Patterns, o: &Value, named: &mut BTreeSet<String>, bound: &mut BTreeSet<String>) {
        match o {
            Value::Obj(ob) => {
                for (k, x) in ob.iter() {
                    match x {
                        Value::Str(s) if (k == "left" || k == "right") && super::pyre::is_full("[A-Z_][A-Z0-9_]*", s) => {
                            named.insert(s.clone());
                        }
                        Value::Str(s) if k == "pattern" => bound.extend(findall(&px.text_mv_ref, s)),
                        Value::Str(s) if k == "capture" => {
                            bound.insert(s.clone());
                        }
                        Value::Str(s) => named.extend(findall(&px.text_mv_ref, s)),
                        _ => {}
                    }
                    walk(px, x, named, bound);
                }
            }
            Value::List(l) => l.iter().for_each(|x| walk(px, x, named, bound)),
            _ => {}
        }
    }
    let (mut named, mut bound) = (BTreeSet::new(), BTreeSet::new());
    walk(px, val, &mut named, &mut bound);
    named.difference(&bound).cloned().collect()
}

/// `is_format_args`: a `formatArgs` term the library python/format-args reads (Python's `brace` or `percent` style).
fn is_format_args(prog: &Program, val: &Value) -> bool {
    let Some(o) = val.as_obj() else { return false };
    prog.lang == "python"
        && matches!(o.get("style").and_then(Value::as_str), Some("brace" | "percent"))
        && o.keys_within(&["style", "string", "args", "unused", "missing"])
        && (o.has("unused") || o.has("missing"))
}

/// `format_args`: an unused argument or a missing field of the string's tree at node v (the call or the `%`
/// expression), matching the side's rule.
fn format_args(px: &Patterns, prog: &mut Program, val: &Obj, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    prog.use_("format-args");
    let style = val.get("style").and_then(Value::as_str).expect("a style").to_owned();
    let mut lits = vec![format!("fa_{style}_tree({v}, _, _)")];
    if let Some(u) = val.get("unused") {
        let a = vs.fresh("a");
        lits.push(format!("fa_{style}_unused({v}, {a})"));
        lits.extend(lower(px, prog, u, &a, vs, &[])?);
    }
    if let Some(m) = val.get("missing") {
        let f = vs.fresh("f");
        lits.push(format!("fa_{style}_missing({v}, {f})"));
        lits.extend(lower(px, prog, m, &f, vs, &[])?);
    }
    Ok(lits)
}

/// `format_args_decided`: beside a negated `formatArgs`, Ruff's verdict is known.
fn format_args_decided(val: &Obj, v: &str) -> Vec<String> {
    let style = val.get("style").and_then(Value::as_str).unwrap_or("");
    let mut lits = vec![format!("!fa_unknown({v})")];
    if val.has("unused") {
        lits.push(format!("!fa_{style}_unused_open({v})"));
    }
    if val.has("missing") {
        lits.push(format!("!fa_{style}_missing_open({v})"));
    }
    lits
}

fn nth_child(px: &Patterns, prog: &mut Program, c: &Obj, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let (p, s, k) = (vs.fresh("p"), vs.fresh("s"), vs.fresh("k"));
    let at = nth_position(c.get("position")).expect("a position").at(&k);
    let order = if truthy(c.get("reverse")) { format!("precedes({v}, {s})") } else { format!("precedes({s}, {v})") };
    if let Some(of) = c.get("ofRule") {
        let rel = prog.helper("counted", "the siblings the JSON `nthChild.ofRule` counts");
        let body = lower(px, prog, of, "n", &mut Vars::default(), &[])?;
        prog.add_rule(format!("{rel}(n)"), body);
        let mut out = vec![format!("child({p}, {v}, _), {rel}({v})"), format!("{k} = count : {{ child({p}, {s}, _), {order}, {rel}({s}) }}")];
        out.extend(at);
        return Ok(out);
    }
    let mut out = vec![format!("child({p}, {v}, _), {k} = count : {{ child({p}, {s}, _), {order} }}")];
    out.extend(at);
    Ok(out)
}

fn site_filters(px: &Patterns, prog: &mut Program, filters: &Obj, s: &str, vs: &mut Vars) -> R<Vec<String>> {
    let mut lits = Vec::new();
    // Main's `directives: "eslint"` is no filter of its own: `builtin` and `exported` also read ESLint's `/* global */`
    // and `/* exported */` comments (the names library's `site_builtin_directives`, `site_exported_directives`).
    let directives = filters.get("directives").is_some_and(|d| d.as_str() == Some("eslint"));
    for (bk, bv) in filters.iter() {
        let bo = bv.as_obj();
        if bk == "directives" && directives {
            continue;
        } else if directives && (bk == "builtin" || bk == "exported") && matches!(bv, Value::Bool(_)) {
            let not = if bv.is_true() { "" } else { "!" };
            lits.push(format!("{not}site_{bk}_directives({s})"));
        } else if bk == "form" && bv.is_str() {
            lits.push(format!("main_form({s}, {})", q_val(bv)));
        } else if bk == "form" && bv.is_list() {
            lits.push(one_of(prog, vs, &format!("main_form({s}, {{}})"), bv.as_list().expect("a list"), "f", "binding forms the JSON accepts"));
        } else if bk == "site" && bo.is_some_and(|o| o.key_set_is(&["kind"])) && bv.get("kind").is_some_and(Value::is_str) {
            let st = vs.fresh("st");
            let kq = q_val(bv.get("kind").expect("kind"));
            lits.push(format!("({st} = {s}, kind({st}, {kq}) ; inside({s}, {st}, {kq}))"));
        } else if bk == "site" {
            let st = vs.fresh("st");
            lits.push(format!("({st} = {s} ; within({s}, {st}))"));
            lits.extend(lower(px, prog, bv, &st, vs, &[])?);
        } else if bk == "value" {
            let x = vs.fresh("x");
            lits.push(format!("binding_value({s}, {x})"));
            lits.extend(lower(px, prog, bv, &x, vs, &[])?);
        } else if (bk == "occurrence" || bk == "annotation") && bo.is_some() {
            let x = vs.fresh("o");
            lits.push(format!("site_{bk}({s}, {x})"));
            lits.extend(lower(px, prog, bv, &x, vs, &[])?);
        } else if bv.as_list().is_some_and(|l| l.iter().all(Value::is_str)) {
            lits.push(one_of(prog, vs, &format!("site_{}({s}, {{}})", snake(bk)), bv.as_list().expect("a list"), "f", &format!("the JSON `binds.{bk}` list")));
        } else if bv.is_true() {
            lits.push(format!("site_{}({s})", snake(bk)));
        } else if bv.is_false() {
            lits.push(format!("!site_{}({s})", snake(bk)));
        } else if matches!(bv, Value::Str(_) | Value::Int(_) | Value::Float(_)) {
            lits.push(format!("site_{}({s}, {})", snake(bk), q_val(bv)));
        } else if bk == "bindings" && bo.is_some_and(|o| o.keys_within(&["min", "max", "within"])) && ["min", "max"].iter().all(|x| bv.get(x).is_none_or(Value::is_int)) {
            let (sc, x, s2, k) = (vs.fresh("sc"), vs.fresh("x"), vs.fresh("s"), vs.fresh("k"));
            let mut inner = format!("binding({sc}, {x}, {s2}, _)");
            if let Some(w) = bv.get("within") {
                let w = mv_v(w);
                inner += &format!(", ({s2} = {w} ; within({s2}, {w}))");
            }
            lits.push(format!("binding({sc}, {x}, {s}, _)"));
            lits.push(format!("{k} = count : {{ {inner} }}"));
            if let Some(m) = bv.get("min") {
                lits.push(format!("{k} >= {}", py_str(m)));
            }
            if let Some(m) = bv.get("max") {
                lits.push(format!("{k} <= {}", py_str(m)));
            }
        } else if bk == "redefines" && bo.is_some_and(|o| o.keys_within(&["form"])) {
            let (sc, x, s2) = (vs.fresh("sc"), vs.fresh("x"), vs.fresh("s"));
            let form = bv.get("form");
            lits.push(format!("binding({sc}, {x}, {s}, _)"));
            match form {
                Some(f @ Value::Str(_)) => lits.push(format!("binding({sc}, {x}, {s2}, _), main_form({s2}, {})", q_val(f))),
                Some(Value::List(fs)) => {
                    let f = vs.fresh("f");
                    lits.push(format!("binding({sc}, {x}, {s2}, _), main_form({s2}, {f})"));
                    lits.push(format!("({})", fs.iter().map(|x_| format!("{f} = {}", q_val(x_))).collect::<Vec<_>>().join(" ; ")));
                }
                _ => lits.push(format!("binding({sc}, {x}, {s2}, _)")),
            }
            lits.push(format!("{s2} < {s}"));
        } else if bk == "redefines"
            && let Some(o) = bo.filter(|o| o.keys_within(&["every", "site", "form", "scope", "used", "referenced", "exported", "builtin", "directives", "value", "annotation"]))
        {
            // The variable's other definitions: some earlier one the term accepts (its filters and sub-rules on it),
            // or with `every`, each other one (main's TypeScript `redefines`; vacuous where there is no other).
            let every = o.get("every").is_some_and(Value::is_true);
            let accepts = prog.helper("redefined", "the definitions the JSON `binds.redefines` accepts");
            let mut body = vec!["binding(_, _, n, _)".to_owned()];
            body.extend(site_filters(px, prog, &o.without(&["every"]), "n", &mut Vars::default())?);
            prog.add_rule(format!("{accepts}(n)"), body);
            let (sc, x, s2) = (vs.fresh("sc"), vs.fresh("x"), vs.fresh("s"));
            lits.push(format!("binding({sc}, {x}, {s}, _)"));
            if every {
                lits.push(format!("count : {{ binding({sc}, {x}, {s2}, _), {s2} != {s}, !{accepts}({s2}) }} = 0"));
            } else {
                lits.push(format!("binding({sc}, {x}, {s2}, _), {s2} < {s}, {accepts}({s2})"));
            }
        } else {
            let term = Value::obj(vec![("binds", Value::obj(vec![(bk, bv.clone())]))]);
            lits.push(ast_atom(prog, s, term));
        }
    }
    Ok(lits)
}

/// `$option`: the configured value picks a case, else the default.
fn option_cases(px: &Patterns, prog: &mut Program, name: &Value, rule: &Obj, v: &str, conj: bool) -> R<Vec<String>> {
    let rel = prog.helper("option", &format!("the JSON `$option` on {}: one rule per configured value, plus the default", q_val(name)));
    let qn = q_val(name);
    let cases = cases_of(&Value::Obj(rule.clone()));
    let guard: Vec<String> = cases.iter().map(|(k, _)| format!("!option({qn}, {})", q(k))).collect();
    let mut choices: Vec<(Vec<String>, Value)> = cases.iter().map(|(c, b)| (vec![format!("option({qn}, {})", q(c))], b.clone())).collect();
    if let Some(d) = rule.get("default") {
        choices.push((guard, d.clone()));
    }
    let head = format!("{rel}(n)");
    for (when, body) in choices {
        if let (Value::List(bs), true) = (&body, conj) {
            let mut lits = Vec::new();
            for b in bs {
                lits.extend(lower(px, prog, b, "n", &mut Vars::default(), &[])?);
            }
            if lits.is_empty() {
                lits.push("node(_, n, _, _, _)".into());
            }
            let mut all = when.clone();
            all.extend(lits);
            prog.add_rule(head.clone(), all);
        } else {
            let bs: Vec<Value> = if let Value::List(bs) = &body { bs.clone() } else { vec![body.clone()] };
            for b in bs {
                let lits = lower(px, prog, &b, "n", &mut Vars::default(), &[])?;
                let mut all = when.clone();
                all.extend(lits);
                prog.add_rule(head.clone(), all);
            }
        }
    }
    let any = prog.rules.iter().any(|r| matches!(r, super::program::PRule::Rule(id) if prog.arena[*id].head == head));
    if !any {
        prog.decls.shift_remove(&rel);
        return Ok(vec!["false".into()]);
    }
    Ok(vec![format!("{rel}({v})")])
}

fn is_kind_only(a: &Value) -> bool {
    a.as_obj().is_some_and(|o| o.key_set_is(&["kind"]))
}

fn is_value_eq(a: &Value) -> bool {
    a.as_obj().is_some_and(|o| o.key_set_is(&["value"])) && a.get("value").and_then(Value::as_obj).is_some_and(|o| o.key_set_is(&["eq"]))
}

/// One body per alternative, with plain kind and value alternatives folded into one literal each.
pub fn grouped(px: &Patterns, prog: &mut Program, alts: &[Value], v: &str, vs: &mut Vars) -> R<Vec<Vec<String>>> {
    let kinds: Vec<Value> = alts.iter().filter(|a| is_kind_only(a) && a.get("kind").is_some_and(Value::is_str)).map(|a| a.get("kind").expect("kind").clone()).collect();
    let vals: Vec<Value> = alts.iter().filter(|a| is_value_eq(a)).map(|a| a.get("value").and_then(|x| x.get("eq")).expect("eq").clone()).collect();
    let mut bodies = Vec::new();
    if kinds.len() > 1 {
        bodies.push(vec![one_of(prog, vs, &format!("kind({v}, {{}})"), &kinds, "k", "node kinds from a JSON `any`")]);
    }
    if vals.len() > 1 {
        bodies.push(vec![one_of(prog, vs, &format!("value({v}, {{}})"), &vals, "v", "values from a JSON `any`")]);
    }
    for a in alts {
        let folded = (kinds.len() > 1 && is_kind_only(a) && a.get("kind").is_some_and(Value::is_str)) || (vals.len() > 1 && is_value_eq(a));
        if !folded {
            bodies.push(lower(px, prog, a, v, vs, &[])?);
        }
    }
    Ok(bodies)
}

fn alternatives(px: &Patterns, prog: &mut Program, alts: &[Value], v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let mut bodies = grouped(px, prog, alts, v, vs)?;
    if bodies.len() == 1 {
        return Ok(bodies.remove(0));
    }
    let name = prog.helper("alt", "the JSON `any`: n matches if any one of these rules does");
    for body in grouped(px, prog, alts, "n", &mut Vars::default())? {
        prog.add_rule(format!("{name}(n)"), body);
    }
    Ok(vec![format!("{name}({v})")])
}

/// The stop argument of within_stop / precedes_stop: one kind, a relation of kinds, or a helper relation.
fn stop_set(px: &Patterns, prog: &mut Program, stop: &Value) -> R<String> {
    if is_kind_only(stop) {
        return Ok(q_val(stop.get("kind").expect("kind")));
    }
    let any = stop.as_obj().filter(|o| o.key_set_is(&["any"])).and_then(|o| o.get("any"));
    if let Some(Value::List(alts)) = any
        && alts.iter().all(is_kind_only)
    {
        let kinds = alts.iter().map(|a| a.get("kind").expect("kind").clone()).collect();
        return Ok(prog.facts("stop", "kind", kinds, "the kinds of the JSON `stopBy` rule: the tree search stops at these nodes"));
    }
    let name = prog.helper("stop", "the JSON `stopBy` rule: the tree search stops at these nodes");
    if let Some(any) = any {
        let alts = any.as_list().cloned().unwrap_or_default();
        for body in grouped(px, prog, &alts, "n", &mut Vars::default())? {
            prog.add_rule(format!("{name}(n)"), body);
        }
    } else {
        let mut body = lower(px, prog, stop, "n", &mut Vars::default(), &[])?;
        // A rule of negations only (`stopBy: {not: …}`) ranges over the nodes.
        let stmt = format!("{name}(n) :- {}.", body.join(", "));
        // n named by a positive literal, outside any aggregate's braces.
        let names_n = |l: &String| {
            let (mut depth, mut out) = (0i32, String::new());
            for c in l.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ if depth == 0 => out.push(c),
                    _ => {}
                }
            }
            !l.trim_start().starts_with('!') && fullmatch(r"(?s).*(?<![\w$])n\b.*", &out).is_some()
        };
        if !body.iter().any(names_n) || super::souffle::check_rule(px, &stmt, &BTreeSet::new(), &[]).is_ok_and(|b| b.iter().any(|x| x == "n")) {
            body.insert(0, "node(_, n, _, _, _)".to_owned());
        }
        prog.add_rule(format!("{name}(n)"), body);
    }
    Ok(name)
}

const OP_CHARS: &str = "-+*/%@&|^<>=!:.~";

/// x's top-level alternatives; one group around the whole is opened.
fn top_alts(x: &str) -> Vec<String> {
    fn split(x: &[char]) -> Vec<String> {
        let (mut out, mut depth, mut cur, mut i, mut cls) = (Vec::new(), 0i32, String::new(), 0usize, false);
        while i < x.len() {
            let ch = x[i];
            if ch == '\\' {
                cur.extend(x[i..(i + 2).min(x.len())].iter());
                i += 2;
                continue;
            }
            if cls {
                cls = ch != ']';
            } else if ch == '[' {
                cls = true;
            } else if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
            } else if ch == '|' && depth == 0 {
                out.push(std::mem::take(&mut cur));
                i += 1;
                continue;
            }
            cur.push(ch);
            i += 1;
        }
        out.push(cur);
        out
    }
    let xc: Vec<char> = x.chars().collect();
    if xc.len() >= 2 && xc[0] == '(' && xc[xc.len() - 1] == ')' && split(&xc).len() == 1 {
        let start = if xc.len() >= 4 && xc[1] == '?' && xc[2] == ':' { 3 } else { 1 };
        let inner: Vec<char> = xc[start..xc.len() - 1].to_vec();
        let (mut depth, mut ok) = (0i32, true);
        for (i, &ch) in inner.iter().enumerate() {
            if ch == '(' && (i == 0 || inner[i - 1] != '\\') {
                depth += 1;
            } else if ch == ')' && (i == 0 || inner[i - 1] != '\\') {
                depth -= 1;
                if depth < 0 {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return split(&inner);
        }
    }
    split(&xc)
}

fn class_of(body: &str) -> &'static str {
    if !body.is_empty() && body.chars().all(|c| OP_CHARS.contains(c)) {
        "o"
    } else if !body.is_empty() && body.chars().all(char::is_alphabetic) {
        "w"
    } else {
        ""
    }
}

/// `in OP_CHARS` of a one-character (or empty) string: "" is in every string.
fn in_ops(s: &str) -> bool {
    s.is_empty() || (s.chars().count() == 1 && OP_CHARS.contains(s))
}

/// What kind of character starts (first) or ends alt: "w" a word character, "o" an operator character, "" other.
fn edge_class(alt: &str, first: bool) -> &'static str {
    let a: Vec<char> = alt.chars().collect();
    if a.is_empty() {
        return "";
    }
    if first {
        if a[0] == '\\' {
            let ch: String = a.get(1).map(|c| c.to_string()).unwrap_or_default();
            return if !ch.is_empty() && ch.chars().all(char::is_alphanumeric) { "" } else if in_ops(&ch) { "o" } else { "" };
        }
        if a[0] == '[' {
            // alt[1:alt.find("]", 2)]
            let end = (2..a.len()).find(|&k| a[k] == ']').map_or(a.len().saturating_sub(1) as isize, |k| k as isize);
            let body: String = if (end as usize) > 1 { a[1..end as usize].iter().filter(|&&c| c != '\\').collect() } else { String::new() };
            return class_of(&body);
        }
        return if a[0].is_alphabetic() || a[0] == '_' { "w" } else if OP_CHARS.contains(a[0]) { "o" } else { "" };
    }
    let mut a = a;
    while a.last() == Some(&'?') {
        a.pop();
    }
    if a.last() == Some(&']') {
        let open = a.iter().rposition(|&c| c == '[').map_or(0, |k| k + 1);
        let body: String = if a.len() - 1 > open { a[open..a.len() - 1].iter().filter(|&&c| c != '\\').collect() } else { String::new() };
        return class_of(&body);
    }
    if a.len() >= 2 && a[a.len() - 2] == '\\' {
        let ch = a[a.len() - 1];
        return if ch.is_alphanumeric() { "" } else if OP_CHARS.contains(ch) { "o" } else { "" };
    }
    let last: String = a.last().map(|c| c.to_string()).unwrap_or_default();
    if !last.is_empty() && last.chars().all(char::is_alphabetic) {
        "w"
    } else if in_ops(&last) {
        "o"
    } else {
        ""
    }
}

/// A regex over the source between two stored children that holds where a token in it matches rx.
fn token_gap_regex(rx: &str, where_: &str) -> String {
    let mut flags = String::new();
    let mut rx = rx.to_owned();
    if let Some(c) = super::pyre::find(&re(r"^\(\?([a-zA-Z]+)\)"), &rx.clone()) {
        flags = g(&c, 1).to_owned();
        rx = rx[c.get(0).expect("group 0").end()..].to_owned();
    }
    let wrap = |a: &str| if flags.is_empty() { format!("(?:{a})") } else { format!("(?{flags}:{a})") };
    let anchored = rx.starts_with('^') && rx.ends_with('$') && !rx.ends_with("\\$") && rx.chars().count() > 1;
    let body = if anchored { super::text::chars_slice(&rx, 1, 1) } else { rx.clone() };
    if !anchored && matches!(body.as_str(), "" | "^" | "." | "$") {
        return match where_ {
            "any" => r"(?s)^.*\S.*$",
            "first" => r"(?s)^\s*\S.*$",
            _ => r"(?s)^.*\S\s*$",
        }
        .to_owned();
    }
    let (alts, classes): (Vec<String>, Vec<(&str, &str)>) = if !anchored {
        (vec![format!(r"\S*?{}\S*", wrap(&body))], vec![("", "")])
    } else {
        let alts = top_alts(&body);
        let classes = alts.iter().map(|a| (edge_class(a, true), edge_class(a, false))).collect();
        (alts.iter().map(|a| wrap(a)).collect(), classes)
    };
    let left = |c: &str| match c {
        "w" => r"\W",
        "o" => r"[^\-+*/%@\&|\^<>=!:.\~]",
        _ => "",
    };
    let pieces: Vec<String> = alts
        .iter()
        .zip(classes)
        .map(|(a, (lc, rc))| {
            let lpre = if where_ == "first" { r"\s*".to_owned() } else if !left(lc).is_empty() { format!("(?:.*{})?", left(lc)) } else { ".*".to_owned() };
            let rpost = if where_ == "last" { r"\s*".to_owned() } else if !left(rc).is_empty() { format!("(?:{}.*)?", left(rc)) } else { ".*".to_owned() };
            format!("{lpre}{a}{rpost}")
        })
        .collect();
    format!("(?s)^(?:{})$", pieces.join("|"))
}

/// A neighbour step to an anonymous token that steps on, the same way, to its neighbour
/// (`precedes: {stopBy: neighbor, regex: "^==$", precedes: {..}}`, E711's `x == None`): read as a step from v to its
/// named neighbour w with the token between them. A token the tape leaves out is the whole source gap between v and
/// w (blanks around one token the anchored regex matches); one it stores starts where v's blank gap ends and ends
/// where w's begins. The inner step's tests are lowered at w. None when the step is not of this shape.
fn token_chain(px: &Patterns, prog: &mut Program, k: &str, sub_: &Obj, v: &str, vs: &mut Vars) -> R<Option<Vec<String>>> {
    let neighbor = |o: &Obj| o.get("stopBy").is_none_or(|s| s.as_str() == Some("neighbor"));
    let shape = sub_.without(&["stopBy"]).key_set_is(&["regex", k]);
    let (Some(rx_full), Some(inner)) = (sub_.get("regex").and_then(Value::as_str), sub_.get(k).and_then(Value::as_obj)) else {
        return Ok(None);
    };
    if !neighbor(sub_) || !shape || !neighbor(inner) || inner.has("field") {
        return Ok(None);
    }
    let mut flags = String::new();
    let mut rx = rx_full.to_owned();
    if let Some(c) = super::pyre::find(&re(r"^\(\?([a-zA-Z]+)\)"), &rx.clone()) {
        flags = g(&c, 1).to_owned();
        rx = rx[c.get(0).expect("group 0").end()..].to_owned();
    }
    if !(rx.starts_with('^') && rx.ends_with('$') && !rx.ends_with("\\$") && rx.chars().count() > 2) {
        return Ok(None);
    }
    let inside = super::text::chars_slice(&rx, 1, 1);
    let body = if flags.is_empty() { format!("(?:{inside})") } else { format!("(?{flags}:{inside})") };
    let [w, gv, g2, t, p_, o, e, s, x] = ["s", "g", "g", "t", "p", "o", "e", "st", "en"].map(|n| vs.fresh(n));
    let (side, back) = if k == "precedes" { ("after", "before") } else { ("before", "after") };
    let one = format!(r"(?s)^\s*{body}\s*$");
    let sib = if k == "precedes" { format!("next_sibling({v}, {w})") } else { format!("next_sibling({w}, {v})") };
    let gap = [sib.clone(), format!("structural_{side}({v}, {gv})"), format!("match({}, {gv})", q(&one))];
    let at = if k == "precedes" {
        [
            format!("node(_, {v}, _, _, {x})"),
            format!("{o} = {x} + strlen({gv})"),
            format!("token({p_}, _, {t}, {o}, {e})"),
            format!("node(_, {w}, _, {s}, _)"),
            format!("{s} = {e} + strlen({g2})"),
        ]
    } else {
        [
            format!("node(_, {v}, _, {s}, _)"),
            format!("{o} = {s} - strlen({gv})"),
            format!("token({p_}, _, {t}, {e}, {o})"),
            format!("node(_, {w}, _, _, {x})"),
            format!("{x} = {e} - strlen({g2})"),
        ]
    };
    let mut stored = vec![
        sib,
        format!("structural_{side}({v}, {gv})"),
        format!("match(\"\\\\s*\", {gv})"),
        format!("child({p_}, {v}, _)"),
        format!("structural_{back}({w}, {g2})"),
        format!("match(\"\\\\s*\", {g2})"),
    ];
    stored.extend(at);
    stored.push(format!("match({}, {t})", q(&whole(px, &caps(rx_full)))));
    // one relation of (v, w), a rule per form, so its stored-token read carries its reason (`.stored`)
    let (rv, rw) = (re(&format!(r"(?<![\w$]){}\b", esc(v))), re(&format!(r"(?<![\w$]){}\b", esc(&w))));
    let ren = |ls: &[String]| -> Vec<String> {
        ls.iter().map(|l| sub(&rw, &sub(&rv, l, |_| "n".to_owned()), |_| "m".to_owned())).collect()
    };
    let rules = [ren(&gap), ren(&stored)];
    let sig = format!("(n, m)\u{2}{}", rules.iter().map(|r| r.join("\u{1}")).collect::<Vec<_>>().join("\u{2}"));
    let name = match prog.token_rels.iter().find(|(s, _)| *s == sig) {
        Some((_, n)) => n.clone(),
        None => {
            let name = format!("tok_{k}_{}", prog.token_rels.len() + 1);
            prog.token_rels.push((sig, name.clone()));
            let what = if k == "precedes" { "the named node after n" } else { "the named node before n" };
            prog.lib.insert(
                name.clone(),
                (
                    format!(".decl {name}(n: node, m: node)\n.stored {name} \"its token read is of a token the tape stores between two named siblings; one the tape leaves out is the source gap, in its own rule\""),
                    vec![format!(
                        "// {name}(n, m): m is {what}, with one token between them that matches {}: a token the tape stores, or one it leaves out, read from the source between the two",
                        dumps(&Value::Str(rx_full.to_owned()), true, ", ", ": ", false)
                    )],
                    rules.into_iter().map(|b| (format!("{name}(n, m)"), b)).collect(),
                ),
            );
            name
        }
    };
    let mut lits = vec![format!("{name}({v}, {w})")];
    lits.extend(lower(px, prog, &Value::Obj(inner.clone()), &w, vs, &["field", "stopBy"])?);
    Ok(Some(lits))
}

fn relation(px: &Patterns, prog: &mut Program, k: &str, sub_: &Obj, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let f = sub_.get("field").cloned();
    let f_truthy = truthy(f.as_ref());
    let stop = sub_.get("stopBy").cloned().unwrap_or(Value::Str("neighbor".into()));
    let is = |s: &str| stop.as_str() == Some(s);
    let c = vs.fresh(match k {
        "has" => "c",
        "inside" => "p",
        _ => "s",
    });
    let mut lits = Vec::new();
    let mut skip: Vec<&str> = vec!["field", "stopBy"];
    let qf = || q_val(f.as_ref().unwrap_or(&Value::Null));
    if k == "has" || k == "inside" {
        let (lo, hi) = if k == "has" { (c.clone(), v.to_owned()) } else { (v.to_owned(), c.clone()) };
        if is("neighbor") {
            lits.push(if f_truthy { format!("field({hi}, {}, {lo})", qf()) } else { format!("child({hi}, {lo}, _)") });
        } else if is("end") && sub_.get("kind").is_some_and(Value::is_str) {
            lits.push(format!("{k}({v}, {c}, {})", q_val(sub_.get("kind").expect("kind"))));
            skip.push("kind");
        } else if is("end") {
            lits.push(format!("within({lo}, {hi})"));
        } else {
            lits.push(format!("within_stop({lo}, {hi}, {})", stop_set(px, prog, &stop)?));
        }
        if f_truthy && !is("neighbor") {
            lits.push(format!("field(_, {}, {lo})", qf()));
        }
    } else {
        if let Some(chain) = token_chain(px, prog, k, sub_, v, vs)? {
            return Ok(chain);
        }
        let (a, b) = if k == "precedes" { (v.to_owned(), c.clone()) } else { (c.clone(), v.to_owned()) };
        if is("neighbor") {
            lits.push(format!("next_sibling({a}, {b})"));
        } else if is("end") {
            lits.push(format!("precedes({a}, {b})"));
        } else {
            lits.push(format!("precedes_stop({a}, {b}, {})", stop_set(px, prog, &stop)?));
        }
    }
    lits.extend(lower(px, prog, &Value::Obj(sub_.clone()), &c, vs, &skip)?);
    let only_regex = sub_.keys().filter(|k| *k != "field" && *k != "stopBy").collect::<Vec<_>>() == vec!["regex"];
    if only_regex && !f_truthy && (is("neighbor") || is("end")) && !((k == "has" || k == "inside") && is("end")) && k != "inside" {
        let (t, gv) = (vs.fresh("t"), vs.fresh("g"));
        let rx = sub_.get("regex").expect("regex").clone();
        let rx_str = rx.as_str().map(str::to_owned);
        let tok_rx = rx_str.as_ref().map(|r| q(&whole(px, &caps(r))));
        let stored_tok = |prog: &mut Program, vs: &mut Vars, var: &str| -> R<Vec<String>> {
            match &tok_rx {
                Some(t) => Ok(vec![format!("match({t}, {var})")]),
                None => lower(px, prog, &Value::obj(vec![("regex", rx.clone())]), var, vs, &[]),
            }
        };
        let gap = |where_: &str, var: &str| -> Vec<String> {
            match &rx_str {
                Some(r) => vec![format!("match({}, {var})", q(&token_gap_regex(r, where_)))],
                None => Vec::new(),
            }
        };
        let (p_, o, st, en) = (vs.fresh("p"), vs.fresh("o"), vs.fresh("st"), vs.fresh("en"));
        let mut toks: Vec<Vec<String>> = Vec::new();
        if k == "has" {
            let mut t0 = vec![format!("op({v}, {t})")];
            t0.extend(stored_tok(prog, vs, &t)?);
            toks.push(t0);
            if rx_str.is_some() {
                let c2 = vs.fresh("c");
                let mut a = vec![format!("child({v}, {c2}, _)"), format!("structural_before({c2}, {gv})")];
                a.extend(gap("any", &gv));
                toks.push(a);
                let mut b = vec![format!("child({v}, {c2}, _)"), format!("structural_after({c2}, {gv})")];
                b.extend(gap("any", &gv));
                toks.push(b);
                let mut c3 = vec![format!("node_text({v}, {gv})"), format!("!child({v}, _, _)")];
                c3.extend(gap("any", &gv));
                toks.push(c3);
            }
        } else if is("neighbor") {
            let (side, where_) = if k == "precedes" { ("after", "first") } else { ("before", "last") };
            let at = if k == "precedes" {
                vec![format!("node(_, {v}, _, _, {en})"), format!("{o} = {en} + strlen({gv})"), format!("token({p_}, _, {t}, {o}, _)")]
            } else {
                vec![format!("node(_, {v}, _, {st}, _)"), format!("{o} = {st} - strlen({gv})"), format!("token({p_}, _, {t}, _, {o})")]
            };
            let mut a = vec![format!("structural_{side}({v}, {gv})"), format!("match(\"\\\\s*\", {gv})"), format!("child({p_}, {v}, _)")];
            a.extend(at);
            a.extend(stored_tok(prog, vs, &t)?);
            toks.push(a);
            if rx_str.is_some() {
                let mut b = vec![format!("structural_{side}({v}, {gv})")];
                b.extend(gap(where_, &gv));
                toks.push(b);
            }
        } else {
            let side = if k == "precedes" { "after" } else { "before" };
            let (at, sib) = if k == "precedes" {
                (vec![format!("node(_, {v}, _, _, {en})"), format!("token({p_}, _, {t}, {st}, _)"), format!("{st} >= {en}")], format!("precedes({v}, {o})"))
            } else {
                (vec![format!("node(_, {v}, _, {st}, _)"), format!("token({p_}, _, {t}, _, {en})"), format!("{en} <= {st}")], format!("precedes({o}, {v})"))
            };
            let mut a = vec![format!("child({p_}, {v}, _)")];
            a.extend(at);
            a.extend(stored_tok(prog, vs, &t)?);
            toks.push(a);
            if rx_str.is_some() {
                let mut b = vec![format!("structural_{side}({v}, {gv})")];
                b.extend(gap("any", &gv));
                toks.push(b);
                let mut c3 = vec![sib, format!("structural_{side}({o}, {gv})")];
                c3.extend(gap("any", &gv));
                toks.push(c3);
            }
        }
        let ren_re = re(&format!(r"(?<![\w$]){}\b", esc(v)));
        let ren = |ls: &[String]| -> Vec<String> { ls.iter().map(|l| sub(&ren_re, l, |_| "n".to_owned())).collect() };
        let mut rules: Vec<Vec<String>> = vec![ren(&lits)];
        rules.extend(toks.iter().map(|tk| ren(tk)));
        let sig = rules.iter().map(|r| r.join("\u{1}")).collect::<Vec<_>>().join("\u{2}");
        let name = match prog.token_rels.iter().find(|(s, _)| *s == sig) {
            Some((_, n)) => n.clone(),
            None => {
                let name = format!("tok_{k}_{}", prog.token_rels.len() + 1);
                prog.token_rels.push((sig, name.clone()));
                let what = match k {
                    "has" => "a child",
                    "precedes" => "a later sibling",
                    _ => "an earlier sibling",
                };
                prog.lib.insert(
                    name.clone(),
                    (
                        format!(".decl {name}(n: node)\n.stored {name} \"its token reads are of tokens the tape stores; one the tape leaves out is read from the source gap, in its own rule\""),
                        vec![format!(
                            "// {name}(n): {what} of n, a named node or an anonymous token, matches {}: a named node, a token the tape stores, or one it leaves out, read from the source between stored children",
                            dumps(&rx, true, ", ", ": ", false)
                        )],
                        rules.into_iter().map(|b| (format!("{name}(n)"), b)).collect(),
                    ),
                );
                name
            }
        };
        lits = vec![format!("{name}({v})")];
    }
    Ok(lits)
}

fn resolves(px: &Patterns, prog: &mut Program, r: &Obj, v: &str, vs: &mut Vars) -> R<Vec<String>> {
    let mut lits = Vec::new();
    let mut rest = Obj::default();
    if r.get("seenBefore").is_some_and(Value::is_true)
        && let Some(Value::List(imported)) = r.get("imported")
    {
        return Ok(main_terms::seen_before(prog, imported, v, vs));
    }
    if r.get("relative").and_then(Value::as_str) == Some("text")
        && let Some(Value::List(imported)) = r.get("imported")
    {
        return Ok(main_terms::statement_imported(prog, imported, v, vs, truthy(r.get("prefix"))));
    }
    for (k, val) in r.iter() {
        if k == "import" && val.is_true() {
            prog.use_("names");
            lits.push(format!("sym_of({v}, _)"));
        } else if k == "imported" && val.is_list() {
            prog.use_("names");
            lits.push(one_of(prog, vs, &format!("imported_sym({v}, {{}})"), val.as_list().expect("a list"), "q", "symbols from the JSON `resolves.imported`"));
        } else if k == "import" && val.is_list() {
            prog.use_("names");
            if truthy(r.get("prefix")) {
                let x = vs.fresh("q");
                let alt = val.as_list().expect("a list").iter().map(|s| py_escape(&py_str(s))).collect::<Vec<_>>().join("|");
                lits.push(format!("sym_of({v}, {x}), {}", match_str(px, &format!(r"^({alt})(\.|$)"), &x)));
            } else {
                lits.push(one_of(prog, vs, &format!("sym_of({v}, {{}})"), val.as_list().expect("a list"), "q", "symbols from the JSON `resolves.import`"));
            }
        } else if k == "prefix" {
            continue;
        } else if (k == "directives" || k == "scriptDeclarations") && r.has("global") {
            // They widen `global` (below): no filter of their own.
            continue;
        } else if k == "directives" && val.is_str() {
            lits.push(format!("global_directives({v}, {})", q_val(val)));
        } else if k == "global" {
            prog.use_("names");
            // Main's `directives: "eslint"` adds `/* global */` comments' names (less those turned off), and
            // `scriptDeclarations` a script's top-level declarations, which are the global variables themselves.
            let dir = r.get("directives").and_then(Value::as_str) == Some("eslint");
            let script = r.get("scriptDeclarations").is_some_and(Value::is_true);
            let rel = |g: &str| if dir || script { format!("global_ref_with({v}, {g}, {}, {})", dir as u8, script as u8) } else { format!("global_ref({v}, {g})") };
            if let Value::List(l) = val {
                lits.push(one_of(prog, vs, &rel("{}"), l, "g", "names from the JSON `resolves.global`"));
            } else {
                lits.push(rel("_"));
            }
        } else if k == "sloppyFunctions" && r.get("this").and_then(Value::as_str) == Some("global") {
            continue;
        } else if k == "this" && val.as_str() == Some("global") {
            // `this` is the global object: a script's top level, and with `sloppyFunctions` a sloppy function
            // called plainly (tsx/eslint-a-globals).
            prog.use_("eslint-a-globals");
            let sloppy = r.get("sloppyFunctions").is_some_and(Value::is_true);
            lits.push(if sloppy { format!("global_this({v})") } else { format!("global_this_top({v})") });
        } else if k == "unresolved" && (matches!(val, Value::Bool(true) | Value::Int(1)) || matches!(val, Value::Float(f) if *f == 1.0) || val.as_str() == Some("star")) {
            prog.use_("names");
            if prog.lang == "python" {
                lits.extend(main_terms::unresolved(prog, val, v, vs));
            } else {
                // Another language's names library says it: `unresolved_ref(r)`, r resolves to nothing.
                lits.push(format!("unresolved_ref({v})"));
            }
        } else if k == "binding" && val.is_obj() {
            prog.use_("names");
            let s = vs.fresh("b");
            lits.push(format!("ref_site({v}, {s})"));
            lits.extend(site_filters(px, prog, val.as_obj().expect("an object"), &s, vs)?);
        } else {
            rest.insert(k, val.clone());
        }
    }
    if !rest.is_empty() {
        lits.push(ast_atom(prog, v, Value::obj(vec![("resolves", Value::Obj(rest))])));
    }
    Ok(lits)
}

/// A util as rules: `out` gets (head, body) per rule.
pub fn render_util(px: &Patterns, prog: &mut Program, name: &str, rule: &Value, out: &mut Vec<(String, Vec<String>)>) -> R<()> {
    let rel = prog.util_rel(name);
    prog.util_rels.insert(rel.clone());
    let params = rule.get("params").filter(|p| p.truthy());
    if let Some(params) = params {
        let params: Vec<String> = params.as_list().ok_or("util params are not a list")?.iter().map(py_str).collect();
        let ro = rule.as_obj().expect("an object");
        let body_rule = ro.get("rule").cloned().unwrap_or_else(|| Value::Obj(ro.without(&["params"])));
        let args = params.join(", ");
        prog.decls.insert(rel.clone(), format!(".decl {rel}(n: node, {})", params.iter().map(|p| format!("{}: node", col(p))).collect::<Vec<_>>().join(", ")));
        prog.notes.insert(rel.clone(), vec![format!("// {rel}(n, …): the JSON util `{name}`, called with `matches` and arguments {args}")]);
        let body = lower(px, prog, &body_rule, "n", &mut Vars::default(), &[])?;
        out.push((format!("{rel}(n, {args})"), body));
        return Ok(());
    }
    prog.decls.insert(rel.clone(), format!(".decl {rel}(n: node)"));
    prog.notes.insert(rel.clone(), vec![format!("// {rel}(n): the JSON util `{name}`, used where the JSON says `matches: {}`", q(name))]);
    if let Some(Value::List(alts)) = rule.as_obj().filter(|o| o.key_set_is(&["any"])).and_then(|o| o.get("any")) {
        for body in grouped(px, prog, alts, "n", &mut Vars::default())? {
            out.push((format!("{rel}(n)"), body));
        }
    } else {
        let body = lower(px, prog, rule, "n", &mut Vars::default(), &[])?;
        out.push((format!("{rel}(n)"), body));
    }
    Ok(())
}

/// Metavariables a JSON `transform` defines, through `$option` cases too.
pub fn transform_targets(tr: &Value) -> BTreeSet<String> {
    let Some(o) = tr.as_obj() else { return BTreeSet::new() };
    if o.has("$option") {
        let mut out = BTreeSet::new();
        for (_, s) in cases_of(tr) {
            out.extend(transform_targets(&s));
        }
        out.extend(transform_targets(o.get("default").unwrap_or(&Value::Obj(Obj::default()))));
        return out;
    }
    o.keys().filter(|t| !t.starts_with('$')).map(str::to_owned).collect()
}

/// Python's `lits[start:]` for an int start.
fn tail_from(lits: &[String], start: i128) -> Vec<String> {
    let n = lits.len() as i128;
    let s = if start < 0 { (n + start).max(0) } else { start.min(n) };
    lits[s as usize..].to_vec()
}

/// Body literals for a JSON `transform`: each target a functor of its source's text (or value).
pub fn transform_lits(prog: &mut Program, tr: &Value, vs: &mut Vars, defined: &BTreeSet<String>) -> R<Vec<String>> {
    let o = tr.as_obj().ok_or("transform is not an object")?;
    if let Some(optname) = o.get("$option") {
        let name = q_val(optname);
        let cases = cases_of(tr);
        let every = transform_targets(tr);
        let mut branches: Vec<(Vec<String>, Value)> = cases.iter().map(|(c, s)| (vec![format!("option({name}, {})", q(c))], s.clone())).collect();
        let guard = cases.iter().map(|(c, _)| format!("!option({name}, {})", q(c))).collect();
        branches.push((guard, o.get("default").cloned().unwrap_or(Value::Obj(Obj::default()))));
        let mut parts = Vec::new();
        for (guard, s) in branches {
            let mut body = guard;
            body.extend(transform_lits(prog, &s, vs, defined)?);
            let has_ = transform_targets(&s);
            body.extend(every.iter().filter(|t| !has_.contains(*t)).map(|t| format!("unset({t})")));
            parts.push(body.join(", "));
        }
        return Ok(vec![format!("({})", parts.join(" ; "))]);
    }
    let mut lits: Vec<String> = Vec::new();
    for (t, spec) in o.iter() {
        let mut start: i128 = lits.len() as i128;
        if prog.transform_defs.is_none() {
            prog.transform_defs = Some(Default::default());
        }
        let single = spec.as_obj().filter(|s| s.len() == 1);
        let Some(single) = single else {
            prog.untranslated += 1;
            lits.push(format!("ast(n, {})", compact(&Value::obj(vec![("transform", Value::obj(vec![(t, spec.clone())]))]))));
            continue;
        };
        let (op, a) = single.0[0].clone();
        let ao = a.as_obj();
        let src = ao.map(|ao| mv_v(ao.get("source").unwrap_or(&Value::Str(String::new())))).unwrap_or_default();
        let read = |lits: &mut Vec<String>, vs: &mut Vars| -> String {
            if defined.contains(&src) {
                return src.clone();
            }
            let x = vs.fresh("t");
            let reader = if truthy(a.get("value")) { "value" } else { "text" };
            lits.push(format!("{reader}({src}, {x})"));
            x
        };
        let need_obj = || -> R<&Obj> { ao.ok_or_else(|| "'str' object has no attribute 'get'".to_owned()) };
        if op == "count" {
            lits.push(format!("list_len({src}, {t})"));
            prog.numeric.insert(t.to_owned());
        } else if op == "line" {
            lits.push(format!("line({src}, {t})"));
            prog.numeric.insert(t.to_owned());
        } else if op == "replace" && need_obj()?.get("replace").is_some_and(Value::is_str) && need_obj()?.get("by").is_some_and(Value::is_str) {
            let ao = need_obj()?;
            let x = read(&mut lits, vs);
            let f = format!("{t} = @replace_all({x}, {}, {})", q_val(ao.get("replace").expect("replace")), q_val(ao.get("by").expect("by")));
            if ao.has("default") && !defined.contains(&src) {
                let last = lits.pop().expect("the read");
                lits.push(format!("(unset({src}), {t} = {} ; !unset({src}), {last}, {f})", q_val(ao.get("default").expect("default"))));
            } else {
                lits.push(f);
            }
            prog.symbolic.insert(t.to_owned());
        } else if op == "convert" && need_obj()?.get("toCase").is_some_and(|c| c.is_str() || c.is_obj()) {
            let ao = need_obj()?;
            let x = read(&mut lits, vs);
            let seps_list: Vec<String> = ao.get("separatedBy").filter(|s| s.truthy()).and_then(Value::as_list).map(|l| l.iter().map(py_str).collect()).unwrap_or_default();
            let seps = q(&seps_list.join(","));
            let case = ao.get("toCase").expect("toCase");
            if case.is_str() {
                lits.push(format!("{t} = @convert({x}, {}, {seps})", q_val(case)));
            } else {
                let name = q_val(case.get("$option").ok_or("KeyError: '$option'")?);
                let ccases = cases_of(case);
                let mut parts: Vec<String> = ccases.iter().map(|(c, v)| format!("option({name}, {}), {t} = @convert({x}, {}, {seps})", q(c), q_val(v))).collect();
                let mut last: Vec<String> = ccases.iter().map(|(c, _)| format!("!option({name}, {})", q(c))).collect();
                let default = case.get("default").cloned().unwrap_or(Value::Str("lowerCase".into()));
                last.push(format!("{t} = @convert({x}, {}, {seps})", q_val(&default)));
                parts.push(last.join(", "));
                lits.push(format!("({})", parts.join(" ; ")));
            }
            prog.symbolic.insert(t.to_owned());
        } else if op == "substring" {
            let ao = need_obj()?;
            let x = read(&mut lits, vs);
            let s0 = ao.get("startChar").cloned().unwrap_or(Value::Int(0));
            let end = ao.get("endChar").cloned().filter(|e| !e.is_null());
            if !s0.truthy() && end.is_none() {
                lits.push(format!("{t} = {x}"));
            } else {
                let e = end.map_or(format!("strlen({x})"), |e| py_str(&e));
                lits.push(format!("{t} = @slice({x}, {}, {e})", py_str(&s0)));
            }
            // Python's `start` is the start char from here on (it shadows the literals' offset)
            start = match &s0 {
                Value::Int(i) => *i,
                Value::Bool(b) => *b as i128,
                Value::Null => 0,
                _ => return Err("slice indices must be integers".into()),
            };
            prog.symbolic.insert(t.to_owned());
        } else if op == "toPrecision" && need_obj()?.get("digits").is_some_and(Value::is_str) {
            let ao = need_obj()?;
            let x = read(&mut lits, vs);
            let d = vs.fresh("d");
            lits.push(format!("text({}, {d}), {t} = @to_precision({x}, to_number({d}))", mv_v(ao.get("digits").expect("digits"))));
            prog.symbolic.insert(t.to_owned());
        } else {
            prog.untranslated += 1;
            lits.push(format!("ast(n, {})", compact(&Value::obj(vec![("transform", Value::obj(vec![(t, spec.clone())]))]))));
        }
        let defs_tail = tail_from(&lits, start);
        prog.transform_defs.as_mut().expect("transform defs").insert(t.to_owned(), (src.clone(), defs_tail));
    }
    Ok(lits)
}

/// Today's ast fix template as a native edit.
pub fn fix_edits(viol: &Obj) -> Option<Vec<String>> {
    match viol.get("fix") {
        Some(Value::Str(fx)) => Some(vec![format!("replace(n, {})", tick(fx))]),
        Some(Value::Obj(fx)) if fx.get("template").is_some_and(Value::is_str) => {
            let rng = fx.get("range").map(py_str).unwrap_or_default();
            let target = rng.strip_prefix("capture.").map_or("n".to_owned(), cap);
            Some(vec![format!("replace({target}, {})", tick(fx.get("template").and_then(Value::as_str).expect("a template")))])
        }
        _ => None,
    }
}

/// The metavariables a JSON message interpolates, in order.
fn message_vars(msg: Option<&Value>) -> Vec<String> {
    let Some(Value::Str(m)) = msg else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for x in findall(&re(r"\$\$?\$?([A-Z_][A-Z0-9_]*)"), m) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

/// Metavariables that become columns of the finding: `each`, a capture anchor, and those in the message.
pub fn violation_columns(viol: &Obj) -> Vec<String> {
    let mut cols = Vec::new();
    if let Some(Value::Str(e)) = viol.get("each") {
        cols.push(mv(e));
    }
    let anchor = viol.get("anchor").filter(|a| a.truthy()).map(py_str).unwrap_or_default();
    if let Some(c) = anchor.strip_prefix("capture.") {
        cols.push(cap(c));
    }
    cols.extend(message_vars(viol.get("message")));
    let mut out: Vec<String> = Vec::new();
    for c in cols {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// A message that depends on an option (`{"$option": O, "cases": {…}, "default": …}`, or a `template` with `{}`), as
/// one text. A finding's key is its policy, code, path and line; its message is free, so the message need not
/// follow the option. It is the first plain text a case or the default gives, else the template with the option's
/// name in the place of its value (`limit <max>`).
fn plain_message(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Obj(o) if o.has("$option") => {
            let name = o.get("$option").map(py_str).unwrap_or_default();
            if let Some(t) = o.get("template").and_then(Value::as_str) {
                return t.replacen("{}", &format!("<{name}>"), 1);
            }
            let cases: Vec<Value> = cases_of(v).into_iter().map(|(_, b)| b).collect();
            let plain = o.get("default").filter(|d| d.is_str()).or_else(|| cases.iter().find(|c| c.is_str()));
            match plain.or(o.get("default")).or(cases.first()) {
                Some(x) => plain_message(x),
                None => String::new(),
            }
        }
        other => py_str(other),
    }
}

pub fn violation_lines(viol: &Obj, rel: &str, cols: &[String], ast: bool, numeric: &BTreeSet<String>, symbolic: &BTreeSet<String>) -> Vec<String> {
    let code = viol.get("code").cloned().unwrap_or_else(|| Value::Str(rel.to_owned()));
    let mut msg = viol.get("message").cloned().unwrap_or(Value::Str(String::new()));
    if is_option(&msg) {
        msg = Value::Str(plain_message(&msg));
    }
    if let Value::Str(m) = &msg {
        let mut m = m.clone();
        let mut sorted: Vec<&String> = cols.iter().collect();
        sorted.sort_by_key(|c| std::cmp::Reverse(c.chars().count()));
        for c in sorted {
            let rep = format!("{{{}}}", col(c));
            m = sub(&re(&format!(r"\$\$?\$?{}\b", esc(c))), &m, |_| rep.clone());
        }
        msg = Value::Str(m);
    }
    let colsig: String = cols
        .iter()
        .map(|c| format!(", {}: {}", col(c), if numeric.contains(c) { "number" } else if symbolic.contains(c) { "symbol" } else { "loc" }))
        .collect();
    let mut head = format!(".violation {rel}(at: node{colsig})");
    if code.as_str() != Some(rel) {
        head += &format!(" code {}", q_val(&code));
    }
    let mut lines = vec![format!("{head} {}", q_val(&msg))];
    let anchor = viol.get("anchor").filter(|a| a.truthy()).map(py_str);
    if let Some(a) = &anchor {
        if let Some(c) = a.strip_prefix("capture.") {
            lines.push(format!("    anchor {}", col(c)));
        } else if a != "witness.span" {
            lines.push(format!("    anchor {}", a.replace("witness.", "")));
        }
    }
    if let Some(Value::Obj(sup)) = viol.get("suppress").filter(|s| s.truthy()) {
        for (tool, spec) in sup.iter() {
            // ESLint's and noqa's `rules`/`codes`, clippy's `lints`.
            let names = ["codes", "rules", "lints"].into_iter().find_map(|k| spec.get(k).filter(|c| c.truthy()));
            match names.and_then(Value::as_list) {
                Some(ns) if !ns.is_empty() => lines.push(format!("    suppress {tool} {}", ns.iter().map(q_val).collect::<Vec<_>>().join(", "))),
                _ => lines.push(format!("    suppress {tool}")),
            }
        }
    }
    if let Some(fx) = viol.get("fix")
        && !ast
    {
        lines.push(format!("    fix {}", compact(fx)));
    }
    if let Some(e) = viol.get("each")
        && !e.is_str()
    {
        lines.push(format!("    each {}", compact(e)));
    }
    lines
}

