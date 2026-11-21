//! A version's fields beside its rules (decision 0276 §3): read where they are, each alone, and stated on a builder a
//! field at a time. Nothing here decodes a program or takes one: a pass that adds an input, declares a relation or
//! edits a setting states that edit, and reads what it needs from the version. A field's rows are its `.about`
//! relations (`program_facts::Field`), so stating one replaces its relations' rows and keeps every other field's.

use super::super::front::ast::{Gate, LatticeDecl, LayoutFact, Native, Report, Shape, SpanIn, Value};
use super::super::front::name::{List, Lists};
use super::super::program_facts::{self, Facts, Field};
use super::{ABOUT, Builder, Cell, Version};
use std::collections::{BTreeMap, BTreeSet};

/// A gate (`Program::gates`' entry): its key, relation and level, the relations that must be quiet and the gates it
/// waits on, as `program_facts` writes them.
pub type GateEntry = (String, String, usize, Vec<String>, Vec<String>);

impl Version {
    /// The `.about` rows of `relations`, as facts of the version's program (`ABOUT`): what a field's reader reads.
    fn about_values(&self, relations: &[&str]) -> Facts {
        let mut out = Facts::new();
        out.insert("program".to_owned(), vec![[Value::Str(ABOUT.to_owned().into())].into()]);
        for f in self.about_facts() {
            let name = self.text(f.name());
            if !relations.contains(&name) {
                continue;
            }
            let rows = f.rows().map(|cells| {
                let row: Vec<Value> = std::iter::once(Value::Str(ABOUT.to_owned().into())).chain(cells.skip(1).map(|c| self.value(c))).collect();
                List::from(row)
            });
            out.insert(name.to_owned(), rows.collect());
        }
        out
    }
    /// A cell as a value.
    pub fn value(&self, c: Cell) -> Value {
        match c {
            Cell::Int(i) => Value::Int(i),
            Cell::Str(a) => Value::Str(self.text(a).to_owned().into()),
            Cell::Branded(b) => Value::Branded(b),
        }
    }
    /// The version's fields of `relations`, read (`program_facts::Fields`).
    fn read<T>(&self, relations: &[&str], read: impl FnOnce(&program_facts::Fields<'_>) -> Result<T, String>) -> Result<T, String> {
        let facts = self.about_values(relations);
        read(&program_facts::fields(&facts, ABOUT)?)
    }

    // -- the parts the image holds itself --

    /// The inputs' names, in the program's order.
    pub fn input_names(&self) -> Vec<String> {
        self.inputs().map(|a| self.text(a).to_owned()).collect()
    }
    /// The outputs' names, in the program's order.
    pub fn output_names(&self) -> Vec<String> {
        self.outputs().map(|a| self.text(a).to_owned()).collect()
    }
    /// Whether `rel` is an input.
    pub fn has_input(&self, rel: &str) -> bool {
        self.inputs().any(|a| self.text(a) == rel)
    }
    /// Whether `rel` is an output.
    pub fn has_output(&self, rel: &str) -> bool {
        self.outputs().any(|a| self.text(a) == rel)
    }
    /// Whether relation `rel` is declared.
    pub fn declares(&self, rel: &str) -> bool {
        self.find(rel).is_some_and(|a| self.decl(a).is_some())
    }
    /// Relation `rel`'s declared columns (name, type).
    pub fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        let d = self.decl(self.find(rel)?)?;
        Some(d.cols().map(|(n, t)| (self.text(n).to_owned(), self.text(t).to_owned())).collect())
    }
    /// The declared relations' names, in name order.
    pub fn declared_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.decls().into_iter().map(|d| self.text(d.name()).to_owned()).collect();
        names.sort();
        names
    }
    /// Relation `rel`'s facts, as values (`None` where it has none stated).
    pub fn fact_values(&self, rel: &str, lists: &Lists) -> Option<Vec<List<Value>>> {
        let f = self.facts(self.find(rel)?)?;
        Some(f.rows().map(|cells| lists.values(&cells.map(|c| self.value(c)).collect::<Vec<_>>())).collect())
    }
    /// The relations with facts stated, in name order.
    pub fn fact_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.all_facts().into_iter().map(|f| self.text(f.name()).to_owned()).collect();
        names.sort();
        names
    }

    // -- the `.about` fields, each read alone --

    pub fn nullary_names(&self) -> Result<BTreeSet<String>, String> {
        self.read(&["nullary"], |f| f.nullary())
    }
    pub fn qualifier_map(&self) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
        self.read(&["qualifier"], |f| f.qualifiers())
    }
    pub fn tape_columns(&self) -> Result<BTreeMap<String, BTreeSet<usize>>, String> {
        self.read(&["tape_column"], |f| f.tapes())
    }
    pub fn lattice_decls(&self) -> Result<BTreeMap<String, LatticeDecl>, String> {
        self.read(&["lattice", "lattice_merge", "lattice_widen"], |f| f.lattice())
    }
    pub fn demand_names(&self) -> Result<Vec<String>, String> {
        self.read(&["demand"], |f| f.demand())
    }
    pub fn kindset_map(&self) -> Result<BTreeMap<String, Vec<String>>, String> {
        self.read(&["kindset", "kindset_kind"], |f| f.kindsets())
    }
    pub fn demand_dropped(&self) -> Result<Vec<(String, String)>, String> {
        self.read(&["demand_dropped"], |f| f.demand_dropped())
    }
    pub fn magic_extras(&self) -> Result<Option<usize>, String> {
        self.read(&["magic_extras"], |f| f.magic_extras())
    }
    pub fn magic_report(&self) -> Result<Option<Report>, String> {
        self.read(&["magic_report", "report_list"], |f| f.magic_report())
    }
    pub fn gate_list(&self) -> Result<Vec<(String, Gate)>, String> {
        self.read(&["gate", "gate_src", "gate_quiet", "gate_dep"], |f| f.gates())
    }
    pub fn shape_list(&self) -> Result<Vec<Shape>, String> {
        self.read(&["shape", "shape_after", "shape_bound"], |f| f.shapes())
    }
    pub fn policies(&self) -> Result<Vec<serde_json::Value>, String> {
        self.read(&["policy"], |f| f.policies())
    }
    /// The setting directives (`.mode block`, `.scope include "…"`, `.language python`), by name, as written.
    pub fn settings(&self) -> Result<Vec<(String, String)>, String> {
        self.read(&["setting"], |f| f.settings())
    }
    /// Setting `key`'s value, as written, where the program has one (its first).
    pub fn setting(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.settings()?.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
    }
    pub fn native_map(&self) -> Result<BTreeMap<String, Native>, String> {
        self.read(program_facts::NATIVE_RELATIONS, |f| f.natives())
    }
    pub fn rx_spans(&self) -> Result<BTreeSet<(SpanIn, String)>, String> {
        self.read(&["rx_span"], |f| f.rx_spans())
    }
    pub fn ast_patterns(&self) -> Result<BTreeSet<String>, String> {
        self.read(&["ast_pattern"], |f| f.ast_patterns())
    }
    pub fn doc_scopes(&self) -> Result<crate::code_rule_dsl::logic::program_facts::DocScopes, String> {
        self.read(&["doc_scoped", "doc_scope", "doc_scope_glob"], |f| f.doc_scopes())
    }
    pub fn text_globs(&self) -> Result<Vec<String>, String> {
        self.read(&["text_glob"], |f| f.text_globs())
    }
    pub fn answer_map(&self) -> Result<BTreeMap<String, Vec<String>>, String> {
        self.read(&["answer"], |f| f.answers())
    }
}

