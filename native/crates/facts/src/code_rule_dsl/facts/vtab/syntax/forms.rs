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
/// (as the run holds them, in their declared order), its words, and the texts of symbols those relations hold.
pub struct FormCall<'a> {
    pub batch: &'a Batch,
    pub keys: Option<&'a [i64]>,
    pub reads: &'a [Arc<dyn crate::code_rule_dsl::facts::vtab::Source>],
    pub args: &'a [&'a str],
    /// Each symbol's text, of the atoms a read relation's symbol columns hold.
    pub texts: &'a dyn Fn(&[i64]) -> Result<Vec<String>, String>,
}

/// A form's rows, flat, `width` cells each. A symbol cell holds `text`'s index of its text until the caller interns
/// the texts (once, all together) and writes each one's atom in its place; or an atom a relation it read holds
/// (`atom`), kept as it is.
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
    /// A symbol cell's value that is an atom already (a symbol column's value of a relation the form read).
    pub fn atom(a: i64) -> i64 {
        debug_assert!(a >= 0);
        -a - 1
    }
    /// A symbol cell's atom, its text's index made `ids`' atom.
    pub fn symbol(cell: i64, ids: &[i64]) -> i64 {
        if cell < 0 { -cell - 1 } else { ids[cell as usize] }
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
const TAPE: &[&dyn Form] = &[&SameSpan, &Unwrap, &NearestText, &Subtrees, &Reach, &FieldText, &FieldAndText, &Text, &Dotted, &NearestOf, &Walk];

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

/// `tape.subtrees reads roots`: `r(s)`, `s` a node of `roots` (its first column) or a node within one: each root's
/// subtree, its nodes in tape order (Python's `ann_ctx`, the annotations' nodes). Made whole, from the roots' rows.
pub struct Subtrees;

impl Form for Subtrees {
    fn name(&self) -> &'static str {
        "tape.subtrees"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let [roots] = call.reads else { return Err("tape.subtrees reads one relation of roots".into()) };
        let mut keys: Vec<i64> = (0..roots.len()).map(|r| roots.value(r, 0)).collect();
        keys.sort_unstable();
        keys.dedup();
        // A root under another root gives its nodes once: its subtree is the outer one's.
        let mut covered: Option<(*const Projected, u32)> = None;
        each_node(call.batch, Some(&keys), |p, local| {
            let f = p.f();
            let end = f.subtree_end(p.node(local));
            if covered.is_some_and(|(q, e)| std::ptr::eq(q, p) && p.node(local) < e) {
                return Ok(());
            }
            covered = Some((p as *const Projected, end));
            let mut s = local;
            while s < p.len() && p.node(s) < end {
                out.push(&[p.global(s)]);
                s += 1;
            }
            Ok(())
        })
    }
}

/// `tape.reach STEP… [reads roots [except]]`: `r(n)`, the nodes reached down from roots, each step from a node of a
/// kind to some of its children (Python's `pt_vpos`, the positions whose value is read, and `py_cond`, the conditions
/// tested). Its roots are `roots`' nodes (its first column) and each `^STEP`'s targets from every node of its kind;
/// a node of `except` (its first column) is no root. Each `STEP` is `KIND.FIELD` (its children under that field),
/// `KIND#I` (its `I`th child, as `child` counts) or `KIND#*` (every child), with these marks:
///
/// - `^STEP`: a root, from every node of `KIND` on the tape;
/// - `=STEP`: its targets enter as roots do (an `except` node is none);
/// - `STEP~WRAP`: a target of kind `WRAP` gives its first child in its place (`not (t)` tests `t`).
///
/// Made whole: every step goes down, and each node's steps are taken once, in tape order.
pub struct Reach;

/// One of `tape.reach`'s steps.
struct Step<'a> {
    from: usize,
    pick: Pick<'a>,
    root: bool,
    enters: bool,
    through: Option<usize>,
}

enum Pick<'a> {
    Field(&'a str),
    Child(usize),
    Every,
}

