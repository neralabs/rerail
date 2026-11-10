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
use std::sync::{Arc, LazyLock, Mutex};
use tree_sitter::{Language, Node as TsNode};

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
}

/// Per state, the implied token word to each named child (and to the end) it reaches through
/// tokens alone: the only shortest one.
#[derive(Default)]
struct Implied {
    to: FxHashMap<(u32, Label), Box<[Label]>>,
    end: Vec<Option<Box<[Label]>>>,
}

/// Every visible kind's automaton for one grammar.
pub(in crate::code_rule_dsl) struct TokenGrammar {
    names: FxHashMap<String, Sym>,
    texts: Vec<String>,
    kinds: FxHashMap<Sym, Automaton>,
    /// Per tree-sitter field id: its symbol (0 where the grammar has none).
    field_sym: Vec<Sym>,
    pub(in crate::code_rule_dsl) unsupported: BTreeSet<String>,
    /// The grammar's extras (comments and the like): in no automaton, anywhere among children.
    extras: BTreeSet<String>,
    /// Every token text some kind's automaton implies somewhere.
    implied_texts: BTreeSet<String>,
}

/// A parent's gaps ([`TokenGrammar::gaps`]): per stored child, the implied word just before it
/// (empty where the gap is empty, stored, or before a token or an extra), and the word after the
/// last child. Each token is its text and its field (`""` for none).
#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct Gaps<'g> {
    pub(in crate::code_rule_dsl) before: Vec<Vec<(&'g str, &'g str)>>,
    pub(in crate::code_rule_dsl) end: Vec<(&'g str, &'g str)>,
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
    pub(in crate::code_rule_dsl) fn from_json(json: &str, language: &Language) -> Result<Self, String> {
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
            let sym = builder.sym(name);
            kinds.insert(sym, Automaton { moves, accepting, implied });
        }
        let extras = grammar["extras"].as_array().into_iter().flatten().filter_map(|extra| extra["name"].as_str().map(str::to_owned)).collect();
        let Builder { names, texts, .. } = builder;
        let field_sym = (0..=language.field_count() as u16)
            .map(|id| language.field_name_for_id(id).and_then(|name| names.get(name).copied()).unwrap_or(0))
            .collect();
        let implied_texts = kinds
            .values()
            .flat_map(|automaton| automaton.implied.to.values().chain(automaton.implied.end.iter().flatten()))
            .flatten()
            .map(|label| texts[label.name as usize].clone())
            .collect();
        Ok(Self { names, texts, kinds, field_sym, unsupported, extras, implied_texts })
    }

    pub(in crate::code_rule_dsl) fn kind_count(&self) -> usize {
        self.kinds.len()
    }

    #[cfg(test)]
    fn text(&self, sym: Sym) -> &str {
        &self.texts[sym as usize]
    }

    /// Which of `node`'s children the tape stores, in order: `false` for an implied token (see the
    /// module). `None` where it stores them all: a node with no automaton, or one whose children
    /// the automaton cannot walk. A MISSING child walks the automaton as the child it stands for
    /// and is always stored: it is a syntax error.
    pub(in crate::code_rule_dsl) fn stored_children(&self, node: TsNode<'_>) -> Option<Vec<bool>> {
        if node.is_error() || !node.is_named() || node.child_count() == 0 {
            return None;
        }
        let automaton = self.kinds.get(self.names.get(node.kind())?)?;
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return None;
        }
        let mut stored = Vec::with_capacity(node.child_count() as usize);
        let mut gap: Vec<(usize, Label)> = Vec::new();
        let mut missing = Vec::new();
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
                    missing.push(position);
                }
                let name = *self.names.get(child.kind())?;
                let field = cursor.field_id().map_or(0, |id| self.field_sym.get(usize::from(id.get())).copied().unwrap_or(0));
                if child.is_named() {
                    state = gap_end(automaton, state, &gap, Some(Label { token: false, name, field }), &mut stored)?;
                    gap.clear();
                } else {
                    if child.child_count() > 0 {
                        return None;
                    }
                    gap.push((position, Label { token: true, name, field }));
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        gap_end(automaton, state, &gap, None, &mut stored)?;
        for position in missing {
            stored[position] = true;
        }
        Some(stored)
    }
}

