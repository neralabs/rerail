//! The verifier every stream program is built through (`10` §3.1). It checks, per instruction:
//! operands defined earlier and of the right type, parameters in range, class tables that exist,
//! and `rounds` bodies that nest and feed back values of the type they started with. The set of
//! operations is `Op`'s, closed: the text parser refuses any other mnemonic, so a loop over bytes,
//! a branch or a host call cannot be written.

use super::program::{ClassTable, MAX_PLANES, Op, Ty, Value, clamp_planes, mod_planes};

/// How deep `rounds` bodies may nest. Python's lexer nests two.
pub(in crate::code_rule_dsl) const MAX_ROUNDS_DEPTH: usize = 2;

/// Why the verifier refused a program: the rule, and the instruction it broke at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Refusal {
    pub(in crate::code_rule_dsl) at: usize,
    pub(in crate::code_rule_dsl) rule: Rule,
}

/// The verifier's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Rule {
    /// An operand is not an earlier instruction.
    OperandNotEarlier,
    /// An operand is `Rounds` or `End`, which define no value.
    OperandHasNoValue,
    /// An operand's type is not the one the operation takes.
    OperandType,
    /// An operand is defined inside a `rounds` body that has already ended at this point, but
    /// not in a body that encloses it: only the last round's values are visible after the body,
    /// and only through the body's own `End`.
    OperandOutOfScope,
    /// A shift is not `1..=63` bytes.
    ShiftRange,
    /// A count mod `n` with `n` not in `2..=16`.
    ModRange,
    /// A clamped count with `max` not in `1..=255`.
    ClampRange,
    /// A `class` names a table that does not exist, or selects no class.
    ClassTable,
    /// A plane index or a compared value outside the value's planes.
    PlaneRange,
    /// `rounds` repeats a body fewer than 1 or more than 64 times.
    RoundsCount,
    /// `rounds` nested deeper than `MAX_ROUNDS_DEPTH`.
    RoundsDepth,
    /// A `Feed` not directly after its `Rounds` (or another `Feed`).
    FeedPlace,
    /// An `End` with no open body, or a body left open at the end.
    RoundsUnbalanced,
    /// An `End` that does not feed each of its body's feeds exactly once, from a value of the
    /// body of the feed's type.
    EndFeeds,
    /// An output names an undefined value, a value with no type, or a name used twice.
    Output,
}

fn refuse(at: usize, rule: Rule) -> Refusal {
    Refusal { at, rule }
}

/// One open `rounds` body while verifying.
struct Open {
    begin: usize,
    feeds: Vec<Value>,
    /// Whether the body has an instruction other than a `Feed` yet.
    started: bool,
}

