//! Persisted positional tape indexes and the SQL-demanded relation.

pub(super) mod sql_backend;

use super::tape::TapeDoc;

pub(crate) const POSITIONAL_INDEX_VERSION: i64 = 2;

pub(crate) fn kind_index_token(kind: u16) -> String {
    format!("k{kind:04x}")
}

fn search_index_document(
    tape_id: crate::content_store::TapeId,
    file: &TapeDoc,
) -> crate::content_store::TapeSearchIndex {
    let mut kinds = String::new();
    for node in 0..file.transducer_len() as u32 {
        if node != 0 {
            kinds.push(' ');
        }
        kinds.push_str(&kind_index_token(file.transducer_kind(node)));
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
pub(crate) fn persist_search_index_batch(
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
