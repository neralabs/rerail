//! Named-only tapes (RER-131): which anonymous tokens a node's kind and named children imply.
//!
//! A tree-sitter node's visible children follow its rule in `grammar.json`. For each visible kind
//! this module builds the rule's automaton over the children (named children labelled by kind and
//! field, anonymous tokens by kind and field; hidden rules inlined) and determinizes it. Between
//! two named children, and after the last, the tokens are a *gap*. Each gap has a **default**: the
//! shortest token word (ties broken by a fixed order) that leads from the automaton's state to the
//! next named child, or to the end.
//!
//! [`fold`] keeps a gap's tokens only when they are not its default, or when the decoder could not
//! rebuild them exactly: [`unfold`] rebuilds a default gap's tokens from the source between the
//! named neighbours, skipping white space a word at a time, and `fold` replays that recovery and
//! keeps the gap when any kind, field or span would differ. So `unfold(fold(t)) == t` holds by
//! construction, whatever the grammar; a node the automaton cannot walk (an ERROR, a contextual
//! alias, a grammar the corpus disagrees with) keeps all of its tokens and is flagged.
//! Nothing here is chosen by hand: a new grammar version is a new automaton, never a
//! reclassification.

use super::Pending;
use crate::code_rule_dsl::language::GrammarDialect;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use rustc_hash::FxHashMap;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use tree_sitter::Language;

/// An interned name (a kind or a field; 0 is "no field").
type Sym = u32;

/// A child label: a named child or a token, with its field.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct Label {
    token: bool,
    name: Sym,
    field: Sym,
}

/// One kind's determinized automaton and its gap defaults.
struct Automaton {
    /// Per state: its moves, by label.
    moves: Vec<FxHashMap<Label, u32>>,
    accepting: Vec<bool>,
    /// Per state: the default token word to each named label, with the state it reaches before
    /// that label; and the default word to the end.
    to_named: Vec<FxHashMap<Label, (Vec<Label>, u32)>>,
    to_end: Vec<Option<Vec<Label>>>,
}

/// Every visible kind's automaton for one grammar.
pub(in crate::code_rule_dsl) struct TokenGrammar {
    names: FxHashMap<String, Sym>,
    texts: Vec<String>,
    kinds: FxHashMap<Sym, Automaton>,
    /// Per tree-sitter kind id: its name's symbol (`None` if the grammar never names it).
    kind_sym: Vec<Option<Sym>>,
    field_sym: Vec<Sym>,
    /// Per token symbol: the tree-sitter id a token of that name has.
    token_id: FxHashMap<Sym, u16>,
    /// Per field symbol: its tree-sitter field id.
    field_id: FxHashMap<Sym, u16>,
    pub(in crate::code_rule_dsl) unsupported: BTreeSet<String>,
}

/// What [`fold`] kept and left out.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct FoldStats {
    pub(in crate::code_rule_dsl) nodes: usize,
    pub(in crate::code_rule_dsl) named: usize,
    pub(in crate::code_rule_dsl) tokens: usize,
    pub(in crate::code_rule_dsl) implied: usize,
    pub(in crate::code_rule_dsl) kept_not_default: usize,
    pub(in crate::code_rule_dsl) kept_unrecoverable: usize,
    pub(in crate::code_rule_dsl) kept_flagged: usize,
    pub(in crate::code_rule_dsl) flagged_nodes: usize,
    /// Flagged nodes by kind name (why the automaton could not walk them).
    pub(in crate::code_rule_dsl) flagged_kinds: BTreeMap<String, usize>,
    /// Gaps kept because they are not the default, by parent kind and word.
    pub(in crate::code_rule_dsl) not_default: BTreeMap<String, usize>,
}

impl FoldStats {
    pub(in crate::code_rule_dsl) fn add(&mut self, other: &Self) {
        self.nodes += other.nodes;
        self.named += other.named;
        self.tokens += other.tokens;
        self.implied += other.implied;
        self.kept_not_default += other.kept_not_default;
        self.kept_unrecoverable += other.kept_unrecoverable;
        self.kept_flagged += other.kept_flagged;
        self.flagged_nodes += other.flagged_nodes;
        for (kind, count) in &other.flagged_kinds {
            *self.flagged_kinds.entry(kind.clone()).or_default() += count;
        }
        for (gap, count) in &other.not_default {
            *self.not_default.entry(gap.clone()).or_default() += count;
        }
    }
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