impl Builder<'_> {
    /// States field `f`: its relations' rows are `f`'s, every other field's kept.
    pub(in crate::code_rule_dsl::logic::program_image) fn set_field(&mut self, f: Field<'_>) {
        // The program's own row comes with the first about row stated (`set_about_rows`).
        let mut facts = Facts::new();
        program_facts::write_field(&mut facts, ABOUT, &f);
        for &name in f.relations() {
            let rows: Vec<Vec<Cell>> = facts.remove(name).unwrap_or_default().iter().map(|row| row.iter().map(|v| self.cell(v)).collect()).collect();
            self.set_about_rows(name, rows);
        }
    }
    /// A value as a cell of this builder's atoms.
    pub fn cell(&mut self, v: &Value) -> Cell {
        match v {
            Value::Int(i) => Cell::Int(*i),
            Value::Str(s) => Cell::Str(self.atom(s)),
            Value::Branded(b) => Cell::Branded(*b),
        }
    }

    /// Adds `rel` to the inputs, after the others, unless it is one.
    pub(in crate::code_rule_dsl::logic::program_image) fn add_input(&mut self, rel: &str) {
        let mut names = self.input_names();
        if !names.iter().any(|n| n == rel) {
            names.push(rel.to_owned());
            self.set_inputs(&names);
        }
    }
    /// Adds `rel` to the outputs, after the others, unless it is one.
    pub(in crate::code_rule_dsl::logic::program_image) fn add_output(&mut self, rel: &str) {
        let mut names = self.output_names();
        if !names.iter().any(|n| n == rel) {
            names.push(rel.to_owned());
            self.set_outputs(&names);
        }
    }
    /// Keeps the inputs `keep` takes.
    pub(in crate::code_rule_dsl::logic::program_image) fn retain_inputs(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let names = self.input_names();
        let kept: Vec<&String> = names.iter().filter(|n| keep(n)).collect();
        if kept.len() != names.len() {
            self.set_inputs(&kept);
        }
    }
    /// Keeps the outputs `keep` takes.
    pub(in crate::code_rule_dsl::logic::program_image) fn retain_outputs(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let names = self.output_names();
        let kept: Vec<&String> = names.iter().filter(|n| keep(n)).collect();
        if kept.len() != names.len() {
            self.set_outputs(&kept);
        }
    }
}

