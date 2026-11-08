//! Python 3.13 canonical expression text for annotations, produced in one native file pass.
//! The AST parser is Ruff's existing native grammar parser; Python never executes here.
use ruff_python_ast::{
    self as ast, Expr,
    visitor::{self, Visitor},
};
use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_text_size::Ranged;
use std::collections::HashMap;

include!("python_print/printable.generated.rs");
type Result<T> = std::result::Result<T, String>;
const TEST: u8 = 4;
const ATOM: u8 = 18;

fn printable(cp: u32) -> bool {
    let i = PRINTABLE.partition_point(|&(_, end)| end < cp);
    PRINTABLE.get(i).is_some_and(|&(start, _)| start <= cp)
}
fn escaped(cp: u32) -> String {
    match cp {
        9 => "\\t".into(),
        10 => "\\n".into(),
        13 => "\\r".into(),
        92 => "\\\\".into(),
        0..=255 => format!("\\x{cp:02x}"),
        256..=65535 => format!("\\u{cp:04x}"),
        _ => format!("\\U{cp:08x}"),
    }
}
/// Python 3.13 repr of scalar Unicode text, shared by the printer and text functors.
pub(in crate::code_rule_dsl) fn string_repr(value: &str) -> String {
    repr(&value.chars().map(u32::from).collect::<Vec<_>>(), false)
}

fn repr(points: &[u32], bytes: bool) -> String {
    let quote = if points.contains(&39) && !points.contains(&34) {
        '"'
    } else {
        '\''
    };
    let mut out = if bytes { "b".to_owned() } else { String::new() };
    out.push(quote);
    for &cp in points {
        if cp == quote as u32 {
            out.push('\\');
            out.push(quote);
        } else if cp == 92 || !printable(cp) || bytes && !(32..=126).contains(&cp) {
            out.push_str(&escaped(cp));
        } else {
            out.push(char::from_u32(cp).expect("printable ranges contain scalar values"));
        }
    }
    out.push(quote);
    out
}

