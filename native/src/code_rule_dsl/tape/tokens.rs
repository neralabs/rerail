//! Named-only tapes (RER-131): which anonymous tokens the tape stores.
//!
//! A tree-sitter node's visible children follow its rule in `grammar.json`. For each visible kind
//! this module builds the rule's automaton over the children (named children labelled by kind and
//! field, anonymous tokens by kind and field; hidden rules inlined) and determinizes it. Between
//! two named children, and after the last, the tokens are a *gap*.
//!
//! A gap's tokens are **implied** where they are the only shortest token word from the gap's
//! state to its next named child (or to the end): the kind and the named children say what they
//! are. The tape's writer leaves implied tokens out as it walks tree-sitter's tree
//! ([`TokenGrammar::stored_children`]); every other token is stored, a leaf under its parent, as
//! tree-sitter has it. So `f(a, b)` stores none of its punctuation, `f(a, b,)` stores its trailing
//! `,` and `)`, and `a + b` stores its `+` (as short as `-`).
//!
//! Two classes of token, by text alone ([`is_delimiter`]):
//! - a **delimiter** (a bracket, a separator, a terminator, a quote) is left out wherever its gap
//!   is implied: where it is stored, the shape alone does not say it is there (`f(a,)`);
//! - every other token (a keyword, an operator, `:` `.` `::` `..` `=>`) is left out only where its
//!   kind implies it in every gap it can stand in ([`kept`]): `return` in Python's
//!   `return_statement`, `=` in a keyword argument. Where some gap holds it beside a choice
//!   (`async def`, `export default function f`), it is stored in every gap. So such a token is
//!   either always stored under its kind or never, and a rule that reads it reads it exactly; one
//!   its kind always implies is the kind's question (`kind(n, "return_statement")`).
//!
//! No reader rebuilds an implied token as a node. A rule that asks about one asks the grammar
//! instead: [`TokenGrammar::gaps`] walks a parent's stored children through its automaton and says
//! which word each gap holds, from the kinds and fields alone (the gap test, t31). A node
//! the automaton cannot walk (an ERROR, a contextual alias, a grammar the tree disagrees with)
//! keeps all of its tokens. Nothing here is chosen by hand: a new grammar version is a new
//! automaton, never a reclassification.

use crate::code_rule_dsl::language::GrammarDialect;
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use tree_sitter::{Language, Node as TsNode, TreeCursor};

/// An interned name (a kind or a field; 0 is "no field").
type Sym = u32;

/// A child label: a named child or a token, with its field.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Label {
    token: bool,
    name: Sym,
    field: Sym,
}

/// One kind's determinized automaton and its implied gaps.
struct Automaton {
    /// Per state: its moves, by label.
    moves: Vec<FxHashMap<Label, u32>>,
    accepting: Vec<bool>,
    implied: Implied,
    /// The tokens other than delimiters that some gap holds beside a choice ([`kept`]): stored
    /// wherever they stand, implied gaps included.
    kept: rustc_hash::FxHashSet<Label>,
}

impl Automaton {
    /// The tokens a tape of this kind may leave out: those of its implied words it does not keep.
    fn dropped(&self) -> impl Iterator<Item = &Label> + '_ {
        self.implied.to.values().chain(self.implied.end.iter().flatten()).flatten().filter(|label| !self.kept.contains(label))
    }
}

/// Whether a token is a delimiter (see the module): a bracket, a separator, a terminator or a
/// quote, whose place the shape records. Every other token is kept wherever a gap holds it beside
/// a choice.
pub(in crate::code_rule_dsl) fn is_delimiter(text: &str) -> bool {
    matches!(
        text,
        "(" | ")" | "[" | "]" | "{" | "}" | "<" | ">" | "," | ";" | "|" | "\"" | "'" | "`" | "\\'" | "${" | "{|" | "|}" | "#[" | "{{" | "}}" | "</" | "/>"
    )
}

