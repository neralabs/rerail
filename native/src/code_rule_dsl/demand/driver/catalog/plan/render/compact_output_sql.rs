//! Compact SQL output templates aligned to the independently authored Phase 7 contracts.

pub(super) const CAPTURE_EACH_DISTINCT: &str = r#"WITH RECURSIVE packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
roots AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
related_matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 3
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 3 AND capture_label = 'A'
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5 AND producer_id = 2
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
descendants(file_index, root_node, related_node, axis_order) AS (
  SELECT root.file_index, root.node, edge.related_node, printf('%020d', edge.axis_order)
  FROM roots root
  JOIN navigation edge ON edge.file_index = root.file_index AND edge.node = root.node
  UNION ALL
  SELECT walk.file_index, walk.root_node, edge.related_node,
         walk.axis_order || '/' || printf('%020d', edge.axis_order)
  FROM descendants walk
  JOIN navigation edge ON edge.file_index = walk.file_index AND edge.node = walk.related_node
),
selected AS (
  SELECT root.file_index, root.node, root.match_index,
         root.start_byte AS witness_start_byte, root.end_byte AS witness_end_byte,
         c.node AS capture_node, c.source_text, c.start_byte, c.end_byte,
         row_number() OVER (
           PARTITION BY root.file_index, root.node, c.node, c.start_byte, c.end_byte
           ORDER BY walk.axis_order, related.match_index, c.capture_index
         ) AS duplicate_order
  FROM roots root
  JOIN descendants walk ON walk.file_index = root.file_index AND walk.root_node = root.node
  JOIN related_matches related ON related.file_index = walk.file_index AND related.node = walk.related_node
  JOIN captures c ON c.producer_id = related.producer_id
    AND c.file_index = related.file_index AND c.match_index = related.match_index
  WHERE NOT EXISTS (
    SELECT 1 FROM status incomplete
    WHERE incomplete.file_index = root.file_index
      AND incomplete.producer_id IN (0, 1, 2, 3)
      AND incomplete.complete <> 1
  )
),
violations AS (
  SELECT 1 AS output_kind,
         'sql-contract-capture-each-distinct' AS rule_id,
         'contract' AS clause_id,
         'SQL' AS code,
         CASE WHEN instr(source_text, char(10)) > 0 OR length(source_text) > 80
              THEN rtrim(substr(CASE WHEN instr(source_text, char(10)) > 0 THEN substr(source_text, 1, instr(source_text, char(10)) - 1) ELSE source_text END, 1, 79)) || '…'
              ELSE source_text END AS message,
         files.path,
         start_byte,
         end_byte,
         'block' AS mode,
         printf('%08d/%08d/%020d/%020d/%020d', 0, selected.file_index, selected.node, start_byte, end_byte) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) AS captures,
         json('{}') AS variables,
         NULL AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte,
         start_byte AS each_start_byte, end_byte AS each_end_byte,
         NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
  WHERE duplicate_order = 1
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-each-distinct' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0, 1, 2, 3)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0, 1, 2, 3)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const CAPTURE_ANCHOR: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'A'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.match_index, m.start_byte AS witness_start_byte,
         m.end_byte AS witness_end_byte, c.source_text, c.start_byte, c.end_byte
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  JOIN captures c USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-capture-anchor' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code,
         CASE WHEN instr(source_text, char(10)) > 0 OR length(source_text) > 80
              THEN rtrim(substr(CASE WHEN instr(source_text, char(10)) > 0 THEN substr(source_text, 1, instr(source_text, char(10)) - 1) ELSE source_text END, 1, 79)) || '…'
              ELSE source_text END AS message,
         files.path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d', 0, selected.file_index, selected.node, selected.match_index) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) AS captures,
         json('{}') AS variables, NULL AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-anchor' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const CAPTURE_ANCHOR_FALLBACK: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id IN (0, 1)
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND capture_label = 'A'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.producer_id, m.match_index,
         m.start_byte AS witness_start_byte, m.end_byte AS witness_end_byte,
         c.source_text,
         COALESCE(c.start_byte, m.start_byte) AS start_byte,
         COALESCE(c.end_byte, m.end_byte) AS end_byte,
         c.start_byte IS NULL AS anchor_fallback,
         row_number() OVER (PARTITION BY m.file_index, m.node ORDER BY m.producer_id, m.match_index) AS choice_order
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  LEFT JOIN captures c USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-capture-anchor-fallback' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code, 'optional capture' AS message,
         files.path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d/%020d', 0, selected.file_index, selected.node, selected.producer_id, selected.match_index) AS stable_order,
         CASE WHEN source_text IS NULL THEN json('{}') ELSE json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) END AS captures,
         CASE WHEN anchor_fallback THEN json_object('anchorFallback', json('true')) ELSE json('{}') END AS variables,
         NULL AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
  WHERE choice_order = 1
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-anchor-fallback' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0, 1)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0, 1)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const CAPTURE_TRANSFORM: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'A'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.match_index, m.start_byte AS witness_start_byte,
         m.end_byte AS witness_end_byte, c.source_text, c.start_byte, c.end_byte
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  JOIN captures c USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-capture-transform' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code,
         CASE WHEN instr(lower(source_text), char(10)) > 0 OR length(lower(source_text)) > 80
              THEN rtrim(substr(CASE WHEN instr(lower(source_text), char(10)) > 0 THEN substr(lower(source_text), 1, instr(lower(source_text), char(10)) - 1) ELSE lower(source_text) END, 1, 79)) || '…'
              ELSE lower(source_text) END AS message,
         files.path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d', 0, selected.file_index, selected.node, selected.match_index) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) AS captures,
         json('{}') AS variables, NULL AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-transform' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const CAPTURE_MESSAGE_TEMPLATE: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'A'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.match_index, m.start_byte AS witness_start_byte,
         m.end_byte AS witness_end_byte, c.source_text,
         c.start_byte AS capture_start_byte, c.end_byte AS capture_end_byte
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  JOIN captures c USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-capture-message-template' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code,
         'found ' || CASE WHEN instr(source_text, char(10)) > 0 OR length(source_text) > 80
                          THEN rtrim(substr(CASE WHEN instr(source_text, char(10)) > 0 THEN substr(source_text, 1, instr(source_text, char(10)) - 1) ELSE source_text END, 1, 79)) || '…'
                          ELSE source_text END AS message,
         files.path, witness_start_byte AS start_byte, witness_end_byte AS end_byte,
         'block' AS mode,
         printf('%08d/%08d/%020d/%020d', 0, selected.file_index, selected.node, selected.match_index) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', capture_end_byte,
           'endColumn', capture_end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= capture_end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= capture_end_byte), 0),
           'startByte', capture_start_byte,
           'startColumn', capture_start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= capture_start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= capture_start_byte), 0)
         ))) AS captures,
         json('{}') AS variables, NULL AS fix,
         json_object(
           'endByte', witness_end_byte,
           'endColumn', witness_end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_end_byte), 0),
           'startByte', witness_start_byte,
           'startColumn', witness_start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-message-template' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const CAPTURE_FIX_RANGE: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