    /// A rule that is one token: a lexical variable (named if visible, else no node).
    fn is_token_rule(&self, name: &str) -> bool {
        match self.rules.get(name) {
            None => true,
            Some(rule) => matches!(
                unwrap_prec(rule)["type"].as_str(),
                Some("STRING" | "PATTERN" | "TOKEN" | "IMMEDIATE_TOKEN")
            ),
        }
    }

    fn build(&mut self, rule: &Value, nfa: &mut Nfa, from: u32, to: u32, field: Sym, stack: &mut Vec<String>) {
        let kind = rule["type"].as_str().unwrap_or("");
        match kind {
            "BLANK" | "PATTERN" => nfa.edge(from, None, to),
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

/// The defaults from each state: a breadth-first search over token moves, in label order, so the
/// first word found to a state is a shortest one, the same one every time.
fn defaults(moves: &[FxHashMap<Label, u32>], accepting: &[bool]) -> (Vec<FxHashMap<Label, (Vec<Label>, u32)>>, Vec<Option<Vec<Label>>>) {
    let mut to_named = Vec::with_capacity(moves.len());
    let mut to_end = Vec::with_capacity(moves.len());
    for start in 0..moves.len() as u32 {
        let mut word: FxHashMap<u32, Vec<Label>> = FxHashMap::default();
        let mut queue = VecDeque::from([start]);
        word.insert(start, Vec::new());
        let mut order = Vec::new();
        while let Some(state) = queue.pop_front() {
            order.push(state);
            let mut tokens: Vec<(Label, u32)> = moves[state as usize].iter().filter(|(l, _)| l.token).map(|(l, &t)| (*l, t)).collect();
            tokens.sort();
            for (label, to) in tokens {
                if !word.contains_key(&to) {
                    let mut w = word[&state].clone();
                    w.push(label);
                    word.insert(to, w);
                    queue.push_back(to);
                }
            }
        }
        let mut named: FxHashMap<Label, (Vec<Label>, u32)> = FxHashMap::default();
        let mut end = None;
        for &state in &order {
            if end.is_none() && accepting[state as usize] {
                end = Some(word[&state].clone());
            }
            for (label, _) in moves[state as usize].iter().filter(|(l, _)| !l.token) {
                named.entry(*label).or_insert_with(|| (word[&state].clone(), state));
            }
        }
        to_named.push(named);
        to_end.push(end);
    }
    (to_named, to_end)
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
            let (to_named, to_end) = defaults(&moves, &accepting);
            let sym = builder.sym(name);
            kinds.insert(sym, Automaton { moves, accepting, to_named, to_end });
        }
        let Builder { names, texts, .. } = builder;
        let kind_sym = (0..language.node_kind_count() as u16)
            .map(|id| language.node_kind_for_id(id).and_then(|name| names.get(name).copied()))
            .collect();
        let field_sym = (0..=language.field_count() as u16)
            .map(|id| language.field_name_for_id(id).and_then(|name| names.get(name).copied()).unwrap_or(0))
            .collect();
        let token_id = names
            .iter()
            .filter_map(|(name, &sym)| {
                let id = language.id_for_node_kind(name, false);
                (id != 0).then_some((sym, id))
            })
            .collect();
        let field_id = (1..=language.field_count() as u16)
            .filter_map(|id| language.field_name_for_id(id).and_then(|name| names.get(name).map(|&sym| (sym, id))))
            .collect();
        Ok(Self { names, texts, kinds, kind_sym, field_sym, token_id, field_id, unsupported })
    }

    pub(in crate::code_rule_dsl) fn kind_count(&self) -> usize {
        self.kinds.len()
    }

    fn text(&self, sym: Sym) -> &str {
        &self.texts[sym as usize]
    }
}

/// The `grammar.json` a dialect's parser was generated from: vendored, or in the crate's source.
pub(in crate::code_rule_dsl) fn grammar_json(lang: GrammarDialect) -> Option<String> {
    let vendored = |dir: &str| Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("grammars").join(dir).join("src/grammar.json"));
    let registry = |dir: &str| -> Option<PathBuf> {
        let home = std::env::var("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cargo"));
        let src = home.join("registry/src");
        std::fs::read_dir(&src).ok()?.filter_map(|entry| Some(entry.ok()?.path().join(dir).join("src/grammar.json"))).find(|path| path.exists())
    };
    let path = match lang {
        GrammarDialect::Python => vendored("tree-sitter-python"),
        GrammarDialect::JavaScript => vendored("tree-sitter-javascript"),
        GrammarDialect::TypeScript => vendored("tree-sitter-typescript/typescript"),
        GrammarDialect::Tsx => vendored("tree-sitter-typescript/tsx"),
        GrammarDialect::Sql => vendored("tree-sitter-sql"),
        GrammarDialect::Vue => vendored("tree-sitter-vue"),
        GrammarDialect::Rust => registry("tree-sitter-rust-0.24.2"),
        GrammarDialect::Php => registry("tree-sitter-php-0.24.2/php"),
        _ => None,
    }?;
    std::fs::read_to_string(&path).ok()
}