/// The tokens other than delimiters ([`is_delimiter`]) that are not implied in every gap they
/// can stand in: from the start or after a named child (where a gap starts), some token word to
/// the next named child (or the end) holds the token and is not the implied word. Words are
/// walked to a bound; a gap whose words pass it, or loop, keeps every token it can hold.
fn kept(moves: &[FxHashMap<Label, u32>], accepting: &[bool], implied: &Implied, texts: &[String]) -> rustc_hash::FxHashSet<Label> {
    const LONGEST: usize = 8;
    const WORDS: usize = 512;
    let mut kept = rustc_hash::FxHashSet::default();
    let mut starts: BTreeSet<u32> = BTreeSet::from([0]);
    for state_moves in moves {
        starts.extend(state_moves.iter().filter(|(label, _)| !label.token).map(|(_, &to)| to));
    }
    let keep = |word: &[Label], kept: &mut rustc_hash::FxHashSet<Label>| {
        kept.extend(word.iter().filter(|label| !is_delimiter(&texts[label.name as usize])));
    };
    for from in starts {
        // Depth first over token moves: (state, word so far).
        let mut stack: Vec<(u32, Vec<Label>, Vec<u32>)> = vec![(from, Vec::new(), vec![from])];
        let mut words = 0usize;
        let mut bounded = true;
        while let Some((state, word, path)) = stack.pop() {
            words += 1;
            if words > WORDS {
                bounded = false;
                break;
            }
            let targets = moves[state as usize].keys().filter(|label| !label.token).map(|label| implied.to.get(&(from, *label)));
            let ends = accepting[state as usize].then(|| implied.end[from as usize].as_ref());
            for implied_word in targets.chain(ends) {
                if !word.is_empty() && implied_word.is_none_or(|implied_word| **implied_word != *word) {
                    keep(&word, &mut kept);
                }
            }
            for (label, &to) in &moves[state as usize] {
                if !label.token {
                    continue;
                }
                let mut next = word.clone();
                next.push(*label);
                if path.contains(&to) || next.len() > LONGEST {
                    bounded = false;
                    break;
                }
                let mut next_path = path.clone();
                next_path.push(to);
                stack.push((to, next, next_path));
            }
            if !bounded {
                break;
            }
        }
        if !bounded {
            // Every token a gap from here can hold.
            let mut seen = BTreeSet::from([from]);
            let mut todo = vec![from];
            while let Some(state) = todo.pop() {
                for (label, &to) in &moves[state as usize] {
                    if label.token {
                        keep(std::slice::from_ref(label), &mut kept);
                        if seen.insert(to) {
                            todo.push(to);
                        }
                    }
                }
            }
        }
    }
    kept
}

/// Per state, the implied token word to each named child (and to the end) it reaches through
/// tokens alone: the only shortest one.
#[derive(Default)]
struct Implied {
    to: FxHashMap<(u32, Label), Box<[Label]>>,
    end: Vec<Option<Box<[Label]>>>,
}

/// Temporary buffers for one tree walk. Ancestor masks stay on the encoder's
/// stack; a completed mask can serve the next parent. Gaps never outlive a call.
#[derive(Default)]
pub(super) struct FilterScratch {
    masks: Vec<Vec<bool>>,
    gap: Vec<(usize, Label)>,
    missing: Vec<usize>,
}

impl FilterScratch {
    pub(super) fn recycle(&mut self, mut mask: Vec<bool>) {
        mask.clear();
        self.masks.push(mask);
    }
}

/// Every visible kind's automaton for one grammar.
pub struct TokenGrammar {
    names: FxHashMap<String, Sym>,
    texts: Vec<String>,
    kinds: FxHashMap<Sym, Automaton>,
    /// Per tree-sitter field id: its symbol (0 where the grammar has none).
    field_sym: Vec<Sym>,
    /// Symbols and filtering need by tree-sitter kind id, computed once per grammar.
    kind_sym: Vec<Option<Sym>>,
    filters: BTreeSet<Sym>,
    pub(in crate::code_rule_dsl) unsupported: BTreeSet<String>,
    /// The grammar's extras (comments and the like): in no automaton, anywhere among children.
    extras: BTreeSet<String>,
    /// Every token text some kind's automaton implies somewhere.
    implied_texts: BTreeSet<String>,
    /// Every token text and field name, sorted, leaked once per dialect ([`Self::intern`]).
    interned: Vec<&'static str>,
}

/// A parent's gaps ([`TokenGrammar::gaps`]): per stored child, the implied word just before it
/// (empty where the gap is empty, stored, or before a token or an extra), and the word after the
/// last child. Each token is its text and its field (`""` for none).
#[derive(Debug, Default)]
pub struct Gaps<'g> {
    pub before: Vec<Vec<(&'g str, &'g str)>>,
    pub end: Vec<(&'g str, &'g str)>,
}

// ---------------------------------------------------------------------------------------------
// Building the automata from grammar.json.

struct Nfa {
    edges: Vec<Vec<(Option<Label>, u32)>>,
}