capture_a AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'A'
),
capture_b AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'B'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.match_index, m.start_byte AS witness_start_byte,
         m.end_byte AS witness_end_byte, a.source_text AS a_text,
         a.start_byte, a.end_byte, b.start_byte AS b_start_byte,
         b.end_byte AS b_end_byte, b.source_text AS replacement
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  JOIN capture_a a USING (producer_id, file_index, match_index)
  JOIN capture_b b USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-capture-fix-range' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code, 'replace ' || a_text AS message,
         files.path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d', 0, selected.file_index, selected.node, selected.match_index) AS stable_order,
         json_object(
           'A', json_array(json_object(
             'endByte', end_byte,
             'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
             'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
             'startByte', start_byte,
             'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
             'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
           )),
           'B', json_array(json_object(
             'endByte', b_end_byte,
             'endColumn', b_end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= b_end_byte), 0),
             'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= b_end_byte), 0),
             'startByte', b_start_byte,
             'startColumn', b_start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= b_start_byte), 0),
             'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= b_start_byte), 0)
           ))
         ) AS captures,
         json('{}') AS variables,
         json_object('span', json(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         )), 'text', replacement) AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         selected.file_index, selected.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM selected JOIN files USING (file_index)
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-capture-fix-range' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const OUTPUT_FIX_AND_SUPPRESSION: &str = r#"WITH packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 0 AND capture_label = 'A'
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.source') AS source,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
selected AS (
  SELECT m.file_index, m.node, m.match_index, m.start_byte AS witness_start_byte,
         m.end_byte AS witness_end_byte, c.source_text, c.start_byte, c.end_byte
  FROM matches m
  JOIN status s USING (producer_id, file_index)
  JOIN captures c USING (producer_id, file_index, match_index)
  WHERE s.complete = 1 AND s.verdict = 1
),
anchored AS (
  SELECT selected.*, files.path, files.source, files.line_starts,
         rtrim(lower(CAST(substr(
           CAST(files.source AS BLOB),
           (SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte) + 1,
           COALESCE((SELECT min(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) > start_byte), length(CAST(files.source AS BLOB))) -
           (SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte)
         ) AS TEXT)), char(10) || char(13)) AS source_line
  FROM selected JOIN files USING (file_index)
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-output-fix-and-suppression' AS rule_id,
         'contract' AS clause_id, 'F401' AS code, source_text AS message,
         path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d', 0, file_index, node, match_index) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) AS captures,
         json('{}') AS variables,
         json_object('span', json(json_object(
           'endByte', witness_end_byte,
           'endColumn', witness_end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_end_byte), 0),
           'startByte', witness_start_byte,
           'startColumn', witness_start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= witness_start_byte), 0)
         )), 'text', 'log(' || source_text || ')') AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         file_index, node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte, NULL AS each_start_byte,
         NULL AS each_end_byte, NULL AS error_code, 0 AS clause_ordinal
  FROM anchored
  WHERE NOT (
    instr(source_line, '# noqa') > 0
    AND substr(substr(source_line, instr(source_line, '# noqa') + 6), 1, 1) IN ('', ':', ' ', char(9))
    AND (
      instr(substr(source_line, instr(source_line, '# noqa') + 6), ':') = 0
      OR instr(
        ',' || replace(replace(replace(replace(
          substr(substr(source_line, instr(source_line, '# noqa') + 6), instr(substr(source_line, instr(source_line, '# noqa') + 6), ':') + 1),
          ' ', ''), char(9), ''), char(13), ''), char(10), '') || ',',
        ',f401,'
      ) > 0
    )
  )
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-output-fix-and-suppression' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