/// Whether the encoder writes named-only tapes: `RERAIL_NAMED_ONLY_TAPES=1` (the RER-131
/// experiment; the format is not switched yet).
pub(in crate::code_rule_dsl) fn named_only() -> bool {
    static ON: LazyLock<bool> = LazyLock::new(|| std::env::var_os("RERAIL_NAMED_ONLY_TAPES").is_some_and(|v| v == "1"));
    *ON
}

impl TokenGrammar {
    /// A dialect's automata, built once per process from its `grammar.json`; `None` where there
    /// is none (a dialect with its own parser, or no grammar on disk).
    pub(in crate::code_rule_dsl) fn of(lang: GrammarDialect, language: &Language) -> Option<Arc<Self>> {
        static BUILT: LazyLock<Mutex<FxHashMap<GrammarDialect, Option<Arc<TokenGrammar>>>>> = LazyLock::new(Default::default);
        let mut built = BUILT.lock().ok()?;
        built
            .entry(lang)
            .or_insert_with(|| grammar_json(lang).and_then(|json| Self::from_json(&json, language).ok()).map(Arc::new))
            .clone()
    }
}

// ---------------------------------------------------------------------------------------------
// Folding and unfolding a tree.

/// What fold and unfold read of a node.
struct View<'a> {
    grammar: &'a TokenGrammar,
    language: &'a Language,
}

impl View<'_> {
    fn name(&self, node: &Pending) -> Option<Sym> {
        if node.error {
            return None;
        }
        match &node.kind_alias {
            Some((alias, _)) => self.grammar.names.get(alias).copied(),
            None => self.grammar.kind_sym.get(usize::from(node.kf & 1023)).copied().flatten(),
        }
    }
    fn named(&self, node: &Pending) -> bool {
        match &node.kind_alias {
            Some((_, named)) => *named,
            None => node.error || self.language.node_kind_is_named(node.kf & 1023),
        }
    }
    fn field(&self, node: &Pending) -> Sym {
        self.grammar.field_sym.get(usize::from(node.kf >> 10)).copied().unwrap_or(0)
    }
}

/// The source's white space as tree-sitter's extras read it (`/\s/`, ASCII): a mask with the
/// high bit of each white-space byte of `word`.
fn white_space(word: u64) -> u64 {
    const LOW: u64 = 0x0101_0101_0101_0101;
    const HIGH: u64 = 0x8080_8080_8080_8080;
    let eq = |byte: u8| {
        let x = word ^ (LOW * u64::from(byte));
        !(((x & !HIGH).wrapping_add(!HIGH)) | x) & HIGH
    };
    eq(b' ') | eq(b'\t') | eq(b'\n') | eq(b'\r') | eq(0x0b) | eq(0x0c)
}

/// The first byte at or after `at` (before `end`) that is not white space, 8 bytes at a time.
fn skip_white(source: &[u8], mut at: usize, end: usize) -> usize {
    while at < end {
        let take = (end - at).min(8);
        let mut bytes = [b'x'; 8];
        bytes[..take].copy_from_slice(&source[at..at + take]);
        let other = !white_space(u64::from_le_bytes(bytes)) & 0x8080_8080_8080_8080;
        if other != 0 {
            return (at + (other.trailing_zeros() / 8) as usize).min(end);
        }
        at += take;
    }
    end
}

