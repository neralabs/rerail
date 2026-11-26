use crate::code_rule_dsl::matcher::SemanticText;
use crate::code_rule_dsl::value::structure::Equality;

/// The semantic `compare` mode a `text` name selects, if it names one.
pub(super) fn text(name: &str) -> Option<SemanticText> {
    Some(match name {
        "value" => SemanticText::Value,
        "key" => SemanticText::Key,
        "structure" => SemanticText::Structure(Equality::Plain),
        "structure-hash" => SemanticText::Structure(Equality::Hash),
        "structure-parts" => SemanticText::Structure(Equality::Parts),
        "member" => SemanticText::Member,
        "name" => SemanticText::Name,
        "reference" => SemanticText::Reference { static_keys: true },
        "reference-no-static-keys" => SemanticText::Reference { static_keys: false },
        _ => return None,
    })
}

/// What a semantic mode that compares for equality only, exactly, compares.
pub(super) fn exact(text: SemanticText) -> Option<&'static str> {
    match text {
        SemanticText::Structure(_) => Some("structures"),
        SemanticText::Reference { .. } => Some("references"),
        _ => None,
    }
}
