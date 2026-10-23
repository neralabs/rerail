//! Structural `pattern` rules: ast-grep's pattern matching for Python as the native engine runs
//! it, over a file's facts instead of its tape.
//!
//! Native (the source of truth):
//! - `native/src/code_rule_dsl/compile/ast_rule.rs:406-437`: a `pattern` term is
//!   `rule_pattern(source, Python)`; a build error, an ERROR root (`has_error`) or a literal
//!   `$name` leaf (`literal_dollar_name`, `:307-349`, Python's `DOLLAR_NAMES` is true:
//!   `language/adapter/syntax.rs:68`) is a grammar gap.
//! - `native/src/code_rule_dsl/compile/mod.rs:1158-1161`: a gap leaves the rule whole in no
//!   dialect; a Python-only rule is then rejected. Gaps therefore compile to `Err` here, with
//!   native's wording.
//! - `native/src/code_rule_dsl/language.rs:60-81, 295-305`: the pattern is `Pattern::try_new`
//!   with Python's expando `µ` and pre-processing, parsed by the same patched grammar the files
//!   are (`StrDoc::try_new`); here the snippet is parsed through `facts::build`, i.e. rerail's
//!   tape of that same tree-sitter parse.
//! - `native/src/code_rule_dsl/matcher/mod.rs:281`: matching is ast-grep's
//!   `Pattern::match_node_with_env`, strictness `Smart` (native never changes it).
//! - `native/src/code_rule_dsl/matcher/captures.rs:136-138, 200-209, 337-339`: a pattern binds
//!   and mentions its `defined_vars` (single and multi captures), and reads nothing.
//!
//! Ported from ast-grep-language 0.50.0: `lib.rs:94-113` `pre_process_pattern` (Python's
//! expando `µ`, `lib.rs:232`). From ast-grep-core 0.50.0:
//! - `matcher/pattern.rs`: `PatternBuilder::build` (27-48), `single` (49-59),
//!   `is_single_node` (273-284), `Pattern::single_matcher` (376-384),
//!   `convert_node_to_pattern` (223-245), `extract_var_from_node` (247-250), `has_error`
//!   (292-302), `defined_vars`/`collect_vars` (310-343), `match_node_with_env` (387-406),
//!   `potential_kinds` (408-428, as `Pattern::root_kind`), `PatternNode::is_trivial` (181-186).
//! - `meta_var.rs`: `extract_meta_var` (235-271), `is_valid_first_char`/`is_valid_meta_var_char`
//!   (273-281), `MetaVarEnv::insert`/`insert_multi`/`match_variable`/`match_multi_var` (32-48,
//!   89-118).
//! - `match_tree/mod.rs`: `match_leaf_meta_var` (61-93), the `Cow<MetaVarEnv>` aggregator
//!   (95-116), `match_node_non_recursive` (118-127), `does_node_match_exactly` (129-150).
//! - `match_tree/match_node.rs`: `match_root_node_impl`, `match_node_impl`,
//!   `match_nodes_impl_recursive`, `may_match_ellipsis_impl` (with its cloned probe),
//!   `match_single_node_while_skip_trivial`, `try_get_ellipsis_mode`, `match_ellipsis` (8-267).
//! - `match_tree/strictness.rs`: the `Smart` arms of `match_terminal`,
//!   `should_skip_cand_for_metavar`, `should_skip_trailing` and `should_skip_goal`.
//! - `matcher/kind.rs`: `kind_utils::is_error_kind`/`are_kinds_matching` (68-74).
//!
//! Node identity: native compares tree-sitter kind ids (the tape's `rec.kind`, the pattern's
//! `StrDoc` kind id); facts key kinds by name, so a kind here is the pair (kind symbol, named
//! flag), which is the grammar's public symbol. An ERROR goal matches any kind
//! (`are_kinds_matching`). Node ids are tape indices on both sides (`does_node_match_exactly`).
//!
//! Refusals (I9):
//! - compile: a contextual pattern (`{context, selector}`): `Err("…/pattern: contextual pattern
//!   …")` (its selector resolves through the grammar's kind ids, which the engine does not export).
//! - eval: a candidate that is a parsed-literal node (`Bound::Parsed`): `refuse("pattern: …")`
//!   (parsed trees carry no extra flags nor tree-sitter kind ids).
//! - eval: rebinding a capture already bound to a parsed-literal node: `refuse("pattern: …")`
//!   (native compares nodes of one document only).
use super::{Bound, Compiler, Env, Eval, Name, Rule};
use crate::facts::{self, Facts};
use serde_json::Value as Json;
use std::sync::Arc;

