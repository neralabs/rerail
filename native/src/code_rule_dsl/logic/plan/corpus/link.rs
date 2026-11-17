//! The programs of a run linked into one (decision 0089, 0091; 0281's path, step 7a-2), over parts that are trees
//! (`front::Program`: the tools, and a debug build's check) or versions (`Versioned`: the run). `combine_linked` is
//! written once over `Part`: what it reads of a part is its header (`Part::head`, the program less its rules) and
//! its rules' heads and reads; what it edits is the header and which rules are kept (`Part::drop_heads`). The link
//! copies each part's rules with their relations renamed (`Names`): a version's from its image, with no tree.

use crate::code_rule_dsl::logic::front::{self, Program};
use crate::code_rule_dsl::logic::program_image::build::Rewrite;
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Builder, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

type Result<T> = std::result::Result<T, crate::code_rule_dsl::logic::plan::Error>;

fn plan(e: impl ToString) -> crate::code_rule_dsl::logic::plan::Error {
    crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string())
}

/// A program of the run, as `combine_linked` reads and edits it.
pub(super) trait Part: Sized + Send {
    /// The programs linked.
    type Linked;
    /// What the link takes of a part's rules.
    type Rules;
    /// A part from its tree.
    fn of_tree(prog: Program) -> Result<Self>;
    /// A part lowered in its library's scope (`Text::lower_in`), given the run's rows (decision 0249).
    fn lowered_in(read: crate::code_rule_dsl::logic::rr::Text, text: &str, library: &crate::code_rule_dsl::logic::rr::Library, scoped: &crate::code_rule_dsl::logic::rr::Scoped, store: Option<&crate::content_store::ContentStore>, extra: &super::added::Added) -> Result<Self>;
    /// The library linked once (decision 0089).
    fn library(l: &crate::code_rule_dsl::logic::rr::Library) -> Result<Self>;
    /// The program less its rules: what is read of it besides them (a tree's is the program).
    fn head(&self) -> &Program;
    /// `head`, to edit.
    fn head_mut(&mut self) -> &mut Program;
    /// Each rule: its head's relation, and the relations its atoms and aggregates read.
    fn rules(&self) -> Vec<(String, Vec<String>)>;
    /// Whether a rule makes `rel`.
    fn has_head(&self, rel: &str) -> bool;
    fn rule_count(&self) -> usize;
    /// The rules of `heads` as text, as a request's key hashes them (each form's own text: keys of one form are
    /// compared alone).
    fn shown(&self, heads: &BTreeSet<String>) -> Vec<String>;
    /// Drops each rule whose head `gone` names.
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool);
    /// Bounded to the documents it shows (`hiding::hide`).
    fn hide(&mut self, bound: u32) -> Result<()>;
    /// `rr::lower::undo_on_request`.
    fn undo_on_request(&mut self);
    /// The part alone, as the linked program.
    fn alone(self) -> Result<Self::Linked>;
    /// Its header, taken out, and its rules, renamed by `names`.
    fn into_parts(self, names: &Names) -> (Program, Self::Rules);
    /// The linked program of `header` and each part's rules, in order. A rule `core.rr` gives (its head is the
    /// engine's) is taken once.
    fn link(header: Program, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<Self::Linked>;
}

/// The names a part's relations take in the linked program: its request's (`q<k>_`), its own (`p<i>_`), the axes
/// over its kind sets by the shared set's, the library's as they are (decision 0089 §2).
pub(super) struct Names {
    pub(super) i: usize,
    pub(super) own: BTreeSet<String>,
    pub(super) request: Option<(usize, BTreeSet<String>)>,
    pub(super) set_of: BTreeMap<String, String>,
}

impl Names {
    pub(super) fn rename(&self, rel: &str) -> String {
        if let Some((k, held)) = &self.request
            && held.contains(rel)
        {
            return format!("q{k}_{rel}");
        }
        if self.own.contains(rel) {
            return format!("p{}_{rel}", self.i);
        }
        // Both axes name their set: `inside`'s (`rr_inside_<set>`) and `has`'s (`rr_has_<set>`). A `has` left on its
        // own name kept a set the merge renamed, so guest hiding could not name it in the guest's grammar, and
        // `has(n, c, "identifier")` found no guest node once two programs ran together.
        match front::axis(rel).and_then(|(set, has)| self.set_of.get(set).map(|s| (s, has))) {
            Some((shared, has)) => format!("{}{shared}", if has { front::DESCENDANTS } else { front::AXIS }),
            None => rel.to_owned(),
        }
    }
}

