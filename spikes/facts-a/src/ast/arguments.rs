//! `arguments` (a Rerail extension), Python `mode: "syntactic"`: a call's arguments bound to a
//! signature's named slots as Ruff's `Arguments::find_argument` finds them.
//!
//! Ported from native (the source of truth):
//! - `native/src/code_rule_dsl/compile/arguments.rs` — `signature` (L17-70), `slot_index`
//!   (L73-81), `parse` (L90-149), `compile_arguments` (L159-195); Python's gate is
//!   `language/adapter/arguments.rs` L46-57 (markers and `syntactic` allowed, `estree` refused).
//! - `native/src/code_rule_dsl/matcher/arguments.rs` — `Arguments::match_node` (L69-112),
//!   `positional` (L115-121), `syntactic` (L196-240), `named_children` (L62-66); the call kinds
//!   are Python's `["call"]` (`language/adapter/arguments.rs` L48).
//! - Captures (`matcher/captures/semantic.rs`): binds = the `bind` names (L21-23), reads none
//!   (L90), mentions = the `bind` names (L129-131); `Semantic::reads_values` is true (semantic.rs
//!   L68-74), so a query holding `arguments` is three-valued.
//!
//! Syntactic mode never yields an unknown slot (native: "Nothing is unknown"), so a verdict is
//! true or false.
//!
//! Refusals (named, never approximated):
//! - compile: `arguments: mode "language"` (also the default without `mode`): Python's call binding
//!   (`Arguments::python`, with starred/`**` unknowns and literal `**{...}` keywords) is not ported.
//! - compile: `arguments: grammar field` when the grammar lacks `arguments`/`name`/`value` fields.
//! - match: `arguments: non-ASCII keyword name` — native compares a keyword's name under NFKC
//!   (`identifier::python_identifier`); without the Unicode tables a non-ASCII keyword compared
//!   against a keyword-passable slot is refused.
//!
//! Compile errors that are native validation errors (unknown key, bad mode, bad signature, bind or
//! absent, `estree` on Python) are reported as `Err` with native's message.
use super::*;

/// Slots one signature may name (compile/arguments.rs L13).
const MAX_SLOTS: usize = 256;

/// How a signature slot may be passed (matcher/arguments.rs `SlotKind`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotKind {
    /// Before `/`: by position only.
    PositionalOnly,
    /// By position or by keyword.
    Either,
    /// After `*`: by keyword only.
    KeywordOnly,
}

/// A compiled `arguments` in syntactic mode.
#[derive(Clone)]
pub struct Arguments {
    pub slots: Vec<(String, SlotKind)>,
    /// Capture names and the slot each binds, by slot index, in the `bind` object's order.
    pub bind: Vec<(Name, usize)>,
    /// Slots that must not be passed, by index.
    pub absent: Vec<usize>,
    /// The kind symbol of Python's `call`.
    call: u32,
    /// Grammar field codes: a call's `arguments`, a keyword argument's `name` and `value`.
    f_arguments: u16,
    f_name: u16,
    f_value: u16,
}

impl Arguments {
    /// Every nested rule: none.
    pub fn sub_rules(&self) -> Vec<&Rule> {
        Vec::new()
    }
    /// What the construct binds itself: its `bind` names.
    pub fn binds(&self) -> Vec<Name> {
        self.bind.iter().map(|(name, _)| *name).collect()
    }
    /// What it reads: nothing.
    pub fn reads(&self) -> Vec<Name> {
        Vec::new()
    }
    /// Native `Semantic::reads_values`: true (the query is three-valued).
    pub fn reads_values(&self) -> bool {
        true
    }

    /// Positional slots, in order.
    fn positional(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, (_, kind))| *kind != SlotKind::KeywordOnly)
            .map(|(index, _)| index)
    }
}