pub(super) const OUTPUT_STABLE_MULTIPLICITY: &str = r#"WITH RECURSIVE packed AS MATERIALIZED (
  SELECT * FROM rerail_matches WHERE batch = :batch
),
status AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 2
),
root_matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 0
),
related_matches AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 3 AND producer_id = 3
),
captures AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 4 AND producer_id = 3 AND capture_label = 'A'
),
navigation AS MATERIALIZED (
  SELECT * FROM packed WHERE row_kind = 5 AND producer_id = 2
),
files AS MATERIALIZED (
  SELECT CAST(key AS INTEGER) AS file_index, json_extract(value, '$.path') AS path,
         json_extract(value, '$.lineStarts') AS line_starts
  FROM json_each(:files)
),
descendants(file_index, root_node, related_node, axis_order) AS (
  SELECT root.file_index, root.node, edge.related_node, printf('%020d', edge.axis_order)
  FROM root_matches root
  JOIN navigation edge ON edge.file_index = root.file_index AND edge.node = root.node
  UNION ALL
  SELECT walk.file_index, walk.root_node, edge.related_node,
         walk.axis_order || '/' || printf('%020d', edge.axis_order)
  FROM descendants walk
  JOIN navigation edge ON edge.file_index = walk.file_index AND edge.node = walk.related_node
),
candidates AS (
  SELECT root.file_index, root.node, root.match_index AS root_match_index,
         root.start_byte AS witness_start_byte, root.end_byte AS witness_end_byte,
         related.producer_id, related.match_index, captured.node AS capture_node,
         captured.source_text, captured.start_byte, captured.end_byte,
         row_number() OVER (
           PARTITION BY root.file_index, root.node, captured.node, captured.start_byte, captured.end_byte
           ORDER BY walk.axis_order, related.match_index, captured.capture_index
         ) AS duplicate_order
  FROM root_matches root
  JOIN descendants walk ON walk.file_index = root.file_index AND walk.root_node = root.node
  JOIN related_matches related ON related.file_index = walk.file_index AND related.node = walk.related_node
  JOIN captures captured ON captured.producer_id = related.producer_id
    AND captured.file_index = related.file_index AND captured.match_index = related.match_index
  WHERE NOT EXISTS (
    SELECT 1 FROM status incomplete
    WHERE incomplete.file_index = root.file_index
      AND incomplete.producer_id IN (0, 1, 2, 3)
      AND incomplete.complete <> 1
  )
),
violations AS (
  SELECT 1 AS output_kind, 'sql-contract-output-stable-multiplicity' AS rule_id,
         'contract' AS clause_id, 'SQL' AS code,
         CASE WHEN instr(source_text, char(10)) > 0 OR length(source_text) > 80
              THEN rtrim(substr(CASE WHEN instr(source_text, char(10)) > 0 THEN substr(source_text, 1, instr(source_text, char(10)) - 1) ELSE source_text END, 1, 79)) || '…'
              ELSE source_text END AS message,
         files.path, start_byte, end_byte, 'block' AS mode,
         printf('%08d/%08d/%020d/%020d/%020d/%020d', 0, candidates.file_index, candidates.node, start_byte, end_byte, producer_id) AS stable_order,
         json_object('A', json_array(json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ))) AS captures,
         json('{}') AS variables, NULL AS fix,
         json_object(
           'endByte', end_byte,
           'endColumn', end_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'endLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= end_byte), 0),
           'startByte', start_byte,
           'startColumn', start_byte - COALESCE((SELECT max(CAST(point.value AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0),
           'startLine', COALESCE((SELECT max(CAST(point.key AS INTEGER)) FROM json_each(files.line_starts) point WHERE CAST(point.value AS INTEGER) <= start_byte), 0)
         ) AS span,
         candidates.file_index, candidates.node, json('[]') AS bindings,
         witness_start_byte, witness_end_byte,
         start_byte AS each_start_byte, end_byte AS each_end_byte,
         NULL AS error_code, 0 AS clause_ordinal
  FROM candidates JOIN files USING (file_index)
  WHERE duplicate_order = 1
),
failures AS (
  SELECT
    2 AS output_kind,
    'sql-contract-output-stable-multiplicity' AS rule_id,
    'contract' AS clause_id,
    NULL AS code,
    NULL AS message,
    files.path,
    NULL AS start_byte,
    NULL AS end_byte,
    'block' AS mode,
    printf('%08d/%08d/%020d', 0, status.file_index, status.producer_id) AS stable_order,
    json('{}') AS captures,
    json('{}') AS variables,
    NULL AS fix,
    NULL AS span,
    status.file_index,
    NULL AS node,
    json('[]') AS bindings,
    NULL AS witness_start_byte,
    NULL AS witness_end_byte,
    NULL AS each_start_byte,
    NULL AS each_end_byte,
    status.error_code,
    0 AS clause_ordinal
  FROM status
  JOIN files USING (file_index)
  WHERE status.producer_id IN (0, 1, 2, 3)
    AND status.complete <> 1
    AND status.error_code IS NOT NULL
    AND status.producer_id = (
      SELECT min(same.producer_id)
      FROM status same
      WHERE same.file_index = status.file_index
        AND same.complete <> 1
        AND same.error_code = status.error_code
        AND same.producer_id IN (0, 1, 2, 3)
    )
)
SELECT * FROM violations
UNION ALL
SELECT * FROM failures
ORDER BY clause_ordinal, file_index, stable_order;"#;

#[derive(Clone, Copy)]
pub(super) enum Shape {
    CaptureEachDistinct,
    CaptureAnchor,
    CaptureAnchorFallback,
    CaptureTransform,
    CaptureMessageTemplate,
    CaptureFixRange,
    OutputFixAndSuppression,
    OutputStableMultiplicity,
}

pub(super) fn render(
    shape: Shape,
    rule_id: &str,
    clause_id: &str,
    code: &str,
    mode: &str,
) -> String {
    let (template, sample_rule_id, sample_code) = match shape {
        Shape::CaptureEachDistinct => (
            CAPTURE_EACH_DISTINCT,
            "sql-contract-capture-each-distinct",
            "SQL",
        ),
        Shape::CaptureAnchor => (CAPTURE_ANCHOR, "sql-contract-capture-anchor", "SQL"),
        Shape::CaptureAnchorFallback => (
            CAPTURE_ANCHOR_FALLBACK,
            "sql-contract-capture-anchor-fallback",
            "SQL",
        ),
        Shape::CaptureTransform => (CAPTURE_TRANSFORM, "sql-contract-capture-transform", "SQL"),
        Shape::CaptureMessageTemplate => (
            CAPTURE_MESSAGE_TEMPLATE,
            "sql-contract-capture-message-template",
            "SQL",
        ),
        Shape::CaptureFixRange => (CAPTURE_FIX_RANGE, "sql-contract-capture-fix-range", "SQL"),
        Shape::OutputFixAndSuppression => (
            OUTPUT_FIX_AND_SUPPRESSION,
            "sql-contract-output-fix-and-suppression",
            "F401",
        ),
        Shape::OutputStableMultiplicity => (
            OUTPUT_STABLE_MULTIPLICITY,
            "sql-contract-output-stable-multiplicity",
            "SQL",
        ),
    };
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    template
        .replace(&quote(sample_rule_id), &quote(rule_id))
        .replace(
            "'contract' AS clause_id",
            &format!("{} AS clause_id", quote(clause_id)),
        )
        .replace(
            &format!("{} AS code", quote(sample_code)),
            &format!("{} AS code", quote(code)),
        )
        .replace("'block' AS mode", &format!("{} AS mode", quote(mode)))
        .trim_end_matches(';')
        .to_owned()
}