/// Verify a program, giving every value's type.
pub(super) fn verify(
    tables: &[ClassTable],
    insts: &[Op],
    outputs: &[(String, Value)],
) -> Result<Vec<Option<Ty>>, Refusal> {
    let mut types: Vec<Option<Ty>> = Vec::with_capacity(insts.len());
    // For every value, the begin index of each body it is defined in, innermost last.
    let mut scope: Vec<Vec<usize>> = Vec::with_capacity(insts.len());
    let mut open: Vec<Open> = vec![];

    for (i, op) in insts.iter().enumerate() {
        let here: Vec<usize> = open.iter().map(|o| o.begin).collect();
        for v in op.operands() {
            let v = v as usize;
            if v >= i {
                return Err(refuse(i, Rule::OperandNotEarlier));
            }
            if types[v].is_none() {
                return Err(refuse(i, Rule::OperandHasNoValue));
            }
            if !visible(&scope[v], &here) {
                return Err(refuse(i, Rule::OperandOutOfScope));
            }
        }
        let mask = |v: &Value| types[*v as usize] == Some(Ty::Mask);
        let all_masks = |vs: &[Value]| vs.iter().all(mask);
        let ty = match op {
            Op::Starts | Op::Const(_) => Some(Ty::Mask),
            Op::Class { table, bits } => {
                if usize::from(*table) >= tables.len() || *bits == 0 {
                    return Err(refuse(i, Rule::ClassTable));
                }
                Some(Ty::Mask)
            }
            Op::Not(_)
            | Op::And(..)
            | Op::Or(..)
            | Op::Xor(..)
            | Op::AndNot(..)
            | Op::Select(..)
            | Op::Add(..)
            | Op::Sub(..)
            | Op::AddBack(..)
            | Op::Toggle(_) => {
                if !all_masks(&op.operands()) {
                    return Err(refuse(i, Rule::OperandType));
                }
                Some(Ty::Mask)
            }
            Op::Prev(a, k) | Op::Next(a, k) => {
                if !mask(a) {
                    return Err(refuse(i, Rule::OperandType));
                }
                if !(1..=63).contains(k) {
                    return Err(refuse(i, Rule::ShiftRange));
                }
                Some(Ty::Mask)
            }
            Op::ScanMod { n, up, down, reset } => {
                if !all_masks(&[*up, *down, *reset]) {
                    return Err(refuse(i, Rule::OperandType));
                }
                if !(2..=16).contains(n) {
                    return Err(refuse(i, Rule::ModRange));
                }
                Some(Ty::Planes(mod_planes(*n)))
            }
            Op::ScanClamp { max, up, down, reset } => {
                if !all_masks(&[*up, *down, *reset]) {
                    return Err(refuse(i, Rule::OperandType));
                }
                if *max == 0 {
                    return Err(refuse(i, Rule::ClampRange));
                }
                let w = clamp_planes(*max);
                debug_assert!(usize::from(w) <= MAX_PLANES);
                Some(Ty::Planes(w))
            }
            Op::Plane(a, p) | Op::PlaneEq(a, p) => {
                let Some(Ty::Planes(w)) = types[*a as usize] else {
                    return Err(refuse(i, Rule::OperandType));
                };
                let in_range = match op {
                    Op::Plane(..) => *p < w,
                    _ => u16::from(*p) < 1 << w,
                };
                if !in_range {
                    return Err(refuse(i, Rule::PlaneRange));
                }
                Some(Ty::Mask)
            }
            Op::Count(a) => {
                if !mask(a) {
                    return Err(refuse(i, Rule::OperandType));
                }
                Some(Ty::Count)
            }
            Op::Rounds { n, .. } => {
                if !(1..=64).contains(n) {
                    return Err(refuse(i, Rule::RoundsCount));
                }
                if open.len() == MAX_ROUNDS_DEPTH {
                    return Err(refuse(i, Rule::RoundsDepth));
                }
                None
            }
            Op::Feed(a) => {
                match open.last() {
                    Some(o) if !o.started => {}
                    _ => return Err(refuse(i, Rule::FeedPlace)),
                }
                let t = types[*a as usize];
                if t == Some(Ty::Count) {
                    return Err(refuse(i, Rule::OperandType));
                }
                t
            }
            Op::End(feeds) => {
                let Some(body) = open.last() else {
                    return Err(refuse(i, Rule::RoundsUnbalanced));
                };
                let mut fed: Vec<Value> = feeds.iter().map(|&(f, _)| f).collect();
                fed.sort_unstable();
                let mut want = body.feeds.clone();
                want.sort_unstable();
                if fed != want {
                    return Err(refuse(i, Rule::EndFeeds));
                }
                for &(f, v) in feeds {
                    let in_body = (v as usize) > body.begin;
                    if !in_body || types[v as usize] != types[f as usize] {
                        return Err(refuse(i, Rule::EndFeeds));
                    }
                }
                None
            }
        };
        // Track the bodies: any instruction but a `Feed` starts the innermost body.
        match op {
            Op::Feed(_) => open.last_mut().expect("checked").feeds.push(i as Value),
            _ => {
                if let Some(o) = open.last_mut() {
                    o.started = true;
                }
            }
        }
        if let Op::End(_) = op {
            open.pop();
        }
        types.push(ty);
        scope.push(here);
        if let Op::Rounds { .. } = op {
            open.push(Open { begin: i, feeds: vec![], started: false });
        }
    }
    if !open.is_empty() {
        return Err(refuse(insts.len(), Rule::RoundsUnbalanced));
    }
    let mut names: Vec<&str> = vec![];
    for (name, v) in outputs {
        let ok = (*v as usize) < types.len() && types[*v as usize].is_some();
        if !ok || names.contains(&name.as_str()) {
            return Err(refuse(insts.len(), Rule::Output));
        }
        names.push(name);
    }
    Ok(types)
}

/// Whether a value defined inside the bodies `defined` (outermost first) may be read where the
/// open bodies are `here`. Either the reader is inside every body the value is in (`defined` is a
/// prefix of `here`), or those bodies have ended and the reader is in one that enclosed them
/// (`here` is a prefix of `defined`), where the value is the last round's.
fn visible(defined: &[usize], here: &[usize]) -> bool {
    let k = defined.len().min(here.len());
    defined[..k] == here[..k]
}