/// The header `prog` (its rules taken out) merged into `all`'s, its relations renamed by `names`.
pub(super) fn link_header(all: &mut Program, prog: Program, names: &Names) {
    let rename = |rel: &str| names.rename(rel);
    all.nullary.extend(prog.nullary.iter().map(|rel| rename(rel)));
    for (rel, cols) in prog.decls {
        all.decls.entry(rename(&rel).into()).or_insert(cols);
    }
    for (rel, decl) in prog.lattice {
        all.lattice.insert(rename(&rel), decl);
    }
    for (rel, rows) in prog.facts {
        all.facts.insert(rename(&rel), rows);
    }
    for (rel, words) in prog.qualifiers {
        all.qualifiers.entry(rename(&rel)).or_default().extend(words);
    }
    for (rel, cols) in prog.tapes {
        all.tapes.entry(rename(&rel)).or_default().extend(cols);
    }
    for (rel, mut native) in prog.natives {
        native.rename_reads(rename);
        all.natives.insert(rename(&rel), native);
    }
    for list in [(&mut all.inputs, prog.inputs), (&mut all.outputs, prog.outputs), (&mut all.demand, prog.demand)] {
        for rel in list.1 {
            let rel = rename(&rel);
            if !list.0.contains(&rel) {
                list.0.push(rel);
            }
        }
    }
    all.policies.extend(prog.policies);
    // Its join shape directives, on its relations as renamed (O-join-shape-directives).
    for mut shape in prog.shapes {
        shape.head = rename(&shape.head);
        shape.lit = rename(&shape.lit);
        if let front::ShapeWait::After(a) = &mut shape.wait {
            *a = rename(a);
        }
        if !all.shapes.contains(&shape) {
            all.shapes.push(shape);
        }
    }
    // What the batch makes for the program's rules: its regexes over spans, its AST patterns.
    all.rx_spans.extend(prog.rx_spans);
    all.ast_patterns.extend(prog.ast_patterns);
    for mut setting in prog.settings {
        if matches!(setting.0.as_str(), "input" | "output")
            && let Some((rel, params)) = setting.1.split_once('(')
        {
            setting.1 = format!("{}({params}", rename(rel));
        }
        if !all.settings.contains(&setting) {
            all.settings.push(setting);
        }
    }
}

impl Part for Program {
    type Linked = Program;
    type Rules = Vec<front::Rule>;
    fn of_tree(prog: Program) -> Result<Self> {
        Ok(prog)
    }
    fn lowered_in(read: crate::code_rule_dsl::logic::rr::Text, text: &str, library: &crate::code_rule_dsl::logic::rr::Library, scoped: &crate::code_rule_dsl::logic::rr::Scoped, store: Option<&crate::content_store::ContentStore>, extra: &super::added::Added) -> Result<Self> {
        let mut prog = read.lower_in(text, library, scoped, store)?.expect("a modular text");
        extra.add_to(&mut prog);
        Ok(prog)
    }
    fn library(l: &crate::code_rule_dsl::logic::rr::Library) -> Result<Self> {
        Ok(l.program().clone())
    }
    fn head(&self) -> &Program {
        self
    }
    fn head_mut(&mut self) -> &mut Program {
        self
    }
    fn rules(&self) -> Vec<(String, Vec<String>)> {
        self.rules
            .iter()
            .map(|r| {
                let reads = r
                    .body
                    .iter()
                    .filter_map(|l| match l {
                        front::Literal::Atom(a) | front::Literal::Agg { atom: a, .. } => Some(a.rel.to_string()),
                        _ => None,
                    })
                    .collect();
                (r.head.rel.to_string(), reads)
            })
            .collect()
    }
    fn has_head(&self, rel: &str) -> bool {
        self.rules.iter().any(|r| r.head.rel == rel)
    }
    fn rule_count(&self) -> usize {
        self.rules.len()
    }
    fn shown(&self, heads: &BTreeSet<String>) -> Vec<String> {
        self.rules.iter().filter(|r| heads.contains(r.head.rel.as_str())).map(|r| format!("{r:?}")).collect()
    }
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool) {
        self.rules.retain(|r| !gone(r.head.rel.as_str()));
    }
    fn hide(&mut self, bound: u32) -> Result<()> {
        super::hiding::hide(self, bound);
        Ok(())
    }
    fn undo_on_request(&mut self) {
        crate::code_rule_dsl::logic::rr::lower::undo_on_request(self);
    }
    fn alone(self) -> Result<Self::Linked> {
        Ok(self)
    }
    fn into_parts(mut self, names: &Names) -> (Program, Self::Rules) {
        let mut rules = std::mem::take(&mut self.rules);
        let atom = |a: &mut front::Atom| a.rel = names.rename(&a.rel).into();
        for rule in &mut rules {
            atom(&mut rule.head);
            for l in &mut rule.body {
                match l {
                    front::Literal::Atom(a) => atom(a),
                    front::Literal::Agg { atom: a, .. } => atom(a),
                    front::Literal::Nearest { kindset, .. } => {
                        if let Some(shared) = names.set_of.get(kindset) {
                            *kindset = shared.clone();
                        }
                    }
                    front::Literal::Cmp { .. } | front::Literal::Ast { .. } => {}
                }
            }
        }
        (self, rules)
    }
    fn link(mut header: Program, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<Self::Linked> {
        for rule in rules.into_iter().flatten() {
            if !(core.contains(rule.head.rel.as_str()) && header.rules.contains(&rule)) {
                header.rules.push(rule);
            }
        }
        Ok(header)
    }
}