impl Form for Reach {
    fn name(&self) -> &'static str {
        "tape.reach"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        use crate::code_rule_dsl::facts::core::kind_tag;
        use std::cmp::Reverse;
        // The kinds the steps name, one set each, and the steps.
        fn name_at<'a>(n: &'a str, names: &mut Vec<&'a str>) -> usize {
            match names.iter().position(|m| *m == n) {
                Some(i) => i,
                None => {
                    names.push(n);
                    names.len() - 1
                }
            }
        }
        let mut names: Vec<&str> = Vec::new();
        let mut steps: Vec<Step<'_>> = Vec::new();
        for &w in call.args {
            let (root, w) = w.strip_prefix('^').map_or((false, w), |w| (true, w));
            let (enters, w) = w.strip_prefix('=').map_or((false, w), |w| (true, w));
            let (w, through) = w.split_once('~').map_or((w, None), |(w, t)| (w, Some(t)));
            let (from, pick) = if let Some((k, i)) = w.split_once('#') {
                (k, if i == "*" { Pick::Every } else { Pick::Child(i.parse().map_err(|_| format!("tape.reach: `{w}`'s child is no number"))?) })
            } else if let Some((k, f)) = w.split_once('.') {
                (k, Pick::Field(f))
            } else {
                return Err(format!("tape.reach: `{w}` is no step (`KIND.FIELD`, `KIND#I` or `KIND#*`)"));
            };
            let from = name_at(from, &mut names);
            let through = through.map(|t| name_at(t, &mut names));
            steps.push(Step { from, pick, root, enters, through });
        }
        if steps.is_empty() {
            return Err("tape.reach takes its steps".into());
        }
        let (roots, except) = match call.reads {
            [] => (None, None),
            [r] => (Some(r), None),
            [r, e] => (Some(r), Some(e)),
            _ => return Err("tape.reach reads its roots, then the nodes that are none".into()),
        };
        let sorted = |s: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| {
            let mut k: Vec<i64> = (0..s.len()).map(|r| s.value(r, 0)).collect();
            k.sort_unstable();
            k.dedup();
            k
        };
        let keys = roots.map(sorted).unwrap_or_default();
        let except = except.map(sorted).unwrap_or_default();
        let slices: Vec<[&str; 1]> = names.iter().map(|n| [*n]).collect();
        let mut kinds: Vec<KindSet<'_>> = slices.iter().map(|n| KindSet::new(n)).collect();
        // Each dialect's code of each step's field.
        let mut fields: Vec<(u8, Vec<Option<u16>>)> = Vec::new();
        let scan = steps.iter().any(|s| s.root);
        let mut heap: std::collections::BinaryHeap<Reverse<u32>> = std::collections::BinaryHeap::new();
        let mut targets: Vec<u32> = Vec::new();
        let mut doc = |p: &Projected, mine: &[i64]| -> Result<(), String> {
            let f = p.f();
            let kind = |l: u32| f.kind(p.node(l));
            let excluded = |l: u32| except.binary_search(&p.global(l)).is_ok();
            heap.clear();
            let mut hint = NONE;
            for &n in mine {
                if let Some(l) = p.local_of_near(n as u32, &mut hint).filter(|&l| l < p.len())
                    && !excluded(l)
                {
                    heap.push(Reverse(l));
                }
            }
            // One step's targets from `at` (of kind atom `k`), into `targets`.
            let mut take = |si: usize, at: u32, k: u32, kinds: &mut Vec<KindSet<'_>>, targets: &mut Vec<u32>| {
                let s = &steps[si];
                targets.clear();
                match s.pick {
                    Pick::Every => p.each_child(at, |c| targets.push(c)),
                    Pick::Child(i) => targets.extend(p.children(at).nth(i)),
                    Pick::Field(_) => {
                        let tag = kind_tag(k);
                        let at_tag = match fields.iter().position(|(t, _)| *t == tag) {
                            Some(i) => i,
                            None => {
                                let codes = crate::code_rule_dsl::language::from_tape_tag(tag).map(|d| d.kinds());
                                let row = steps.iter().map(|s| match (s.pick_field(), codes) {
                                    (Some(n), Some(c)) => c.field_id(n),
                                    _ => None,
                                });
                                fields.push((tag, row.collect()));
                                fields.len() - 1
                            }
                        };
                        let Some(code) = fields[at_tag].1[si] else { return };
                        f.each_child_field(p.node(at), |c, fc| {
                            if fc == code {
                                let c = p.local_of_node(c);
                                if c != NONE {
                                    targets.push(c);
                                }
                            }
                        });
                    }
                }
                if let Some(w) = s.through {
                    for t in targets.iter_mut() {
                        if kinds[w].has(kind(*t)) {
                            *t = p.children(*t).next().unwrap_or(NONE);
                        }
                    }
                    targets.retain(|&t| t != NONE);
                }
                if s.enters || s.root {
                    targets.retain(|&t| !excluded(t));
                }
            };
            if scan {
                for l in 0..p.len() {
                    let k = kind(l);
                    for si in 0..steps.len() {
                        if steps[si].root && kinds[steps[si].from].has(k) {
                            take(si, l, k, &mut kinds, &mut targets);
                            heap.extend(targets.iter().map(|&t| Reverse(t)));
                        }
                    }
                }
            }
            // In tape order: a node's every way in is from a node before it (its parent, or a root), so its copies
            // come off the heap together.
            let mut last = NONE;
            while let Some(Reverse(at)) = heap.pop() {
                if at == last {
                    continue;
                }
                last = at;
                out.push(&[p.global(at)]);
                let k = kind(at);
                for si in 0..steps.len() {
                    if !steps[si].root && kinds[steps[si].from].has(k) {
                        take(si, at, k, &mut kinds, &mut targets);
                        heap.extend(targets.iter().map(|&t| Reverse(t)));
                    }
                }
            }
            Ok(())
        };
        let of_doc = |n: i64| (n >> 32) as u32;
        if scan {
            for (i, p) in (&call.batch.docs).into_iter().enumerate() {
                if call.batch.docs.withheld(i) || p.len() == 0 {
                    continue;
                }
                let d = of_doc(p.global(0));
                let lo = keys.partition_point(|&n| of_doc(n) < d);
                let hi = keys.partition_point(|&n| of_doc(n) <= d);
                doc(p, &keys[lo..hi])?;
            }
        } else {
            let mut lo = 0;
            while lo < keys.len() {
                let d = of_doc(keys[lo]);
                let hi = lo + keys[lo..].partition_point(|&n| of_doc(n) == d);
                if let Some(i) = call.batch.docs.position(d)
                    && !call.batch.docs.withheld(i)
                {
                    doc(&call.batch.docs[i], &keys[lo..hi])?;
                }
                lo = hi;
            }
        }
        Ok(())
    }
}