/// Python's expando character (ast-grep-language `impl_lang_expando!(Python, …, 'µ')`).
const EXPANDO: char = 'µ';
const ELLIPSIS: &str = "µµµ";

/// ast-grep's `MetaVariable`, names interned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetaVar {
    /// `$A` (named only) / `$$A` (any node).
    Capture(Name, bool),
    /// `$_` / `$$_`.
    Dropped(bool),
    /// `$$$` / `$$$_X`.
    Multiple,
    /// `$$$A`.
    MultiCapture(Name),
}

/// ast-grep's `PatternNode`; a kind is (kind symbol, named), `error` its ERROR kind.
#[derive(Clone, Debug)]
pub enum PatternNode {
    MetaVar(MetaVar),
    /// A node without children.
    Terminal { text: String, named: bool, kind: u32, error: bool },
    Internal { kind: u32, named: bool, error: bool, children: Vec<PatternNode> },
}

impl PatternNode {
    /// `is_trivial`: an anonymous terminal, skipped in the goal after an ellipsis.
    fn is_trivial(&self) -> bool {
        matches!(self, PatternNode::Terminal { named: false, .. })
    }
}

/// A compiled structural pattern (plain source; strictness `Smart`).
#[derive(Clone, Debug)]
pub struct Pattern {
    pub node: PatternNode,
}

