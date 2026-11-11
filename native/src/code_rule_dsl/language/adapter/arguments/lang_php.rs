//! Moved from `arguments.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// Named arguments that bind any parameter; Ruff's syntactic reading.
impl ArgumentRules for Php {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &[
        "function_call_expression",
        "member_call_expression",
        "nullsafe_member_call_expression",
        "scoped_call_expression",
        "object_creation_expression",
    ];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => {
                markers("PHP has no positional-only or keyword-only parameters")
            }
            ArgumentFeature::Syntactic => Ok(()),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; php has none (use language or syntactic)",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        if arguments.syntactic {
            arguments.php_syntactic(call)
        } else {
            arguments.php(call)
        }
    }
}