impl TokenGrammar {
    /// Every token text some kind's automaton implies somewhere: what a tape may leave out.
    pub(in crate::code_rule_dsl) fn implied_texts(&self) -> &BTreeSet<String> {
        &self.implied_texts
    }

    /// Whether `kind` is one of the grammar's extras (a comment and the like).
    pub(in crate::code_rule_dsl) fn is_extra(&self, kind: &str) -> bool {
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
                state = walk(state, &pending)?;
                pending.clear();
            }
            state = *automaton.moves[state as usize].get(&label)?;
        }
        if pending.is_empty() {
            gaps.end = automaton.implied.end[state as usize].as_deref().map(word).unwrap_or_default();
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
    let implied = match next {
        Some(label) => automaton.implied.to.get(&(state, label)),
        None => automaton.implied.end[state as usize].as_ref(),
    };
    if implied.is_some_and(|word| word.len() == gap.len() && word.iter().zip(gap).all(|(a, (_, b))| a == b)) {
        for &(position, _) in gap {
            stored[position] = false;
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

/// What the automata say about where tokens are implied, for the census of rules that test
/// tokens (`token_readers`).
#[cfg(test)]
impl TokenGrammar {
    /// Per kind: the (token, field) pairs some implied word of its automaton holds. `""` is no
    /// field.
    pub(in crate::code_rule_dsl) fn implied_tokens(&self) -> BTreeMap<String, BTreeSet<(String, String)>> {
        let mut out: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for (kind, automaton) in &self.kinds {
            let words = automaton.implied.to.values().chain(automaton.implied.end.iter().flatten());
            let entry = out.entry(self.texts[*kind as usize].clone()).or_default();
            for label in words.flatten() {
                entry.insert((self.texts[label.name as usize].clone(), self.texts[label.field as usize].clone()));
            }
        }
        out
    }

    /// Per kind: the kinds whose automata have it as a named child.
    pub(in crate::code_rule_dsl) fn parents(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (kind, automaton) in &self.kinds {
            for label in automaton.moves.iter().flat_map(|moves| moves.keys()).filter(|label| !label.token) {
                out.entry(self.texts[label.name as usize].clone()).or_default().insert(self.texts[*kind as usize].clone());
            }
        }
        out
    }

    /// Every token text some automaton moves on.
    pub(in crate::code_rule_dsl) fn token_texts(&self) -> BTreeSet<String> {
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
pub(in crate::code_rule_dsl) fn grammar_json(lang: GrammarDialect) -> Option<&'static str> {
    macro_rules! grammar {
        ($path:literal) => {
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/grammars/", $path, "/grammar.json"))
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
    pub(in crate::code_rule_dsl) fn of_dialect(lang: GrammarDialect) -> Option<Arc<Self>> {
        let grammar = super::Grammar::new(lang).ok()?;
        Self::of(lang, grammar.kinds.grammar()?)
    }

    /// A dialect's automata, built once per process from its `grammar.json`; `None` where there
    /// is none (a dialect with its own parser, or no grammar on disk).
    pub(in crate::code_rule_dsl) fn of(lang: GrammarDialect, language: &Language) -> Option<Arc<Self>> {
        static BUILT: LazyLock<Mutex<FxHashMap<GrammarDialect, Option<Arc<TokenGrammar>>>>> = LazyLock::new(Default::default);
        let mut built = BUILT.lock().ok()?;
        built
            .entry(lang)
            .or_insert_with(|| grammar_json(lang).and_then(|json| Self::from_json(json, language).ok()).map(Arc::new))
            .clone()
    }
}