impl Pattern {
    /// No nested rules.
    pub fn sub_rules(&self) -> Vec<&Rule> {
        Vec::new()
    }
    /// `defined_vars`: every single and multi capture, sorted and distinct.
    pub fn binds(&self) -> Vec<Name> {
        fn collect(node: &PatternNode, out: &mut Vec<Name>) {
            match node {
                PatternNode::MetaVar(MetaVar::Capture(n, _) | MetaVar::MultiCapture(n)) => out.push(*n),
                PatternNode::MetaVar(_) | PatternNode::Terminal { .. } => {}
                PatternNode::Internal { children, .. } => children.iter().for_each(|c| collect(c, out)),
            }
        }
        let mut out = Vec::new();
        collect(&self.node, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
    /// A pattern reads no capture (captures.rs `binds_and_reads`).
    pub fn reads(&self) -> Vec<Name> {
        Vec::new()
    }
    /// `potential_kinds`: the root's kind symbol, `None` for a metavariable or ERROR root (any
    /// kind). A sound prefilter: the root's named flag is checked when matching.
    pub fn root_kind(&self) -> Option<u32> {
        match &self.node {
            PatternNode::Terminal { kind, error: false, .. } | PatternNode::Internal { kind, error: false, .. } => Some(*kind),
            _ => None,
        }
    }
}

/// The capture names a pattern text defines (for JSON-level capture analysis before compiling);
/// empty where the pattern does not build (its rule is rejected at compile).
pub(super) fn pattern_var_names(p: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let built = parse(p, &mut |n: &str| {
        names.push(n.to_owned());
        0
    });
    if built.is_err() {
        return Vec::new();
    }
    names.sort_unstable();
    names.dedup();
    names
}

// ---------------------------------------------------------------------------------------------
// Compile.

/// ast-grep-language `pre_process_pattern`: `$` before `A-Z`/`_`, and `$$$` before anything or
/// at the end, become the expando.
fn pre_process(query: &str) -> String {
    let mut ret = String::with_capacity(query.len());
    let mut dollars = 0usize;
    for c in query.chars() {
        if c == '$' {
            dollars += 1;
            continue;
        }
        let replace = matches!(c, 'A'..='Z' | '_') || dollars == 3;
        ret.extend(std::iter::repeat_n(if replace { EXPANDO } else { '$' }, dollars));
        dollars = 0;
        ret.push(c);
    }
    ret.extend(std::iter::repeat_n(if dollars == 3 { EXPANDO } else { '$' }, dollars));
    ret
}

fn is_valid_first_char(c: char) -> bool {
    matches!(c, 'A'..='Z' | '_')
}
fn is_valid_meta_var_char(c: char) -> bool {
    is_valid_first_char(c) || c.is_ascii_digit()
}

enum RawVar<'s> {
    Capture(&'s str, bool),
    Dropped(bool),
    Multiple,
    MultiCapture(&'s str),
}

/// `extract_meta_var(src, 'µ')`.
fn extract_meta_var(src: &str) -> Option<RawVar<'_>> {
    if src == ELLIPSIS {
        return Some(RawVar::Multiple);
    }
    if let Some(trimmed) = src.strip_prefix(ELLIPSIS) {
        if !trimmed.chars().all(is_valid_meta_var_char) {
            return None;
        }
        return Some(if trimmed.starts_with('_') { RawVar::Multiple } else { RawVar::MultiCapture(trimmed) });
    }
    let trimmed = src.strip_prefix(EXPANDO)?;
    let (trimmed, named) = match trimmed.strip_prefix(EXPANDO) {
        Some(t) => (t, false),
        None => (trimmed, true),
    };
    if !trimmed.starts_with(is_valid_first_char) || !trimmed.chars().all(is_valid_meta_var_char) {
        return None;
    }
    Some(if trimmed.starts_with('_') { RawVar::Dropped(named) } else { RawVar::Capture(trimmed, named) })
}

/// `is_single_node`: one child, or two whose second is missing or has an empty kind name.
fn is_single_node(f: &Facts, id: u32) -> bool {
    let kids = f.children(id);
    match kids.len() {
        1 => true,
        2 => {
            let second = kids[1] as usize;
            f.flags[second] & 4 != 0 || facts::name(f.kind[second]).is_empty()
        }
        _ => false,
    }
}

/// `convert_node_to_pattern`: a node whose whole text is a metavariable is one; else a leaf is
/// a terminal and an inner node keeps its non-missing children.
fn convert(f: &Facts, id: u32, names: &mut dyn FnMut(&str) -> Name) -> PatternNode {
    let text = f.text(id);
    if let Some(var) = extract_meta_var(text) {
        return PatternNode::MetaVar(match var {
            RawVar::Capture(n, named) => MetaVar::Capture(names(n), named),
            RawVar::Dropped(named) => MetaVar::Dropped(named),
            RawVar::Multiple => MetaVar::Multiple,
            RawVar::MultiCapture(n) => MetaVar::MultiCapture(names(n)),
        });
    }
    let i = id as usize;
    let (kind, named, error) = (f.kind[i], f.named(id), f.flags[i] & 8 != 0);
    let kids = f.children(id);
    if kids.is_empty() {
        PatternNode::Terminal { text: text.to_owned(), named, kind, error }
    } else {
        let children = kids
            .iter()
            .filter(|&&c| f.flags[c as usize] & 4 == 0)
            .map(|&c| convert(f, c, names))
            .collect();
        PatternNode::Internal { kind, named, error, children }
    }
}

/// `Pattern::try_new(src, Python)`: pre-process, parse, descend the single-child chain, convert;
/// ast-grep's `PatternError` messages.
fn parse(src: &str, names: &mut dyn FnMut(&str) -> Name) -> Result<PatternNode, String> {
    let processed = pre_process(src);
    let f = match facts::build("pattern.py", Arc::new(processed.clone())) {
        Ok(facts::Built::Facts(f)) => f,
        Ok(facts::Built::Unparsed) => return Err(format!("Fails to parse the pattern query: `{processed}`")),
        Err(e) => return Err(format!("Fails to parse the pattern query: `{e}`")),
    };
    // The tape lists nodes in preorder: the root is node 0.
    let root = 0u32;
    if f.len() == 0 || f.children(root).is_empty() {
        return Err(format!("No AST root is detected. Please check the pattern source `{processed}`."));
    }
    if !is_single_node(&f, root) {
        return Err(format!("Multiple AST nodes are detected. Please check the pattern source `{processed}`."));
    }
    let mut node = root;
    while is_single_node(&f, node) {
        node = f.children(node)[0];
    }
    let pattern = convert(&f, node, names);
    if matches!(pattern, PatternNode::MetaVar(MetaVar::MultiCapture(_) | MetaVar::Multiple)) {
        return Err(format!(
            "Standalone multi meta variable `{processed}` is invalid. Use `$VAR` or wrap `$$$VAR` in a larger pattern."
        ));
    }
    Ok(pattern)
}

/// native `literal_dollar_name`: a terminal the pre-processing turned into `µ…` that is no
/// metavariable, when the source spells it with `$`.
fn literal_dollar_name(node: &PatternNode, source: &str) -> Option<String> {
    fn written(source: &str, spelling: &str) -> bool {
        source.match_indices(spelling).any(|(at, _)| {
            !source[..at].ends_with('$')
                && !source[at + spelling.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
        })
    }
    match node {
        PatternNode::Terminal { text, .. } => {
            let name = text.trim_start_matches(EXPANDO);
            let sigils = (text.len() - name.len()) / EXPANDO.len_utf8();
            if sigils == 0 || name.is_empty() {
                return None;
            }
            let spelling = format!("{}{name}", "$".repeat(sigils));
            written(source, &spelling).then_some(spelling)
        }
        PatternNode::Internal { children, .. } => children.iter().find_map(|c| literal_dollar_name(c, source)),
        PatternNode::MetaVar(_) => None,
    }
}

impl Compiler<'_> {
    /// A rule's `pattern` value: a string (`compile_pattern_text`); a contextual object is
    /// refused.
    pub(super) fn compile_pattern(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        match v {
            Json::String(p) => self.compile_pattern_text(p, at),
            Json::Object(_) => Err(format!(
                "{at}/pattern: contextual pattern ({{context, selector}}) unsupported: its selector resolves through grammar kind ids the engine does not export"
            )),
            _ => Err(format!("{at}/pattern must be a string or a {{ context, selector }} object")),
        }
    }

    /// A plain pattern as written (untrimmed: native parses it as is). Native's grammar gaps
    /// (build error, ERROR root, literal `$name`) reject the Python rule, so they are `Err`.
    pub(super) fn compile_pattern_text(&mut self, p: &str, at: &str) -> Result<Rule, String> {
        let names = &mut *self.names;
        let node = parse(p, &mut |n: &str| names.id(n)).map_err(|e| format!("{at}/pattern parse error: {e}"))?;
        if matches!(node, PatternNode::Terminal { error: true, .. } | PatternNode::Internal { error: true, .. }) {
            return Err(format!("{at}/pattern parse error: pattern contains an error node"));
        }
        if let Some(name) = literal_dollar_name(&node, p) {
            return Err(format!(
                "{at}/pattern: `{name}` is not a metavariable (names use A-Z, 0-9 and _) and Python source has no `$`, so the pattern never matches"
            ));
        }
        Ok(Rule::Pattern(Box::new(Pattern { node })))
    }
}

// ---------------------------------------------------------------------------------------------
// Evaluate.

/// `MatchOneNode` under `Smart`: a goal is never skipped, so `SkipGoal`/`SkipBoth` cannot occur.
#[derive(Clone, Copy, PartialEq, Eq)]
enum One {
    Both,
    SkipCand,
    No,
}

enum Flow {
    Continue,
    Fallthrough,
    Return,
}

/// `try_get_ellipsis_mode`: `Some(name)` for `$$$NAME` / `Some(None)` for `$$$`.
fn ellipsis(goal: &PatternNode) -> Option<Option<Name>> {
    match goal {
        PatternNode::MetaVar(MetaVar::Multiple) => Some(None),
        PatternNode::MetaVar(MetaVar::MultiCapture(n)) => Some(Some(*n)),
        _ => None,
    }
}

impl Env {
    fn multi_get(&self, name: Name) -> Option<&[Bound]> {
        self.multi.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_slice())
    }
    fn multi_set(&mut self, name: Name, nodes: Vec<Bound>) {
        if let Some(slot) = self.multi.iter_mut().find(|(n, _)| *n == name) {
            slot.1 = nodes;
        } else {
            self.multi.push((name, nodes));
        }
    }
}

impl Eval<'_> {
    /// `Pattern::match_node_with_env`: the env changes only when the pattern matches.
    pub(super) fn match_pattern(&self, x: &Pattern, node: Bound, env: &mut Env) -> bool {
        let Bound::Node(n) = node else {
            self.refuse("pattern: structural pattern over a parsed-literal node (no extra flags or tree-sitter kinds there)".into());
            return false;
        };
        let mut trial = env.clone();
        // `match_root_node_impl`: under Smart a root metavariable binds the candidate itself,
        // extras included.
        let ok = match &x.node {
            PatternNode::MetaVar(var) => self.p_meta_var(var, n, &mut trial),
            goal => self.p_node(goal, n, &mut trial) == One::Both,
        };
        if ok {
            *env = trial;
        }
        ok
    }