/// What a reader of a program's fields asks, of a version (its own fields, read where they are), of an edit of one
/// (`Builder`, `Edit`: read through what the edit states) or, until the tree goes (decision 0276 step 6), of a
/// program a text was parsed to. A reader generic over it reads each alike, and none is encoded or decoded for it.
/// Each `.about` field is read alone, from its relations' rows (`about`).
pub trait ProgramFields {
    fn input_names(&self) -> Vec<String>;
    fn output_names(&self) -> Vec<String>;
    fn declares(&self, rel: &str) -> bool;
    /// Relation `rel`'s declared columns (name, type).
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>>;
    /// The declared relations' names, in name order.
    fn declared_names(&self) -> Vec<String>;
    /// The relations with facts stated (rows or none), in name order.
    fn fact_names(&self) -> Vec<String>;
    /// Whether `rel`'s facts are stated (rows or none).
    fn states_facts(&self, rel: &str) -> bool;
    /// Relation `rel`'s fact rows, as values (none where it has none).
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>>;
    /// Whether `rel` has a fact row.
    fn has_fact_rows(&self, rel: &str) -> bool {
        !self.fact_rows(rel).is_empty()
    }
    /// The `.about` rows of `relations`, as the program's facts (`ABOUT`): what a field's reader reads.
    fn about(&self, relations: &[&str]) -> Facts;