fn is_identifier(name: &str) -> bool {
    // compile/resolve.rs `is_identifier`.
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A validated signature: each slot's name and how it may be passed (compile/arguments.rs
/// `signature`).
fn signature(value: &serde_json::Map<String, Json>, path: &str) -> Result<Vec<(String, SlotKind)>, String> {
    let path = format!("{path}/signature");
    let items = value
        .get("signature")
        .ok_or_else(|| format!("{path} is required"))?
        .as_array()
        .ok_or_else(|| format!("{path} must be an array of slot names"))?;
    if items.len() > MAX_SLOTS {
        return Err(format!("{path} lists {} entries; at most {MAX_SLOTS} are allowed", items.len()));
    }
    let mut slots: Vec<(String, SlotKind)> = Vec::new();
    let mut kind = SlotKind::Either;
    let (mut slash, mut star) = (false, false);
    for (index, item) in items.iter().enumerate() {
        let name = item.as_str().ok_or_else(|| format!("{path}/{index} must be a string"))?;
        match name {
            "/" => {
                if slash || star {
                    return Err(format!("{path}/{index}: `/` may appear once, before `*`"));
                }
                slash = true;
                for slot in &mut slots {
                    slot.1 = SlotKind::PositionalOnly;
                }
            }
            "*" => {
                if star {
                    return Err(format!("{path}/{index}: `*` may appear once"));
                }
                star = true;
                kind = SlotKind::KeywordOnly;
            }
            _ if !is_identifier(name) || name.len() > 256 => {
                return Err(format!("{path}/{index}: `{name}` is not a slot name (an identifier), `/` or `*`"));
            }
            _ if slots.iter().any(|(slot, _)| slot == name) => {
                return Err(format!("{path}/{index}: `{name}` is listed twice"));
            }
            _ => slots.push((name.to_owned(), kind)),
        }
    }
    if slots.is_empty() {
        return Err(format!("{path} must name a slot"));
    }
    Ok(slots)
}

fn slot_index(slots: &[(String, SlotKind)], name: &Json, path: &str) -> Result<usize, String> {
    let name = name.as_str().ok_or_else(|| format!("{path} must be a slot name"))?;
    slots
        .iter()
        .position(|(slot, _)| slot == name)
        .ok_or_else(|| format!("{path}: `{name}` is not a slot of the signature"))
}

impl Compiler<'_> {
    /// `arguments: {signature, bind?, absent?, mode?}` (compile/arguments.rs `parse` +
    /// `compile_arguments` for Python). Only `mode: "syntactic"` compiles; see the module doc.
    pub(super) fn compile_arguments(&mut self, v: &Json, at: &str) -> Result<Rule, String> {
        let path = format!("{at}/arguments");
        let entry = obj(v, &path)?;
        if let Some(k) = entry.keys().find(|k| !["signature", "bind", "absent", "mode"].contains(&k.as_str())) {
            return Err(format!("unknown field `{k}` at {path}"));
        }
        let mode = entry.get("mode");
        if let Some(m) = mode {
            if !matches!(m.as_str(), Some("language" | "syntactic" | "estree")) {
                return Err(format!("{path}/mode must be \"language\", \"syntactic\" or \"estree\""));
            }
        }
        let slots = signature(entry, &path)?;
        let mut bind = Vec::new();
        if let Some(map) = entry.get("bind") {
            let bind_path = format!("{path}/bind");
            let map = obj(map, &bind_path)?;
            if map.is_empty() {
                return Err(format!("{bind_path} must bind a capture"));
            }
            for (name, slot) in map {
                let valid = name.chars().next().is_some_and(|first| first.is_ascii_uppercase())
                    && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                if !valid {
                    return Err(format!(
                        "{bind_path}: `{name}` is not a metavariable name (A-Z first, then A-Z, 0-9, _, without `$`)"
                    ));
                }
                bind.push((name.clone(), slot_index(&slots, slot, &format!("{bind_path}/{name}"))?));
            }
        }
        let mut absent = Vec::new();
        if let Some(items) = entry.get("absent") {
            let absent_path = format!("{path}/absent");
            let items = items.as_array().ok_or_else(|| format!("{absent_path} must be an array of slot names"))?;
            if items.is_empty() {
                return Err(format!("{absent_path} must not be empty"));
            }
            for (index, item) in items.iter().enumerate() {
                let slot = slot_index(&slots, item, &format!("{absent_path}/{index}"))?;
                if bind.iter().any(|(_, bound)| *bound == slot) {
                    return Err(format!("{absent_path}/{index}: `{}` is bound, so it is passed", slots[slot].0));
                }
                absent.push(slot);
            }
        }
        if bind.is_empty() && absent.is_empty() {
            return Err(format!("{path} must `bind` a slot or list `absent` ones"));
        }
        match mode.and_then(Json::as_str) {
            Some("syntactic") => {}
            Some("estree") => {
                return Err(format!(
                    "{path}/mode: `estree` reads ESLint's `node.arguments`; use language or syntactic for Python"
                ));
            }
            _ => {
                return Err(format!(
                    "{path}: arguments: mode \"language\" (Python call binding with starred and `**` unknowns) is not ported"
                ));
            }
        }
        let field = |name: &str| {
            self.grammar
                .field_code(name)
                .ok_or_else(|| format!("{path}: arguments: grammar field `{name}` missing"))
        };
        let (f_arguments, f_name, f_value) = (field("arguments")?, field("name")?, field("value")?);
        let bind = bind.into_iter().map(|(name, slot)| (self.names.id(&name), slot)).collect();
        Ok(Rule::Arguments(Box::new(Arguments {
            slots,
            bind,
            absent,
            call: facts::intern("call"),
            f_arguments,
            f_name,
            f_value,
        })))
    }
}