impl Nfa {
    fn state(&mut self) -> u32 {
        self.edges.push(Vec::new());
        (self.edges.len() - 1) as u32
    }
    fn edge(&mut self, from: u32, label: Option<Label>, to: u32) {
        self.edges[from as usize].push((label, to));
    }
}

struct Builder<'g> {
    rules: &'g serde_json::Map<String, Value>,
    /// Supertypes and inlined rules: no node of their own, like a `_`-named rule.
    hidden: BTreeSet<String>,
    names: FxHashMap<String, Sym>,
    texts: Vec<String>,
    recursive: bool,
}

const PREC: [&str; 4] = ["PREC", "PREC_LEFT", "PREC_RIGHT", "PREC_DYNAMIC"];

fn unwrap_prec(mut rule: &Value) -> &Value {
    while PREC.contains(&rule["type"].as_str().unwrap_or("")) {
        rule = &rule["content"];
    }
    rule
}

impl<'g> Builder<'g> {
    fn sym(&mut self, name: &str) -> Sym {
        if let Some(&sym) = self.names.get(name) {
            return sym;
        }
        let sym = self.texts.len() as Sym;
        self.texts.push(name.to_owned());
        self.names.insert(name.to_owned(), sym);
        sym
    }

    /// A rule that is one token: a lexical variable (named if visible, else no node). A string
    /// under a precedence is not one: tree-sitter makes it a node with the string its anonymous
    /// child (Python's `pass_statement`).
    fn is_token_rule(&self, name: &str) -> bool {
        match self.rules.get(name) {
            None => true,
            Some(rule) => matches!(rule["type"].as_str(), Some("STRING" | "PATTERN" | "TOKEN" | "IMMEDIATE_TOKEN")),
        }
    }

    fn build(&mut self, rule: &Value, nfa: &mut Nfa, from: u32, to: u32, field: Sym, stack: &mut Vec<String>) {
        let kind = rule["type"].as_str().unwrap_or("");
        match kind {
            "BLANK" => nfa.edge(from, None, to),
            // Hidden text: no node.
            "PATTERN" => nfa.edge(from, None, to),
            "STRING" => {
                let name = self.sym(rule["value"].as_str().unwrap_or(""));
                nfa.edge(from, Some(Label { token: true, name, field }), to);
            }
            "TOKEN" | "IMMEDIATE_TOKEN" => {
                let inner = unwrap_prec(&rule["content"]);
                if inner["type"] == "STRING" {
                    let name = self.sym(inner["value"].as_str().unwrap_or(""));
                    nfa.edge(from, Some(Label { token: true, name, field }), to);
                } else {
                    // An auxiliary token: hidden, no node.
                    nfa.edge(from, None, to);
                }
            }
            "SEQ" => {
                let mut at = from;
                for member in rule["members"].as_array().into_iter().flatten() {
                    let next = nfa.state();
                    self.build(member, nfa, at, next, field, stack);
                    at = next;
                }
                nfa.edge(at, None, to);
            }
            "CHOICE" => {
                for member in rule["members"].as_array().into_iter().flatten() {
                    self.build(member, nfa, from, to, field, stack);
                }
            }
            "REPEAT" | "REPEAT1" => {
                let (start, end) = (nfa.state(), nfa.state());
                nfa.edge(from, None, start);
                self.build(&rule["content"], nfa, start, end, field, stack);
                nfa.edge(end, None, start);
                nfa.edge(end, None, to);
                if kind == "REPEAT" {
                    nfa.edge(from, None, to);
                }
            }
            "FIELD" => {
                let field = self.sym(rule["name"].as_str().unwrap_or(""));
                self.build(&rule["content"], nfa, from, to, field, stack);
            }
            "ALIAS" => {
                let name = self.sym(rule["value"].as_str().unwrap_or(""));
                let token = !rule["named"].as_bool().unwrap_or(false);
                nfa.edge(from, Some(Label { token, name, field }), to);
            }
            "SYMBOL" => {
                let name = rule["name"].as_str().unwrap_or("");
                let hidden = name.starts_with('_') || self.hidden.contains(name);
                if hidden && self.rules.contains_key(name) && !self.is_token_rule(name) {
                    if stack.iter().any(|open| open == name) {
                        self.recursive = true;
                        nfa.edge(from, None, to);
                        return;
                    }
                    stack.push(name.to_owned());
                    let inner = &self.rules[name];
                    self.build(inner, nfa, from, to, field, stack);
                    stack.pop();
                } else if hidden {
                    nfa.edge(from, None, to);
                } else {
                    let name = self.sym(name);
                    nfa.edge(from, Some(Label { token: false, name, field }), to);
                }
            }
            _ => {
                if PREC.contains(&kind) || kind == "RESERVED" {
                    self.build(&rule["content"], nfa, from, to, field, stack);
                } else {
                    nfa.edge(from, None, to);
                }
            }
        }
    }
}

