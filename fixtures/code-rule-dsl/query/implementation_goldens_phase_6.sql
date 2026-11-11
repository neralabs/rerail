-- Independently authored Phase 6 clause-level SQL contracts.

-- fixture: clause-forbid-content-regex
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_selected_1 AS (
  SELECT candidate.* FROM clause_0_term_0 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_term_0 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_1 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-forbid-path-regex
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_selected_1 AS (
  SELECT candidate.* FROM clause_0_term_0 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_term_0 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_1 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-select-when-exists
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_1 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 1 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_select_when_2 AS (
  SELECT selected.* FROM clause_0_term_0 selected WHERE EXISTS (SELECT 1 FROM clause_0_term_1 required WHERE required.file_index = selected.file_index)
),
clause_0_selected_3 AS (
  SELECT candidate.* FROM clause_0_select_when_2 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_select_when_2 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_3 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0, 1) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0, 1) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0, 1))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-select-when-absent
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_1 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 1 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_2 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 2 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_3 AS (
  SELECT left_input.batch, left_input.row_kind, left_input.producer_id, left_input.file_index, left_input.task_index, left_input.match_index, left_input.capture_index, left_input.capture_occurrence, left_input.capture_sequence_start, left_input.capture_sequence_count, left_input.node, left_input.related_node, left_input.axis_kind, left_input.axis_order, left_input.fact_kind, left_input.document_id, left_input.mapped_document_id, left_input.start_ordinal, left_input.output_label, left_input.capture_label, left_input.text_value, left_input.start_byte, left_input.end_byte, CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END AS verdict, left_input.complete, left_input.work_used, left_input.error_code, left_input.field_id, left_input.related_named, left_input.source_text, left_input.structural_key, left_input.named_leaf, left_input.capture_kind, left_input.capture_named, left_input.equality_occurrence, left_input.bindings, left_input.choice_order FROM clause_0_term_1 left_input
  INNER JOIN clause_0_term_2 right_input
    ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node
),
clause_0_term_4 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 1 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_5 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 2 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_6 AS (
  SELECT left_input.batch, left_input.row_kind, left_input.producer_id, left_input.file_index, left_input.task_index, left_input.match_index, left_input.capture_index, left_input.capture_occurrence, left_input.capture_sequence_start, left_input.capture_sequence_count, left_input.node, left_input.related_node, left_input.axis_kind, left_input.axis_order, left_input.fact_kind, left_input.document_id, left_input.mapped_document_id, left_input.start_ordinal, left_input.output_label, left_input.capture_label, left_input.text_value, left_input.start_byte, left_input.end_byte, CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END AS verdict, left_input.complete, left_input.work_used, left_input.error_code, left_input.field_id, left_input.related_named, left_input.source_text, left_input.structural_key, left_input.named_leaf, left_input.capture_kind, left_input.capture_named, left_input.equality_occurrence, left_input.bindings, left_input.choice_order FROM clause_0_term_4 left_input
  INNER JOIN clause_0_term_5 right_input
    ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node
),
clause_0_relation_7_walk(file_index, source_node, candidate, axis_order, eligible, path, source_bindings, source_batch, source_row_kind, source_producer_id, source_file_index, source_task_index, source_match_index, source_capture_index, source_capture_occurrence, source_capture_sequence_start, source_capture_sequence_count, source_node, source_related_node, source_axis_kind, source_axis_order, source_fact_kind, source_document_id, source_mapped_document_id, source_start_ordinal, source_output_label, source_capture_label, source_text_value, source_start_byte, source_end_byte, source_verdict, source_complete, source_work_used, source_error_code, source_field_id, source_related_named, source_source_text, source_structural_key, source_named_leaf, source_capture_kind, source_capture_named, source_equality_occurrence, source_choice_order) AS (
  SELECT source.file_index, source.node, edge.related_node, 1, 1, printf('/%d', edge.related_node), source.bindings, source.batch, source.row_kind, source.producer_id, source.file_index, source.task_index, source.match_index, source.capture_index, source.capture_occurrence, source.capture_sequence_start, source.capture_sequence_count, source.node, source.related_node, source.axis_kind, source.axis_order, source.fact_kind, source.document_id, source.mapped_document_id, source.start_ordinal, source.output_label, source.capture_label, source.text_value, source.start_byte, source.end_byte, source.verdict, source.complete, source.work_used, source.error_code, source.field_id, source.related_named, source.source_text, source.structural_key, source.named_leaf, source.capture_kind, source.capture_named, source.equality_occurrence, source.choice_order
  FROM clause_0_relation_6 source
  JOIN navigation edge ON edge.producer_id = 3 AND edge.file_index = source.file_index AND edge.node = source.node
  UNION ALL
  SELECT walk.file_index, walk.source_node, edge.related_node, walk.axis_order + 1, 1, walk.path || printf('/%d', edge.related_node), walk.source_bindings, walk.source_batch, walk.source_row_kind, walk.source_producer_id, walk.source_file_index, walk.source_task_index, walk.source_match_index, walk.source_capture_index, walk.source_capture_occurrence, walk.source_capture_sequence_start, walk.source_capture_sequence_count, walk.source_node, walk.source_related_node, walk.source_axis_kind, walk.source_axis_order, walk.source_fact_kind, walk.source_document_id, walk.source_mapped_document_id, walk.source_start_ordinal, walk.source_output_label, walk.source_capture_label, walk.source_text_value, walk.source_start_byte, walk.source_end_byte, walk.source_verdict, walk.source_complete, walk.source_work_used, walk.source_error_code, walk.source_field_id, walk.source_related_named, walk.source_source_text, walk.source_structural_key, walk.source_named_leaf, walk.source_capture_kind, walk.source_capture_named, walk.source_equality_occurrence, walk.source_choice_order
  FROM clause_0_relation_7_walk walk
  JOIN navigation edge ON edge.producer_id = 3 AND edge.file_index = walk.file_index AND edge.node = walk.candidate
),
clause_0_relation_7_context AS (
  SELECT walk.source_batch AS batch, walk.source_row_kind AS row_kind, walk.source_producer_id AS producer_id, walk.source_file_index AS file_index, walk.source_task_index AS task_index, walk.source_match_index AS match_index, walk.source_capture_index AS capture_index, walk.source_capture_occurrence AS capture_occurrence, walk.source_capture_sequence_start AS capture_sequence_start, walk.source_capture_sequence_count AS capture_sequence_count, walk.candidate AS node, walk.source_node AS related_node, walk.source_axis_kind AS axis_kind, walk.source_axis_order AS axis_order, walk.source_fact_kind AS fact_kind, walk.source_document_id AS document_id, walk.source_mapped_document_id AS mapped_document_id, walk.source_start_ordinal AS start_ordinal, walk.source_output_label AS output_label, walk.source_capture_label AS capture_label, walk.source_text_value AS text_value, walk.source_start_byte AS start_byte, walk.source_end_byte AS end_byte, walk.source_verdict AS verdict, walk.source_complete AS complete, walk.source_work_used AS work_used, walk.source_error_code AS error_code, walk.source_field_id AS field_id, walk.source_related_named AS related_named, walk.source_source_text AS source_text, walk.source_structural_key AS structural_key, walk.source_named_leaf AS named_leaf, walk.source_capture_kind AS capture_kind, walk.source_capture_named AS capture_named, walk.source_equality_occurrence AS equality_occurrence, walk.source_bindings AS bindings, walk.source_choice_order AS choice_order
  FROM clause_0_relation_7_walk walk
),
clause_0_term_8 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 4 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_9 AS (
  SELECT left_input.batch, left_input.row_kind, left_input.producer_id, left_input.file_index, left_input.task_index, left_input.match_index, left_input.capture_index, left_input.capture_occurrence, left_input.capture_sequence_start, left_input.capture_sequence_count, left_input.node, left_input.related_node, left_input.axis_kind, left_input.axis_order, left_input.fact_kind, left_input.document_id, left_input.mapped_document_id, left_input.start_ordinal, left_input.output_label, left_input.capture_label, left_input.text_value, left_input.start_byte, left_input.end_byte, CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END AS verdict, left_input.complete, left_input.work_used, left_input.error_code, left_input.field_id, left_input.related_named, left_input.source_text, left_input.structural_key, left_input.named_leaf, left_input.capture_kind, left_input.capture_named, left_input.equality_occurrence, left_input.bindings, left_input.choice_order FROM clause_0_relation_7_context left_input
  INNER JOIN clause_0_term_8 right_input
    ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node
),
clause_0_relation_7_accepted AS (
  SELECT walk.file_index, walk.source_node, related.bindings, related.choice_order, row_number() OVER (PARTITION BY walk.file_index, walk.source_node, walk.source_bindings ORDER BY walk.candidate ASC, walk.candidate, related.choice_order, related.producer_id, related.match_index) AS accepted_order, walk.source_batch, walk.source_row_kind, walk.source_producer_id, walk.source_file_index, walk.source_task_index, walk.source_match_index, walk.source_capture_index, walk.source_capture_occurrence, walk.source_capture_sequence_start, walk.source_capture_sequence_count, walk.source_node, walk.source_related_node, walk.source_axis_kind, walk.source_axis_order, walk.source_fact_kind, walk.source_document_id, walk.source_mapped_document_id, walk.source_start_ordinal, walk.source_output_label, walk.source_capture_label, walk.source_text_value, walk.source_start_byte, walk.source_end_byte, walk.source_verdict, walk.source_complete, walk.source_work_used, walk.source_error_code, walk.source_field_id, walk.source_related_named, walk.source_source_text, walk.source_structural_key, walk.source_named_leaf, walk.source_capture_kind, walk.source_capture_named, walk.source_equality_occurrence, walk.source_choice_order
  FROM clause_0_relation_7_walk walk
  JOIN clause_0_relation_9 related ON related.file_index = walk.file_index AND related.node = walk.candidate AND related.related_node = walk.source_node
  WHERE walk.eligible
),
clause_0_relation_7 AS (
  SELECT accepted.source_batch AS batch, accepted.source_row_kind AS row_kind, accepted.source_producer_id AS producer_id, accepted.source_file_index AS file_index, accepted.source_task_index AS task_index, accepted.source_match_index AS match_index, accepted.source_capture_index AS capture_index, accepted.source_capture_occurrence AS capture_occurrence, accepted.source_capture_sequence_start AS capture_sequence_start, accepted.source_capture_sequence_count AS capture_sequence_count, accepted.source_node AS node, accepted.source_related_node AS related_node, accepted.source_axis_kind AS axis_kind, accepted.source_axis_order AS axis_order, accepted.source_fact_kind AS fact_kind, accepted.source_document_id AS document_id, accepted.source_mapped_document_id AS mapped_document_id, accepted.source_start_ordinal AS start_ordinal, accepted.source_output_label AS output_label, accepted.source_capture_label AS capture_label, accepted.source_text_value AS text_value, accepted.source_start_byte AS start_byte, accepted.source_end_byte AS end_byte, accepted.source_verdict AS verdict, accepted.source_complete AS complete, accepted.source_work_used AS work_used, accepted.source_error_code AS error_code, accepted.source_field_id AS field_id, accepted.source_related_named AS related_named, accepted.source_source_text AS source_text, accepted.source_structural_key AS structural_key, accepted.source_named_leaf AS named_leaf, accepted.source_capture_kind AS capture_kind, accepted.source_capture_named AS capture_named, accepted.source_equality_occurrence AS equality_occurrence, accepted.bindings, accepted.choice_order FROM clause_0_relation_7_accepted accepted WHERE 1 = 1 AND accepted.accepted_order = 1
),
clause_0_relation_10 AS (
  SELECT universe.batch, universe.row_kind, universe.producer_id, universe.file_index, universe.task_index, universe.match_index, universe.capture_index, universe.capture_occurrence, universe.capture_sequence_start, universe.capture_sequence_count, universe.node, universe.related_node, universe.axis_kind, universe.axis_order, universe.fact_kind, universe.document_id, universe.mapped_document_id, universe.start_ordinal, universe.output_label, universe.capture_label, universe.text_value, universe.start_byte, universe.end_byte, CASE WHEN EXISTS (SELECT 1 FROM clause_0_relation_7 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict IS NULL AND 1 = 1) THEN NULL ELSE 1 END AS verdict, universe.complete, universe.work_used, universe.error_code, universe.field_id, universe.related_named, universe.source_text, universe.structural_key, universe.named_leaf, universe.capture_kind, universe.capture_named, universe.equality_occurrence, universe.bindings, universe.choice_order FROM clause_0_relation_3 universe
  WHERE universe.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_relation_7 excluded
    WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict = 1
      AND 1 = 1
  )
),
clause_0_select_when_11 AS (
  SELECT selected.* FROM clause_0_term_0 selected WHERE EXISTS (SELECT 1 FROM clause_0_relation_10 required WHERE required.file_index = selected.file_index)
),
clause_0_selected_12 AS (
  SELECT candidate.* FROM clause_0_select_when_11 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_select_when_11 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_12 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0, 1, 2, 3, 4) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0, 1, 2, 3, 4) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0, 1, 2, 3, 4))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-require-each
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_1 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 1 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_require_each_2 AS (
  SELECT subject.* FROM clause_0_term_0 subject WHERE NOT EXISTS (SELECT 1 FROM clause_0_term_1 evidence WHERE (SELECT json_extract(binding.value, '$.text') FROM json_each(subject.bindings) binding WHERE json_extract(binding.value, '$.label') = 'A') = (SELECT json_extract(binding.value, '$.text') FROM json_each(evidence.bindings) binding WHERE json_extract(binding.value, '$.label') = 'A') AND CAST(subject.file_index AS TEXT) = CAST(evidence.file_index AS TEXT))
),
clause_0_selected_3 AS (
  SELECT candidate.* FROM clause_0_require_each_2 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_require_each_2 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_3 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0, 1) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0, 1) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0, 1))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-budget-maximum
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_budget_1 AS (
  SELECT usage.* FROM clause_0_term_0 usage GROUP BY 'all' HAVING count(*) > 1
),
clause_0_selected_2 AS (
  SELECT candidate.* FROM clause_0_budget_1 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_budget_1 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_2 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-budget-distinct
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_budget_1 AS (
  SELECT usage.* FROM clause_0_term_0 usage GROUP BY 'all' HAVING count(DISTINCT json_array((SELECT json_extract(binding.value, '$.text') FROM json_each(usage.bindings) binding WHERE json_extract(binding.value, '$.label') = 'A'))) > 1
),
clause_0_selected_2 AS (
  SELECT candidate.* FROM clause_0_budget_1 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_budget_1 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_2 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-budget-group
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_budget_1 AS (
  SELECT usage.* FROM clause_0_term_0 usage GROUP BY CAST(usage.file_index AS TEXT) HAVING count(*) > 1
),
clause_0_selected_2 AS (
  SELECT candidate.* FROM clause_0_budget_1 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_budget_1 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_2 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-file-condition
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_1 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 1 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_2 AS (
  SELECT left_input.batch, left_input.row_kind, left_input.producer_id, left_input.file_index, left_input.task_index, left_input.match_index, left_input.capture_index, left_input.capture_occurrence, left_input.capture_sequence_start, left_input.capture_sequence_count, left_input.node, left_input.related_node, left_input.axis_kind, left_input.axis_order, left_input.fact_kind, left_input.document_id, left_input.mapped_document_id, left_input.start_ordinal, left_input.output_label, left_input.capture_label, left_input.text_value, left_input.start_byte, left_input.end_byte, CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END AS verdict, left_input.complete, left_input.work_used, left_input.error_code, left_input.field_id, left_input.related_named, left_input.source_text, left_input.structural_key, left_input.named_leaf, left_input.capture_kind, left_input.capture_named, left_input.equality_occurrence, left_input.bindings, left_input.choice_order FROM clause_0_term_0 left_input
  INNER JOIN clause_0_term_1 right_input
    ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node
),
clause_0_selected_3 AS (
  SELECT candidate.* FROM clause_0_relation_2 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_relation_2 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_3 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0, 1) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0, 1) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0, 1))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;

