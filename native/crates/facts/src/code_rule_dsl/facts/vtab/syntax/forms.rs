//! Rust forms of node functions (`.form r FORM …`, `front::ast::Native::Form`): a relation whose rows a registered
//! Rust function makes over a batch's documents, read in place on their tapes (decision 0353), for the values its
//! readers ask of one column (its demand), else every row. The relation's rules stay as its definition and the
//! forms' tests' oracle; a run reads the form.
//!
//! Forms that know no language are here (`tape.*`). A language's forms are its adapter crate's, registered at load
//! (`install_forms`) beside its families.

use super::{Batch, Projected};
use rerail_blocks::vector::Vector;
use rerail_clif::ids::NONE;
use std::sync::{Arc, OnceLock};

/// What a form is asked for: the batch, the keyed column's values asked (`None`: every row), the relations it reads
/// (as the run holds them, in their declared order) and its words.
pub struct FormCall<'a> {
    pub batch: &'a Batch,
    pub keys: Option<&'a [i64]>,
    pub reads: &'a [Arc<dyn crate::code_rule_dsl::facts::vtab::Source>],
    pub args: &'a [&'a str],
}

/// A form's rows, flat, `width` cells each. A symbol cell holds `text`'s index of its text until the caller interns
/// the texts (once, all together) and writes each one's atom in its place.
pub struct FormRows {
    pub width: usize,
    pub cells: Vector<i64>,
    pub texts: Vec<String>,
}

impl FormRows {
    pub fn new(width: usize) -> FormRows {
        FormRows { width, cells: Vector::new(), texts: Vec::new() }
    }
    pub fn push(&mut self, row: &[i64]) {
        debug_assert_eq!(row.len(), self.width);
        self.cells.extend_from_slice(row);
    }
    /// A symbol cell's value: its text's index, made an atom by the caller.
    pub fn text(&mut self, s: String) -> i64 {
        self.texts.push(s);
        (self.texts.len() - 1) as i64
    }
    pub fn len(&self) -> usize {
        self.cells.len() / self.width.max(1)
    }
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
}

/// A relation's Rust form, registered by name.
pub trait Form: Send + Sync {
    fn name(&self) -> &'static str;
    /// Its rows' version, in a relation's identity (`plan::identities`): raised when its Rust makes other rows.
    fn version(&self) -> u32;
    /// Its relation's columns' types in the engine's names (`node`, `int`, `sym`), checked against the relation's
    /// declaration.
    fn columns(&self) -> &'static [&'static str];
    /// The columns a demand may key it by: its rows for given values of one of them made without the others'.
    fn keyed(&self) -> &'static [usize];
    /// Its rows for `call`, keyed by column `column` where `call.keys` is given.
    fn rows(&self, call: &FormCall<'_>, column: usize, out: &mut FormRows) -> Result<(), String>;
}

static FORMS: OnceLock<Vec<&'static dyn Form>> = OnceLock::new();

/// The language-free forms.
const TAPE: &[&dyn Form] = &[&SameSpan, &Unwrap, &NearestText];

/// Registers a language's forms (once, at load), with the tape's.
pub fn install_forms(forms: Vec<&'static dyn Form>) {
    let _ = FORMS.set(TAPE.iter().copied().chain(forms).collect());
}

/// The form named `name`.
pub fn form(name: &str) -> Option<&'static dyn Form> {
    FORMS.get().map_or(TAPE, |f| f.as_slice()).iter().copied().find(|f| f.name() == name)
}

/// The documents of `keys` (node ids, `doc << 32 | low`) with each one's nodes' local ids, in key order; every
/// document's every node without keys. `each(p, local)` is called for each.
pub fn each_node(batch: &Batch, keys: Option<&[i64]>, mut each: impl FnMut(&Projected, u32) -> Result<(), String>) -> Result<(), String> {
    match keys {
        None => {
            for (i, p) in (&batch.docs).into_iter().enumerate() {
                if batch.docs.withheld(i) {
                    continue;
                }
                for local in 0..p.len() {
                    each(p, local)?;
                }
            }
        }
        Some(keys) => {
            let mut doc: Option<(u32, &Projected)> = None;
            let mut hint = NONE;
            for &n in keys {
                let key = (n >> 32) as u32;
                let p = match doc {
                    Some((k, p)) if k == key => p,
                    _ => {
                        let Some(i) = batch.docs.position(key) else { continue };
                        let p = &batch.docs[i];
                        doc = Some((key, p));
                        hint = NONE;
                        p
                    }
                };
                let Some(local) = p.local_of_near(n as u32, &mut hint).filter(|&l| l < p.len()) else { continue };
                each(p, local)?;
            }
        }
    }
    Ok(())
}

/// `tape.same_span`: `r(n, c)`, how many of `n`'s ancestors in an unbroken chain up from it span what it does (an
/// expression statement over its call, a `type` over its expression). Keyed by `n`.
pub struct SameSpan;

impl Form for SameSpan {
    fn name(&self) -> &'static str {
        "tape.same_span"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "int"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[0]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        each_node(call.batch, call.keys, |p, local| {
            let span = p.span(local);
            let (mut at, mut c) = (local, 0i64);
            loop {
                let up = p.parent(at as usize);
                if up == NONE || p.span(up) != span {
                    break;
                }
                c += 1;
                at = up;
            }
            out.push(&[p.global(local), c]);
            Ok(())
        })
    }
}