    fn settings(&self) -> Vec<(String, String)> {
        field(self, &["setting"], |f| f.settings())
    }
    /// The values of setting `key`, in order, trimmed.
    fn setting_values(&self, key: &str) -> Vec<String> {
        self.settings().into_iter().filter(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).collect()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        field(self, &["policy"], |f| f.policies())
    }
    fn text_globs(&self) -> Vec<String> {
        field(self, &["text_glob"], |f| f.text_globs())
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        field(self, &["doc_scoped", "doc_scope", "doc_scope_glob"], |f| f.doc_scopes())
    }
    fn natives(&self) -> BTreeMap<String, Native> {
        field(self, program_facts::NATIVE_RELATIONS, |f| f.natives())
    }
    fn nullary(&self) -> BTreeSet<String> {
        field(self, &["nullary"], |f| f.nullary())
    }
    fn qualifiers(&self) -> BTreeMap<String, BTreeSet<String>> {
        field(self, &["qualifier"], |f| f.qualifiers())
    }
    fn tapes(&self) -> BTreeMap<String, BTreeSet<usize>> {
        field(self, &["tape_column"], |f| f.tapes())
    }
    fn lattice(&self) -> BTreeMap<String, LatticeDecl> {
        field(self, &["lattice", "lattice_merge", "lattice_widen"], |f| f.lattice())
    }
    fn kindsets(&self) -> BTreeMap<String, Vec<String>> {
        field(self, &["kindset", "kindset_kind"], |f| f.kindsets())
    }
    fn rx_spans(&self) -> BTreeSet<(SpanIn, String)> {
        field(self, &["rx_span"], |f| f.rx_spans())
    }
    fn ast_patterns(&self) -> BTreeSet<String> {
        field(self, &["ast_pattern"], |f| f.ast_patterns())
    }
    fn demand(&self) -> Vec<String> {
        field(self, &["demand"], |f| f.demand())
    }
    fn demand_dropped(&self) -> Vec<(String, String)> {
        field(self, &["demand_dropped"], |f| f.demand_dropped())
    }
    fn gates(&self) -> Vec<(String, Gate)> {
        field(self, &["gate", "gate_src", "gate_quiet", "gate_dep"], |f| f.gates())
    }
    fn shapes(&self) -> Vec<Shape> {
        field(self, &["shape", "shape_after", "shape_bound"], |f| f.shapes())
    }
    /// Each relation's layout facts (`.layout`, decision 0361).
    fn layouts(&self) -> BTreeMap<String, Vec<LayoutFact>> {
        field(self, &["layout"], |f| f.layouts())
    }
    fn answers(&self) -> BTreeMap<String, Vec<String>> {
        field(self, &["answer"], |f| f.answers())
    }
    fn asked(&self) -> BTreeMap<String, Vec<BTreeSet<String>>> {
        field(self, &["asked"], |f| f.asked())
    }
    fn magic_extras(&self) -> Option<usize> {
        field(self, &["magic_extras"], |f| f.magic_extras())
    }
    fn magic_report(&self) -> Option<Report> {
        field(self, &["magic_report", "report_list"], |f| f.magic_report())
    }
}

