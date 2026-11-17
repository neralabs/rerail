//! A grammar's token automata (`tokens`), built from its `grammar.json`. The lang crate's build script includes this
//! file and builds every vendored grammar's automata with the build (`GRAMMARS`), so a process decodes them
//! (`decode`) and never determinizes: building Python's took 20 ms of a small run's 0.3 s, every process. Nothing
//! here names the crate: the build script mounts it as a module of its own.

use rustc_hash::FxHashMap;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The grammars built with the build: a dialect's variant name and its `grammar.json`'s directory under
/// `native/grammars` (as `tokens::grammar_json` reads it).
pub(crate) const GRAMMARS: [(&str, &str); 21] = [
    ("Python", "tree-sitter-python/src"),
    ("JavaScript", "tree-sitter-javascript/src"),
    ("TypeScript", "tree-sitter-typescript/typescript/src"),
    ("Tsx", "tree-sitter-typescript/tsx/src"),
    ("Sql", "tree-sitter-sql/src"),
    ("Vue", "tree-sitter-vue/src"),
    ("Markdown", "tree-sitter-markdown/markdown/src"),
    ("MarkdownInline", "tree-sitter-markdown/markdown-inline/src"),
    ("Rust", "token-grammars/rust"),
    ("Php", "token-grammars/php"),
    ("Json", "token-grammars/json"),
    ("Java", "token-grammars/java"),
    ("CSharp", "token-grammars/csharp"),
    ("Html", "token-grammars/html"),
    ("Css", "token-grammars/css"),
    ("Kotlin", "token-grammars/kotlin"),
    ("Swift", "token-grammars/swift"),
    ("Scala", "token-grammars/scala"),
    ("Go", "token-grammars/go"),
    ("C", "token-grammars/c"),
    ("Cpp", "token-grammars/cpp"),
];

/// The grammars of `GRAMMARS` only tests parse (`GrammarDialect`'s test-only dialects): their arms are built
/// under `cfg(any(test, feature = "test-hooks"))`, as the dialects are. Their lean parsers' oracles read them.
pub(crate) const TEST_ONLY: [&str; 3] = ["Kotlin", "Swift", "Scala"];

/// An interned name (a kind or a field; 0 is "no field").
pub(crate) type Sym = u32;

/// A child label: a named child or a token, with its field.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Label {
    pub(crate) token: bool,
    pub(crate) name: Sym,
    pub(crate) field: Sym,
}

/// One kind's determinized automaton and its implied gaps.
pub(crate) struct Automaton {
    /// Per state: its moves, by label.
    pub(crate) moves: Vec<FxHashMap<Label, u32>>,
    pub(crate) accepting: Vec<bool>,
    pub(crate) implied: Implied,
    /// The tokens other than delimiters that some gap holds beside a choice ([`kept`]): stored
    /// wherever they stand, implied gaps included.
    pub(crate) kept: rustc_hash::FxHashSet<Label>,
    /// The fields its moves label children with, sorted ([`TokenGrammar::fields_of`]).
    pub(crate) fields: Vec<Sym>,
}

impl Automaton {
    /// The tokens a tape of this kind may leave out: those of its implied words it does not keep.
    pub(crate) fn dropped(&self) -> impl Iterator<Item = &Label> + '_ {
        self.implied.to.values().chain(self.implied.end.iter().flatten()).flatten().filter(|label| !self.kept.contains(label))
    }
}