struct Printer<'a> {
    source: &'a str,
}
impl Printer<'_> {
    /// Python repr(value) for non-string Constant expressions. The syntax
    /// printer uses source expressions such as 1e309; value facts use inf.
    fn annotation_constant(&self, e: &Expr) -> Option<Result<String>> {
        match e {
            Expr::NumberLiteral(n) => Some(Ok(match &n.value {
                ast::Number::Int(i) => i.to_string(),
                ast::Number::Float(f) => crate::code_rule_dsl::value::python::float_repr(*f),
                ast::Number::Complex { real, imag } => {
                    let component = |f| {
                        let text = crate::code_rule_dsl::value::python::float_repr(f);
                        text.strip_suffix(".0").unwrap_or(&text).to_owned()
                    };
                    if *real == 0.0 {
                        format!("{}j", component(*imag))
                    } else {
                        format!("({}{}{}j)", component(*real), if *imag >= 0.0 { "+" } else { "" }, component(*imag))
                    }
                }
            })),
            Expr::BytesLiteral(_) | Expr::BooleanLiteral(_) | Expr::NoneLiteral(_) => Some(self.expr(e, TEST)),
            Expr::EllipsisLiteral(_) => Some(Ok("Ellipsis".to_owned())),
            _ => None,
        }
    }

    fn expr(&self, e: &Expr, needed: u8) -> Result<String> {
        let (own, text) = match e {
            Expr::Name(n) => (ATOM, n.id.to_string()),
            Expr::Attribute(n) => {
                let space = if matches!(
                    &*n.value,
                    Expr::NumberLiteral(ast::ExprNumberLiteral {
                        value: ast::Number::Int(_),
                        ..
                    })
                ) {
                    " "
                } else {
                    ""
                };
                (
                    ATOM,
                    format!("{}{space}.{}", self.expr(&n.value, ATOM)?, n.attr),
                )
            }
            Expr::Subscript(n) => {
                let body = match &*n.slice {
                    Expr::Tuple(t) if !t.elts.is_empty() => self.items(&t.elts, TEST, true)?,
                    _ => self.expr(&n.slice, TEST)?,
                };
                (ATOM, format!("{}[{body}]", self.expr(&n.value, ATOM)?))
            }
            Expr::BinOp(n) => {
                let (p, op) = match n.op {
                    ast::Operator::BitOr => (9, "|"),
                    ast::Operator::BitXor => (10, "^"),
                    ast::Operator::BitAnd => (11, "&"),
                    ast::Operator::LShift => (12, "<<"),
                    ast::Operator::RShift => (12, ">>"),
                    ast::Operator::Add => (13, "+"),
                    ast::Operator::Sub => (13, "-"),
                    ast::Operator::Mult => (14, "*"),
                    ast::Operator::MatMult => (14, "@"),
                    ast::Operator::Div => (14, "/"),
                    ast::Operator::FloorDiv => (14, "//"),
                    ast::Operator::Mod => (14, "%"),
                    ast::Operator::Pow => (16, "**"),
                };
                let right = op == "**";
                (
                    p,
                    format!(
                        "{} {op} {}",
                        self.expr(&n.left, p + u8::from(right))?,
                        self.expr(&n.right, p + u8::from(!right))?
                    ),
                )
            }
            Expr::UnaryOp(n) => {
                let (p, op) = match n.op {
                    ast::UnaryOp::Not => (7, "not "),
                    ast::UnaryOp::UAdd => (15, "+"),
                    ast::UnaryOp::USub => (15, "-"),
                    ast::UnaryOp::Invert => (15, "~"),
                };
                (p, format!("{op}{}", self.expr(&n.operand, p)?))
            }
            Expr::BoolOp(n) => {
                let (p, sep) = if n.op == ast::BoolOp::And {
                    (6, " and ")
                } else {
                    (5, " or ")
                };
                (
                    p,
                    n.values
                        .iter()
                        .map(|e| self.expr(e, p + 1))
                        .collect::<Result<Vec<_>>>()?
                        .join(sep),
                )
            }
            Expr::Compare(n) => {
                let mut s = self.expr(&n.left, 9)?;
                for (op, e) in n.ops.iter().zip(n.comparators.iter()) {
                    s.push_str(&format!(" {} {}", op.as_str(), self.expr(e, 9)?));
                }
                (8, s)
            }
            Expr::If(n) => (
                4,
                format!(
                    "{} if {} else {}",
                    self.expr(&n.body, 5)?,
                    self.expr(&n.test, 5)?,
                    self.expr(&n.orelse, 4)?
                ),
            ),
            Expr::Named(n) => (
                1,
                format!(
                    "{} := {}",
                    self.expr(&n.target, ATOM)?,
                    self.expr(&n.value, ATOM)?
                ),
            ),
            Expr::Lambda(n) => {
                let args = n
                    .parameters
                    .as_deref()
                    .map(|p| self.parameters(p))
                    .transpose()?
                    .unwrap_or_default();
                (
                    4,
                    format!(
                        "lambda{}: {}",
                        if args.is_empty() {
                            String::new()
                        } else {
                            format!(" {args}")
                        },
                        self.expr(&n.body, 4)?
                    ),
                )
            }
            Expr::Call(n) => {
                let mut args = n
                    .arguments
                    .args
                    .iter()
                    .map(|e| self.expr(e, TEST))
                    .collect::<Result<Vec<_>>>()?;
                for k in &n.arguments.keywords {
                    args.push(format!(
                        "{}{}",
                        k.arg.as_ref().map_or("**".to_owned(), |a| format!("{a}=")),
                        self.expr(&k.value, TEST)?
                    ));
                }
                (
                    ATOM,
                    format!("{}({})", self.expr(&n.func, ATOM)?, args.join(", ")),
                )
            }
            Expr::Tuple(n) => {
                let s = self.items(&n.elts, TEST, true)?;
                if n.elts.is_empty() {
                    (ATOM, "()".into())
                } else {
                    (2, s)
                }
            }
            Expr::List(n) => (ATOM, format!("[{}]", self.items(&n.elts, TEST, false)?)),
            Expr::Set(n) => (
                ATOM,
                if n.elts.is_empty() {
                    "{*()}".into()
                } else {
                    format!("{{{}}}", self.items(&n.elts, TEST, false)?)
                },
            ),
            Expr::Dict(n) => {
                let mut items = Vec::new();
                for item in &n.items {
                    items.push(match item.key.as_ref() {
                        Some(k) => {
                            format!("{}: {}", self.expr(k, TEST)?, self.expr(&item.value, TEST)?)
                        }
                        None => format!("**{}", self.expr(&item.value, 9)?),
                    });
                }
                (ATOM, format!("{{{}}}", items.join(", ")))
            }
            Expr::Slice(n) => {
                let p = |v: &Option<Box<Expr>>| {
                    v.as_deref()
                        .map(|e| self.expr(e, TEST))
                        .transpose()
                        .map(|s| s.unwrap_or_default())
                };
                let s = format!("{}:{}", p(&n.lower)?, p(&n.upper)?);
                (
                    ATOM,
                    if n.step.is_some() {
                        format!("{s}:{}", p(&n.step)?)
                    } else {
                        s
                    },
                )
            }
            Expr::Starred(n) => (ATOM, format!("*{}", self.expr(&n.value, 9)?)),
            Expr::Await(n) => (17, format!("await {}", self.expr(&n.value, ATOM)?)),
            Expr::Yield(n) => (
                3,
                format!(
                    "yield{}",
                    n.value
                        .as_deref()
                        .map(|e| self.expr(e, ATOM).map(|s| format!(" {s}")))
                        .transpose()?
                        .unwrap_or_default()
                ),
            ),
            Expr::YieldFrom(n) => (3, format!("yield from {}", self.expr(&n.value, ATOM)?)),
            Expr::ListComp(n) => (
                ATOM,
                format!(
                    "[{}{}]",
                    self.expr(&n.elt, TEST)?,
                    self.generators(&n.generators)?
                ),
            ),
            Expr::SetComp(n) => (
                ATOM,
                format!(
                    "{{{}{}}}",
                    self.expr(&n.elt, TEST)?,
                    self.generators(&n.generators)?
                ),
            ),
            Expr::DictComp(n) => (
                ATOM,
                format!(
                    "{{{}: {}{}}}",
                    self.expr(
                        n.key
                            .as_deref()
                            .ok_or("dictionary comprehension lacks a key")?,
                        TEST
                    )?,
                    self.expr(&n.value, TEST)?,
                    self.generators(&n.generators)?
                ),
            ),
            Expr::Generator(n) => (
                ATOM,
                format!(
                    "({}{})",
                    self.expr(&n.elt, TEST)?,
                    self.generators(&n.generators)?
                ),
            ),
            Expr::BooleanLiteral(n) => (ATOM, if n.value { "True" } else { "False" }.into()),
            Expr::NoneLiteral(_) => (ATOM, "None".into()),
            Expr::EllipsisLiteral(_) => (ATOM, "...".into()),
            Expr::NumberLiteral(n) => (
                ATOM,
                match &n.value {
                    ast::Number::Int(i) => i.to_string(),
                    ast::Number::Float(f) => float(*f),
                    ast::Number::Complex { real, imag } => {
                        let i = float(*imag);
                        let i = i.strip_suffix(".0").unwrap_or(&i);
                        if *real == 0.0 {
                            format!("{i}j")
                        } else {
                            format!(
                                "({}{}{i}j)",
                                float(*real),
                                if *imag >= 0.0 { "+" } else { "" }
                            )
                        }
                    }
                },
            ),
            Expr::StringLiteral(n) => {
                let mut points = Vec::new();
                for part in n.value.iter() {
                    points.extend(self.literal(part.range(), false)?);
                }
                let first = n.value.iter().next().ok_or("empty literal")?;
                let start = usize::from(first.range().start());
                let prefix = if self.source[start..].starts_with('u') {
                    "u"
                } else {
                    ""
                };
                (ATOM, format!("{prefix}{}", repr(&points, false)))
            }
            Expr::BytesLiteral(n) => {
                let mut points = Vec::new();
                for part in n.value.iter() {
                    points.extend(self.literal(part.range(), true)?);
                }
                (ATOM, repr(&points, true))
            }
            Expr::FString(n) => (ATOM, self.fstring(n)?),
            Expr::TString(_) | Expr::IpyEscapeCommand(_) => {
                return Err("annotation printer requires Python 3.13 syntax".into());
            }
        };
        Ok(if own < needed {
            format!("({text})")
        } else {
            text
        })
    }
    fn items(&self, items: &[Expr], needed: u8, single_comma: bool) -> Result<String> {
        let mut s = items
            .iter()
            .map(|e| self.expr(e, needed))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        if single_comma && items.len() == 1 {
            s.push(',');
        }
        Ok(s)
    }
    fn parameters(&self, p: &ast::Parameters) -> Result<String> {
        let mut items = Vec::new();
        for v in p.posonlyargs.iter().chain(p.args.iter()) {
            let mut s = v.name().to_string();
            if let Some(e) = &v.default {
                s.push('=');
                s.push_str(&self.expr(e, TEST)?);
            }
            items.push(s);
            if items.len() == p.posonlyargs.len() {
                items.push("/".into());
            }
        }
        if let Some(v) = &p.vararg {
            items.push(format!("*{}", v.name));
        } else if !p.kwonlyargs.is_empty() {
            items.push("*".into());
        }
        for v in &p.kwonlyargs {
            let mut s = v.name().to_string();
            if let Some(e) = &v.default {
                s.push('=');
                s.push_str(&self.expr(e, TEST)?);
            }
            items.push(s);
        }
        if let Some(v) = &p.kwarg {
            items.push(format!("**{}", v.name));
        }
        Ok(items.join(", "))
    }
    fn generators(&self, gs: &[ast::Comprehension]) -> Result<String> {
        let mut out = String::new();
        for g in gs {
            out.push_str(&format!(
                "{}for {} in {}",
                if g.is_async { " async " } else { " " },
                self.expr(&g.target, 2)?,
                self.expr(&g.iter, 5)?
            ));
            for c in &g.ifs {
                out.push_str(&format!(" if {}", self.expr(c, 5)?));
            }
        }
        Ok(out)
    }
    fn literal(&self, range: ruff_text_size::TextRange, bytes: bool) -> Result<Vec<u32>> {
        let text = &self.source[usize::from(range.start())..usize::from(range.end())];
        let i = text.bytes().take_while(u8::is_ascii_alphabetic).count();
        let prefix = text[..i].to_ascii_lowercase();
        let rest = &text[i..];
        let quote = ["\"\"\"", "'''", "\"", "'"]
            .into_iter()
            .find(|q| rest.starts_with(q) && rest.ends_with(q) && rest.len() >= 2 * q.len())
            .ok_or("invalid annotation literal")?;
        let body = rest[quote.len()..rest.len() - quote.len()]
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if prefix.contains('r') {
            Ok(body.chars().map(u32::from).collect())
        } else {
            crate::code_rule_dsl::value::python::decode_escapes(&body, bytes)
                .ok_or("invalid annotation escape".into())
        }
    }
    fn elements(
        &self,
        elements: &ast::InterpolatedStringElements,
        spec: bool,
        parts: &mut Vec<(String, bool)>,
    ) -> Result<()> {
        let literal = |s: &str| {
            let s = s.replace('{', "{{").replace('}', "}}");
            if spec {
                s.replace('\\', "\\\\")
                    .replace('\'', "\\'")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
            } else {
                s
            }
        };
        for e in elements.iter() {
            match e {
                ast::InterpolatedStringElement::Literal(l) => {
                    push_literal(parts, literal(&l.value))
                }
                ast::InterpolatedStringElement::Interpolation(f) => {
                    if let Some(d) = &f.debug_text {
                        push_literal(parts, literal(d.as_str()));
                    }
                    let expr = self.expr(&f.expression, 5)?;
                    let mut s = format!("{{{}{expr}", if expr.starts_with('{') { " " } else { "" });
                    let conversion = f.conversion.to_char().or_else(|| {
                        (f.debug_text.is_some() && f.format_spec.is_none()).then_some('r')
                    });
                    if let Some(c) = conversion {
                        s.push('!');
                        s.push(c);
                    }
                    if let Some(format) = &f.format_spec {
                        let mut ps = Vec::new();
                        self.elements(&format.elements, true, &mut ps)?;
                        s.push(':');
                        for (p, _) in ps {
                            s.push_str(&p);
                        }
                    }
                    s.push('}');
                    parts.push((s, false));
                }
            }
        }
        Ok(())
    }
    fn fstring(&self, n: &ast::ExprFString) -> Result<String> {
        let mut parts = Vec::new();
        for part in n.value.iter() {
            match part {
                ast::FStringPart::Literal(s) => {
                    push_literal(&mut parts, s.value.replace('{', "{{").replace('}', "}}"))
                }
                ast::FStringPart::FString(s) => self.elements(&s.elements, false, &mut parts)?,
            }
        }
        let mut quotes = vec!["'", "\"", "\"\"\"", "'''"];
        let mut out = String::new();
        let mut fallback = false;
        for (s, literal) in &parts {
            if *literal {
                let (body, choices) = literal_helper(s, &quotes);
                if !choices.iter().any(|q| quotes.contains(q)) {
                    fallback = true;
                    break;
                }
                quotes = choices;
                out.push_str(&body);
            } else {
                if s.contains('\n') {
                    quotes.retain(|q| q.len() == 3);
                }
                let choices: Vec<_> = quotes.iter().copied().filter(|q| !s.contains(q)).collect();
                if !choices.is_empty() {
                    quotes = choices;
                }
                out.push_str(s);
            }
        }
        if fallback {
            quotes = vec!["'''"];
            out.clear();
            for (s, literal) in parts {
                if literal {
                    let forced = format!("\"{s}");
                    let r = repr(&forced.chars().map(u32::from).collect::<Vec<_>>(), false);
                    out.push_str(&r[2..r.len() - 1]);
                } else {
                    out.push_str(&s);
                }
            }
        }
        let q = quotes.first().ok_or("no quotes for f-string")?;
        Ok(format!("f{q}{out}{q}"))
    }
}
fn push_literal(parts: &mut Vec<(String, bool)>, s: String) {
    if let Some((last, true)) = parts.last_mut() {
        last.push_str(&s);
    } else {
        parts.push((s, true));
    }
}
fn literal_helper(s: &str, quotes: &[&'static str]) -> (String, Vec<&'static str>) {
    let mut escaped_s = String::new();
    for c in s.chars() {
        if c == '\\' || !printable(c as u32) {
            escaped_s.push_str(&escaped(c as u32));
        } else {
            escaped_s.push(c);
        }
    }
    let mut choices: Vec<_> = quotes
        .iter()
        .copied()
        .filter(|q| !escaped_s.contains(q))
        .collect();
    if choices.is_empty() {
        let r = repr(&s.chars().map(u32::from).collect::<Vec<_>>(), false);
        let quote = quotes
            .iter()
            .copied()
            .find(|q| q.starts_with(r.chars().next().unwrap()))
            .unwrap_or(if r.starts_with('\'') { "'" } else { "\"" });
        return (r[1..r.len() - 1].to_owned(), vec![quote]);
    }
    if let Some(last) = escaped_s.chars().last() {
        choices.sort_by_key(|q| q.starts_with(last));
        if choices[0].starts_with(last) {
            escaped_s.pop();
            escaped_s.push('\\');
            escaped_s.push(last);
        }
    }
    (escaped_s, choices)
}
fn float(f: f64) -> String {
    crate::code_rule_dsl::value::python::float_repr(f)
        .replace("inf", "1e309")
        .replace("nan", "(1e309-1e309)")
}

/// Canonical text requested by relation, keyed by semantic source span.
#[derive(Default)]
pub(super) struct Texts {
    pub annotations: HashMap<(u32, u32), String>,
    pub annotation_constants: HashMap<(u32, u32), String>,
    pub receivers: HashMap<(u32, u32), String>,
    pub calls: HashMap<(u32, u32), String>,
    pub functions: HashMap<(u32, u32), String>,
}

/// Columns requested by the program's relation projection, not runtime feature flags.
#[derive(Clone, Copy, Default)]
pub(super) struct Requested {
    pub annotations: bool,
    pub annotation_constants: bool,
    pub receivers: bool,
    pub calls: bool,
    pub functions: bool,
}

/// One parse for the demanded text columns. Call columns use actual Call keys, never wrapper nodes.
pub(super) fn texts(source: &str, requested: Requested) -> Result<Texts> {
    let parsed = parse_unchecked(
        source,
        ParseOptions::from(Mode::Module).with_target_version(ast::PythonVersion::PY313),
    );
    if !parsed.errors().is_empty() || !parsed.unsupported_syntax_errors().is_empty() {
        return Ok(Texts::default());
    }
    struct Collect<'a> {
        printer: Printer<'a>,
        rows: Texts,
        requested: Requested,
        error: Option<String>,
        in_annotation: bool,
    }
    impl<'a> Visitor<'a> for Collect<'a> {
        fn visit_annotation(&mut self, e: &'a Expr) {
            let previous = self.in_annotation;
            self.in_annotation = true;
            if self.requested.annotations {
                match self.printer.expr(e, TEST) {
                    Ok(s) => {
                        let r = e.range();
                        self.rows
                            .annotations
                            .insert((u32::from(r.start()), u32::from(r.end())), s);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            visitor::walk_annotation(self, e);
            self.in_annotation = previous;
        }
        fn visit_expr(&mut self, e: &'a Expr) {
            if self.requested.annotation_constants && self.in_annotation
                && let Some(result) = self.printer.annotation_constant(e)
            {
                match result {
                    Ok(text) => {
                        let range = e.range();
                        self.rows.annotation_constants.insert((u32::from(range.start()), u32::from(range.end())), text);
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            if self.requested.receivers
                && let Expr::Call(call) = e
                && let Expr::Attribute(attribute) = &*call.func
            {
                match self.printer.expr(&attribute.value, TEST) {
                    Ok(s) => {
                        let r = e.range();
                        self.rows
                            .receivers
                            .insert((u32::from(r.start()), u32::from(r.end())), s);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if let Expr::Call(call) = e {
                let range = (u32::from(e.range().start()), u32::from(e.range().end()));
                if self.requested.calls {
                    match self.printer.expr(e, TEST) {
                        Ok(text) => { self.rows.calls.insert(range, text); }
                        Err(error) => self.error = Some(error),
                    }
                }
                if self.requested.functions {
                    match self.printer.expr(&call.func, TEST) {
                        Ok(text) => { self.rows.functions.insert(range, text); }
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            visitor::walk_expr(self, e);
        }
    }
    let mut c = Collect {
        printer: Printer { source },
        rows: Texts::default(),
        requested,
        error: None,
        in_annotation: false,
    };
    c.visit_body(
        parsed
            .syntax()
            .as_module()
            .ok_or("not a Python module")?
            .body
            .as_slice(),
    );
    match c.error {
        Some(e) => Err(e),
        None => Ok(c.rows),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn canonical_call_pairs_match_python_313_recordings() {
        let rows: serde_json::Value = serde_json::from_str(include_str!("python_print/calls.json")).unwrap();
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let values = super::texts(source, super::Requested { calls: true, functions: true, ..Default::default() }).unwrap();
            let mut calls: Vec<_> = values.calls.into_iter().collect();
            calls.sort_by_key(|(span, _)| *span);
            let actual: Vec<Vec<String>> = calls.into_iter().map(|(span, text)| vec![text, values.functions[&span].clone()]).collect();
            assert_eq!(serde_json::to_value(actual).unwrap(), row["calls"], "{source}");
        }
    }

    #[test]
    fn canonical_annotations_match_python_313_recordings() {
        let rows: serde_json::Value =
            serde_json::from_str(include_str!("python_print/expressions.json")).unwrap();
        for row in rows.as_array().unwrap() {
            let source = row["source"].as_str().unwrap();
            let values = super::texts(source, super::Requested { annotations: true, ..Default::default() }).unwrap().annotations;
            let expected: Vec<_> = row["annotations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect();
            let mut actual: Vec<_> = values.into_iter().collect();
            actual.sort_by_key(|(span, _)| *span);
            assert_eq!(
                actual.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
                expected,
                "{source}"
            );
        }
    }
}