/// Field `relations` of `p`, read (`program_facts::Fields`). A program's fields read as they were written: a row
/// that does not read is a corrupt image, not an input.
fn field<P: ProgramFields + ?Sized, T>(p: &P, relations: &[&str], read: impl FnOnce(&program_facts::Fields<'_>) -> Result<T, String>) -> T {
    let facts = p.about(relations);
    program_facts::fields(&facts, ABOUT).and_then(|f| read(&f)).unwrap_or_else(|e| panic!("a program's field {relations:?} does not read: {e}"))
}

/// An edit of a program's fields, a field at a time, beside the reads (`ProgramFields`): what a pass that edits fields
/// alone takes, written by an edit's `FieldsW` (`Builder::fields`).
pub trait FieldsMut: ProgramFields {
    /// Declares `rel`'s columns, replacing any declaration.
    fn declare(&mut self, rel: &str, cols: &[(&str, &str)]);
    fn undeclare(&mut self, rel: &str);
    /// Adds `rel` to the inputs, after the others, unless it is one.
    fn add_input(&mut self, rel: &str);
    fn retain_inputs(&mut self, keep: &dyn Fn(&str) -> bool);
    /// States the inputs, in order.
    fn set_inputs(&mut self, names: &[String]);
    /// States the outputs, in order.
    fn set_outputs(&mut self, names: &[String]);
    /// Adds `rel` to the outputs, after the others, unless it is one.
    fn add_output(&mut self, rel: &str);
    fn retain_outputs(&mut self, keep: &dyn Fn(&str) -> bool);
    /// States `rel`'s facts, replacing any.
    fn set_facts(&mut self, rel: &str, rows: &[List<Value>]);
    /// Takes `rel`'s facts out.
    fn drop_facts(&mut self, rel: &str);
    /// Adds rows to `rel`'s facts, after its others.
    fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        if rows.is_empty() {
            return;
        }
        let mut all = self.fact_rows(rel);
        all.extend(rows.iter().cloned());
        self.set_facts(rel, &all);
    }
    /// States `.about` field `f`, every other field kept.
    fn set_field(&mut self, f: Field<'_>);
    /// Declares `rel` with `cols` unless it is declared.
    fn declare_absent(&mut self, rel: &str, cols: &[(&str, &str)]) {
        if !self.declares(rel) {
            self.declare(rel, cols);
        }
    }
}

impl Version {
    /// Its `.about` relations that `keep` keeps, as rows.
    pub fn about_kept(&self, keep: &dyn Fn(&str) -> bool) -> Facts {
        let names: Vec<String> = self.about_facts().iter().map(|f| self.text(f.name()).to_owned()).filter(|n| keep(n)).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        self.about_held(&names)
    }
    /// The `.about` rows of `names` the version holds, to be written: `about`'s, but the program's own row only where
    /// the version holds one (its readers are given it, held or not; a write of it would state a row the version
    /// lacks, and so another image than a run from scratch's).
    fn about_held(&self, names: &[&str]) -> Facts {
        let mut about = ProgramFields::about(self, names);
        if !self.about_facts().iter().any(|f| self.text(f.name()) == "program") {
            about.remove("program");
        }
        about
    }
}

impl ProgramFields for Version {
    fn input_names(&self) -> Vec<String> {
        Version::input_names(self)
    }
    fn output_names(&self) -> Vec<String> {
        Version::output_names(self)
    }
    fn declares(&self, rel: &str) -> bool {
        Version::declares(self, rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        Version::columns_of(self, rel)
    }
    fn declared_names(&self) -> Vec<String> {
        Version::declared_names(self)
    }
    fn fact_names(&self) -> Vec<String> {
        Version::fact_names(self)
    }
    fn states_facts(&self, rel: &str) -> bool {
        self.find(rel).is_some_and(|a| self.facts(a).is_some())
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        self.fact_values(rel, &Lists::default()).unwrap_or_default()
    }
    fn has_fact_rows(&self, rel: &str) -> bool {
        self.find(rel).and_then(|a| self.facts(a)).is_some_and(|f| f.rows().len() > 0)
    }
    fn about(&self, relations: &[&str]) -> Facts {
        self.about_values(relations)
    }
}

impl Builder<'_> {
    /// States `now`'s fields (its declarations, facts, inputs, outputs and `.about` rows) where they differ from
    /// `was`'s, each read by name: `now` a version made apart from `was`, the version this builder edits, its rules
    /// the builder's own (`manager`'s edit of the last version to the new one).
    pub(in crate::code_rule_dsl::logic::program_image) fn state_fields_as(&mut self, was: &Version, now: &Version) {
        for rel in now.declared_names() {
            let cols = now.columns_of(&rel);
            if was.columns_of(&rel) != cols {
                let cols = cols.unwrap_or_default();
                let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
                Builder::declare(self, &rel, &pairs);
            }
        }
        for rel in was.declared_names() {
            if !now.declares(&rel) {
                self.undeclare(&rel);
            }
        }
        for rel in now.fact_names() {
            let rows = now.fact_rows(&rel);
            if !was.states_facts(&rel) || was.fact_rows(&rel) != rows {
                self.set_facts(&rel, &rows);
            }
        }
        for rel in was.fact_names() {
            if !now.states_facts(&rel) {
                self.drop_facts(&rel);
            }
        }
        let inputs = now.input_names();
        if was.input_names() != inputs {
            self.set_inputs(&inputs);
        }
        let outputs = now.output_names();
        if was.output_names() != outputs {
            self.set_outputs(&outputs);
        }
        let names: BTreeSet<String> = [was, now].iter().flat_map(|v| v.about_facts().into_iter().map(|f| v.text(f.name()).to_owned()).collect::<Vec<_>>()).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let about = now.about_held(&names);
        if was.about_held(&names) != about {
            self.set_about(&about);
        }
    }

    /// States `v`'s header on this builder as `set_program_header` states a program's: its declarations, facts, inputs
    /// and outputs, and the `.about` relations `about` keeps (an image's `module_*` rows are no program's fields).
    pub(in crate::code_rule_dsl::logic::program_image) fn copy_fields(&mut self, v: &Version, about: &dyn Fn(&str) -> bool) {
        for rel in ProgramFields::declared_names(v) {
            let cols = ProgramFields::columns_of(v, &rel).unwrap_or_default();
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            Builder::declare(self, &rel, &pairs);
        }
        for rel in ProgramFields::fact_names(v) {
            self.set_facts(&rel, &ProgramFields::fact_rows(v, &rel));
        }
        self.set_inputs(&ProgramFields::input_names(v));
        self.set_outputs(&ProgramFields::output_names(v));
        self.set_about(&v.about_kept(about));
    }

    /// A cell of this builder's atoms as a value.
    fn value_of(&self, c: Cell) -> Value {
        match c {
            Cell::Int(i) => Value::Int(i),
            Cell::Str(a) => Value::Str(self.text(a).to_owned().into()),
            Cell::Branded(b) => Value::Branded(b),
        }
    }
}

/// An edit's fields, read through what it states: what it has not stated is its parent's.
impl ProgramFields for Builder<'_> {
    fn input_names(&self) -> Vec<String> {
        Builder::input_names(self)
    }
    fn output_names(&self) -> Vec<String> {
        Builder::output_names(self)
    }
    fn declares(&self, rel: &str) -> bool {
        Builder::declares(self, rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        Builder::columns_of(self, rel)
    }
    fn declared_names(&self) -> Vec<String> {
        Builder::declared_names(self)
    }
    fn fact_names(&self) -> Vec<String> {
        Builder::fact_names(self)
    }
    fn states_facts(&self, rel: &str) -> bool {
        self.fact_cells(rel).is_some()
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        let Some(rows) = self.fact_cells(rel) else { return Vec::new() };
        rows.into_iter().map(|row| List::from(row.into_iter().map(|c| self.value_of(c)).collect::<Vec<_>>())).collect()
    }
    fn about(&self, relations: &[&str]) -> Facts {
        let mut out = Facts::new();
        out.insert("program".to_owned(), vec![[Value::Str(ABOUT.to_owned().into())].into()]);
        for &name in relations {
            let rows = self.about_cells(name);
            if rows.is_empty() {
                continue;
            }
            let rows = rows.into_iter().map(|cells| {
                let row: Vec<Value> = std::iter::once(Value::Str(ABOUT.to_owned().into())).chain(cells.into_iter().skip(1).map(|c| self.value_of(c))).collect();
                List::from(row)
            });
            out.insert(name.to_owned(), rows.collect());
        }
        out
    }
}

/// A program's fields, written (decision 0276 §3): the one surface an edit's fields are written through, grouped by
/// field: declarations, inputs and outputs, facts, and the `.about` fields (qualifiers, answers, gates, settings,
/// natives, lattices, the nullary relations, tapes). `Builder::fields` opens it; `Builder`'s own setters are its
/// implementation, private to the image. It reads as the edit does (`ProgramFields`): what it stated, else the
/// parent's.
pub struct FieldsW<'b, 'v>(&'b mut Builder<'v>);

impl<'v> Builder<'v> {
    /// The edit's fields, to write (`FieldsW`).
    pub fn fields(&mut self) -> FieldsW<'_, 'v> {
        FieldsW(self)
    }
}

impl FieldsW<'_, '_> {
    pub fn copy_fields(&mut self, v: &Version, about: &dyn Fn(&str) -> bool) {
        self.0.copy_fields(v, about)
    }
    pub fn set_lattice_keys(&mut self, rel: &str, keys: usize) {
        self.0.set_lattice_keys(rel, keys)
    }
    pub fn drop_native(&mut self, rel: &str) {
        self.0.drop_native(rel)
    }
    pub fn keep_relation_fields(&mut self, natives: &dyn Fn(&str) -> bool, lattice: &dyn Fn(&str) -> bool, gates: &dyn Fn(&str) -> bool) {
        self.0.keep_relation_fields(natives, lattice, gates)
    }
    pub fn add_qualifiers<S: AsRef<str>>(&mut self, words: &[(S, S)]) {
        self.0.add_qualifiers(words)
    }
    pub fn set_qualifiers<S: AsRef<str>>(&mut self, words: &[(S, S)]) {
        self.0.set_qualifiers(words)
    }
    pub fn add_tree_gates(&mut self, gates: &[(String, crate::code_rule_dsl::logic::front::ast::Gate)]) {
        self.0.add_tree_gates(gates)
    }
    pub fn set_answers(&mut self, answers: &std::collections::BTreeMap<String, Vec<String>>) {
        self.0.set_answers(answers)
    }
    pub fn set_asked(&mut self, asked: &super::about::Asked) {
        self.0.set_asked(asked)
    }
    pub fn keep_asked(&mut self, keep: &dyn Fn(&str) -> bool) {
        self.0.keep_asked(keep)
    }
    pub fn add_gates(&mut self, gates: &[GateEntry]) {
        self.0.add_gates(gates)
    }
    pub fn add_setting(&mut self, key: &str, text: &str) {
        self.0.add_setting(key, text)
    }
    pub fn add_nullary<S: AsRef<str>>(&mut self, rels: &[S]) {
        self.0.add_nullary(rels)
    }
    pub fn set_tapes<S: AsRef<str>>(&mut self, tapes: &[(S, usize)]) {
        self.0.set_tapes(tapes)
    }
    pub fn set_field(&mut self, f: Field<'_>) {
        self.0.set_field(f)
    }
    pub fn add_input(&mut self, rel: &str) {
        self.0.add_input(rel)
    }
    pub fn add_output(&mut self, rel: &str) {
        self.0.add_output(rel)
    }
    pub fn retain_inputs(&mut self, keep: impl FnMut(&str) -> bool) {
        self.0.retain_inputs(keep)
    }
    pub fn retain_outputs(&mut self, keep: impl FnMut(&str) -> bool) {
        self.0.retain_outputs(keep)
    }
    pub fn state_fields_as(&mut self, was: &Version, now: &Version) {
        self.0.state_fields_as(was, now)
    }
    pub fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        self.0.declare(rel, cols)
    }
    pub fn undeclare(&mut self, rel: &str) {
        self.0.undeclare(rel)
    }
    pub fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        self.0.set_facts(rel, rows)
    }
    pub fn drop_facts(&mut self, rel: &str) {
        self.0.drop_facts(rel)
    }
    pub fn set_fact_cells(&mut self, rel: &str, rows: Vec<Vec<Cell>>) {
        self.0.set_fact_cells(rel, rows)
    }
    pub fn set_about_rows(&mut self, name: &str, rows: Vec<Vec<Cell>>) {
        self.0.set_about_rows(name, rows)
    }
    pub fn set_inputs<S: AsRef<str>>(&mut self, names: &[S]) {
        self.0.set_inputs(names)
    }
    pub fn set_outputs<S: AsRef<str>>(&mut self, names: &[S]) {
        self.0.set_outputs(names)
    }
    pub fn state_fields_of(&mut self, e: &Builder<'_>) {
        self.0.state_fields_of(e)
    }
    pub fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        self.0.add_facts(rel, rows)
    }
    pub fn set_about(&mut self, about: &Facts) {
        self.0.set_about(about)
    }
}

