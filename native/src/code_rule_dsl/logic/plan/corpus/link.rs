//! The programs of a run linked into one (decision 0089, 0091; 0281's path, step 7a-2), over parts that are versions
//! (`Versioned`). `combine_linked_as` is written over `Part`: what it reads of a part is its header (`Part::head`, the program less its rules) and
//! its rules' heads and reads; what it edits is the header and which rules are kept (`Part::drop_heads`). The link
//! copies each part's rules with their relations renamed (`Names`): a version's from its image, with no tree.

use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::program_image::build::Rewrite;
use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut, ProgramFields};
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Builder, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

type Result<T> = std::result::Result<T, crate::code_rule_dsl::logic::plan::Error>;

fn plan(e: impl ToString) -> crate::code_rule_dsl::logic::plan::Error {
    crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string())
}

/// A program of the run, as `combine_linked_as` reads and edits it.
pub(super) trait Part: Sized + Send {
    /// The programs linked.
    type Linked;
    /// Whether a link of this form is kept (decision 0373): `link_counted`, `linked_version` and `spliced` are.
    const KEEPS: bool = false;
    /// What the link takes of a part's rules.
    type Rules;
    /// A part from its version.
    fn of_version(v: Arc<Version>) -> Result<Self>;
    /// A part lowered in its library's scope (`Text::lower_in`), given the run's rows (decision 0249); `found`, its
    /// image as the caller read it with the others' (`images::read_all`).
    fn lowered_in(read: crate::code_rule_dsl::logic::rr::Text, text: &str, library: &crate::code_rule_dsl::logic::rr::Library, scoped: &crate::code_rule_dsl::logic::rr::Scoped, store: Option<&crate::content_store::ContentStore>, found: Option<Option<crate::code_rule_dsl::logic::rr::images::Found>>, extra: &super::added::Added) -> Result<Self>;
    /// The library linked once (decision 0089).
    fn library(l: &crate::code_rule_dsl::logic::rr::Library) -> Result<Self>;
    /// The linked program's fields before its rules, which `Merged` merges each part's into.
    type Fields: FieldsMut;
    /// No fields.
    fn fields() -> Self::Fields;
    /// A part's header as `into_parts` takes it out.
    type Header;
    /// The header's fields.
    fn header_fields(h: &Self::Header) -> &dyn ProgramFields;
    /// The program's fields: what is read of it besides its rules.
    fn head(&self) -> &dyn ProgramFields;
    /// `f` on the program's fields, to edit them.
    fn edit_head(&mut self, f: &mut dyn FnMut(&mut dyn FieldsMut)) -> Result<()>;
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
    /// Bounded to the documents it shows (`preparation::hide::Hide`).
    fn hide(&mut self, bound: u32) -> Result<()>;
    /// `rr::lower::undone_on_request`.
    fn undo_on_request(&mut self) -> Result<()>;
    /// The part alone, as the linked program.
    fn alone(self) -> Result<Self::Linked>;
    /// Its header, taken out, and its rules, renamed by `names`.
    fn into_parts(self, names: &Names) -> (Self::Header, Self::Rules);
    /// The linked program of `fields` and each part's rules, in order. A rule `core.rr` gives (its head is the
    /// engine's) is taken once.
    fn link(fields: Self::Fields, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<Self::Linked>;
    /// `link`, and how many of the linked rules are each part's (in the parts' order), where a link of this form is
    /// kept (decision 0373: `Versioned`'s).
    fn link_counted(fields: Self::Fields, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<(Self::Linked, Option<Vec<usize>>)> {
        Ok((Self::link(fields, rules, core)?, None))
    }
    /// The linked program as the version a kept link holds, where it is one.
    fn linked_version(_linked: &Self::Linked) -> Option<&Arc<Version>> {
        None
    }
    /// The last link `last`, whose parts' rules are `counts` of its rules in order, with each of `changed` (a part's
    /// index and its rules) in its part's place, and the parts' counts after: what `link_counted` gives where the
    /// changed parts' interfaces are their last ones. `None` where the form keeps no link or `counts` are not `last`'s.
    fn spliced(_last: &Arc<Version>, _counts: &[usize], _changed: Vec<(usize, Self::Rules)>, _core: &BTreeSet<String>) -> Result<Option<(Self::Linked, Vec<usize>)>> {
        Ok(None)
    }
}

/// `linked` without the repeats of a rule `core.rr` gives after its first (`Part::link`), and each part's count of
/// rules after (`counts`, the parts' rules in order, before).
fn without_core_repeats(linked: Arc<Version>, counts: &[usize], core: &BTreeSet<String>) -> Result<(Arc<Version>, Vec<usize>)> {
    let mut seen = BTreeSet::new();
    let mut repeats = BTreeSet::new();
    let mut counts = counts.to_vec();
    let (mut part, mut left) = (0usize, counts.first().copied().unwrap_or(0));
    for r in linked.rules() {
        while left == 0 && part + 1 < counts.len() {
            part += 1;
            left = counts[part];
        }
        left = left.saturating_sub(1);
        if core.contains(linked.rel_name(r.head().rel())) && !seen.insert(linked.show_rule(r)) {
            repeats.insert(r.id());
            counts[part] -= 1;
        }
    }
    if repeats.is_empty() {
        return Ok((linked, counts));
    }
    let mut b = Builder::on(&linked);
    b.retain(|r| !repeats.contains(&r.id()));
    Ok((b.finish().map_err(plan)?.0, counts))
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

/// The parts' fields merged (`merge`) into the linked program's (`all`), each part's relations renamed by its
/// names: declarations, rows, inputs and outputs stated on `all` as they come, the fields of lists and maps gathered
/// and stated once (`finish`). The combine states the documents read, the text globs and the shared kind sets.
pub(super) struct Merged<F: FieldsMut> {
    all: F,
    pub(super) doc_scopes: Option<Vec<(Vec<String>, Vec<String>)>>,
    pub(super) text_globs: Vec<String>,
    pub(super) kindsets: BTreeMap<String, Vec<String>>,
    nullary: BTreeSet<String>,
    lattice: BTreeMap<String, front::ast::LatticeDecl>,
    qualifiers: BTreeMap<String, BTreeSet<String>>,
    tapes: BTreeMap<String, BTreeSet<usize>>,
    natives: BTreeMap<String, front::ast::Native>,
    demand: Vec<String>,
    policies: Vec<serde_json::Value>,
    shapes: Vec<front::ast::Shape>,
    rx_spans: BTreeSet<(front::ast::SpanIn, String)>,
    ast_patterns: BTreeSet<String>,
    settings: Vec<(String, String)>,
}

impl<F: FieldsMut> Merged<F> {
    pub(super) fn new(all: F) -> Self {
        Merged {
            all,
            doc_scopes: None,
            text_globs: Vec::new(),
            kindsets: BTreeMap::new(),
            nullary: BTreeSet::new(),
            lattice: BTreeMap::new(),
            qualifiers: BTreeMap::new(),
            tapes: BTreeMap::new(),
            natives: BTreeMap::new(),
            demand: Vec::new(),
            policies: Vec::new(),
            shapes: Vec::new(),
            rx_spans: BTreeSet::new(),
            ast_patterns: BTreeSet::new(),
            settings: Vec::new(),
        }
    }

    /// A part's fields `prog` merged, its relations renamed by `names`. Its documents read, text globs, kind sets and
    /// globals (`front::GLOBAL` settings) are the combine's, not merged.
    pub(super) fn merge(&mut self, prog: &dyn ProgramFields, names: &Names) {
        let rename = |rel: &str| names.rename(rel);
        self.nullary.extend(prog.nullary().iter().map(|rel| rename(rel)));
        for rel in prog.declared_names() {
            let to = rename(&rel);
            if !self.all.declares(&to) {
                let cols = prog.columns_of(&rel).unwrap_or_default();
                let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
                self.all.declare(&to, &pairs);
            }
        }
        for (rel, decl) in prog.lattice() {
            self.lattice.insert(rename(&rel), decl);
        }
        for rel in prog.fact_names() {
            let rows = prog.fact_rows(&rel);
            self.all.set_facts(&rename(&rel), &rows);
        }
        for (rel, words) in prog.qualifiers() {
            self.qualifiers.entry(rename(&rel)).or_default().extend(words);
        }
        for (rel, cols) in prog.tapes() {
            self.tapes.entry(rename(&rel)).or_default().extend(cols);
        }
        for (rel, mut native) in prog.natives() {
            native.rename_reads(rename);
            self.natives.insert(rename(&rel), native);
        }
        for rel in prog.input_names() {
            self.all.add_input(&rename(&rel));
        }
        for rel in prog.output_names() {
            self.all.add_output(&rename(&rel));
        }
        for rel in prog.demand() {
            let rel = rename(&rel);
            if !self.demand.contains(&rel) {
                self.demand.push(rel);
            }
        }
        self.policies.extend(prog.policies());
        // Its join shape directives, on its relations as renamed (O-join-shape-directives).
        for mut shape in prog.shapes() {
            shape.head = rename(&shape.head);
            shape.lit = rename(&shape.lit);
            if let front::ShapeWait::After(a) = &mut shape.wait {
                *a = rename(a);
            }
            if !self.shapes.contains(&shape) {
                self.shapes.push(shape);
            }
        }
        // What the batch makes for the program's rules: its regexes over spans, its AST patterns.
        self.rx_spans.extend(prog.rx_spans());
        self.ast_patterns.extend(prog.ast_patterns());
        for mut setting in prog.settings() {
            if setting.0 == front::GLOBAL {
                continue;
            }
            if matches!(setting.0.as_str(), "input" | "output")
                && let Some((rel, params)) = setting.1.split_once('(')
            {
                setting.1 = format!("{}({params}", rename(rel));
            }
            if !self.settings.contains(&setting) {
                self.settings.push(setting);
            }
        }
    }

    /// The linked fields, every gathered one stated.
    pub(super) fn finish(mut self) -> F {
        use crate::code_rule_dsl::logic::program_facts::Field;
        let all = &mut self.all;
        if !self.nullary.is_empty() {
            all.set_field(Field::Nullary(&self.nullary));
        }
        if !self.lattice.is_empty() {
            all.set_field(Field::Lattice(&self.lattice));
        }
        if !self.qualifiers.is_empty() {
            all.set_field(Field::Qualifiers(&self.qualifiers));
        }
        if !self.tapes.is_empty() {
            all.set_field(Field::Tapes(&self.tapes));
        }
        if !self.natives.is_empty() {
            all.set_field(Field::Natives(&self.natives));
        }
        if !self.demand.is_empty() {
            all.set_field(Field::Demand(&self.demand));
        }
        if !self.policies.is_empty() {
            all.set_field(Field::Policies(&self.policies));
        }
        if !self.shapes.is_empty() {
            all.set_field(Field::Shapes(&self.shapes));
        }
        if !self.rx_spans.is_empty() {
            all.set_field(Field::RxSpans(&self.rx_spans));
        }
        if !self.ast_patterns.is_empty() {
            all.set_field(Field::AstPatterns(&self.ast_patterns));
        }
        // Stated if empty too: the linked fields read back as a program's (`Builder::set_field`'s program row).
        all.set_field(Field::Settings(&self.settings));
        if self.doc_scopes.is_some() {
            all.set_field(Field::DocScopes(self.doc_scopes.as_deref()));
        }
        if !self.text_globs.is_empty() {
            all.set_field(Field::TextGlobs(&self.text_globs));
        }
        if !self.kindsets.is_empty() {
            all.set_field(Field::Kindsets(&self.kindsets));
        }
        self.all
    }
}

/// A part as a version: its image (a stored one handed on unread), its fields' edits made on it as an overlay
/// (`edit_head`), and the relations whose rules are dropped. The drops are made on the version where a pass reads
/// its rules (`settle`), or at the link, which copies the rules kept.
pub(super) struct Versioned {
    v: Arc<Version>,
    dropped: BTreeSet<String>,
}

impl Versioned {
    fn of(v: Arc<Version>) -> Self {
        Versioned { v, dropped: BTreeSet::new() }
    }
    fn kept(&self) -> impl Iterator<Item = crate::code_rule_dsl::logic::program_image::RuleRef<'_>> {
        self.v.rules().filter(|r| !self.dropped.contains(self.v.rel_name(r.head().rel())))
    }
    /// The drops made on the version.
    fn settle(&mut self) -> Result<()> {
        if self.dropped.is_empty() {
            return Ok(());
        }
        let v = Arc::clone(&self.v);
        let mut b = Builder::on(&v);
        b.retain(|r| !self.dropped.contains(v.rel_name(r.head().rel())));
        self.v = b.finish().map_err(plan)?.0;
        self.dropped.clear();
        Ok(())
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
    const KEEPS: bool = true;
    type Rules = (Arc<Version>, BTreeSet<String>, Names);
    type Fields = Builder<'static>;
    fn fields() -> Builder<'static> {
        Builder::base()
    }
    type Header = Arc<Version>;
    fn header_fields(h: &Arc<Version>) -> &dyn ProgramFields {
        &**h
    }
    fn of_version(v: Arc<Version>) -> Result<Self> {
        Ok(Versioned::of(v))
    }
    fn lowered_in(read: crate::code_rule_dsl::logic::rr::Text, text: &str, library: &crate::code_rule_dsl::logic::rr::Library, scoped: &crate::code_rule_dsl::logic::rr::Scoped, store: Option<&crate::content_store::ContentStore>, found: Option<Option<crate::code_rule_dsl::logic::rr::images::Found>>, extra: &super::added::Added) -> Result<Self> {
        let v = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "a program's version in scope: its image, or lowered".into());
            read.lower_in_version(text, library, scoped, store, found)?.expect("a modular text")
        };
        let _s = crate::code_rule_dsl::logic::profile::span(|| "a program's part: its header".into());
        let mut part = Versioned::of(v);
        part.edit_head(&mut |b| extra.add_to(b))?;
        Ok(part)
    }
    fn library(l: &crate::code_rule_dsl::logic::rr::Library) -> Result<Self> {
        Ok(Versioned::of(Arc::clone(&l.version)))
    }
    fn head(&self) -> &dyn ProgramFields {
        &*self.v
    }
    fn edit_head(&mut self, f: &mut dyn FnMut(&mut dyn FieldsMut)) -> Result<()> {
        let edited = {
            let mut b = Builder::on(&self.v);
            f(&mut b);
            if b.is_empty() { None } else { Some(b.finish().map_err(plan)?.0) }
        };
        if let Some(v) = edited {
            self.v = v;
        }
        Ok(())
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
        *self = Versioned::of(v);
        Ok(())
    }
    fn undo_on_request(&mut self) -> Result<()> {
        let Versioned { v, dropped } = self;
        let edited = {
            let mut b = Builder::on(v);
            crate::code_rule_dsl::logic::rr::lower::undo_on_request_in(&mut b, &mut crate::code_rule_dsl::logic::rr::lower::KeptRules { v, dropped });
            if b.is_empty() { None } else { Some(b.finish().map_err(plan)?.0) }
        };
        if let Some(e) = edited {
            *v = e;
        }
        Ok(())
    }
    fn alone(mut self) -> Result<Self::Linked> {
        self.settle()?;
        Ok(self.v)
    }
    fn into_parts(self, names: &Names) -> (Arc<Version>, Self::Rules) {
        let names = Names { i: names.i, own: names.own.clone(), request: names.request.clone(), set_of: names.set_of.clone() };
        (Arc::clone(&self.v), (self.v, self.dropped, names))
    }
    fn link(fields: Builder<'static>, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<Self::Linked> {
        Ok(Self::link_counted(fields, rules, core)?.0)
    }
    fn link_counted(fields: Builder<'static>, rules: Vec<Self::Rules>, core: &BTreeSet<String>) -> Result<(Self::Linked, Option<Vec<usize>>)> {
        let mut b = fields;
        // The fields hold no rules: the first count (the header's own rules, in the kept link's record) is none.
        let mut counts = vec![0];
        for (v, dropped, names) in &rules {
            let mut n = 0;
            for r in v.rules().filter(|r| !dropped.contains(v.rel_name(r.head().rel()))) {
                b.rewrite(v, r, &mut Renaming { v, names }).finish().map_err(plan)?;
                n += 1;
            }
            counts.push(n);
        }
        let (linked, _) = b.finish().map_err(plan)?;
        // A rule `core.rr` gives, taken once: the copies after the first dropped.
        let (linked, counts) = without_core_repeats(linked, &counts, core)?;
        Ok((linked, Some(counts)))
    }
    fn linked_version(linked: &Self::Linked) -> Option<&Arc<Version>> {
        Some(linked)
    }
    fn spliced(last: &Arc<Version>, counts: &[usize], changed: Vec<(usize, Self::Rules)>, core: &BTreeSet<String>) -> Result<Option<(Self::Linked, Vec<usize>)>> {
        let last_rules: Vec<_> = last.rules().collect();
        if counts.iter().sum::<usize>() != last_rules.len() {
            return Ok(None);
        }
        let mut changed: BTreeMap<usize, Self::Rules> = changed.into_iter().collect();
        let mut b = Builder::derive(last);
        let mut now = Vec::with_capacity(counts.len());
        let mut at = 0;
        for (j, &n) in counts.iter().enumerate() {
            if let Some((v, dropped, names)) = changed.remove(&j) {
                let mut m = 0;
                for r in v.rules().filter(|r| !dropped.contains(v.rel_name(r.head().rel()))) {
                    b.rewrite(&v, r, &mut Renaming { v: &v, names: &names }).finish().map_err(plan)?;
                    m += 1;
                }
                now.push(m);
            } else {
                for r in &last_rules[at..at + n] {
                    b.copy_as_is(last, *r);
                }
                now.push(n);
            }
            at += n;
        }
        if !changed.is_empty() {
            return Ok(None);
        }
        let (linked, _) = b.finish().map_err(plan)?;
        Ok(Some(without_core_repeats(linked, &now, core)?))
    }
}
