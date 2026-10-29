//! G's positional index, literal and kind anchors and literal materialization (1c040f4a),
//! test-only (open item O21): W's search index keeps kind presence only.
use super::*;

pub(super) const POSITIONAL_INDEX_VERSION: i64 = 3;
const VALUE_INDEX_LIMIT: usize = 96;

fn exact_anchor_key(program: &Program) -> Option<(u16, &[u8])> {
    if program.words.len() != 4
        || !program.directives.is_empty()
        || !matches!(split(program.words[0]), (KIND, 0, _))
        || !matches!(split(program.words[1]), (TEXT, 0, _))
        || !matches!(split(program.words[2]), (CAPTURE, 0, _))
        || !matches!(split(program.words[3]), (EMIT, 0, _))
    {
        return None;
    }
    let kind = split(program.words[0]).2;
    let predicate = split(program.words[1]).2;
    let TextPredicate::Equal(string) = program.predicates[usize::from(predicate)] else {
        return None;
    };
    Some((kind, &program.strings[usize::from(string)]))
}

fn append_kind_index_token(output: &mut String, kind: u16) {
    write!(output, "k{kind:04x}").expect("writing to a string cannot fail");
}

fn kind_index_token(kind: u16) -> String {
    let mut token = String::with_capacity(5);
    append_kind_index_token(&mut token, kind);
    token
}

fn value_indexable(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= VALUE_INDEX_LIMIT
        && std::str::from_utf8(value).is_ok_and(|value| {
            value
                .chars()
                .all(|character| !character.is_whitespace() && !character.is_control())
        })
}

fn value_index_token(kind: u16, value: &[u8]) -> Option<String> {
    if !value_indexable(value) {
        return None;
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = (value.len() > 16).then(|| Sha256::digest(value));
    let bytes = digest.as_ref().map_or(value, |digest| &digest[..16]);
    let marker = if digest.is_some() { 'h' } else { 'e' };
    let mut token = String::with_capacity(5 + bytes.len() * 2);
    write!(token, "{marker}{kind:04x}").expect("writing to a string cannot fail");
    for &byte in bytes {
        token.push(HEX[usize::from(byte >> 4)] as char);
        token.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Some(token)
}

#[derive(Clone)]
pub(super) struct SearchIndexDraft {
    node_count: u32,
    kind_tokens: Arc<String>,
    value_tokens: Arc<String>,
    source_text: Arc<String>,
}

impl SearchIndexDraft {
    pub(super) fn observe_node<'a>(
        kind_tokens: &mut String,
        values: &mut BTreeSet<(u16, &'a [u8])>,
        kind: u16,
        value: &'a [u8],
    ) {
        if !kind_tokens.is_empty() {
            kind_tokens.push(' ');
        }
        append_kind_index_token(kind_tokens, kind);
        if value_indexable(value) {
            values.insert((kind, value));
        }
    }

    pub(super) fn from_observed<'a>(
        source_text: Arc<String>,
        kind_tokens: String,
        values: impl IntoIterator<Item = (u16, &'a [u8])>,
        node_count: u32,
    ) -> Self {
        let mut value_tokens = String::new();
        for (kind, value) in values {
            let Some(token) = value_index_token(kind, value) else {
                continue;
            };
            if !value_tokens.is_empty() {
                value_tokens.push(' ');
            }
            value_tokens.push_str(&token);
        }
        Self {
            node_count,
            kind_tokens: Arc::new(kind_tokens),
            value_tokens: Arc::new(value_tokens),
            source_text,
        }
    }

    pub(super) fn to_index(
        &self,
        tape_id: crate::content_store::TapeId,
    ) -> crate::content_store::positional_search::TapeSearchIndex {
        crate::content_store::positional_search::TapeSearchIndex {
            tape_id,
            index_version: POSITIONAL_INDEX_VERSION,
            node_count: self.node_count,
            kind_tokens: Arc::clone(&self.kind_tokens),
            value_tokens: Arc::clone(&self.value_tokens),
            source_text: Arc::clone(&self.source_text),
        }
    }
}

pub(crate) fn search_index_document(
    tape_id: crate::content_store::TapeId,
    file: &TapeDoc,
) -> crate::content_store::positional_search::TapeSearchIndex {
    let node_count =
        u32::try_from(file.transducer_len()).expect("tape node count is limited to u32");
    let mut kinds = String::with_capacity(file.transducer_len().saturating_mul(6));
    let mut values = BTreeSet::new();
    for node in 0..node_count {
        if node != 0 {
            kinds.push(' ');
        }
        let kind = file.transducer_kind(node);
        append_kind_index_token(&mut kinds, kind);
        let value = file.transducer_text(node);
        if value_indexable(value) {
            values.insert((kind, value));
        }
    }
    SearchIndexDraft::from_observed(file.transducer_source_arc(), kinds, values, node_count)
        .to_index(tape_id)
}

/// Build query-independent kind, bounded exact-value, and source-word FTS streams. The kind stream
/// preserves preorder positions. The value stream stores one term per distinct file value.
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
        store.put_positional_search_indexes_bulk(&indexes)?;
    }
    Ok(())
}