/// Its fields read through the edit it writes.
impl ProgramFields for FieldsW<'_, '_> {
    fn input_names(&self) -> Vec<String> {
        ProgramFields::input_names(&*self.0)
    }
    fn output_names(&self) -> Vec<String> {
        ProgramFields::output_names(&*self.0)
    }
    fn declares(&self, rel: &str) -> bool {
        ProgramFields::declares(&*self.0, rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        ProgramFields::columns_of(&*self.0, rel)
    }
    fn declared_names(&self) -> Vec<String> {
        ProgramFields::declared_names(&*self.0)
    }
    fn fact_names(&self) -> Vec<String> {
        ProgramFields::fact_names(&*self.0)
    }
    fn states_facts(&self, rel: &str) -> bool {
        ProgramFields::states_facts(&*self.0, rel)
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        ProgramFields::fact_rows(&*self.0, rel)
    }
    fn about(&self, relations: &[&str]) -> Facts {
        ProgramFields::about(&*self.0, relations)
    }
}

impl FieldsMut for FieldsW<'_, '_> {
    fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        FieldsW::declare(self, rel, cols)
    }
    fn undeclare(&mut self, rel: &str) {
        FieldsW::undeclare(self, rel)
    }
    fn add_input(&mut self, rel: &str) {
        FieldsW::add_input(self, rel)
    }
    fn retain_inputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        FieldsW::retain_inputs(self, keep)
    }
    fn set_inputs(&mut self, names: &[String]) {
        FieldsW::set_inputs(self, names)
    }
    fn set_outputs(&mut self, names: &[String]) {
        FieldsW::set_outputs(self, names)
    }
    fn add_output(&mut self, rel: &str) {
        FieldsW::add_output(self, rel)
    }
    fn retain_outputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        FieldsW::retain_outputs(self, keep)
    }
    fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        FieldsW::set_facts(self, rel, rows)
    }
    fn drop_facts(&mut self, rel: &str) {
        FieldsW::drop_facts(self, rel)
    }
    fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        FieldsW::add_facts(self, rel, rows)
    }
    fn set_field(&mut self, f: Field<'_>) {
        FieldsW::set_field(self, f)
    }
}

