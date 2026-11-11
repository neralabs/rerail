//! Moved from `arguments.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Positional-only and keyword-only parameters; keywords; Ruff's syntactic reading.
impl ArgumentRules for Python {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call"];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers | ArgumentFeature::Syntactic => Ok(()),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments`; use language or syntactic for Python",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(if arguments.syntactic {
            arguments.syntactic(call)
        } else {
            arguments.python(call)
        })
    }
}