pub(super) fn load_literal_eligibility_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    programs: &[Program],
) -> crate::content_store::StoreResult<Vec<Vec<Option<bool>>>> {
    let keys = programs
        .iter()
        .map(|program| {
            let (kind, value) = exact_anchor_key(program).ok_or_else(|| {
                crate::content_store::StoreError::Invalid(
                    "program cannot use the exact-value positional index".to_owned(),
                )
            })?;
            Ok(value_index_token(kind, value))
        })
        .collect::<crate::content_store::StoreResult<Vec<_>>>()?;
    if programs.is_empty() {
        return Ok(tape_ids.iter().map(|_| Vec::new()).collect());
    }
    let indexed = keys
        .iter()
        .enumerate()
        .filter_map(|(program, token)| token.clone().map(|token| (program, token)))
        .collect::<Vec<_>>();
    let tokens = indexed
        .iter()
        .map(|(_, token)| token.clone())
        .collect::<Vec<_>>();
    let candidates =
        store.get_exact_value_candidates_bulk(tape_ids, POSITIONAL_INDEX_VERSION, &tokens)?;
    let mut eligibility = vec![vec![None; programs.len()]; tape_ids.len()];
    for (file, file_candidates) in candidates.into_iter().enumerate() {
        for ((program, _), candidate) in indexed.iter().zip(file_candidates) {
            eligibility[file][*program] = candidate;
        }
    }
    Ok(eligibility)
}

/// Bulk-load exact-value file eligibility. `None` means candidate or unknown and widens to terminal
/// scanning. `Some([])` is a complete file rejection. Values outside the bounded domain stay unknown.
pub(crate) fn load_literal_anchor_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    programs: &[Program],
) -> crate::content_store::StoreResult<Vec<Vec<Option<Vec<u32>>>>> {
    Ok(load_literal_eligibility_batch(store, tape_ids, programs)?
        .into_iter()
        .map(|file| {
            file.into_iter()
                .map(|candidate| match candidate {
                    Some(false) => Some(Vec::new()),
                    Some(true) | None => None,
                })
                .collect()
        })
        .collect())
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn eligibility_masks(eligibility: Vec<Vec<Option<bool>>>, programs: usize) -> Vec<Vec<u32>> {
    let words = programs.div_ceil(32);
    eligibility
        .into_iter()
        .map(|file| {
            let mut bits = vec![0_u32; words];
            for (program, candidate) in file.into_iter().enumerate() {
                if candidate != Some(false) {
                    bits[program / 32] |= 1_u32 << (program % 32);
                }
            }
            bits
        })
        .collect()
}

pub(super) fn widen_anchor_candidates(
    programs: &[Program],
    files: &[TapeDoc],
    selections: Vec<Vec<Option<Vec<u32>>>>,
) -> Vec<Vec<Vec<u32>>> {
    selections
        .into_iter()
        .zip(files)
        .map(|(mut file, tape)| {
            let required = file
                .iter()
                .zip(programs)
                .filter_map(|(roots, program)| {
                    roots.is_none().then(|| match split(program.words[0]) {
                        (KIND, 0, kind) => Some(kind),
                        _ => None,
                    })?
                })
                .collect::<BTreeSet<_>>();
            let mut by_kind = required
                .iter()
                .map(|&kind| (kind, Vec::new()))
                .collect::<BTreeMap<_, _>>();
            if !by_kind.is_empty() {
                for node in 0..tape.transducer_len() as u32 {
                    if let Some(roots) = by_kind.get_mut(&tape.transducer_kind(node)) {
                        roots.push(node);
                    }
                }
            }
            for (roots, program) in file.iter_mut().zip(programs) {
                if roots.is_some() {
                    continue;
                }
                *roots = Some(match split(program.words[0]) {
                    (KIND, 0, kind) => by_kind.get(&kind).cloned().unwrap_or_default(),
                    _ => (0..tape.transducer_len() as u32).collect(),
                });
            }
            file.into_iter().map(Option::unwrap).collect()
        })
        .collect()
}

/// Load positional kind roots for structural programs. Missing indexes stay unknown. The matcher
/// can use extra roots, but it must not infer a negative result from an unknown index.
pub(crate) fn load_kind_anchor_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    kinds: &[u16],
) -> crate::content_store::StoreResult<Vec<Vec<Option<Vec<u32>>>>> {
    if kinds.is_empty() {
        return Ok(tape_ids.iter().map(|_| Vec::new()).collect());
    }
    let requests = tape_ids
        .iter()
        .flat_map(|&tape_id| {
            kinds
                .iter()
                .map(move |&kind| crate::content_store::positional_search::SearchAnchorLookup {
                    tape_id,
                    index_version: POSITIONAL_INDEX_VERSION,
                    kind_token: kind_index_token(kind),
                    value_token: None,
                })
        })
        .collect::<Vec<_>>();
    let loaded = store.get_search_anchors_bulk(&requests)?;
    Ok(loaded
        .chunks(kinds.len())
        .map(|file| {
            file.iter()
                .map(|selection| match selection {
                    crate::content_store::positional_search::StoredAnchorSelection::Roots(roots) => {
                        Some(roots.clone())
                    }
                    crate::content_store::positional_search::StoredAnchorSelection::Unknown
                    | crate::content_store::positional_search::StoredAnchorSelection::Candidate => None,
                })
                .collect()
        })
        .collect())
}