impl Eval<'_> {
    /// Native `Arguments::match_node` with `Arguments::syntactic`.
    pub(super) fn match_arguments(&self, x: &Arguments, node: Bound, env: &mut Env) -> bool {
        let Bound::Node(n) = node else { return false };
        if self.f.kind[n as usize] != x.call || !self.f.named(n) {
            return false;
        }
        let passed = match self.args_syntactic(x, n) {
            Ok(passed) => passed,
            Err(why) => {
                self.refuse(why);
                return false;
            }
        };
        if x.absent.iter().any(|&slot| passed[slot].is_some()) {
            return false;
        }
        // ast-grep's `MetaVarEnv::insert`: a name bound already must match the node exactly.
        let mut trial = env.clone();
        for &(name, slot) in &x.bind {
            let Some(value) = passed[slot] else { return false };
            let value = Bound::Node(value);
            if let Some(prev) = trial.get(name) {
                if !self.match_exactly(prev, value) {
                    return false;
                }
            }
            trial.set(name, value);
        }
        *env = trial;
        true
    }

    fn args_field_of(&self, n: u32, field: u16) -> Option<u32> {
        self.f.children(n).iter().copied().find(|&c| self.f.field[c as usize] == field)
    }

    fn args_kind_is(&self, n: u32, kind: &str) -> bool {
        &*facts::name(self.f.kind[n as usize]) == kind
    }

    /// Ruff's `Arguments::find_argument` (matcher/arguments.rs `syntactic`): a keyword of the
    /// slot's name (the first one), else the positional argument at the slot's position among
    /// the arguments before the first starred one. `**kw` names no keyword. Per slot, the node
    /// passed (`None`: absent).
    fn args_syntactic(&self, x: &Arguments, call: u32) -> Result<Vec<Option<u32>>, String> {
        let mut passed = vec![None; x.slots.len()];
        let Some(arguments) = self.args_field_of(call, x.f_arguments) else {
            return Ok(passed);
        };
        let mut positional = Vec::new();
        let mut keywords: Vec<(u32, u32)> = Vec::new();
        if self.args_kind_is(arguments, "generator_expression") {
            positional.push(arguments);
        } else {
            let mut starred = false;
            for &argument in self.f.children(arguments) {
                // `named_children`: named, and no comment.
                if !self.f.named(argument) || facts::name(self.f.kind[argument as usize]).ends_with("comment") {
                    continue;
                }
                match &*facts::name(self.f.kind[argument as usize]) {
                    "keyword_argument" => {
                        if let (Some(name), Some(value)) =
                            (self.args_field_of(argument, x.f_name), self.args_field_of(argument, x.f_value))
                        {
                            keywords.push((name, value));
                        }
                    }
                    "dictionary_splat" => {}
                    "list_splat" | "parenthesized_list_splat" => starred = true,
                    _ if starred => {}
                    _ => positional.push(argument),
                }
            }
        }
        for (position, slot) in x.positional().enumerate() {
            if let Some(&argument) = positional.get(position) {
                passed[slot] = Some(argument);
            }
        }
        for (slot, (name, kind)) in x.slots.iter().enumerate() {
            if *kind == SlotKind::PositionalOnly {
                continue;
            }
            for &(keyword, value) in &keywords {
                let text = self.f.text(keyword);
                if !text.is_ascii() {
                    return Err(format!(
                        "arguments: non-ASCII keyword name `{text}` (NFKC comparison needs the Unicode tables)"
                    ));
                }
                if text == name {
                    passed[slot] = Some(value);
                    break;
                }
            }
        }
        Ok(passed)
    }
}
