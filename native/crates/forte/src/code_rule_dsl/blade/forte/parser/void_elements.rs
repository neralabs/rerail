//! Forte's `Ast\Elements\VoidElements`: the elements that never have closing tags.

/// `VoidElements::$voidElements` (keys are lowercase; `isset` compares bytes).
pub(super) fn is_void_element(tag_name: &[u8]) -> bool {
    matches!(
        tag_name,
        b"area"
            | b"base"
            | b"br"
            | b"col"
            | b"embed"
            | b"hr"
            | b"img"
            | b"input"
            | b"link"
            | b"meta"
            | b"param"
            | b"source"
            | b"track"
            | b"wbr"
    )
}