    fn p_extra(&self, n: u32) -> bool {
        self.f.flags[n as usize] & 2 != 0
    }

    /// `are_kinds_matching`: an ERROR goal matches any kind.
    fn p_kind(&self, kind: u32, named: bool, error: bool, cand: u32) -> bool {
        error || (self.f.kind[cand as usize] == kind && self.f.named(cand) == named)
    }

    /// `match_node_impl` under Smart.
    fn p_node(&self, goal: &PatternNode, cand: u32, env: &mut Env) -> One {
        match goal {
            PatternNode::Terminal { text, named, kind, error } => {
                // `match_terminal`: an anonymous goal compares kinds only.
                if self.p_kind(*kind, *named, *error, cand) && (!*named || text == self.f.text(cand)) {
                    return One::Both;
                }
                if self.p_extra(cand) || !self.f.named(cand) { One::SkipCand } else { One::No }
            }
            PatternNode::MetaVar(var) => {
                if self.p_extra(cand) {
                    return One::SkipCand;
                }
                if self.p_meta_var(var, cand, env) { One::Both } else { One::No }
            }
            PatternNode::Internal { kind, named, error, children } => {
                if !self.p_kind(*kind, *named, *error, cand) {
                    return One::No;
                }
                if self.p_nodes(children, self.f.children(cand), env) { One::Both } else { One::No }
            }
        }
    }

