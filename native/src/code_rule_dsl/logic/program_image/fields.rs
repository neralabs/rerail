//! A version's fields beside its rules (decision 0276 §3): read where they are, each alone, and stated on a builder a
//! field at a time. Nothing here decodes a program or takes one: a pass that adds an input, declares a relation or
//! edits a setting states that edit, and reads what it needs from the version. A field's rows are its `.about`
//! relations (`program_facts::Field`), so stating one replaces its relations' rows and keeps every other field's.

use super::super::front::ast::{Gate, LatticeDecl, Native, Report, Shape, SpanIn, Value};
use super::super::front::name::{List, Lists};
use super::super::program_facts::{self, Facts, Field};
use super::{ABOUT, Builder, Cell, Version};
use std::collections::{BTreeMap, BTreeSet};

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
    pub fn doc_scopes(&self) -> Result<Option<Vec<(Vec<String>, Vec<String>)>>, String> {
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
    pub fn set_field(&mut self, f: Field<'_>) {
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
    pub fn add_input(&mut self, rel: &str) {
        let mut names = self.input_names();
        if !names.iter().any(|n| n == rel) {
            names.push(rel.to_owned());
            self.set_inputs(&names);
        }
    }
    /// Adds `rel` to the outputs, after the others, unless it is one.
    pub fn add_output(&mut self, rel: &str) {
        let mut names = self.output_names();
        if !names.iter().any(|n| n == rel) {
            names.push(rel.to_owned());
            self.set_outputs(&names);
        }
    }
    /// Keeps the inputs `keep` takes.
    pub fn retain_inputs(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let names = self.input_names();
        let kept: Vec<&String> = names.iter().filter(|n| keep(n)).collect();
        if kept.len() != names.len() {
            self.set_inputs(&kept);
        }
    }
    /// Keeps the outputs `keep` takes.
    pub fn retain_outputs(&mut self, mut keep: impl FnMut(&str) -> bool) {
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
    fn answers(&self) -> BTreeMap<String, Vec<String>> {
        field(self, &["answer"], |f| f.answers())
    }
    fn asked(&self) -> BTreeMap<String, Vec<BTreeSet<String>>> {
        field(self, &["asked"], |f| f.asked())
    }
}

/// Field `relations` of `p`, read (`program_facts::Fields`). A program's fields read as they were written: a row
/// that does not read is a corrupt image, not an input.
fn field<P: ProgramFields + ?Sized, T>(p: &P, relations: &[&str], read: impl FnOnce(&program_facts::Fields<'_>) -> Result<T, String>) -> T {
    let facts = p.about(relations);
    program_facts::fields(&facts, ABOUT).and_then(|f| read(&f)).unwrap_or_else(|e| panic!("a program's field {relations:?} does not read: {e}"))
}

/// An edit of a program's fields, a field at a time, beside the reads (`ProgramFields`): a builder's (the version's
/// edit) or, until the tree goes, a parsed program's. A pass generic over it states one edit for both.
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

impl FieldsMut for Builder<'_> {
    fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        Builder::declare(self, rel, cols)
    }
    fn undeclare(&mut self, rel: &str) {
        Builder::undeclare(self, rel)
    }
    fn add_input(&mut self, rel: &str) {
        Builder::add_input(self, rel)
    }
    fn retain_inputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        Builder::retain_inputs(self, keep)
    }
    fn set_inputs(&mut self, names: &[String]) {
        Builder::set_inputs(self, names)
    }
    fn set_outputs(&mut self, names: &[String]) {
        Builder::set_outputs(self, names)
    }
    fn add_output(&mut self, rel: &str) {
        Builder::add_output(self, rel)
    }
    fn retain_outputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        Builder::retain_outputs(self, keep)
    }
    fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        Builder::set_facts(self, rel, rows)
    }
    fn drop_facts(&mut self, rel: &str) {
        Builder::drop_facts(self, rel)
    }
    fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        Builder::add_facts(self, rel, rows)
    }
    fn set_field(&mut self, f: Field<'_>) {
        Builder::set_field(self, f)
    }
}

type Tree = super::super::front::ast::Program;