/// A part as a version: its image (a stored one handed on unread), its header as edited, and the relations whose
/// rules are dropped. The edits are made on the version where a pass reads its rules (`settle`), or at the link,
/// which copies the rules kept under the header as edited.
pub(super) struct Versioned {
    v: Arc<Version>,
    was: Program,
    head: Program,
    edited: bool,
    dropped: BTreeSet<String>,
}

impl Versioned {
    fn of(v: Arc<Version>) -> Result<Self> {
        let was = v.header(&Default::default()).map_err(plan)?;
        Ok(Versioned { v, head: was.clone(), was, edited: false, dropped: BTreeSet::new() })
    }
    fn kept(&self) -> impl Iterator<Item = crate::code_rule_dsl::logic::program_image::RuleRef<'_>> {
        self.v.rules().filter(|r| !self.dropped.contains(self.v.rel_name(r.head().rel())))
    }
    /// The edits made on the version.
    fn settle(&mut self) -> Result<()> {
        if !self.edited && self.dropped.is_empty() {
            return Ok(());
        }
        let v = Arc::clone(&self.v);
        let mut b = Builder::on(&v);
        b.set_header(&self.was, &self.head);
        b.retain(|r| !self.dropped.contains(v.rel_name(r.head().rel())));
        self.v = b.finish().map_err(plan)?.0;
        self.was = self.head.clone();
        self.edited = false;
        self.dropped.clear();
        Ok(())
    }
}

/// A version part's rules as `undo_on_request` reads and drops them.
struct Kept<'a> {
    v: &'a Version,
    dropped: &'a mut BTreeSet<String>,
}

impl crate::code_rule_dsl::logic::rr::lower::RuleSet for Kept<'_> {
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool) {
        for r in self.v.rules() {
            let head = self.v.rel_name(r.head().rel());
            if gone(head) {
                self.dropped.insert(head.to_owned());
            }
        }
    }
    fn reads(&self, rel: &str) -> bool {
        use crate::code_rule_dsl::logic::program_image::Lit;
        self.v
            .rules()
            .filter(|r| !self.dropped.contains(self.v.rel_name(r.head().rel())))
            .flat_map(|r| r.body())
            .any(|l| matches!(l.view(), Lit::Atom(a) if self.v.rel_name(a.rel()) == rel))
    }
}

/// The renames of a part's rules as they are copied (`Names`).
struct Renaming<'a> {
    v: &'a Version,
    names: &'a Names,
}

impl Rewrite for Renaming<'_> {
    fn head_as(&mut self, a: AtomRef<'_>) -> Option<String> {
        let rel = self.v.rel_name(a.rel());
        Some(self.names.rename(rel)).filter(|r| r != rel)
    }
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        let rel = self.v.rel_name(a.rel());
        Some(self.names.rename(rel)).filter(|r| r != rel).map(|r| (r, a.neg(), a.gated()))
    }
    fn kindset(&mut self, set: AtomId) -> Option<String> {
        self.names.set_of.get(self.v.text(set)).cloned()
    }
}