    /// `match_leaf_meta_var`.
    fn p_meta_var(&self, var: &MetaVar, cand: u32, env: &mut Env) -> bool {
        match var {
            MetaVar::Capture(name, named) => !(*named && !self.f.named(cand)) && self.p_insert(env, *name, cand),
            MetaVar::Dropped(named) => !(*named && !self.f.named(cand)),
            // Matched at the parent level (`debug_assert!(false)` upstream, `Some(())` in release).
            MetaVar::Multiple => true,
            MetaVar::MultiCapture(name) => self.p_insert_multi(env, *name, &[cand]),
        }
    }

    /// `match_nodes_impl_recursive` under Smart (every trailing candidate is skippable).
    fn p_nodes(&self, goals: &[PatternNode], cands: &[u32], env: &mut Env) -> bool {
        let (mut gi, mut ci) = (0usize, 0usize);
        if cands.is_empty() {
            return false;
        }
        loop {
            match self.p_ellipsis(goals, &mut gi, cands, &mut ci, env) {
                None => return false,
                Some(Flow::Return) => return true,
                Some(Flow::Continue) => continue,
                Some(Flow::Fallthrough) => {}
            }
            match self.p_single(goals, &mut gi, cands, &mut ci, env) {
                None => return false,
                Some(Flow::Return) => return true,
                Some(Flow::Continue) => continue,
                Some(Flow::Fallthrough) => {}
            }
            if gi < goals.len() {
                gi += 1;
                if ci < cands.len() {
                    ci += 1;
                }
            }
            if gi >= goals.len() {
                return true;
            }
            if ci >= cands.len() {
                return false;
            }
        }
    }

    /// `may_match_ellipsis_impl`.
    fn p_ellipsis(&self, goals: &[PatternNode], gi: &mut usize, cands: &[u32], ci: &mut usize, env: &mut Env) -> Option<Flow> {
        let Some(current) = goals.get(*gi) else {
            return Some(Flow::Return);
        };
        let Some(name) = ellipsis(current) else {
            return Some(Flow::Fallthrough);
        };
        let mut matched: Vec<u32> = Vec::new();
        *gi += 1;
        if *gi >= goals.len() {
            matched.extend_from_slice(&cands[*ci..]);
            *ci = cands.len();
            return self.p_bind_ellipsis(env, name, matched, 0).then_some(Flow::Return);
        }
        // Anonymous goals after an ellipsis are skipped, and as many trailing nodes unbound.
        let mut skipped = 0usize;
        while goals[*gi].is_trivial() {
            *gi += 1;
            skipped += 1;
            if *gi >= goals.len() {
                matched.extend_from_slice(&cands[*ci..]);
                *ci = cands.len();
                return self.p_bind_ellipsis(env, name, matched, skipped).then_some(Flow::Return);
            }
        }
        if ellipsis(&goals[*gi]).is_some() {
            let &cand = cands.get(*ci)?;
            matched.push(cand);
            *ci += 1;
            if *ci >= cands.len() {
                return None;
            }
            return self.p_bind_ellipsis(env, name, matched, skipped).then_some(Flow::Continue);
        }
        loop {
            // The next goal is probed on a copy; the real match re-runs in `p_single`.
            let mut probe = env.clone();
            if self.p_node(&goals[*gi], cands[*ci], &mut probe) == One::Both {
                return self.p_bind_ellipsis(env, name, matched, skipped).then_some(Flow::Fallthrough);
            }
            matched.push(cands[*ci]);
            *ci += 1;
            if *ci >= cands.len() {
                return None;
            }
        }
    }