impl ProgramFields for Tree {
    fn input_names(&self) -> Vec<String> {
        self.inputs.clone()
    }
    fn output_names(&self) -> Vec<String> {
        self.outputs.clone()
    }
    fn declares(&self, rel: &str) -> bool {
        self.decls.contains_key(rel)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        self.decls.get(rel).map(|cols| cols.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect())
    }
    fn declared_names(&self) -> Vec<String> {
        self.decls.keys().map(|n| n.to_string()).collect()
    }
    fn fact_names(&self) -> Vec<String> {
        self.facts.keys().cloned().collect()
    }
    fn states_facts(&self, rel: &str) -> bool {
        self.facts.contains_key(rel)
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        self.facts.get(rel).cloned().unwrap_or_default()
    }
    fn has_fact_rows(&self, rel: &str) -> bool {
        self.facts.get(rel).is_some_and(|rows| !rows.is_empty())
    }
    fn about(&self, relations: &[&str]) -> Facts {
        let mut about = super::about_of(self);
        about.retain(|k, _| k == "program" || relations.contains(&k.as_str()));
        about
    }
    fn settings(&self) -> Vec<(String, String)> {
        self.settings.clone()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        self.policies.clone()
    }
    fn text_globs(&self) -> Vec<String> {
        self.text_globs.clone()
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        self.doc_scopes.clone()
    }
    fn natives(&self) -> BTreeMap<String, Native> {
        self.natives.clone()
    }
    fn nullary(&self) -> BTreeSet<String> {
        self.nullary.clone()
    }
    fn qualifiers(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.qualifiers.clone()
    }
    fn tapes(&self) -> BTreeMap<String, BTreeSet<usize>> {
        self.tapes.clone()
    }
    fn lattice(&self) -> BTreeMap<String, LatticeDecl> {
        self.lattice.clone()
    }
    fn kindsets(&self) -> BTreeMap<String, Vec<String>> {
        self.kindsets.clone()
    }
    fn rx_spans(&self) -> BTreeSet<(SpanIn, String)> {
        self.rx_spans.clone()
    }
    fn ast_patterns(&self) -> BTreeSet<String> {
        self.ast_patterns.clone()
    }
    fn demand(&self) -> Vec<String> {
        self.demand.clone()
    }
    fn demand_dropped(&self) -> Vec<(String, String)> {
        self.demand_dropped.clone()
    }
    fn gates(&self) -> Vec<(String, Gate)> {
        self.gates.clone()
    }
    fn shapes(&self) -> Vec<Shape> {
        self.shapes.clone()
    }
    fn answers(&self) -> BTreeMap<String, Vec<String>> {
        self.answers.clone()
    }
    fn asked(&self) -> BTreeMap<String, Vec<BTreeSet<String>>> {
        self.asked_by.clone()
    }
}

impl FieldsMut for Tree {
    fn declare(&mut self, rel: &str, cols: &[(&str, &str)]) {
        self.decls.insert(rel.to_owned().into(), cols.iter().map(|&(n, t)| (n.to_owned().into(), t.to_owned().into())).collect());
    }
    fn undeclare(&mut self, rel: &str) {
        self.decls.remove(rel);
    }
    fn add_input(&mut self, rel: &str) {
        if !self.inputs.iter().any(|i| i == rel) {
            self.inputs.push(rel.to_owned());
        }
    }
    fn retain_inputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        self.inputs.retain(|i| keep(i));
    }
    fn set_inputs(&mut self, names: &[String]) {
        self.inputs = names.to_vec();
    }
    fn set_outputs(&mut self, names: &[String]) {
        self.outputs = names.to_vec();
    }
    fn add_output(&mut self, rel: &str) {
        if !self.outputs.iter().any(|o| o == rel) {
            self.outputs.push(rel.to_owned());
        }
    }
    fn retain_outputs(&mut self, keep: &dyn Fn(&str) -> bool) {
        self.outputs.retain(|o| keep(o));
    }
    fn set_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        self.facts.insert(rel.to_owned(), rows.to_vec());
    }
    fn drop_facts(&mut self, rel: &str) {
        self.facts.remove(rel);
    }
    fn add_facts(&mut self, rel: &str, rows: &[List<Value>]) {
        self.facts.entry(rel.to_owned()).or_default().extend(rows.iter().cloned());
    }
    fn set_field(&mut self, f: Field<'_>) {
        match f {
            Field::Nullary(x) => self.nullary = x.clone(),
            Field::Qualifiers(x) => self.qualifiers = x.clone(),
            Field::Tapes(x) => self.tapes = x.clone(),
            Field::Shapes(x) => self.shapes = x.to_vec(),
            Field::Lattice(x) => self.lattice = x.clone(),
            Field::Kindsets(x) => self.kindsets = x.clone(),
            Field::Demand(x) => self.demand = x.to_vec(),
            Field::DemandDropped(x) => self.demand_dropped = x.to_vec(),
            Field::MagicExtras(x) => self.magic_extras = x,
            Field::MagicReport(x) => self.magic_report = x.cloned(),
            Field::Gates(x) => self.gates = x.to_vec(),
            Field::Policies(x) => self.policies = x.to_vec(),
            Field::Settings(x) => self.settings = x.to_vec(),
            Field::Natives(x) => self.natives = x.clone(),
            Field::DocScopes(x) => self.doc_scopes = x.map(<[_]>::to_vec),
            Field::TextGlobs(x) => self.text_globs = x.to_vec(),
            Field::AstPatterns(x) => self.ast_patterns = x.clone(),
            Field::Answers(x) => self.answers = x.clone(),
            Field::Asked(x) => self.asked_by = x.clone(),
            Field::RxSpans(x) => self.rx_spans = x.clone(),
        }
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