-- fixture: clause-syntax-error-unknown
WITH packed AS MATERIALIZED (
  SELECT batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence FROM rerail_matches WHERE batch = :batch
),
batch_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 1
),
producer_status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
terminal_match AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3
),
capture AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5
),
fact AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 6
),
document_map AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 7
)
,
clause_0_term_0 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_term_1 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 0 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_2_walk(file_index, source_node, candidate, axis_order, eligible, path, source_bindings, source_batch, source_row_kind, source_producer_id, source_file_index, source_task_index, source_match_index, source_capture_index, source_capture_occurrence, source_capture_sequence_start, source_capture_sequence_count, source_node, source_related_node, source_axis_kind, source_axis_order, source_fact_kind, source_document_id, source_mapped_document_id, source_start_ordinal, source_output_label, source_capture_label, source_text_value, source_start_byte, source_end_byte, source_verdict, source_complete, source_work_used, source_error_code, source_field_id, source_related_named, source_source_text, source_structural_key, source_named_leaf, source_capture_kind, source_capture_named, source_equality_occurrence, source_choice_order) AS (
  SELECT source.file_index, source.node, edge.related_node, 1, 1, printf('/%d', edge.related_node), source.bindings, source.batch, source.row_kind, source.producer_id, source.file_index, source.task_index, source.match_index, source.capture_index, source.capture_occurrence, source.capture_sequence_start, source.capture_sequence_count, source.node, source.related_node, source.axis_kind, source.axis_order, source.fact_kind, source.document_id, source.mapped_document_id, source.start_ordinal, source.output_label, source.capture_label, source.text_value, source.start_byte, source.end_byte, source.verdict, source.complete, source.work_used, source.error_code, source.field_id, source.related_named, source.source_text, source.structural_key, source.named_leaf, source.capture_kind, source.capture_named, source.equality_occurrence, source.choice_order
  FROM clause_0_term_1 source
  JOIN navigation edge ON edge.producer_id = 1 AND edge.file_index = source.file_index AND edge.node = source.node
  UNION ALL
  SELECT walk.file_index, walk.source_node, edge.related_node, walk.axis_order + 1, 1, walk.path || printf('/%d', edge.related_node), walk.source_bindings, walk.source_batch, walk.source_row_kind, walk.source_producer_id, walk.source_file_index, walk.source_task_index, walk.source_match_index, walk.source_capture_index, walk.source_capture_occurrence, walk.source_capture_sequence_start, walk.source_capture_sequence_count, walk.source_node, walk.source_related_node, walk.source_axis_kind, walk.source_axis_order, walk.source_fact_kind, walk.source_document_id, walk.source_mapped_document_id, walk.source_start_ordinal, walk.source_output_label, walk.source_capture_label, walk.source_text_value, walk.source_start_byte, walk.source_end_byte, walk.source_verdict, walk.source_complete, walk.source_work_used, walk.source_error_code, walk.source_field_id, walk.source_related_named, walk.source_source_text, walk.source_structural_key, walk.source_named_leaf, walk.source_capture_kind, walk.source_capture_named, walk.source_equality_occurrence, walk.source_choice_order
  FROM clause_0_relation_2_walk walk
  JOIN navigation edge ON edge.producer_id = 1 AND edge.file_index = walk.file_index AND edge.node = walk.candidate
),
clause_0_relation_2_context AS (
  SELECT walk.source_batch AS batch, walk.source_row_kind AS row_kind, walk.source_producer_id AS producer_id, walk.source_file_index AS file_index, walk.source_task_index AS task_index, walk.source_match_index AS match_index, walk.source_capture_index AS capture_index, walk.source_capture_occurrence AS capture_occurrence, walk.source_capture_sequence_start AS capture_sequence_start, walk.source_capture_sequence_count AS capture_sequence_count, walk.candidate AS node, walk.source_node AS related_node, walk.source_axis_kind AS axis_kind, walk.source_axis_order AS axis_order, walk.source_fact_kind AS fact_kind, walk.source_document_id AS document_id, walk.source_mapped_document_id AS mapped_document_id, walk.source_start_ordinal AS start_ordinal, walk.source_output_label AS output_label, walk.source_capture_label AS capture_label, walk.source_text_value AS text_value, walk.source_start_byte AS start_byte, walk.source_end_byte AS end_byte, walk.source_verdict AS verdict, walk.source_complete AS complete, walk.source_work_used AS work_used, walk.source_error_code AS error_code, walk.source_field_id AS field_id, walk.source_related_named AS related_named, walk.source_source_text AS source_text, walk.source_structural_key AS structural_key, walk.source_named_leaf AS named_leaf, walk.source_capture_kind AS capture_kind, walk.source_capture_named AS capture_named, walk.source_equality_occurrence AS equality_occurrence, walk.source_bindings AS bindings, walk.source_choice_order AS choice_order
  FROM clause_0_relation_2_walk walk
),
clause_0_term_3 AS (
  SELECT match.batch, match.row_kind, match.producer_id, match.file_index, match.task_index, match.match_index, match.capture_index, match.capture_occurrence, match.capture_sequence_start, match.capture_sequence_count, match.node, match.related_node, match.axis_kind, match.axis_order, match.fact_kind, match.document_id, match.mapped_document_id, match.start_ordinal, match.output_label, match.capture_label, match.text_value, match.start_byte, match.end_byte, 1 AS verdict, match.complete, match.work_used, match.error_code, match.field_id, match.related_named, match.source_text, match.structural_key, match.named_leaf, match.capture_kind, match.capture_named, match.equality_occurrence, COALESCE((
    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))
    FROM capture captured
    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index
      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'
    ORDER BY captured.capture_index
  ), json('[]')) AS bindings, '' AS choice_order
  FROM terminal_match match
  INNER JOIN producer_status status USING (producer_id, file_index)
  WHERE match.producer_id = 2 AND status.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM capture first
      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index
        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label
        AND repeated.capture_index > first.capture_index
      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index
        AND first.match_index = match.match_index AND first.capture_label <> 'match'
        AND first.capture_kind = 1 AND repeated.capture_kind = 1
        AND NOT (first.node = repeated.node OR ((first.named_leaf = 1 OR repeated.named_leaf = 1) AND first.source_text = repeated.source_text) OR (first.named_leaf = 0 AND repeated.named_leaf = 0 AND CASE WHEN json_valid(first.structural_key) THEN json_extract(first.structural_key, '$.syntax') ELSE first.structural_key END = CASE WHEN json_valid(repeated.structural_key) THEN json_extract(repeated.structural_key, '$.syntax') ELSE repeated.structural_key END))
    )
),
clause_0_relation_4 AS (
  SELECT left_input.batch, left_input.row_kind, left_input.producer_id, left_input.file_index, left_input.task_index, left_input.match_index, left_input.capture_index, left_input.capture_occurrence, left_input.capture_sequence_start, left_input.capture_sequence_count, left_input.node, left_input.related_node, left_input.axis_kind, left_input.axis_order, left_input.fact_kind, left_input.document_id, left_input.mapped_document_id, left_input.start_ordinal, left_input.output_label, left_input.capture_label, left_input.text_value, left_input.start_byte, left_input.end_byte, CASE WHEN left_input.verdict = 0 OR right_input.verdict = 0 THEN 0 WHEN left_input.verdict IS NULL OR right_input.verdict IS NULL THEN NULL ELSE 1 END AS verdict, left_input.complete, left_input.work_used, left_input.error_code, left_input.field_id, left_input.related_named, left_input.source_text, left_input.structural_key, left_input.named_leaf, left_input.capture_kind, left_input.capture_named, left_input.equality_occurrence, left_input.bindings, left_input.choice_order FROM clause_0_relation_2_context left_input
  INNER JOIN clause_0_term_3 right_input
    ON right_input.file_index = left_input.file_index AND right_input.node = left_input.node
),
clause_0_relation_2_accepted AS (
  SELECT walk.file_index, walk.source_node, related.bindings, related.choice_order, row_number() OVER (PARTITION BY walk.file_index, walk.source_node, walk.source_bindings ORDER BY walk.candidate ASC, walk.candidate, related.choice_order, related.producer_id, related.match_index) AS accepted_order, walk.source_batch, walk.source_row_kind, walk.source_producer_id, walk.source_file_index, walk.source_task_index, walk.source_match_index, walk.source_capture_index, walk.source_capture_occurrence, walk.source_capture_sequence_start, walk.source_capture_sequence_count, walk.source_node, walk.source_related_node, walk.source_axis_kind, walk.source_axis_order, walk.source_fact_kind, walk.source_document_id, walk.source_mapped_document_id, walk.source_start_ordinal, walk.source_output_label, walk.source_capture_label, walk.source_text_value, walk.source_start_byte, walk.source_end_byte, walk.source_verdict, walk.source_complete, walk.source_work_used, walk.source_error_code, walk.source_field_id, walk.source_related_named, walk.source_source_text, walk.source_structural_key, walk.source_named_leaf, walk.source_capture_kind, walk.source_capture_named, walk.source_equality_occurrence, walk.source_choice_order
  FROM clause_0_relation_2_walk walk
  JOIN clause_0_relation_4 related ON related.file_index = walk.file_index AND related.node = walk.candidate AND related.related_node = walk.source_node
  WHERE walk.eligible
),
clause_0_relation_2 AS (
  SELECT accepted.source_batch AS batch, accepted.source_row_kind AS row_kind, accepted.source_producer_id AS producer_id, accepted.source_file_index AS file_index, accepted.source_task_index AS task_index, accepted.source_match_index AS match_index, accepted.source_capture_index AS capture_index, accepted.source_capture_occurrence AS capture_occurrence, accepted.source_capture_sequence_start AS capture_sequence_start, accepted.source_capture_sequence_count AS capture_sequence_count, accepted.source_node AS node, accepted.source_related_node AS related_node, accepted.source_axis_kind AS axis_kind, accepted.source_axis_order AS axis_order, accepted.source_fact_kind AS fact_kind, accepted.source_document_id AS document_id, accepted.source_mapped_document_id AS mapped_document_id, accepted.source_start_ordinal AS start_ordinal, accepted.source_output_label AS output_label, accepted.source_capture_label AS capture_label, accepted.source_text_value AS text_value, accepted.source_start_byte AS start_byte, accepted.source_end_byte AS end_byte, accepted.source_verdict AS verdict, accepted.source_complete AS complete, accepted.source_work_used AS work_used, accepted.source_error_code AS error_code, accepted.source_field_id AS field_id, accepted.source_related_named AS related_named, accepted.source_source_text AS source_text, accepted.source_structural_key AS structural_key, accepted.source_named_leaf AS named_leaf, accepted.source_capture_kind AS capture_kind, accepted.source_capture_named AS capture_named, accepted.source_equality_occurrence AS equality_occurrence, accepted.bindings, accepted.choice_order FROM clause_0_relation_2_accepted accepted WHERE 1 = 1 AND accepted.accepted_order = 1
),
clause_0_relation_5 AS (
  SELECT universe.batch, universe.row_kind, universe.producer_id, universe.file_index, universe.task_index, universe.match_index, universe.capture_index, universe.capture_occurrence, universe.capture_sequence_start, universe.capture_sequence_count, universe.node, universe.related_node, universe.axis_kind, universe.axis_order, universe.fact_kind, universe.document_id, universe.mapped_document_id, universe.start_ordinal, universe.output_label, universe.capture_label, universe.text_value, universe.start_byte, universe.end_byte, CASE WHEN EXISTS (SELECT 1 FROM clause_0_relation_2 excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict IS NULL AND 1 = 1) THEN NULL ELSE 1 END AS verdict, universe.complete, universe.work_used, universe.error_code, universe.field_id, universe.related_named, universe.source_text, universe.structural_key, universe.named_leaf, universe.capture_kind, universe.capture_named, universe.equality_occurrence, universe.bindings, universe.choice_order FROM clause_0_term_0 universe
  WHERE universe.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_relation_2 excluded
    WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict = 1
      AND 1 = 1
  )
),
clause_0_selected_6 AS (
  SELECT candidate.* FROM clause_0_relation_5 candidate
  WHERE candidate.verdict = 1 AND NOT EXISTS (
    SELECT 1 FROM clause_0_relation_5 earlier
    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node
      AND (earlier.choice_order < candidate.choice_order
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)
        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))
  )
),
clause_0 AS (
  SELECT result.batch, result.row_kind, result.producer_id, result.file_index, result.task_index, result.match_index, result.capture_index, result.capture_occurrence, result.capture_sequence_start, result.capture_sequence_count, result.node, result.related_node, result.axis_kind, result.axis_order, result.fact_kind, result.document_id, result.mapped_document_id, result.start_ordinal, result.output_label, result.capture_label, result.text_value, result.start_byte, result.end_byte, result.verdict, result.complete, result.work_used, result.error_code, result.field_id, result.related_named, result.source_text, result.structural_key, result.named_leaf, result.capture_kind, result.capture_named, result.equality_occurrence, result.bindings, result.choice_order, 0 AS clause_ordinal FROM clause_0_selected_6 result
  CROSS JOIN batch_status batch
  WHERE batch.complete = 1
    AND NOT EXISTS (
      SELECT 1 FROM producer_status undecided
      WHERE undecided.producer_id IN (0, 1, 2) AND (undecided.complete <> 1 OR undecided.verdict IS NULL)
    )
  UNION ALL
  SELECT failed.batch, 8 AS row_kind, failed.producer_id, failed.file_index, failed.task_index, failed.match_index, failed.capture_index, failed.capture_occurrence, failed.capture_sequence_start, failed.capture_sequence_count, failed.node, failed.related_node, failed.axis_kind, failed.axis_order, failed.fact_kind, failed.document_id, failed.mapped_document_id, failed.start_ordinal, failed.output_label, failed.capture_label, failed.text_value, failed.start_byte, failed.end_byte, failed.verdict, failed.complete, failed.work_used, failed.error_code, failed.field_id, failed.related_named, failed.source_text, failed.structural_key, failed.named_leaf, failed.capture_kind, failed.capture_named, failed.equality_occurrence, json('[]') AS bindings, '' AS choice_order, 0 AS clause_ordinal
  FROM producer_status failed
  WHERE failed.producer_id IN (0, 1, 2) AND failed.complete <> 1
    AND failed.producer_id = (SELECT min(same.producer_id) FROM producer_status same WHERE same.file_index = failed.file_index AND same.error_code = failed.error_code AND same.complete <> 1 AND same.producer_id IN (0, 1, 2))
)
SELECT * FROM clause_0
ORDER BY clause_ordinal, file_index, node, choice_order, producer_id, match_index, capture_index;