/// A version and an edit of it (`Builder::on`), its fields read through the edit: a pass that adds an input or a
/// fact and then reads them reads what it stated. Rules are the version's until the edit removes them.
pub struct Edit<'v> {
    pub v: &'v std::sync::Arc<Version>,
    pub b: Builder<'v>,
}

impl<'v> Edit<'v> {
    pub fn on(v: &'v std::sync::Arc<Version>) -> Edit<'v> {
        Edit { v, b: Builder::on(v) }
    }
    /// The edited version: `v` itself where the edit stated nothing.
    pub fn finish(self) -> Result<std::sync::Arc<Version>, String> {
        if self.b.is_empty() {
            return Ok(std::sync::Arc::clone(self.v));
        }
        self.b.finish().map(|(v, _)| v)
    }
}

/// What the edit leaves: its builder's reads.
impl ProgramFields for Edit<'_> {
    fn input_names(&self) -> Vec<String> {
        ProgramFields::input_names(&self.b)
    }
    fn output_names(&self) -> Vec<String> {
        ProgramFields::output_names(&self.b)
    }
    fn declares(&self, rel: &str) -> bool {
        ProgramFields::declares(&self.b, rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        ProgramFields::columns_of(&self.b, rel)
    }
    fn declared_names(&self) -> Vec<String> {
        ProgramFields::declared_names(&self.b)
    }
    fn fact_names(&self) -> Vec<String> {
        ProgramFields::fact_names(&self.b)
    }
    fn states_facts(&self, rel: &str) -> bool {
        self.b.states_facts(rel)
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        self.b.fact_rows(rel)
    }
    fn about(&self, relations: &[&str]) -> Facts {
        self.b.about(relations)
    }
}

/// A program's fields as text, for a digest of them (`plan::corpus::kept_link::interface`): every list and map the
/// fields API reads, in one order, declarations and rows by name.
pub struct Shown<'a>(pub &'a dyn ProgramFields);