fn closure(nfa: &Nfa, states: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let mut todo: Vec<u32> = Vec::new();
    for state in states {
        if seen.insert(state) {
            todo.push(state);
        }
    }
    while let Some(state) = todo.pop() {
        for &(label, to) in &nfa.edges[state as usize] {
            if label.is_none() && seen.insert(to) {
                todo.push(to);
            }
        }
    }
    seen.into_iter().collect()
}

const STATE_CAP: usize = 20_000;

fn determinize(nfa: &Nfa, start: u32, accept: u32) -> Option<(Vec<FxHashMap<Label, u32>>, Vec<bool>)> {
    let first = closure(nfa, [start]);
    let mut ids: FxHashMap<Vec<u32>, u32> = FxHashMap::default();
    let mut order = vec![first.clone()];
    ids.insert(first, 0);
    let mut moves = Vec::new();
    let mut accepting = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let set = order[i].clone();
        accepting.push(set.contains(&accept));
        let mut by_label: BTreeMap<Label, Vec<u32>> = BTreeMap::new();
        for &state in &set {
            for &(label, to) in &nfa.edges[state as usize] {
                if let Some(label) = label {
                    by_label.entry(label).or_default().push(to);
                }
            }
        }
        let mut out = FxHashMap::default();
        for (label, targets) in by_label {
            let target = closure(nfa, targets);
            let id = match ids.get(&target) {
                Some(&id) => id,
                None => {
                    if order.len() >= STATE_CAP {
                        return None;
                    }
                    let id = order.len() as u32;
                    ids.insert(target.clone(), id);
                    order.push(target);
                    id
                }
            };
            out.insert(label, id);
        }
        moves.push(out);
        i += 1;
    }
    Some((moves, accepting))
}

