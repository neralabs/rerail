//! Preserve lazy scalar range checks when a binding has no observable use.
use super::*;

impl Step {
    /// A slot copy carries a deferred value. Other operations observe their inputs.
    pub(super) fn observes(&self, slot: usize) -> bool {
        let key = |k: &Key| matches!(k,Key::Slot(s) if *s==slot);
        match self {
            Step::Atom { keys, .. } => keys.iter().any(|(_, k)| key(k)),
            Step::Scalar {
                keys, out, bind, ..
            } => keys.iter().any(|(_, k)| key(k)) || !*bind && *out == slot,
            Step::Cat { a, b, out, bind } => key(a) || key(b) || !*bind && *out == slot,
            Step::Split {
                whole,
                a,
                b,
                ka,
                kb,
            } => key(whole) || a.is_none() && key(ka) || b.is_none() && key(kb),
            Step::Eq { a, b }
            | Step::Ne { a, b }
            | Step::Lt { a, b, .. }
            | Step::Doc { a, b, .. } => key(a) || key(b),
            Step::Func {
                args, out, bind, ..
            } => args.iter().any(key) || !*bind && *out == slot,
            Step::DocOf { n, out, bind } => key(n) || !*bind && *out == slot,
            Step::Test { s, p, .. } => key(s) || key(p),
            Step::In { a, .. } => key(a),
            Step::Let { .. } | Step::Unused { .. } => false,
        }
    }
}

pub(super) fn sums(steps: &mut [Step], head: &[Term]) {
    let mut live = std::collections::BTreeSet::new();
    for term in head {
        match term {
            Term::Slot(s) => {
                live.insert(*s);
            }
            Term::Cat(a, b) => {
                live.extend([*a, *b]);
            }
            _ => {}
        }
    }
    for (position, step) in steps.iter_mut().enumerate().rev() {
        let proof = match step {
            Step::Scalar {
                kind: ScalarKind::Sum,
                out,
                bind,
                ..
            } => Some(crate::code_rule_dsl::logic::analysis::scalars::unused_sum(
                !*bind,
                live.contains(out),
            )),
            Step::Let { a, .. } if !live.contains(a) => {
                Some(crate::code_rule_dsl::logic::analysis::Decision::new(
                    true,
                    "An unused slot copy introduces no observation of its input",
                ))
            }
            _ => None,
        };
        if let Some(proof) = proof.filter(|p| p.proven) {
            crate::code_rule_dsl::logic::profile::decision("native scalar use", || {
                serde_json::json!({"step":position,"proof":proof}).to_string()
            });
            *step = Step::Unused { proof };
            continue;
        }
        let mut read = |key: &Key| {
            if let Key::Slot(s) = key {
                live.insert(*s);
            }
        };
        match step {
            Step::Atom { keys, .. } => {
                for (_, k) in keys {
                    read(k);
                }
            }
            Step::Scalar {
                keys, out, bind, ..
            } => {
                for (_, k) in keys {
                    read(k);
                }
                if !*bind {
                    read(&Key::Slot(*out));
                }
            }
            Step::Cat { a, b, out, bind } => {
                read(a);
                read(b);
                if !*bind {
                    read(&Key::Slot(*out));
                }
            }
            Step::Split {
                whole,
                a,
                b,
                ka,
                kb,
            } => {
                read(whole);
                if a.is_none() {
                    read(ka);
                }
                if b.is_none() {
                    read(kb);
                }
            }
            Step::Eq { a, b }
            | Step::Ne { a, b }
            | Step::Lt { a, b, .. }
            | Step::Doc { a, b, .. } => {
                read(a);
                read(b);
            }
            Step::Let { b, .. } => read(b),
            Step::Func {
                args, out, bind, ..
            } => {
                for k in args {
                    read(k);
                }
                if !*bind {
                    read(&Key::Slot(*out));
                }
            }
            Step::DocOf { n, out, bind } => {
                read(n);
                if !*bind {
                    read(&Key::Slot(*out));
                }
            }
            Step::Test { s, p, .. } => {
                read(s);
                read(p);
            }
            Step::In { a, .. } => read(a),
            Step::Unused { .. } => {}
        }
    }
}