impl Step<'_> {
    fn pick_field(&self) -> Option<&str> {
        match self.pick {
            Pick::Field(f) => Some(f),
            _ => None,
        }
    }
}

/// Fields by name, as a form's words give them: each dialect's codes of the names found once, by a node's kind atom.
pub struct FieldSet<'a> {
    names: &'a [&'a str],
    by_tag: Vec<(u8, Vec<Option<u16>>)>,
}

impl<'a> FieldSet<'a> {
    pub fn new(names: &'a [&'a str]) -> FieldSet<'a> {
        FieldSet { names, by_tag: Vec::new() }
    }
    /// The codes of the names, in their order, in the dialect of the node of kind atom `atom`.
    pub fn codes(&mut self, atom: u32) -> &[Option<u16>] {
        let tag = crate::code_rule_dsl::facts::core::kind_tag(atom);
        let at = match self.by_tag.iter().position(|(t, _)| *t == tag) {
            Some(i) => i,
            None => {
                let kinds = crate::code_rule_dsl::language::from_tape_tag(tag).map(|d| d.kinds());
                self.by_tag.push((tag, self.names.iter().map(|n| kinds.and_then(|k| k.field_id(n))).collect()));
                self.by_tag.len() - 1
            }
        };
        &self.by_tag[at].1
    }
    /// Whether `code` is one of the names' in the dialect of the node of kind atom `atom`.
    pub fn has(&mut self, atom: u32, code: u16) -> bool {
        code != 0 && self.codes(atom).contains(&Some(code))
    }
}