impl TokenGrammar {
    /// Every visible kind's automaton from `json` (a `grammar.json`), its names resolved in
    /// `language`.
    pub fn from_json(json: &str, language: &Language) -> Result<Self, String> {
        let grammar: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
        let rules = grammar["rules"].as_object().ok_or("grammar.json has no rules")?;
        let listed = |key: &str| -> BTreeSet<String> {
            grammar[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned).or_else(|| v["name"].as_str().map(str::to_owned))).collect()
        };
        let hidden: BTreeSet<String> = listed("supertypes").into_iter().chain(listed("inline")).collect();
        let mut builder = Builder { rules, hidden, names: FxHashMap::default(), texts: Vec::new(), recursive: false };
        builder.sym("");
        // Each visible kind's productions: its own rule, and the rules aliased to its name.
        let mut productions: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
        for (name, rule) in rules {
            if !name.starts_with('_') && !builder.hidden.contains(name) && !builder.is_token_rule(name) {
                productions.entry(name.clone()).or_default().push(rule);
            }
        }
        fn aliases<'v>(rule: &'v Value, rules: &'v serde_json::Map<String, Value>, out: &mut BTreeMap<String, Vec<&'v Value>>) {
            if rule["type"] == "ALIAS" && rule["named"].as_bool() == Some(true) {
                let name = rule["value"].as_str().unwrap_or("").to_owned();
                let mut content = &rule["content"];
                if content["type"] == "SYMBOL"
                    && let Some(target) = rules.get(content["name"].as_str().unwrap_or(""))
                {
                    content = target;
                }
                out.entry(name).or_default().push(content);
            }
            if let Some(members) = rule["members"].as_array() {
                for member in members {
                    aliases(member, rules, out);
                }
            }
            if rule.get("content").is_some() {
                aliases(&rule["content"], rules, out);
            }
        }
        let mut aliased = BTreeMap::new();
        for rule in rules.values() {
            aliases(rule, rules, &mut aliased);
        }
        for (name, contents) in aliased {
            productions.entry(name).or_default().extend(contents);
        }
        let mut kinds = FxHashMap::default();
        let mut unsupported = BTreeSet::new();
        for (name, rules_of) in &productions {
            let mut nfa = Nfa { edges: Vec::new() };
            let (start, accept) = (nfa.state(), nfa.state());
            builder.recursive = false;
            for rule in rules_of {
                let mut stack = Vec::new();
                builder.build(rule, &mut nfa, start, accept, 0, &mut stack);
            }
            if builder.recursive {
                // A hidden rule that reaches itself: the automaton would be an approximation.
                unsupported.insert(name.clone());
                continue;
            }
            let Some((moves, accepting)) = determinize(&nfa, start, accept) else {
                unsupported.insert(name.clone());
                continue;
            };
            let implied = implied(&moves, &accepting);
            let kept = kept(&moves, &accepting, &implied, &builder.texts);
            let sym = builder.sym(name);
            kinds.insert(sym, Automaton { moves, accepting, implied, kept });
        }
        let extras = grammar["extras"].as_array().into_iter().flatten().filter_map(|extra| extra["name"].as_str().map(str::to_owned)).collect();
        let Builder { names, texts, .. } = builder;
        let field_sym = (0..=language.field_count() as u16)
            .map(|id| language.field_name_for_id(id).and_then(|name| names.get(name).copied()).unwrap_or(0))
            .collect();
        let kind_sym = (0..language.node_kind_count() as u16)
            .map(|id| language.node_kind_for_id(id).and_then(|name| names.get(name).copied())).collect();
        let filters = kinds.iter().filter(|(_, a)| a.dropped().next().is_some()).map(|(&sym, _)| sym).collect();
        let implied_texts = kinds
            .values()
            .flat_map(Automaton::dropped)
            .map(|label| texts[label.name as usize].clone())
            .collect();
        let mut interned: Vec<&'static str> = texts.iter().map(|text| &*Box::leak(text.clone().into_boxed_str())).collect();
        interned.push("");
        interned.sort_unstable();
        interned.dedup();
        Ok(Self { names, texts, kinds, field_sym, kind_sym, filters, unsupported, extras, implied_texts, interned })
    }

    pub fn kind_count(&self) -> usize {
        self.kinds.len()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    fn text(&self, sym: Sym) -> &str {
        &self.texts[sym as usize]
    }

    /// Which of `node`'s children the tape stores, in order: `false` for an implied token (see the
    /// module). `None` where it stores them all: a node with no automaton, or one whose children
    /// the automaton cannot walk. A MISSING child walks the automaton as the child it stands for
    /// and is always stored: it is a syntax error.
    pub fn stored_children(&self, node: TsNode<'_>) -> Option<Vec<bool>> {
        self.stored_children_in(node, &mut node.walk(), &mut FilterScratch::default())
    }

    pub(super) fn stored_children_in<'tree>(&self, node: TsNode<'tree>, cursor: &mut TreeCursor<'tree>, scratch: &mut FilterScratch) -> Option<Vec<bool>> {
        if node.is_error() || !node.is_named() || node.child_count() == 0 {
            return None;
        }
        let sym = self.kind_sym.get(node.kind_id() as usize).copied().flatten()?;
        if !self.filters.contains(&sym) { return None; }
        let automaton = self.kinds.get(&sym)?;
        cursor.reset(node);
        if !cursor.goto_first_child() {
            return None;
        }
        let mut stored = scratch.masks.pop().unwrap_or_default();
        stored.reserve(node.child_count() as usize);
        scratch.gap.clear();
        scratch.missing.clear();
        let walked = (|| {
            let mut state = 0u32;
            loop {
                let child = cursor.node();
                let position = stored.len();
                stored.push(true);
                if !child.is_extra() {
                    if child.is_error() {
                        return None;
                    }
                    if child.is_missing() {
                        scratch.missing.push(position);
                    }
                    let name = self.kind_sym.get(child.kind_id() as usize).copied().flatten()?;
                    let field = cursor.field_id().map_or(0, |id| self.field_sym.get(usize::from(id.get())).copied().unwrap_or(0));
                    if child.is_named() {
                        state = gap_end(automaton, state, &scratch.gap, Some(Label { token: false, name, field }), &mut stored)?;
                        scratch.gap.clear();
                    } else {
                        if child.child_count() > 0 {
                            return None;
                        }
                        scratch.gap.push((position, Label { token: true, name, field }));
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            gap_end(automaton, state, &scratch.gap, None, &mut stored)?;
            for &position in &scratch.missing {
                stored[position] = true;
            }
            Some(())
        })();
        if walked.is_some() {
            Some(stored)
        } else {
            scratch.recycle(stored);
            None
        }
    }
}

impl TokenGrammar {
    /// Every token text some kind's automaton implies somewhere: what a tape may leave out.
    pub(in crate::code_rule_dsl) fn implied_texts(&self) -> &BTreeSet<String> {
        &self.implied_texts
    }

    /// Whether `regex` tests a token the grammar implies somewhere: it matches one. A regex that
    /// also matches names (`(?s).`, `^`) counts too: `not follows (?s).` asks for a first child,
    /// and a node after an implied `(` was never one.
    pub fn tests_implied(&self, regex: &regex::Regex) -> bool {
        self.implied_texts.iter().any(|token| regex.is_match(token))
    }

    /// Whether `regex` tests a token the grammar implies in `field` somewhere: a `has` in a field
    /// whose implied words hold no such token asks only for stored children.
    pub fn tests_implied_in(&self, regex: &regex::Regex, field: &str) -> bool {
        self.kinds.values().flat_map(Automaton::dropped).any(|label| self.texts[label.field as usize] == field && regex.is_match(&self.texts[label.name as usize]))
    }

    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub fn is_extra(&self, kind: &str) -> bool {
        self.extras.contains(kind)
    }

    /// The field names `kind`'s automaton labels its children with.
    pub(in crate::code_rule_dsl) fn fields_of(&self, kind: &str) -> Vec<&str> {
        let Some(automaton) = self.names.get(kind).and_then(|sym| self.kinds.get(sym)) else { return Vec::new() };
        let fields: BTreeSet<Sym> = automaton.moves.iter().flat_map(|moves| moves.keys()).map(|label| label.field).filter(|&field| field != 0).collect();
        fields.into_iter().map(|field| self.texts[field as usize].as_str()).collect()
    }

    /// The gaps of a `kind` node whose stored children are `children` (named, kind, field; `""`
    /// for no field), in order, as the tape holds them: walked through the kind's automaton, a gap
    /// with no stored token holds the implied word to the next named child (or to the end).
    /// `None` where the kind has no automaton or the automaton refuses the children.
    pub(in crate::code_rule_dsl) fn gaps<'c>(&self, kind: &str, children: impl IntoIterator<Item = (bool, &'c str, &'c str)>) -> Option<Gaps<'_>> {
        let automaton = self.kinds.get(self.names.get(kind)?)?;
        let word = |labels: &[Label]| labels.iter().map(|label| (self.texts[label.name as usize].as_str(), self.texts[label.field as usize].as_str())).collect::<Vec<_>>();
        let walk = |mut state: u32, labels: &[Label]| -> Option<u32> {
            for label in labels {
                state = *automaton.moves[state as usize].get(label)?;
            }
            Some(state)
        };
        let kept_of = |implied: &[Label]| implied.iter().filter(|label| automaton.kept.contains(label)).copied().collect::<Vec<_>>();
        let mut gaps = Gaps::default();
        let (mut state, mut pending) = (0u32, Vec::<Label>::new());
        for (named, child, field) in children {
            gaps.before.push(Vec::new());
            if self.extras.contains(child) {
                continue;
            }
            let label = Label { token: !named, name: *self.names.get(child)?, field: if field.is_empty() { 0 } else { *self.names.get(field)? } };
            if !named {
                pending.push(label);
                continue;
            }
            if pending.is_empty() {
                // An empty gap: the implied word, which is empty where the child follows directly.
                match automaton.implied.to.get(&(state, label)) {
                    Some(implied) => {
                        *gaps.before.last_mut()? = word(implied);
                        state = walk(state, implied)?;
                    }
                    None if automaton.moves[state as usize].contains_key(&label) => {}
                    None => return None,
                }
            } else {
                // A stored gap, or an implied one whose kept tokens ([`kept`]) are stored.
                match walk(state, &pending) {
                    Some(to) => state = to,
                    None => {
                        let implied = automaton.implied.to.get(&(state, label)).filter(|implied| kept_of(implied) == pending)?;
                        *gaps.before.last_mut()? = word(implied);
                        state = walk(state, implied)?;
                    }
                }
                pending.clear();
            }
            state = *automaton.moves[state as usize].get(&label)?;
        }
        let end = automaton.implied.end[state as usize].as_deref();
        if pending.is_empty() || walk(state, &pending).is_none() && end.is_some_and(|implied| kept_of(implied) == pending) {
            gaps.end = end.map(word).unwrap_or_default();
        }
        Some(gaps)
    }
}

/// Closes a gap from `state` before `next` (a named child; `None` at the end): marks its tokens
/// not stored where they are the implied word. The state after `next`; `None` where the automaton
/// refuses the gap or `next`.
fn gap_end(automaton: &Automaton, state: u32, gap: &[(usize, Label)], next: Option<Label>, stored: &mut [bool]) -> Option<u32> {
    let mut at = state;
    for (_, label) in gap {
        at = *automaton.moves[at as usize].get(label)?;
    }
    let after = match next {
        Some(label) => *automaton.moves[at as usize].get(&label)?,
        None if automaton.accepting[at as usize] => at,
        None => return None,
    };
    // No token can be omitted from an empty gap. The transition above still
    // validates the next child (or acceptance); no implied-word lookup is needed.
    if gap.is_empty() {
        return Some(after);
    }
    let implied = match next {
        Some(label) => automaton.implied.to.get(&(state, label)),
        None => automaton.implied.end[state as usize].as_ref(),
    };
    if implied.is_some_and(|word| word.len() == gap.len() && word.iter().zip(gap).all(|(a, (_, b))| a == b)) {
        for (position, label) in gap {
            if !automaton.kept.contains(label) {
                stored[*position] = false;
            }
        }
    }
    Some(after)
}

/// Each state's implied words ([`Implied`]): a breadth-first walk over its token moves, counting
/// the shortest words to each state (a word reaches one state: the automaton is deterministic).
fn implied(moves: &[FxHashMap<Label, u32>], accepting: &[bool]) -> Implied {
    let mut implied = Implied { to: FxHashMap::default(), end: vec![None; moves.len()] };
    for from in 0..moves.len() as u32 {
        // Per state reached: its distance, its count of shortest words (at most 2), the move it was reached by.
        let mut seen: FxHashMap<u32, (u32, u8, Option<(u32, Label)>)> = FxHashMap::default();
        seen.insert(from, (0, 1, None));
        let mut order = vec![from];
        let mut queue = VecDeque::from([from]);
        while let Some(state) = queue.pop_front() {
            let (distance, count, _) = seen[&state];
            for (label, &to) in &moves[state as usize] {
                if !label.token {
                    continue;
                }
                match seen.get_mut(&to) {
                    None => {
                        seen.insert(to, (distance + 1, count, Some((state, *label))));
                        order.push(to);
                        queue.push_back(to);
                    }
                    Some(entry) if entry.0 == distance + 1 => entry.1 = entry.1.saturating_add(count).min(2),
                    Some(_) => {}
                }
            }
        }
        let word = |mut state: u32| -> Box<[Label]> {
            let mut word = Vec::new();
            while let Some((before, label)) = seen[&state].2 {
                word.push(label);
                state = before;
            }
            word.reverse();
            word.into()
        };
        // Per target (a named label, or the end): the shortest distance, its words, the state.
        let mut best: FxHashMap<Option<Label>, (u32, u8, u32)> = FxHashMap::default();
        for &state in &order {
            let (distance, count, _) = seen[&state];
            let targets = moves[state as usize].keys().filter(|label| !label.token).map(|label| Some(*label));
            let targets = targets.chain(accepting[state as usize].then_some(None));
            for target in targets {
                match best.get_mut(&target) {
                    None => {
                        best.insert(target, (distance, count, state));
                    }
                    Some(entry) if entry.0 == distance => entry.1 = entry.1.saturating_add(count).min(2),
                    Some(_) => {}
                }
            }
        }
        for (target, (_, count, state)) in best {
            if count != 1 {
                continue;
            }
            match target {
                Some(label) => {
                    implied.to.insert((from, label), word(state));
                }
                None => implied.end[from as usize] = Some(word(state)),
            }
        }
    }
    implied
}

/// What the automata say about where tokens are implied, for the check of rules that test
/// tokens (`compile::token_tests`).
impl TokenGrammar {
    /// Per kind: the (token, field) pairs a tape of it may leave out: those some implied word of
    /// its automaton holds, less the kept ones ([`kept`]). `""` is no
    /// field.
    pub fn implied_tokens(&self) -> BTreeMap<String, BTreeSet<(String, String)>> {
        let mut out: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for (kind, automaton) in &self.kinds {
            let entry = out.entry(self.texts[*kind as usize].clone()).or_default();
            for label in automaton.dropped() {
                entry.insert((self.texts[label.name as usize].clone(), self.texts[label.field as usize].clone()));
            }
        }
        out
    }

    /// Per kind: the kinds whose automata have it as a named child.
    pub fn parents(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (kind, automaton) in &self.kinds {
            for label in automaton.moves.iter().flat_map(|moves| moves.keys()).filter(|label| !label.token) {
                out.entry(self.texts[label.name as usize].clone()).or_default().insert(self.texts[*kind as usize].clone());
            }
        }
        out
    }

    /// Every token text some automaton moves on.
    pub fn token_texts(&self) -> BTreeSet<String> {
        self.kinds
            .values()
            .flat_map(|automaton| automaton.moves.iter().flat_map(|moves| moves.keys()))
            .filter(|label| label.token)
            .map(|label| self.texts[label.name as usize].clone())
            .collect()
    }
}

/// The `grammar.json` a dialect's parser was generated from, built into the engine (the vendored
/// grammars, and byte-identical copies of the Rust and PHP crates' under `grammars/token-grammars`);
/// `None` for a dialect whose tapes keep all their tokens.
pub fn grammar_json(lang: GrammarDialect) -> Option<&'static str> {
    macro_rules! grammar {
        ($path:literal) => {
            include_str!(concat!(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."), "/grammars/", $path, "/grammar.json"))
        };
    }
    Some(match lang {
        GrammarDialect::Python => grammar!("tree-sitter-python/src"),
        GrammarDialect::JavaScript => grammar!("tree-sitter-javascript/src"),
        GrammarDialect::TypeScript => grammar!("tree-sitter-typescript/typescript/src"),
        GrammarDialect::Tsx => grammar!("tree-sitter-typescript/tsx/src"),
        GrammarDialect::Sql => grammar!("tree-sitter-sql/src"),
        GrammarDialect::Vue => grammar!("tree-sitter-vue/src"),
        GrammarDialect::Rust => grammar!("token-grammars/rust"),
        GrammarDialect::Php => grammar!("token-grammars/php"),
        _ => return None,
    })
}

impl TokenGrammar {
    /// A dialect's automata, for its tape grammar's tree-sitter language; `None` where it has none.
    pub fn of_dialect(lang: GrammarDialect) -> Option<Arc<Self>> {
        // Built once per dialect, as `of` builds it; a lookup after that, for the token queries.
        static BY_DIALECT: [OnceLock<Option<Arc<TokenGrammar>>>; 256] = [const { OnceLock::new() }; 256];
        BY_DIALECT
            .get(lang as usize)?
            .get_or_init(|| {
                let grammar = super::Grammar::new(lang).ok()?;
                Self::of(lang, grammar.kinds.grammar()?)
            })
            .clone()
    }

    /// `text` (a token text or field name of this grammar) for the life of the process: the
    /// grammar is built once per dialect and never dropped.
    pub(in crate::code_rule_dsl) fn intern(&self, text: &str) -> &'static str {
        match self.interned.binary_search_by(|known| (*known).cmp(text)) {
            Ok(at) => self.interned[at],
            Err(_) => "",
        }
    }

    /// A dialect's automata, built once per process from its `grammar.json`; `None` where there
    /// is none (a dialect with its own parser, or no grammar on disk).
    pub fn of(lang: GrammarDialect, language: &Language) -> Option<Arc<Self>> {
        static BUILT: LazyLock<Mutex<FxHashMap<GrammarDialect, Option<Arc<TokenGrammar>>>>> = LazyLock::new(Default::default);
        let mut built = BUILT.lock().ok()?;
        built
            .entry(lang)
            .or_insert_with(|| grammar_json(lang).and_then(|json| Self::from_json(json, language).ok()).map(Arc::new))
            .clone()
    }
}

/// `parent`'s stored children and its gaps; `None` where its kind has no automaton.
pub fn stored_gaps<'t, 'g, D: ast_grep_core::Doc>(grammar: &'g TokenGrammar, parent: &ast_grep_core::Node<'t, D>) -> Option<(Vec<ast_grep_core::Node<'t, D>>, Gaps<'g>)> {
    let kind = parent.kind();
    let mut fields: std::collections::HashMap<usize, &str> = std::collections::HashMap::new();
    for field in grammar.fields_of(&kind) {
        for child in parent.field_children(field) {
            fields.insert(child.node_id(), field);
        }
    }
    let children: Vec<ast_grep_core::Node<'t, D>> = parent.children().collect();
    let kinds: Vec<(bool, String, &str)> = children.iter().map(|child| (child.is_named(), child.kind().into_owned(), fields.get(&child.node_id()).copied().unwrap_or(""))).collect();
    let gaps = grammar.gaps(&kind, kinds.iter().map(|(named, kind, field)| (*named, kind.as_str(), *field)))?;
    Some((children, gaps))
}