    /// `match_single_node_while_skip_trivial` under Smart.
    fn p_single(&self, goals: &[PatternNode], gi: &mut usize, cands: &[u32], ci: &mut usize, env: &mut Env) -> Option<Flow> {
        loop {
            let Some(&cand) = cands.get(*ci) else {
                // `should_skip_goal`: only ellipses may remain.
                while let Some(goal) = goals.get(*gi) {
                    ellipsis(goal)?;
                    *gi += 1;
                }
                return Some(Flow::Fallthrough);
            };
            match self.p_node(&goals[*gi], cand, env) {
                One::Both => return Some(Flow::Fallthrough),
                One::SkipCand => *ci += 1,
                One::No => return None,
            }
        }
    }

    /// The env aggregator's `match_ellipsis`: a named ellipsis binds the run less the last
    /// `skipped` nodes.
    fn p_bind_ellipsis(&self, env: &mut Env, name: Option<Name>, mut matched: Vec<u32>, skipped: usize) -> bool {
        let Some(name) = name else { return true };
        matched.truncate(matched.len().saturating_sub(skipped));
        self.p_insert_multi(env, name, &matched)
    }

    /// `MetaVarEnv::insert`: a bound name must match the candidate exactly; the candidate then
    /// replaces it.
    fn p_insert(&self, env: &mut Env, name: Name, cand: u32) -> bool {
        match env.get(name) {
            Some(Bound::Node(prev)) if !self.p_exactly(prev, cand) => return false,
            Some(Bound::Parsed(_)) => {
                self.refuse("pattern: rebinding a capture bound to a parsed-literal node".into());
                return false;
            }
            _ => {}
        }
        env.set(name, Bound::Node(cand));
        true
    }

    /// `MetaVarEnv::insert_multi`: a bound list must equal the candidates' named nodes pairwise.
    fn p_insert_multi(&self, env: &mut Env, name: Name, cands: &[u32]) -> bool {
        if let Some(prev) = env.multi_get(name) {
            let mut prev_nodes = Vec::with_capacity(prev.len());
            for b in prev {
                match *b {
                    Bound::Node(n) => prev_nodes.push(n),
                    Bound::Parsed(_) => {
                        self.refuse("pattern: rebinding a multi capture bound to parsed-literal nodes".into());
                        return false;
                    }
                }
            }
            let mut old = prev_nodes.iter().copied().filter(|&n| self.f.named(n));
            let mut new = cands.iter().copied().filter(|&n| self.f.named(n));
            loop {
                match (old.next(), new.next()) {
                    (Some(a), Some(b)) => {
                        if !self.p_exactly(a, b) {
                            return false;
                        }
                    }
                    (None, None) => break,
                    _ => return false,
                }
            }
        }
        env.multi_set(name, cands.iter().map(|&n| Bound::Node(n)).collect());
        true
    }

    /// `does_node_match_exactly` over the tape: the same node; else text equality where either
    /// has no named child (`is_named_leaf`); else equal kinds and pairwise equal children.
    fn p_exactly(&self, a: u32, b: u32) -> bool {
        if a == b {
            return true;
        }
        let f = self.f;
        let named_leaf = |x: u32| f.children(x).iter().all(|&c| !f.named(c));
        if named_leaf(a) || named_leaf(b) {
            return f.text(a) == f.text(b);
        }
        if f.kind[a as usize] != f.kind[b as usize] || f.named(a) != f.named(b) {
            return false;
        }
        let (ca, cb) = (f.children(a), f.children(b));
        ca.len() == cb.len() && ca.iter().zip(cb).all(|(&x, &y)| self.p_exactly(x, y))
    }
}