/// The words before a lone `/`, and those after it (none when there is no `/`).
fn halves<'a>(args: &'a [&'a str]) -> (&'a [&'a str], &'a [&'a str]) {
    match args.iter().position(|w| *w == "/") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

/// Every document of `batch` not withheld.
fn each_doc(batch: &Batch, mut each: impl FnMut(&Projected) -> Result<(), String>) -> Result<(), String> {
    for (i, p) in (&batch.docs).into_iter().enumerate() {
        if !batch.docs.withheld(i) {
            each(p)?;
        }
    }
    Ok(())
}

/// `tape.field_text KIND… / FIELD`: `r(n, t)`, `n` a node of a `KIND`, `t` the text of its child under `FIELD`
/// (Python's `def_name`: a definition's name). Made whole.
pub struct FieldText;

impl Form for FieldText {
    fn name(&self) -> &'static str {
        "tape.field_text"
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
        let (kinds, field @ [_]) = halves(call.args) else { return Err("tape.field_text takes the kinds, `/`, then a field".into()) };
        let (mut kinds, mut fields) = (KindSet::new(kinds), FieldSet::new(field));
        each_doc(call.batch, |p| {
            let f = p.f();
            for l in 0..p.len() {
                let x = p.node(l);
                let k = f.kind(x);
                if !kinds.has(k) {
                    continue;
                }
                let Some(code) = fields.codes(k)[0] else { continue };
                f.each_child_field(x, |c, fc| {
                    if fc == code && p.local_of_node(c) != NONE {
                        let t = out.text(f.text(c).to_owned());
                        out.push(&[p.global(l), t]);
                    }
                });
            }
            Ok(())
        })
    }
}

/// `tape.field_and_text KIND… / FIELD TEXT`: `r(n, c, t)`, `n` a node of a `KIND`, `c` its child under `FIELD` and
/// `t` the text of its child under `TEXT` (Python's `attr_of`: `o.a`'s object and attribute name). Made whole.
pub struct FieldAndText;

impl Form for FieldAndText {
    fn name(&self) -> &'static str {
        "tape.field_and_text"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "node", "sym"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let (kinds, fields @ [_, _]) = halves(call.args) else { return Err("tape.field_and_text takes the kinds, `/`, a node's field and a text's".into()) };
        let (mut kinds, mut fields) = (KindSet::new(kinds), FieldSet::new(fields));
        let (mut nodes, mut texts): (Vec<i64>, Vec<u32>) = (Vec::new(), Vec::new());
        each_doc(call.batch, |p| {
            let f = p.f();
            for l in 0..p.len() {
                let x = p.node(l);
                let k = f.kind(x);
                if !kinds.has(k) {
                    continue;
                }
                let [Some(of), Some(tf)] = *fields.codes(k) else { continue };
                nodes.clear();
                texts.clear();
                f.each_child_field(x, |c, fc| {
                    let cl = p.local_of_node(c);
                    if cl == NONE {
                        return;
                    }
                    if fc == of {
                        nodes.push(p.global(cl));
                    }
                    if fc == tf {
                        texts.push(c);
                    }
                });
                for &c in &nodes {
                    for &t in &texts {
                        let t = out.text(f.text(t).to_owned());
                        out.push(&[p.global(l), c, t]);
                    }
                }
            }
            Ok(())
        })
    }
}

/// `tape.text KIND… [/ FIELD…] [reads except…]`: `r(n, t)`, `n` a node of a `KIND` not hanging from its parent under
/// a `FIELD` nor a node of an `except` relation (its first column), and `t` its text (Python's `nameref`: an
/// identifier that names, not an attribute's or a definition's name). Made whole.
pub struct Text;

impl Form for Text {
    fn name(&self) -> &'static str {
        "tape.text"
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
        let (kinds, fields) = halves(call.args);
        let (mut kinds, mut fields) = (KindSet::new(kinds), FieldSet::new(fields));
        let mut except: Vec<i64> = call.reads.iter().flat_map(|r| (0..r.len()).map(|i| r.value(i, 0))).collect();
        except.sort_unstable();
        except.dedup();
        each_doc(call.batch, |p| {
            let f = p.f();
            for l in 0..p.len() {
                let x = p.node(l);
                let k = f.kind(x);
                if !kinds.has(k) || fields.has(k, f.field(x)) {
                    continue;
                }
                let n = p.global(l);
                if except.binary_search(&n).is_ok() {
                    continue;
                }
                let t = out.text(f.text(x).to_owned());
                out.push(&[n, t]);
            }
            Ok(())
        })
    }
}