/// Whether a token is a delimiter (see the module): a bracket, a separator, a terminator or a
/// quote, whose place the shape records. Every other token is kept wherever a gap holds it beside
/// a choice.
pub(crate) fn is_delimiter(text: &str) -> bool {
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
pub(crate) struct Implied {
    pub(crate) to: FxHashMap<(u32, Label), Box<[Label]>>,
    pub(crate) end: Vec<Option<Box<[Label]>>>,
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

/// A DFA's states: each one's moves by label, and whether it accepts.
type Dfa = (Vec<FxHashMap<Label, u32>>, Vec<bool>);

fn determinize(nfa: &Nfa, start: u32, accept: u32) -> Option<Dfa> {
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

/// Each state's implied words ([`Implied`]): a breadth-first walk over its token moves, counting
/// the shortest words to each state (a word reaches one state: the automaton is deterministic).
fn implied(moves: &[FxHashMap<Label, u32>], accepting: &[bool]) -> Implied {
    let mut implied = Implied { to: FxHashMap::default(), end: vec![None; moves.len()] };
    for from in 0..moves.len() as u32 {
        // Per state reached: its distance, its count of shortest words (at most 2), the move it was reached by.
        type Seen = (u32, u8, Option<(u32, Label)>);
        let mut seen: FxHashMap<u32, Seen> = FxHashMap::default();
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

/// A grammar's automata as built: its names (a symbol is an index; 0 is ""), each visible kind's automaton, the
/// kinds it has none for, and its extras.
pub(crate) struct Built {
    pub(crate) texts: Vec<String>,
    pub(crate) kinds: Vec<(Sym, Automaton)>,
    pub(crate) unsupported: BTreeSet<String>,
    pub(crate) extras: BTreeSet<String>,
}

/// Every visible kind's automaton from `json` (a `grammar.json`).
pub(crate) fn build(json: &str) -> Result<Built, String> {
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
    let mut kinds: FxHashMap<Sym, Automaton> = FxHashMap::default();
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
        let fields: BTreeSet<Sym> = moves.iter().flat_map(|moves| moves.keys()).map(|label| label.field).filter(|&field| field != 0).collect();
        kinds.insert(sym, Automaton { moves, accepting, implied, kept, fields: fields.into_iter().collect() });
    }
    let extras: BTreeSet<String> = grammar["extras"].as_array().into_iter().flatten().filter_map(|extra| extra["name"].as_str().map(str::to_owned)).collect();
    let mut kinds: Vec<(Sym, Automaton)> = kinds.into_iter().collect();
    kinds.sort_by_key(|(sym, _)| *sym);
    Ok(Built { texts: builder.texts, kinds, unsupported, extras })
}

// ---------------------------------------------------------------------------------------------
// The built automata as bytes: little-endian `u32`s, maps in their keys' order.

const FORMAT: u32 = 0x7461_0001;

struct Writer(Vec<u8>);

impl Writer {
    fn u(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    fn n(&mut self, v: usize) {
        self.u(u32::try_from(v).expect("an automaton's count fits a u32"));
    }
    fn text(&mut self, text: &str) {
        self.n(text.len());
        self.0.extend(text.as_bytes());
    }
    fn label(&mut self, label: &Label) {
        self.u(u32::from(label.token));
        self.u(label.name);
        self.u(label.field);
    }
    fn word(&mut self, word: &[Label]) {
        self.n(word.len());
        word.iter().for_each(|label| self.label(label));
    }
    fn texts<'a>(&mut self, texts: impl ExactSizeIterator<Item = &'a String>) {
        self.n(texts.len());
        texts.for_each(|text| self.text(text));
    }
}

pub(crate) fn encode(built: &Built) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.u(FORMAT);
    w.texts(built.texts.iter());
    w.texts(built.unsupported.iter());
    w.texts(built.extras.iter());
    w.n(built.kinds.len());
    for (sym, automaton) in &built.kinds {
        w.u(*sym);
        w.n(automaton.moves.len());
        for (moves, accepting) in automaton.moves.iter().zip(&automaton.accepting) {
            w.u(u32::from(*accepting));
            let moves: BTreeMap<&Label, &u32> = moves.iter().collect();
            w.n(moves.len());
            for (label, to) in moves {
                w.label(label);
                w.u(*to);
            }
        }
        let to: BTreeMap<&(u32, Label), &Box<[Label]>> = automaton.implied.to.iter().collect();
        w.n(to.len());
        for ((from, label), word) in to {
            w.u(*from);
            w.label(label);
            w.word(word);
        }
        for end in &automaton.implied.end {
            match end {
                Some(word) => {
                    w.u(1);
                    w.word(word);
                }
                None => w.u(0),
            }
        }
        let kept: BTreeSet<&Label> = automaton.kept.iter().collect();
        w.n(kept.len());
        kept.into_iter().for_each(|label| w.label(label));
        w.n(automaton.fields.len());
        automaton.fields.iter().for_each(|field| w.u(*field));
    }
    w.0
}

struct Reader<'b>(&'b [u8]);

impl Reader<'_> {
    fn u(&mut self) -> Option<u32> {
        let (head, rest) = self.0.split_first_chunk::<4>()?;
        self.0 = rest;
        Some(u32::from_le_bytes(*head))
    }
    fn n(&mut self) -> Option<usize> {
        self.u().map(|v| v as usize)
    }
    fn text(&mut self) -> Option<String> {
        let n = self.n()?;
        let (text, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        String::from_utf8(text.to_vec()).ok()
    }
    fn label(&mut self) -> Option<Label> {
        Some(Label { token: self.u()? != 0, name: self.u()?, field: self.u()? })
    }
    fn word(&mut self) -> Option<Box<[Label]>> {
        (0..self.n()?).map(|_| self.label()).collect()
    }
    fn texts<T: FromIterator<String>>(&mut self) -> Option<T> {
        (0..self.n()?).map(|_| self.text()).collect()
    }
}

/// The automata `encode` wrote; `None` for bytes it did not write.
pub(crate) fn decode(bytes: &[u8]) -> Option<Built> {
    let mut r = Reader(bytes);
    if r.u()? != FORMAT {
        return None;
    }
    let texts: Vec<String> = r.texts()?;
    let unsupported = r.texts()?;
    let extras = r.texts()?;
    let mut kinds = Vec::with_capacity(r.0.len().min(1 << 16) / 64);
    for _ in 0..r.n()? {
        let sym = r.u()?;
        let states = r.n()?;
        let (mut moves, mut accepting) = (Vec::with_capacity(states), Vec::with_capacity(states));
        for _ in 0..states {
            accepting.push(r.u()? != 0);
            let n = r.n()?;
            let mut out = FxHashMap::with_capacity_and_hasher(n, Default::default());
            for _ in 0..n {
                let label = r.label()?;
                out.insert(label, r.u()?);
            }
            moves.push(out);
        }
        let mut to = FxHashMap::default();
        for _ in 0..r.n()? {
            let from = r.u()?;
            let label = r.label()?;
            to.insert((from, label), r.word()?);
        }
        let end = (0..states).map(|_| if r.u()? != 0 { r.word().map(Some) } else { Some(None) }).collect::<Option<Vec<_>>>()?;
        let kept = (0..r.n()?).map(|_| r.label()).collect::<Option<_>>()?;
        let fields = (0..r.n()?).map(|_| r.u()).collect::<Option<_>>()?;
        kinds.push((sym, Automaton { moves, accepting, implied: Implied { to, end }, kept, fields }));
    }
    r.0.is_empty().then_some(Built { texts, kinds, unsupported, extras })
}

// ---------------------------------------------------------------------------------------------
// The census the token tests ask (`token_census`): what the automata say, by name, without the automata.

/// Per kind, the (token, field) pairs a tape of it may leave out; per kind, the kinds that have it as a named child;
/// every token text some automaton moves on; and the grammar's extras.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Census {
    pub(crate) implied: BTreeMap<String, BTreeSet<(String, String)>>,
    pub(crate) parents: BTreeMap<String, BTreeSet<String>>,
    pub(crate) tokens: BTreeSet<String>,
    pub(crate) extras: BTreeSet<String>,
}

/// The census of `kinds`' automata, their names in `texts`, and the grammar's `extras`.
pub(crate) fn census<'a>(texts: &[String], kinds: impl Iterator<Item = (&'a Sym, &'a Automaton)>, extras: &BTreeSet<String>) -> Census {
    let mut out = Census { extras: extras.clone(), ..Census::default() };
    for (kind, automaton) in kinds {
        let name = &texts[*kind as usize];
        let entry = out.implied.entry(name.clone()).or_default();
        for label in automaton.dropped() {
            entry.insert((texts[label.name as usize].clone(), texts[label.field as usize].clone()));
        }
        for label in automaton.moves.iter().flat_map(|moves| moves.keys()) {
            if label.token {
                out.tokens.insert(texts[label.name as usize].clone());
            } else {
                out.parents.entry(texts[label.name as usize].clone()).or_default().insert(name.clone());
            }
        }
    }
    out
}

pub(crate) fn encode_census(census: &Census) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.u(FORMAT);
    w.n(census.implied.len());
    for (kind, pairs) in &census.implied {
        w.text(kind);
        w.n(pairs.len());
        for (token, field) in pairs {
            w.text(token);
            w.text(field);
        }
    }
    w.n(census.parents.len());
    for (kind, parents) in &census.parents {
        w.text(kind);
        w.texts(parents.iter());
    }
    w.texts(census.tokens.iter());
    w.texts(census.extras.iter());
    w.0
}

/// The census `encode_census` wrote; `None` for bytes it did not write.
pub(crate) fn decode_census(bytes: &[u8]) -> Option<Census> {
    let mut r = Reader(bytes);
    if r.u()? != FORMAT {
        return None;
    }
    let implied = (0..r.n()?).map(|_| Some((r.text()?, (0..r.n()?).map(|_| Some((r.text()?, r.text()?))).collect::<Option<_>>()?))).collect::<Option<_>>()?;
    let parents = (0..r.n()?).map(|_| Some((r.text()?, r.texts()?))).collect::<Option<_>>()?;
    let tokens = r.texts()?;
    let extras = r.texts()?;
    r.0.is_empty().then_some(Census { implied, parents, tokens, extras })
}
