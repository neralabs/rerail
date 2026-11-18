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

/// What a reader of a program's fields asks, of a version (its own fields, read where they are) or, until the tree
/// goes (decision 0276 step 6), of a program a text was parsed to. A reader generic over it reads a run's version and
/// a phase program's text alike, and neither is encoded nor decoded for it.
pub trait ProgramFields {
    fn settings(&self) -> Vec<(String, String)>;
    fn declares(&self, rel: &str) -> bool;
    fn declared_names(&self) -> Vec<String>;
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>>;
    /// Whether `rel` has a fact row.
    fn has_fact_rows(&self, rel: &str) -> bool;
    /// Relation `rel`'s fact rows, as values (none where it has none).
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>>;
    fn input_names(&self) -> Vec<String>;
    fn output_names(&self) -> Vec<String>;
    fn text_globs(&self) -> Vec<String>;
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>>;
    fn policies(&self) -> Vec<serde_json::Value>;
    /// The values of setting `key`, in order, trimmed.
    fn setting_values(&self, key: &str) -> Vec<String> {
        self.settings().into_iter().filter(|(k, _)| k == key).map(|(_, v)| v.trim().to_owned()).collect()
    }
}

impl ProgramFields for Version {
    fn settings(&self) -> Vec<(String, String)> {
        Version::settings(self).expect("a version's settings are read")
    }
    fn declares(&self, rel: &str) -> bool {
        Version::declares(self, rel)
    }
    fn declared_names(&self) -> Vec<String> {
        Version::declared_names(self)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        Version::columns_of(self, rel)
    }
    fn has_fact_rows(&self, rel: &str) -> bool {
        self.find(rel).and_then(|a| self.facts(a)).is_some_and(|f| f.rows().len() > 0)
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        self.fact_values(rel, &Lists::default()).unwrap_or_default()
    }
    fn input_names(&self) -> Vec<String> {
        Version::input_names(self)
    }
    fn output_names(&self) -> Vec<String> {
        Version::output_names(self)
    }
    fn text_globs(&self) -> Vec<String> {
        Version::text_globs(self).expect("a version's text globs are read")
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        Version::doc_scopes(self).expect("a version's scopes are read")
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        Version::policies(self).expect("a version's policies are read")
    }
}

impl ProgramFields for super::super::front::ast::Program {
    fn settings(&self) -> Vec<(String, String)> {
        self.settings.clone()
    }
    fn declares(&self, rel: &str) -> bool {
        self.decls.contains_key(rel)
    }
    fn declared_names(&self) -> Vec<String> {
        self.decls.keys().map(|n| n.to_string()).collect()
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        self.decls.get(rel).map(|cols| cols.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect())
    }
    fn has_fact_rows(&self, rel: &str) -> bool {
        self.facts.get(rel).is_some_and(|rows| !rows.is_empty())
    }
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        self.facts.get(rel).cloned().unwrap_or_default()
    }
    fn input_names(&self) -> Vec<String> {
        self.inputs.clone()
    }
    fn output_names(&self) -> Vec<String> {
        self.outputs.clone()
    }
    fn text_globs(&self) -> Vec<String> {
        self.text_globs.clone()
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        self.doc_scopes.clone()
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        self.policies.clone()
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
    fn value(&self, c: Cell) -> Value {
        match c {
            Cell::Int(i) => Value::Int(i),
            Cell::Str(a) => Value::Str(self.b.text(a).to_owned().into()),
            Cell::Branded(b) => Value::Branded(b),
        }
    }
}

impl ProgramFields for Edit<'_> {
    fn settings(&self) -> Vec<(String, String)> {
        ProgramFields::settings(&**self.v)
    }
    fn declares(&self, rel: &str) -> bool {
        self.b.declares(rel)
    }
    fn declared_names(&self) -> Vec<String> {
        ProgramFields::declared_names(&**self.v)
    }
    fn columns_of(&self, rel: &str) -> Option<Vec<(String, String)>> {
        Version::columns_of(self.v, rel)
    }
    fn has_fact_rows(&self, rel: &str) -> bool {
        self.b.fact_cells(rel).is_some_and(|rows| !rows.is_empty())
    }
    /// As the edit leaves them.
    fn fact_rows(&self, rel: &str) -> Vec<List<Value>> {
        let Some(rows) = self.b.fact_cells(rel) else { return Vec::new() };
        rows.into_iter().map(|row| List::from(row.into_iter().map(|c| self.value(c)).collect::<Vec<_>>())).collect()
    }
    fn input_names(&self) -> Vec<String> {
        self.b.input_names()
    }
    fn output_names(&self) -> Vec<String> {
        self.b.output_names()
    }
    fn text_globs(&self) -> Vec<String> {
        ProgramFields::text_globs(&**self.v)
    }
    fn doc_scopes(&self) -> Option<Vec<(Vec<String>, Vec<String>)>> {
        ProgramFields::doc_scopes(&**self.v)
    }
    fn policies(&self) -> Vec<serde_json::Value> {
        ProgramFields::policies(&**self.v)
    }
}
