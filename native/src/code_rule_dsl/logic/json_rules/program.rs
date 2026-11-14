//! The translator's program under construction (`translate.Program`): declarations, notes, the library part
//! main_terms' lowerings share, and the rules. Python's rule dicts are shared by reference between lists; here
//! they live in one arena (`Program::arena`) and every list holds their indices.

use super::value::Value;
use indexmap::IndexMap;
use std::collections::BTreeSet;

/// A rule as (head, body literals).
pub type Rule = (String, Vec<String>);
/// A library relation: (declaration, note lines, rules).
pub type LibEntry = (String, Vec<String>, Vec<Rule>);

/// A rule of the body or a helper: Python's `{"head", "lits", "edits", "drop", "head_note"}` dict.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub head: String,
    pub lits: Vec<String>,
    pub edits: Option<Vec<String>>,
    pub drop: bool,
    pub head_note: Option<String>,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.head.split('(').next().unwrap_or("")
    }
}

/// `prog.rules`: a helper's rule (an arena index) or a relation of facts.
#[derive(Clone, Debug)]
pub enum PRule {
    Facts { name: String, vals: Vec<Value> },
    Rule(usize),
}

/// The key of an `arguments` slot's relations: (signature, slot, syntactic).
pub type ArgKey = (Vec<String>, String, bool);

#[derive(Default)]
pub struct Program {
    pub lang: String,
    pub rules: Vec<PRule>,
    pub decls: IndexMap<String, String>,
    pub uses: BTreeSet<String>,
    pub untranslated: usize,
    pub helpers: usize,
    pub lib: IndexMap<String, LibEntry>,
    pub names: BTreeSet<String>,
    pub notes: IndexMap<String, Vec<String>>,
    pub numeric: BTreeSet<String>,
    pub symbolic: BTreeSet<String>,
    pub scope: IndexMap<String, String>,
    pub util_rels: BTreeSet<String>,
    /// `relation`'s token relations: (signature, name).
    pub token_rels: Vec<(String, String)>,
    /// `transform_lits`'s definitions: target -> (source, literals). None until a transform is lowered.
    pub transform_defs: Option<IndexMap<String, (String, Vec<String>)>>,
    pub arg_rels: Vec<(ArgKey, (String, String))>,
    pub arena: Vec<Entry>,
    /// The guest captures a `host` rule reads (`host_reads`), compared by text across documents.
    pub host_reads: BTreeSet<String>,
}

impl Program {
    pub fn new(lang: &str) -> Self {
        Program { lang: lang.to_owned(), ..Default::default() }
    }

    /// The relation of the util `name` where it is called or defined: the clause's own, if it has one.
    pub fn util_rel(&self, name: &str) -> String {
        self.scope.get(name).cloned().unwrap_or_else(|| super::lower::util_name(name))
    }

    pub fn helper(&mut self, base: &str, note: &str) -> String {
        self.helpers += 1;
        let name = format!("{base}_{}", self.helpers);
        self.decls.insert(name.clone(), format!(".decl {name}(n: node)"));
        self.notes.insert(name.clone(), vec![format!("// {name}(n): {note}")]);
        name
    }

    pub fn facts(&mut self, base: &str, col: &str, values: Vec<Value>, note: &str) -> String {
        self.helpers += 1;
        let name = format!("{base}_{}", self.helpers);
        self.decls.insert(name.clone(), format!(".decl {name}({col}: symbol)"));
        self.notes.insert(name.clone(), vec![format!("// {name}: {note}")]);
        self.rules.push(PRule::Facts { name: name.clone(), vals: values });
        name
    }

    pub fn use_(&mut self, module: &str) {
        let lib = match self.lang.as_str() {
            "python" => "python",
            "typescript" | "javascript" | "tsx" => "tsx",
            other => other,
        };
        self.uses.insert(format!("{lib}/{module}"));
    }

    /// A new entry in the arena.
    pub fn entry(&mut self, e: Entry) -> usize {
        self.arena.push(e);
        self.arena.len() - 1
    }

    /// A helper rule: `prog.rules.append((head, lits))`.
    pub fn add_rule(&mut self, head: String, lits: Vec<String>) {
        let id = self.entry(Entry { head, lits, ..Default::default() });
        self.rules.push(PRule::Rule(id));
    }
}

/// Fresh variable names: `p1`, `k2`, … per rule.
#[derive(Default)]
pub struct Vars {
    pub n: usize,
}

impl Vars {
    pub fn fresh(&mut self, p: &str) -> String {
        self.n += 1;
        format!("{p}{}", self.n)
    }
}