/// The tokens of a default gap, read from the source from `at`: each the next text that is not
/// white space, spelling the token's kind. `None` where the source does not spell them.
fn recover(grammar: &TokenGrammar, word: &[Label], source: &[u8], mut at: usize, end: usize) -> Option<Vec<(Label, u32, u32)>> {
    let mut out = Vec::with_capacity(word.len());
    for &label in word {
        let text = grammar.text(label.name).as_bytes();
        at = skip_white(source, at, end);
        if text.is_empty() || at + text.len() > end || &source[at..at + text.len()] != text {
            return None;
        }
        out.push((label, at as u32, text.len() as u32));
        at += text.len();
    }
    Some(out)
}

fn children_of(nodes: &[Pending]) -> Vec<Vec<u32>> {
    let mut children = vec![Vec::new(); nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            children[parent as usize].push(index as u32);
        }
    }
    children
}

/// The named-only tree of `nodes` (preorder, parents set, sizes unset): the nodes kept, each with
/// `tokens_kept` set where its tokens are all kept, and the supertype memberships renumbered.
pub(in crate::code_rule_dsl) fn fold(
    nodes: &[Pending],
    supertypes: &[(u32, u16)],
    grammar: &TokenGrammar,
    language: &Language,
    source: &str,
) -> (Vec<Pending>, Vec<(u32, u16)>, FoldStats) {
    let view = View { grammar, language };
    let source = source.as_bytes();
    let children = children_of(nodes);
    let has_class: FxHashMap<u32, ()> = supertypes.iter().map(|&(at, _)| (at, ())).collect();
    let mut keep = vec![true; nodes.len()];
    let mut flagged = vec![false; nodes.len()];
    let mut stats = FoldStats { nodes: nodes.len(), ..FoldStats::default() };
    for (index, node) in nodes.iter().enumerate() {
        if view.named(node) {
            stats.named += 1;
        } else {
            stats.tokens += 1;
        }
        if !view.named(node) {
            continue;
        }
        let tokens_of = |c: &[u32]| c.iter().filter(|&&c| !view.named(&nodes[c as usize])).count();
        let automaton = view.name(node).and_then(|sym| grammar.kinds.get(&sym));
        let Some(automaton) = automaton else {
            let kept = tokens_of(&children[index]);
            if kept > 0 {
                flagged[index] = true;
                stats.flagged_nodes += 1;
                stats.kept_flagged += kept;
                let name = node.kind_alias.as_ref().map(|a| a.0.clone()).or_else(|| language.node_kind_for_id(node.kf & 1023).map(str::to_owned)).unwrap_or_default();
                *stats.flagged_kinds.entry(name).or_default() += 1;
            }
            continue;
        };
        // Walk the children; a gap's fate is decided at its named child, or at the end.
        let mut drops: Vec<u32> = Vec::new();
        let mut not_default: Vec<String> = Vec::new();
        let mut unrecoverable = 0usize;
        let mut ok = true;
        let mut state = 0u32;
        let mut gap: Vec<u32> = Vec::new();
        let mut gap_extra = false;
        let mut at = node.start as usize;
        let mut gap_start = at;
        let end = (node.start + node.length) as usize;
        let mut close = |gap: &mut Vec<u32>, gap_extra: bool, gap_start: usize, state: &mut u32, next: Option<Label>, limit: usize, drops: &mut Vec<u32>, not_default: &mut Vec<String>, unrecoverable: &mut usize| -> bool {
            let mut word = Vec::with_capacity(gap.len());
            for &t in gap.iter() {
                let token = &nodes[t as usize];
                let Some(name) = view.name(token) else { return false };
                word.push(Label { token: true, name, field: view.field(token) });
            }
            let mut s = *state;
            for label in &word {
                match automaton.moves[s as usize].get(label) {
                    Some(&to) => s = to,
                    None => return false,
                }
            }
            let default = match next {
                Some(label) => {
                    if !automaton.moves[s as usize].contains_key(&label) {
                        return false;
                    }
                    automaton.to_named[*state as usize].get(&label).map(|(w, _)| w.clone())
                }
                None => {
                    if !automaton.accepting[s as usize] {
                        return false;
                    }
                    automaton.to_end[*state as usize].clone()
                }
            };
            *state = match next {
                Some(label) => automaton.moves[s as usize][&label],
                None => s,
            };
            if gap.is_empty() {
                return true;
            }
            if default.as_ref() != Some(&word) {
                not_default.push(word.iter().map(|l| grammar.text(l.name)).collect::<Vec<_>>().join(" "));
                return true;
            }
            // The default: can the decoder rebuild exactly these tokens?
            let exact = !gap_extra
                && gap.iter().all(|&t| {
                    let token = &nodes[t as usize];
                    !token.extra && !token.missing && token.kind_alias.is_none() && !has_class.contains_key(&t) && children[t as usize].is_empty()
                })
                && recover(grammar, &word, source, gap_start, limit).is_some_and(|spans| {
                    spans.iter().zip(gap.iter()).all(|(&(label, start, length), &t)| {
                        let token = &nodes[t as usize];
                        token.start == start && token.length == length && grammar.token_id.get(&label.name) == Some(&(token.kf & 1023))
                    })
                });
            if exact {
                drops.extend(gap.iter().copied());
            } else {
                *unrecoverable += gap.len();
            }
            true
        };
        for &c in &children[index] {
            let child = &nodes[c as usize];
            if child.extra {
                gap_extra = true;
                at = (child.start + child.length) as usize;
                continue;
            }
            if !view.named(child) {
                if gap.is_empty() {
                    gap_start = at;
                }
                gap.push(c);
                at = (child.start + child.length) as usize;
                continue;
            }
            let Some(name) = view.name(child) else {
                ok = false;
                break;
            };
            if gap.is_empty() {
                gap_start = at;
            }
            let label = Label { token: false, name, field: view.field(child) };
            if !close(&mut gap, gap_extra, gap_start, &mut state, Some(label), child.start as usize, &mut drops, &mut not_default, &mut unrecoverable) {
                ok = false;
                break;
            }
            gap.clear();
            gap_extra = false;
            at = (child.start + child.length) as usize;
            gap_start = at;
        }
        if ok {
            if gap.is_empty() {
                gap_start = at;
            }
            ok = close(&mut gap, gap_extra, gap_start, &mut state, None, end, &mut drops, &mut not_default, &mut unrecoverable);
        }
        if !ok {
            // The decoder runs this kind's automaton, so a node it cannot walk is always flagged.
            let kept = tokens_of(&children[index]);
            flagged[index] = true;
            stats.flagged_nodes += 1;
            stats.kept_flagged += kept;
            let name = view.name(node).map(|s| grammar.text(s).to_owned()).unwrap_or_default();
            *stats.flagged_kinds.entry(name).or_default() += 1;
            continue;
        }
        stats.implied += drops.len();
        stats.kept_unrecoverable += unrecoverable;
        let parent = view.name(node).map(|s| grammar.text(s).to_owned()).unwrap_or_default();
        for word in not_default {
            stats.kept_not_default += word.split(' ').count();
            *stats.not_default.entry(format!("{parent} [{word}]")).or_default() += 1;
        }
        for d in drops {
            keep[d as usize] = false;
        }
    }
    // Renumber the kept nodes. A dropped token is a leaf, so a kept node's parent is kept.
    let mut new_index = vec![u32::MAX; nodes.len()];
    let mut out = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        new_index[index] = out.len() as u32;
        let mut kept = node.clone();
        kept.parent = node.parent.map(|p| new_index[p as usize]);
        kept.size = 1;
        kept.tokens_kept = flagged[index];
        out.push(kept);
    }
    let classes = supertypes.iter().filter(|&&(at, _)| keep[at as usize]).map(|&(at, class)| (new_index[at as usize], class)).collect();
    (out, classes, stats)
}