/// Kinds by name, as a form's words give them, tested on a node's kind atom: each dialect's codes of the names
/// found once, on its first node (a name may have several codes, its aliases').
pub struct KindSet<'a> {
    names: &'a [&'a str],
    by_tag: Vec<(u8, Vec<bool>)>,
}

impl<'a> KindSet<'a> {
    pub fn new(names: &'a [&'a str]) -> KindSet<'a> {
        KindSet { names, by_tag: Vec::new() }
    }
    /// Whether the node of kind atom `atom` (`FactsIn::kind`) is of one of the kinds.
    pub fn has(&mut self, atom: u32) -> bool {
        use crate::code_rule_dsl::facts::core::{kind_code, kind_tag};
        let tag = kind_tag(atom);
        let at = match self.by_tag.iter().position(|(t, _)| *t == tag) {
            Some(i) => i,
            None => {
                let codes = crate::code_rule_dsl::language::from_tape_tag(tag).map(|d| d.kinds()).map_or_else(Vec::new, |k| {
                    (0..k.kind_count()).map(|c| k.kind_name(c as u16).is_some_and(|n| self.names.contains(&n))).collect()
                });
                self.by_tag.push((tag, codes));
                self.by_tag.len() - 1
            }
        };
        self.by_tag[at].1.get(usize::from(kind_code(atom))).copied().unwrap_or(false)
    }
}

/// `tape.unwrap WRAP SKIP…`: `r(e, m)`, `m` is `e` with any wrappers of kind `WRAP` taken off, down through each of
/// its children not of a `SKIP` kind (Python's `unparen`: `((x))` gives `x`, a comment inside the parentheses
/// skipped); any other node gives itself. Keyed by `e` (down from it) and by `m` (up from it through its wrappers).
pub struct Unwrap;

impl Form for Unwrap {
    fn name(&self) -> &'static str {
        "tape.unwrap"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "node"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[0, 1]
    }
    fn rows(&self, call: &FormCall<'_>, column: usize, out: &mut FormRows) -> Result<(), String> {
        let [wrap, skip @ ..] = call.args else { return Err("tape.unwrap takes the wrapper's kind, then the kinds it skips".into()) };
        let wrap_names = [*wrap];
        let (mut wraps, mut skips) = (KindSet::new(&wrap_names), KindSet::new(skip));
        let mut stack: Vec<u32> = Vec::new();
        each_node(call.batch, call.keys, |p, local| {
            let kind = |l: u32| p.f().kind(p.node(l));
            if column == 1 && call.keys.is_some() {
                // Up from `m`: itself, then each wrapper over it while the chain is a wrapper's unskipped child.
                if wraps.has(kind(local)) {
                    return Ok(());
                }
                let m = p.global(local);
                out.push(&[m, m]);
                if skips.has(kind(local)) {
                    return Ok(());
                }
                let mut at = local;
                loop {
                    let up = p.parent(at as usize);
                    if up == NONE || !wraps.has(kind(up)) {
                        break;
                    }
                    out.push(&[p.global(up), m]);
                    at = up;
                }
                return Ok(());
            }
            // Down from `e`: through each wrapper's unskipped children, to the nodes that are no wrapper.
            let e = p.global(local);
            stack.clear();
            stack.push(local);
            while let Some(at) = stack.pop() {
                if !wraps.has(kind(at)) {
                    out.push(&[e, p.global(at)]);
                    continue;
                }
                for c in p.children(at) {
                    if !skips.has(kind(c)) {
                        stack.push(c);
                    }
                }
            }
            Ok(())
        })
    }
}

/// `tape.nearest_text KIND SCOPE… reads r`: `x(s, t)`, for each node `n` of relation `r` (its first column) of kind
/// `KIND`, its text `t` and its nearest ancestor `s` of a `SCOPE` kind (Python's `pt_tgt_name`: a target's name and
/// the scope it binds in). Made whole: it reads `r`'s rows, not the tape's every node.
pub struct NearestText;

impl Form for NearestText {
    fn name(&self) -> &'static str {
        "tape.nearest_text"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "sym"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let [kind_name, scopes @ ..] = call.args else { return Err("tape.nearest_text takes the nodes' kind, then the scopes' kinds".into()) };
        let [nodes] = call.reads else { return Err("tape.nearest_text reads one relation of nodes".into()) };
        let names = [*kind_name];
        let (mut kinds, mut scope) = (KindSet::new(&names), KindSet::new(scopes));
        let mut keys: Vec<i64> = (0..nodes.len()).map(|r| nodes.value(r, 0)).collect();
        keys.sort_unstable();
        keys.dedup();
        each_node(call.batch, Some(&keys), |p, local| {
            let f = p.f();
            let x = p.node(local);
            if !kinds.has(f.kind(x)) {
                return Ok(());
            }
            let mut at = local;
            loop {
                let up = p.parent(at as usize);
                if up == NONE {
                    return Ok(());
                }
                if scope.has(f.kind(p.node(up))) {
                    let t = out.text(f.text(x).to_owned());
                    out.push(&[p.global(up), t]);
                    return Ok(());
                }
                at = up;
            }
        })
    }
}
