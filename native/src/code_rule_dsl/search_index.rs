//! The positional search index of a stored tape: its preorder kind stream, each kind as its atom's FTS token, which
//! the store keeps per tape (`TapeSearchIndex`) and the anchors and the prefilter query.

use crate::code_rule_dsl::tape::TapeDoc;

/// A kind atom as an FTS token: `k` and base 36.
pub fn kind_token(code: u32) -> String {
    let mut digits = [0_u8; 7];
    let mut at = digits.len();
    let mut code = code;
    loop {
        at -= 1;
        digits[at] = b"0123456789abcdefghijklmnopqrstuvwxyz"[(code % 36) as usize];
        code /= 36;
        if code == 0 {
            break;
        }
    }
    format!("k{}", std::str::from_utf8(&digits[at..]).expect("base36 ASCII"))
}

/// The kind code of node `pre` in the prefilter's atoms. A restored tape reports ERROR as
/// tree-sitter's symbol (u16::MAX); the atoms use its tape code, 0, which no other node has.
pub fn kind_code(doc: &TapeDoc, pre: u32) -> u16 {
    if doc.transducer_flags(pre) & 8 != 0 { 0 } else { doc.transducer_kind(pre) }
}

/// Version 3: kind tokens are kind atoms (the prefilter's `kind_token`), as Pulling-it-all-together's
/// `ftsk` spelled them, so dialects never share a token. Version 5: the stream is a tape's
/// records, which hold no implied token (RER-131), so positions renumber.
pub const POSITIONAL_INDEX_VERSION: i64 = 5;

/// The token of `lang`'s kind `kind` in the positional kind stream: its atom (`tape tag · 1024 +
/// kind code`) in the prefilter's spelling. `None` for a dialect without a tape tag.
pub fn kind_index_token(lang: crate::code_rule_dsl::language::GrammarDialect, kind: u16) -> Option<String> {
    let tag = crate::code_rule_dsl::language::tape_tag(lang)?;
    Some(kind_token(
        crate::code_rule_dsl::facts::core::kind_atom(tag, kind),
    ))
}

fn search_index_document(
    tape_id: crate::content_store::TapeId,
    file: &TapeDoc,
) -> crate::content_store::TapeSearchIndex {
    use ast_grep_core::Doc;
    let lang = *file.get_lang();
    let mut kinds = String::new();
    for node in 0..file.transducer_len() as u32 {
        if node != 0 {
            kinds.push(' ');
        }
        kinds.push_str(
            &kind_index_token(lang, file.transducer_kind(node)).expect("a stored tape's dialect has a tape tag"),
        );
    }
    crate::content_store::TapeSearchIndex {
        tape_id,
        index_version: POSITIONAL_INDEX_VERSION,
        node_count: u32::try_from(file.transducer_len())
            .expect("tape node count is limited to u32"),
        has_errors: !file.syntax_error_ranges().is_empty(),
        kind_tokens: kinds,
    }
}

/// Build query-independent kind streams. The stream preserves preorder positions.
pub fn persist_search_index_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    files: &[TapeDoc],
) -> crate::content_store::StoreResult<()> {
    if tape_ids.len() != files.len() {
        return Err(crate::content_store::StoreError::Invalid(
            "tape identity count does not match anchor files".to_owned(),
        ));
    }
    for (files, tape_ids) in files.chunks(64).zip(tape_ids.chunks(64)) {
        let indexes = files
            .iter()
            .zip(tape_ids)
            .map(|(file, &tape_id)| search_index_document(tape_id, file))
            .collect::<Vec<_>>();
        store.put_search_indexes_bulk(&indexes)?;
    }
    Ok(())
}
