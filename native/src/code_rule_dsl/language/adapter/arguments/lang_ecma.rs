//! Moved from `arguments.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Arguments by position only; ESLint's `node.arguments`.
impl ArgumentRules for Ecma {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call_expression", "new_expression"];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => {
                markers("JavaScript passes arguments by position only")
            }
            ArgumentFeature::Syntactic => refused(
                "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; TypeScript has no keywords",
            ),
            ArgumentFeature::Estree => Ok(()),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(arguments.typescript(call))
    }
}