/// The full tree of a named-only tree `nodes` (preorder, parents set): every default gap's tokens
/// rebuilt from `source`. Sizes are unset; supertype memberships renumbered.
pub(in crate::code_rule_dsl) fn unfold(
    nodes: &[Pending],
    supertypes: &[(u32, u16)],
    grammar: &TokenGrammar,
    language: &Language,
    source: &str,
) -> Result<(Vec<Pending>, Vec<(u32, u16)>), String> {
    enum Work {
        Node(u32, Option<u32>),
        Token(Label, u32, u32, u32),
    }
    let view = View { grammar, language };
    let bytes = source.as_bytes();
    let children = children_of(nodes);
    let mut out: Vec<Pending> = Vec::with_capacity(nodes.len() * 2);
    let mut new_index = vec![u32::MAX; nodes.len()];
    let mut stack = vec![Work::Node(0, None)];
    let mut sequence: Vec<Work> = Vec::new();
    while let Some(work) = stack.pop() {
        let (index, parent) = match work {
            Work::Token(label, start, length, parent) => {
                let kind = *grammar.token_id.get(&label.name).ok_or("a default token with no kind id")?;
                let field = grammar.field_id.get(&label.field).copied().unwrap_or(0);
                out.push(Pending {
                    kf: kind | (field << 10),
                    parent: Some(parent),
                    start,
                    length,
                    size: 1,
                    extra: false,
                    error: false,
                    missing: false,
                    kind_alias: None,
                    tokens_kept: false,
                });
                continue;
            }
            Work::Node(index, parent) => (index, parent),
        };
        let node = &nodes[index as usize];
        let here = out.len() as u32;
        new_index[index as usize] = here;
        let mut copy = node.clone();
        copy.parent = parent;
        copy.size = 1;
        copy.tokens_kept = false;
        out.push(copy);
        let kids = &children[index as usize];
        let automaton = if node.tokens_kept || !view.named(node) {
            None
        } else {
            view.name(node).and_then(|sym| grammar.kinds.get(&sym))
        };
        let Some(automaton) = automaton else {
            stack.extend(kids.iter().rev().map(|&c| Work::Node(c, Some(here))));
            continue;
        };
        sequence.clear();
        let mut state = 0u32;
        let mut stored: Vec<Label> = Vec::new();
        let mut at = node.start as usize;
        let end = (node.start + node.length) as usize;
        let advance = |state: &mut u32, word: &[Label]| -> Result<(), String> {
            for label in word {
                *state = *automaton.moves[*state as usize].get(label).ok_or("a kept token the automaton refuses")?;
            }
            Ok(())
        };
        let kind = view.name(node).map(|s| grammar.text(s).to_owned()).unwrap_or_default();
        let at_node = |error: &str| format!("{error} in {kind} at {}", node.start);
        for &c in kids {
            let child = &nodes[c as usize];
            if child.extra {
                sequence.push(Work::Node(c, Some(here)));
                at = (child.start + child.length) as usize;
                continue;
            }
            if !view.named(child) {
                let name = view.name(child).ok_or("a kept token outside the grammar")?;
                stored.push(Label { token: true, name, field: view.field(child) });
                sequence.push(Work::Node(c, Some(here)));
                at = (child.start + child.length) as usize;
                continue;
            }
            let name = view.name(child).ok_or("a named child outside the grammar")?;
            let label = Label { token: false, name, field: view.field(child) };
            if stored.is_empty() {
                let (word, _) = automaton.to_named[state as usize].get(&label).ok_or_else(|| at_node(&format!("no default before {}", grammar.text(label.name))))?;
                let spans = recover(grammar, word, bytes, at, child.start as usize).ok_or_else(|| at_node("a default gap the source does not spell"))?;
                sequence.extend(spans.into_iter().map(|(l, s, n)| Work::Token(l, s, n, here)));
                advance(&mut state, word)?;
            } else {
                advance(&mut state, &stored)?;
                stored.clear();
            }
            state = *automaton.moves[state as usize].get(&label).ok_or("a named child the automaton refuses")?;
            sequence.push(Work::Node(c, Some(here)));
            at = (child.start + child.length) as usize;
        }
        if stored.is_empty() {
            let word = automaton.to_end[state as usize].as_ref().ok_or_else(|| at_node("no default at the end"))?;
            let spans = recover(grammar, word, bytes, at, end).ok_or("a default end gap the source does not spell")?;
            sequence.extend(spans.into_iter().map(|(l, s, n)| Work::Token(l, s, n, here)));
        }
        stack.extend(sequence.drain(..).rev());
    }
    let classes = supertypes.iter().map(|&(at, class)| (new_index[at as usize], class)).collect();
    Ok((out, classes))
}