/// Select files from required literal words before a regex terminal batch. A true value remains a
/// candidate, a false value is a complete rejection, and `None` widens to terminal scanning.
pub(crate) fn load_text_candidate_batch(
    store: &crate::content_store::ContentStore,
    tape_ids: &[crate::content_store::TapeId],
    required_words: &[String],
) -> crate::content_store::StoreResult<Vec<Option<bool>>> {
    store.get_text_candidates_bulk(tape_ids, required_words)
}

/// Convert complete exact-literal anchors directly into terminal outputs. The selector already
/// proved the full predicate, so a second matcher or GPU dispatch would duplicate work.
#[cfg(all(test, target_os = "macos"))]
pub(super) fn materialize_literal_anchors(
    programs: &[Program],
    files: &[TapeDoc],
    anchors: &[Vec<Vec<u32>>],
) -> Result<Vec<Vec<Vec<Output>>>, Error> {
    if anchors.len() != files.len() || anchors.iter().any(|file| file.len() != programs.len()) {
        return Err(Error::InvalidAnchor);
    }
    let mut task_ordinal = 0_u32;
    let mut batch = Vec::with_capacity(files.len());
    for (file_index, tape) in files.iter().enumerate() {
        let mut file_outputs = Vec::with_capacity(programs.len());
        for (program_index, program) in programs.iter().enumerate() {
            exact_anchor_key(program).ok_or(Error::UnsupportedBackend)?;
            let capture = split(program.words[2]).2;
            let label = split(program.words[3]).2;
            let roots = &anchors[file_index][program_index];
            if roots
                .iter()
                .any(|&root| root as usize >= tape.transducer_len())
                || roots.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(Error::InvalidAnchor);
            }
            let mut outputs = Vec::with_capacity(roots.len());
            for &node in roots {
                outputs.push(Output {
                    task_ordinal,
                    local_ordinal: 0,
                    start: node,
                    label,
                    captures: vec![Capture {
                        label: capture,
                        node,
                        span: tape.transducer_span(node),
                        text: None,
                        properties: Vec::new(),
                    }],
                    properties: Vec::new(),
                    dynamic_properties: Vec::new(),
                });
                task_ordinal = task_ordinal.checked_add(1).ok_or(Error::BackendCapacity)?;
            }
            file_outputs.push(outputs);
        }
        batch.push(file_outputs);
    }
    Ok(batch)
}