/// `tape.dotted SCOPE… reads names bases`: `r(f, q)`, for each `names(f, x)`, `q` is `x` after the dotted path of
/// `f`'s nearest `SCOPE` ancestor `s`: `bases(s, m)`'s `m`, and `r(s, p)`'s `p` (Python's `qualname`: a definition's
/// path from its module, `pkg.mod.C.m`). `@dot`'s join: `a.x`, and `x` where `a` is empty. Made whole.
pub struct Dotted;

impl Form for Dotted {
    fn name(&self) -> &'static str {
        "tape.dotted"
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
        let [names, bases] = call.reads else { return Err("tape.dotted reads the names, then the scopes' bases".into()) };
        let mut scopes = KindSet::new(call.args);
        // Each relation's rows by node, their texts found once.
        let by_node = |r: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>| -> Result<Vec<(i64, String)>, String> {
            let atoms: Vec<i64> = (0..r.len()).map(|i| r.value(i, 1)).collect();
            let texts = (call.texts)(&atoms)?;
            let mut rows: Vec<(i64, String)> = (0..r.len()).map(|i| r.value(i, 0)).zip(texts).collect();
            rows.sort_unstable();
            rows.dedup();
            Ok(rows)
        };
        let (names, bases) = (by_node(names)?, by_node(bases)?);
        let of = |rows: &[(i64, String)], n: i64| {
            let lo = rows.partition_point(|r| r.0 < n);
            let hi = lo + rows[lo..].partition_point(|r| r.0 == n);
            lo..hi
        };
        let dot = |a: &str, x: &str| if a.is_empty() { x.to_owned() } else { format!("{a}.{x}") };
        // The scopes open at a node: each one's subtree's end, its node and its paths.
        let mut stack: Vec<(u32, i64, Vec<String>)> = Vec::new();
        let of_doc = |n: i64| (n >> 32) as u32;
        let mut lo = 0;
        while lo < names.len() {
            let d = of_doc(names[lo].0);
            let hi = lo + names[lo..].partition_point(|r| of_doc(r.0) == d);
            let mine = &names[lo..hi];
            lo = hi;
            let Some(i) = call.batch.docs.position(d).filter(|&i| !call.batch.docs.withheld(i)) else { continue };
            let p = &call.batch.docs[i];
            let f = p.f();
            stack.clear();
            for l in 0..p.len() {
                let x = p.node(l);
                while stack.last().is_some_and(|(end, _, _)| x >= *end) {
                    stack.pop();
                }
                let n = p.global(l);
                let own = of(mine, n);
                let mut paths: Vec<String> = Vec::new();
                if !own.is_empty()
                    && let Some((_, s, up)) = stack.last()
                {
                    for (_, name) in &mine[own.clone()] {
                        for (_, m) in &bases[of(&bases, *s)] {
                            paths.push(dot(m, name));
                        }
                        for q in up {
                            paths.push(dot(q, name));
                        }
                    }
                    paths.sort_unstable();
                    paths.dedup();
                    for q in &paths {
                        let t = out.text(q.clone());
                        out.push(&[n, t]);
                    }
                }
                if scopes.has(f.kind(x)) {
                    stack.push((f.subtree_end(x), n, paths));
                }
            }
        }
        Ok(())
    }
}

/// `tape.nearest_of SCOPE… reads refs [except]`: `r(n, x, s)`, for each `refs(n, x)` whose `n` is no node of
/// `except` (its first column), `s` the nearest ancestor of `n` of a `SCOPE` kind (Python's `pt_wnear`: where a name's
/// walk up its scopes starts). One pass down each document, the scopes open on a stack. Made whole.
pub struct NearestOf;