impl std::fmt::Debug for Shown<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let p = self.0;
        // A declared relation, and its columns' names and types.
        type Decl = (String, Option<Vec<(String, String)>>);
        let mut decls: Vec<Decl> = p.declared_names().into_iter().map(|r| {
            let cols = p.columns_of(&r);
            (r, cols)
        }).collect();
        decls.sort();
        let mut facts: Vec<(String, Vec<List<Value>>)> = p.fact_names().into_iter().map(|r| {
            let rows = p.fact_rows(&r);
            (r, rows)
        }).collect();
        facts.sort_by(|a, b| a.0.cmp(&b.0));
        f.debug_struct("Fields")
            .field("inputs", &p.input_names())
            .field("outputs", &p.output_names())
            .field("decls", &decls)
            .field("facts", &facts)
            .field("nullary", &p.nullary())
            .field("qualifiers", &p.qualifiers())
            .field("tapes", &p.tapes())
            .field("shapes", &p.shapes())
            .field("lattice", &p.lattice())
            .field("kindsets", &p.kindsets())
            .field("demand", &p.demand())
            .field("demand_dropped", &p.demand_dropped())
            .field("gates", &p.gates())
            .field("policies", &p.policies())
            .field("settings", &p.settings())
            .field("natives", &p.natives())
            .field("doc_scopes", &p.doc_scopes())
            .field("text_globs", &p.text_globs())
            .field("ast_patterns", &p.ast_patterns())
            .field("answers", &p.answers())
            .field("asked", &p.asked())
            .field("rx_spans", &p.rx_spans())
            .field("magic_extras", &p.magic_extras())
            .field("magic_report", &p.magic_report())
            .finish()
    }
}
