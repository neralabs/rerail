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
const TAPE: &[&dyn Form] = &[&SameSpan];

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