impl Form for NearestOf {
    fn name(&self) -> &'static str {
        "tape.nearest_of"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "sym", "node"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let (refs, except) = match call.reads {
            [r] => (r, None),
            [r, e] => (r, Some(e)),
            _ => return Err("tape.nearest_of reads the references, then the nodes that are none".into()),
        };
        let mut scopes = KindSet::new(call.args);
        let mut except: Vec<i64> = except.map_or_else(Vec::new, |e| (0..e.len()).map(|i| e.value(i, 0)).collect());
        except.sort_unstable();
        except.dedup();
        let mut rows: Vec<(i64, i64)> = (0..refs.len()).map(|i| (refs.value(i, 0), refs.value(i, 1))).filter(|(n, _)| except.binary_search(n).is_err()).collect();
        rows.sort_unstable();
        rows.dedup();
        let of_doc = |n: i64| (n >> 32) as u32;
        // A document's references by local id, and the scopes open where the pass is.
        let mut mine: Vec<(u32, i64, i64)> = Vec::new();
        let mut stack: Vec<(u32, i64)> = Vec::new();
        let mut lo = 0;
        while lo < rows.len() {
            let d = of_doc(rows[lo].0);
            let hi = lo + rows[lo..].partition_point(|r| of_doc(r.0) == d);
            let Some(i) = call.batch.docs.position(d).filter(|&i| !call.batch.docs.withheld(i)) else {
                lo = hi;
                continue;
            };
            let p = &call.batch.docs[i];
            let f = p.f();
            mine.clear();
            let mut hint = NONE;
            for &(n, x) in &rows[lo..hi] {
                if let Some(l) = p.local_of_near(n as u32, &mut hint).filter(|&l| l < p.len()) {
                    mine.push((l, n, x));
                }
            }
            lo = hi;
            mine.sort_unstable();
            stack.clear();
            let mut at = 0;
            for &(l, n, x) in &mine {
                // The scopes over `l`: those of every node before it, less those whose subtree ended.
                while at < l {
                    let y = p.node(at);
                    while stack.last().is_some_and(|(end, _)| y >= *end) {
                        stack.pop();
                    }
                    if scopes.has(f.kind(y)) {
                        stack.push((f.subtree_end(y), p.global(at)));
                    }
                    at += 1;
                }
                let y = p.node(l);
                while stack.last().is_some_and(|(end, _)| y >= *end) {
                    stack.pop();
                }
                if let Some(&(_, s)) = stack.last() {
                    out.push(&[n, FormRows::atom(x), s]);
                }
            }
        }
        Ok(())
    }
}

/// `rel.walk reads starts… bound up`: `r(n, x, s)`, each `starts(n, x, s)`'s, and `r(n, x, p)` for `r(n, x, s)` where
/// `bound(s, x)` is not and `up(s, p)` is (Python's `pt_walk`: a name's walk out through its scopes to the first that
/// binds it). Over the relations alone, no tape: each start's walk taken once. Made whole.
pub struct Walk;

impl Form for Walk {
    fn name(&self) -> &'static str {
        "rel.walk"
    }
    fn version(&self) -> u32 {
        1
    }
    fn columns(&self) -> &'static [&'static str] {
        &["node", "sym", "node"]
    }
    fn keyed(&self) -> &'static [usize] {
        &[]
    }
    fn rows(&self, call: &FormCall<'_>, _column: usize, out: &mut FormRows) -> Result<(), String> {
        let [starts @ .., bound, up] = call.reads else { return Err("rel.walk reads its starts, then what binds, then the steps up".into()) };
        if starts.is_empty() {
            return Err("rel.walk reads its starts, then what binds, then the steps up".into());
        }
        let bound: rustc_hash::FxHashSet<(i64, i64)> = (0..bound.len()).map(|i| (bound.value(i, 0), bound.value(i, 1))).collect();
        let mut up: Vec<(i64, i64)> = (0..up.len()).map(|i| (up.value(i, 0), up.value(i, 1))).collect();
        up.sort_unstable();
        up.dedup();
        let mut walk: Vec<i64> = Vec::new();
        let mut seen: Vec<i64> = Vec::new();
        for st in starts {
            for i in 0..st.len() {
                let (n, x, s) = (st.value(i, 0), st.value(i, 1), st.value(i, 2));
                walk.clear();
                seen.clear();
                walk.push(s);
                while let Some(s) = walk.pop() {
                    if seen.contains(&s) {
                        continue;
                    }
                    seen.push(s);
                    out.push(&[n, FormRows::atom(x), s]);
                    if bound.contains(&(s, x)) {
                        continue;
                    }
                    let lo = up.partition_point(|r| r.0 < s);
                    walk.extend(up[lo..].iter().take_while(|r| r.0 == s).map(|r| r.1));
                }
            }
        }
        Ok(())
    }
}