impl Part for Versioned {
    type Linked = Arc<Version>;
    type Rules = (Arc<Version>, BTreeSet<String>, Names);
    fn of_tree(prog: Program) -> Result<Self> {
        let (v, _) = Builder::of_program(&prog).and_then(|b| b.finish()).map_err(plan)?;
        Versioned::of(v)
    }
    fn lowered_in(read: crate::code_rule_dsl::logic::rr::Text, text: &str, library: &crate::code_rule_dsl::logic::rr::Library, scoped: &crate::code_rule_dsl::logic::rr::Scoped, store: Option<&crate::content_store::ContentStore>, extra: &super::added::Added) -> Result<Self> {
        let v = read.lower_in_version(text, library, scoped, store)?.expect("a modular text");
        let mut part = Versioned::of(v)?;
        extra.add_to(part.head_mut());
        Ok(part)
    }
    fn library(l: &crate::code_rule_dsl::logic::rr::Library) -> Result<Self> {
        Versioned::of(Arc::clone(&l.version))
    }
    fn head(&self) -> &Program {
        &self.head
    }
    fn head_mut(&mut self) -> &mut Program {
        self.edited = true;
        &mut self.head
    }
    fn rules(&self) -> Vec<(String, Vec<String>)> {
        self.kept().map(|r| (self.v.rel_name(r.head().rel()).to_owned(), r.reads().map(|a| self.v.rel_name(a.rel()).to_owned()).collect())).collect()
    }
    fn has_head(&self, rel: &str) -> bool {
        !self.dropped.contains(rel) && self.v.rel(rel).is_some_and(|id| self.v.heads(id).next().is_some())
    }
    fn rule_count(&self) -> usize {
        self.kept().count()
    }
    fn shown(&self, heads: &BTreeSet<String>) -> Vec<String> {
        self.kept().filter(|r| heads.contains(self.v.rel_name(r.head().rel()))).map(|r| self.v.show_rule(r)).collect()
    }
    fn drop_heads(&mut self, gone: &dyn Fn(&str) -> bool) {
        let heads: Vec<String> = self.kept().map(|r| self.v.rel_name(r.head().rel()).to_owned()).filter(|h| gone(h)).collect();
        self.dropped.extend(heads);
    }
    fn hide(&mut self, bound: u32) -> Result<()> {
        self.settle()?;
        let mut p = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("hide", Arc::clone(&self.v));
        p.program(&mut super::preparation::hide::Hide { bound });
        let (v, _) = p.finish().map_err(plan)?;
        *self = Versioned::of(v)?;
        Ok(())
    }
    fn undo_on_request(&mut self) {
        self.edited = true;
        let Versioned { v, head, dropped, .. } = self;
        crate::code_rule_dsl::logic::rr::lower::undo_on_request_in(head, &mut Kept { v, dropped });
    }
    fn alone(mut self) -> Result<Self::Linked> {
        self.settle()?;
        Ok(self.v)
    }
    fn into_parts(self, names: &Names) -> (Program, Self::Rules) {
        let names = Names { i: names.i, own: names.own.clone(), request: names.request.clone(), set_of: names.set_of.clone() };
        (self.head, (self.v, self.dropped, names))
    }
    fn link(header: Program, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<Self::Linked> {
        let mut b = Builder::of_program(&header).map_err(plan)?;
        for (v, dropped, names) in &rules {
            for r in v.rules().filter(|r| !dropped.contains(v.rel_name(r.head().rel()))) {
                b.rewrite(v, r, &mut Renaming { v, names }).finish().map_err(plan)?;
            }
        }
        let (linked, _) = b.finish().map_err(plan)?;
        // A rule `core.rr` gives, taken once: the copies after the first dropped.
        let mut seen = BTreeSet::new();
        let repeats: BTreeSet<_> = linked.rules().filter(|r| core.contains(linked.rel_name(r.head().rel())) && !seen.insert(linked.show_rule(*r))).map(|r| r.id()).collect();
        if repeats.is_empty() {
            return Ok(linked);
        }
        let mut b = Builder::on(&linked);
        b.retain(|r| !repeats.contains(&r.id()));
        Ok(b.finish().map_err(plan)?.0)
    }
}
