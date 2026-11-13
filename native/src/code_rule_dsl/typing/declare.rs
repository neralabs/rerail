//! User declarations, `rerail.types/v1` (03 §13; I4, I17; list step 29).
//!
//! Signatures for code that cannot be edited, written in the language's own syntax. For Python that is stub
//! syntax:
//!
//! ```text
//! # rerail.types/v1
//! # module: vendor.client
//! T = TypeVar("T", bound=Hashable)
//!
//! def get(key: str, default: T | None = None) -> T | None: ...
//!
//! class Client:
//!     def fetch(self, url: str, timeout: float = ...) -> bytes: ...
//! ```
//!
//! Each `def` becomes a callable term (`typing::terms`), named `module.name` or `module.Class.name`. A missing
//! annotation is ⊤. The annotation expressions read are names (dotted), subscripts (`list[int]`,
//! `dict[str, T]`), `A | B`, `Optional[X]`, `Union[...]`, `None`, `Callable[[A, B], R]`, `tuple[...]`,
//! `Literal["a", 1]` and quoted forward references.
//!
//! Declarations **fill gaps only** (`fill`): a name the source annotates keeps the source's signature, and where
//! the two differ the conflict is reported, not resolved.

use std::collections::{BTreeMap, HashMap};

use super::terms::{Terms, Tid};
use crate::content_store::atoms::Sym;

pub(crate) const HEADER: &str = "# rerail.types/v1";

#[derive(Debug, PartialEq)]
pub(crate) struct Error {
    pub(crate) line: usize,
    pub(crate) message: String,
}

/// One declaration file's signatures, by qualified name.
#[derive(Debug, Default)]
pub(crate) struct Declarations {
    pub(crate) module: String,
    pub(crate) signatures: BTreeMap<String, Tid>,
}

/// A declared signature that differs from the source's annotation (reported, the source's kept).
#[derive(Debug, PartialEq)]
pub(crate) struct Conflict {
    pub(crate) name: String,
    pub(crate) source: Tid,
    pub(crate) declared: Tid,
}

/// Reads one `rerail.types/v1` file. `intern` gives a name's atom (the single interner).
pub(crate) fn parse(text: &str, terms: &mut Terms, intern: &mut dyn FnMut(&str) -> Sym) -> Result<Declarations, Error> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == HEADER => {}
        _ => return Err(Error { line: 1, message: format!("the first line must be `{HEADER}`") }),
    }
    let mut out = Declarations::default();
    let mut vars: HashMap<String, Tid> = HashMap::new();
    let mut class: Option<(usize, String)> = None;
    for (i, raw) in lines {
        let line = i + 1;
        let indent = raw.len() - raw.trim_start().len();
        let l = raw.trim();
        if let Some(m) = l.strip_prefix("# module:") {
            out.module = m.trim().to_owned();
            continue;
        }
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if class.as_ref().is_some_and(|(at, _)| indent <= *at) {
            class = None;
        }
        let err = |message: String| Error { line, message };
        if let Some(rest) = l.strip_prefix("class ") {
            let name = rest.split(['(', ':']).next().unwrap_or("").trim();
            if name.is_empty() {
                return Err(err("a class needs a name".into()));
            }
            class = Some((indent, name.to_owned()));
        } else if let Some(rest) = l.strip_prefix("def ").or_else(|| l.strip_prefix("async def ")) {
            let open = rest.find('(').ok_or_else(|| err("`def` needs parameters".into()))?;
            let close = matching(rest, open).ok_or_else(|| err("unclosed parameters".into()))?;
            let name = rest[..open].trim();
            let mut params = Vec::new();
            for p in split_top(&rest[open + 1..close], ',') {
                let p = p.trim();
                if p.is_empty() || p == "/" || p == "*" {
                    continue;
                }
                let p = p.split_once('=').map_or(p, |(a, _)| a).trim();
                let t = match p.split_once(':') {
                    Some((_, ann)) => annotation(ann.trim(), terms, &vars, intern).map_err(err)?,
                    None => terms.top(),
                };
                params.push(t);
            }
            let after = rest[close + 1..].trim();
            let ret = match after.strip_prefix("->") {
                Some(r) => annotation(r.trim().trim_end_matches("...").trim().trim_end_matches(':').trim(), terms, &vars, intern).map_err(err)?,
                None => terms.top(),
            };
            let callable = terms.callable(&params, ret);
            let qualified = match &class {
                Some((_, c)) => format!("{}.{c}.{name}", out.module),
                None => format!("{}.{name}", out.module),
            };
            out.signatures.insert(qualified, callable);
        } else if let Some((name, value)) = l.split_once('=')
            && let Some(args) = value.trim().strip_prefix("TypeVar(").and_then(|v| v.strip_suffix(')'))
        {
            let bound = split_top(args, ',').into_iter().find_map(|a| a.trim().strip_prefix("bound=").map(str::to_owned));
            let bound = match bound {
                Some(b) => Some(annotation(b.trim(), terms, &vars, intern).map_err(err)?),
                None => None,
            };
            let name = name.trim();
            let v = terms.var(intern(name), bound);
            vars.insert(name.to_owned(), v);
        } else if !l.starts_with("from ") && !l.starts_with("import ") && l != "..." && l != "pass" {
            return Err(err(format!("not a declaration: {l}")));
        }
    }
    Ok(out)
}

