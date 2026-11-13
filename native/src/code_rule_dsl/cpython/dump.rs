//! `ast.dump(node, include_attributes=False)` as CPython 3.13 writes it (Lib/ast.py's `dump`, `indent=None`,
//! `show_empty=False`): `Class(field=value, …)` over the class's `_fields` in order, a field that is `None` (optional,
//! its class default `None`) or an empty list left out, a list as `[a, b]` (a `None` in it as `None`), a constant, an
//! identifier or an int field as its `repr` (`Constant(value=None)`, `kind='u'`, `Ellipsis`, `inf`, `1j`, `b'x'`).
//!
//! A child's dump is its text within its parent's, so a file's tree is written once and each node's dump is a piece
//! of it (`Dump`). W's pythonAst worker hashed three forms of it (`sha256` of the UTF-8 text, lowercase hex):
//! `dump_sha256`, a node's dump; `body_sha256`, a def's `ast.Module(body=node.body, type_ignores=[])`;
//! `signature_sha256`, a def's `dump(args) | dump(returns) or "None" | dump(ast.Tuple(elts=type_params))` (a `Tuple`
//! made without a `ctx` has `Load()`). A node whose tree holds a value `ast::F::Unknown` stands for, or an int of more
//! than 4300 digits (`int.__repr__` raises), has no dump.

use super::ast::{Const, F, Node};
use super::value::{PyStr, bytes_repr, complex_repr, float_text, str_repr};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// A tree's dump, written once: each node's piece of it, by the node's address.
pub(crate) struct Dump {
    text: String,
    at: HashMap<*const Node, (usize, usize)>,
}

impl Dump {
    /// The dump of `root`'s tree, and of each node in it.
    pub(crate) fn of(root: &Node) -> Self {
        let mut d = Dump { text: String::new(), at: HashMap::new() };
        d.node(root);
        d
    }

    /// `ast.dump(n)` of a node of the tree.
    pub(crate) fn get(&self, n: &Node) -> Option<&str> {
        self.at.get(&std::ptr::from_ref(n)).map(|&(s, e)| &self.text[s..e])
    }

    /// Writes `n`, recording its piece where it has a dump; whether it has.
    fn node(&mut self, n: &Node) -> bool {
        let start = self.text.len();
        self.text.push_str(n.class);
        self.text.push('(');
        let mut ok = true;
        let mut first = true;
        for (name, f) in &n.fields {
            match f {
                F::Absent => continue,
                F::List(items) if items.is_empty() => continue,
                _ => {}
            }
            if !first {
                self.text.push_str(", ");
            }
            first = false;
            self.text.push_str(name);
            self.text.push('=');
            ok &= self.field(f);
        }
        self.text.push(')');
        if ok {
            self.at.insert(std::ptr::from_ref(n), (start, self.text.len()));
        }
        ok
    }

    fn field(&mut self, f: &F) -> bool {
        match f {
            F::Node(n) => self.node(n),
            F::List(items) => {
                self.text.push('[');
                let mut ok = true;
                for (k, item) in items.iter().enumerate() {
                    if k > 0 {
                        self.text.push_str(", ");
                    }
                    ok &= self.field(item);
                }
                self.text.push(']');
                ok
            }
            F::Absent => {
                self.text.push_str("None");
                true
            }
            F::Const(c) => match constant(c) {
                Some(r) => {
                    self.text.push_str(&r);
                    true
                }
                None => false,
            },
            F::Ident(s) => {
                self.text.push_str(&str_repr(&PyStr::Text(s.clone())));
                true
            }
            F::Int(i) => {
                self.text.push_str(&i.to_string());
                true
            }
            F::Unknown => false,
        }
    }

    /// `sha256(ast.dump(n))`, W's `dump_sha256`.
    pub(crate) fn sha256(&self, n: &Node) -> Option<String> {
        Some(hex(Sha256::digest(self.get(n)?.as_bytes()).as_slice()))
    }

    /// `ast.dump(ast.Module(body=def.body, type_ignores=[]))` of a def, in pieces.
    fn body<'d>(&'d self, def: &Node) -> Option<Vec<&'d str>> {
        let F::List(body) = def.get("body") else { return None };
        let mut out = vec!["Module("];
        if !body.is_empty() {
            out.push("body=[");
            for (k, s) in body.iter().enumerate() {
                let F::Node(s) = s else { return None };
                if k > 0 {
                    out.push(", ");
                }
                out.push(self.get(s)?);
            }
            out.push("]");
        }
        out.push(")");
        Some(out)
    }

    /// `dump(args) + "|" + (dump(returns) or "None") + "|" + dump(ast.Tuple(elts=type_params))` of a def, in pieces.
    fn signature<'d>(&'d self, def: &Node) -> Option<Vec<&'d str>> {
        let mut out = vec![self.get(def.node("args")?)?, "|"];
        match def.get("returns") {
            F::Node(r) => out.push(self.get(r)?),
            F::Absent => out.push("None"),
            _ => return None,
        }
        out.push("|Tuple(");
        let F::List(params) = def.get("type_params") else { return None };
        if !params.is_empty() {
            out.push("elts=[");
            for (k, p) in params.iter().enumerate() {
                let F::Node(p) = p else { return None };
                if k > 0 {
                    out.push(", ");
                }
                out.push(self.get(p)?);
            }
            out.push("], ");
        }
        out.push("ctx=Load())");
        Some(out)
    }

    /// W's `body_sha256` of a def.
    pub(crate) fn body_sha256(&self, def: &Node) -> Option<String> {
        Some(digest(&self.body(def)?))
    }

    /// W's `signature_sha256` of a def.
    pub(crate) fn signature_sha256(&self, def: &Node) -> Option<String> {
        Some(digest(&self.signature(def)?))
    }

    /// The text `body_sha256` hashes, for tests.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn body_text(&self, def: &Node) -> Option<String> {
        Some(self.body(def)?.concat())
    }

    /// The text `signature_sha256` hashes, for tests.
    #[cfg(any(test, feature = "test-hooks"))]
    pub(crate) fn signature_text(&self, def: &Node) -> Option<String> {
        Some(self.signature(def)?.concat())
    }
}

fn digest(pieces: &[&str]) -> String {
    let mut h = Sha256::new();
    pieces.iter().for_each(|p| h.update(p.as_bytes()));
    hex(h.finalize().as_slice())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A constant's `repr` (`None` where it raises: an int of more than 4300 digits).
fn constant(c: &Const) -> Option<String> {
    Some(match c {
        Const::None => "None".into(),
        Const::Bool(true) => "True".into(),
        Const::Bool(false) => "False".into(),
        Const::Int(i) => i.decimal()?,
        Const::Float(f) => float_text(*f),
        Const::Complex(re, im) => complex_repr(*re, *im),
        Const::Str(s) => str_repr(s),
        Const::Bytes(b) => bytes_repr(b),
        Const::Ellipsis => "Ellipsis".into(),
    })
}