/// The source's signatures with the declarations filling its gaps: a declared name the source does not annotate
/// is added; one it annotates keeps the source's, and differing ones are reported.
pub(crate) fn fill(source: &BTreeMap<String, Tid>, declared: &Declarations) -> (BTreeMap<String, Tid>, Vec<Conflict>) {
    let mut out = source.clone();
    let mut conflicts = Vec::new();
    for (name, &d) in &declared.signatures {
        match source.get(name) {
            None => {
                out.insert(name.clone(), d);
            }
            Some(&s) if s != d => conflicts.push(Conflict { name: name.clone(), source: s, declared: d }),
            Some(_) => {}
        }
    }
    (out, conflicts)
}

fn matching(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices().skip(open) {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `s` split at `sep` outside brackets and quotes.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut start, mut quote) = (Vec::new(), 0i32, 0, None);
    for (i, c) in s.char_indices() {
        match c {
            '"' | '\'' if quote == Some(c) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(c),
            _ if quote.is_some() => {}
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// An annotation expression's term.
fn annotation(s: &str, terms: &mut Terms, vars: &HashMap<String, Tid>, intern: &mut dyn FnMut(&str) -> Sym) -> Result<Tid, String> {
    let s = s.trim();
    let alts = split_top(s, '|');
    if alts.len() > 1 {
        let members = alts.iter().map(|a| annotation(a, terms, vars, intern)).collect::<Result<Vec<_>, _>>()?;
        return Ok(terms.union(&members, false));
    }
    if s.is_empty() {
        return Err("an empty annotation".into());
    }
    if let Some(q) = s.strip_prefix('"').and_then(|q| q.strip_suffix('"')).or_else(|| s.strip_prefix('\'').and_then(|q| q.strip_suffix('\''))) {
        return Ok(terms.reference(intern(q)));
    }
    if s == "Any" || s == "typing.Any" {
        return Ok(terms.top());
    }
    if let Some(v) = vars.get(s) {
        return Ok(*v);
    }
    let Some(open) = s.find('[') else {
        if !s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            return Err(format!("not an annotation: {s}"));
        }
        return Ok(terms.named(intern(s), &[]));
    };
    let close = matching(s, open).filter(|&c| c == s.len() - 1).ok_or_else(|| format!("unbalanced brackets: {s}"))?;
    let head = s[..open].trim();
    let head = head.strip_prefix("typing.").unwrap_or(head);
    let inner = &s[open + 1..close];
    let parts: Vec<&str> = split_top(inner, ',').into_iter().map(str::trim).filter(|p| !p.is_empty()).collect();
    fn list(parts: &[&str], terms: &mut Terms, vars: &HashMap<String, Tid>, intern: &mut dyn FnMut(&str) -> Sym) -> Result<Vec<Tid>, String> {
        parts.iter().map(|p| annotation(p, terms, vars, intern)).collect()
    }
    Ok(match head {
        "Optional" => {
            let x = list(&parts, terms, vars, intern)?;
            let none = terms.named(intern("None"), &[]);
            terms.union(&[x[0], none], false)
        }
        "Union" => {
            let x = list(&parts, terms, vars, intern)?;
            terms.union(&x, false)
        }
        // `tuple[X, ...]`: any length, each an X (the named `tuple[X]`, not a fixed tuple).
        "tuple" | "Tuple" if parts.len() == 2 && parts[1] == "..." => {
            let x = annotation(parts[0], terms, vars, intern)?;
            terms.named(intern("tuple"), &[x])
        }
        "tuple" | "Tuple" => {
            let x = list(&parts, terms, vars, intern)?;
            terms.tuple(&x)
        }
        "Literal" => {
            let lits: Vec<Tid> = parts.iter().map(|p| terms.literal(intern(p.trim_matches(['"', '\''])))).collect();
            terms.union(&lits, false)
        }
        "Callable" => {
            let params = parts.first().and_then(|p| p.strip_prefix('[')).and_then(|p| p.strip_suffix(']'));
            let params = match params {
                Some(p) => list(&split_top(p, ',').into_iter().map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>(), terms, vars, intern)?,
                None => Vec::new(),
            };
            let ret = match parts.get(1) {
                Some(r) => annotation(r, terms, vars, intern)?,
                None => terms.top(),
            };
            terms.callable(&params, ret)
        }
        _ => {
            let x = list(&parts, terms, vars, intern)?;
            terms.named(intern(head), &x)
        }
    })
}

#[cfg(test)]
pub mod tests;
