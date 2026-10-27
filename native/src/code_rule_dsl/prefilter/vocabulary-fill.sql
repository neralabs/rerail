WITH general(lang,expression,ready) AS NOT MATERIALIZED (
    SELECT p.lang,p.expression,p.ready FROM vocabulary_predicate p
    WHERE NOT EXISTS(SELECT 1 FROM vocabulary_literal l WHERE l.lang=p.lang AND l.expression=p.expression)
),
-- A new anchored predicate's terms lie in its prefix ranges (`nera_prefix_ranges`). A BLOB sorts
-- above every TEXT, so x'' bounds an unbounded range.
ranged(lang,expression,lo,hi) AS MATERIALIZED (
    SELECT g.lang,g.expression,json_extract(r.value,'$[0]'),coalesce(json_extract(r.value,'$[1]'),x'')
    FROM general g CROSS JOIN json_each(nera_prefix_ranges(g.expression)) r WHERE g.ready=0
),
sets(lang,ready,expressions) AS MATERIALIZED (
    SELECT lang,ready,json_group_array(expression) FROM general
    WHERE ready=1 OR nera_prefix_ranges(expression) IS NULL
    GROUP BY lang,ready
)
INSERT OR IGNORE INTO vocabulary_term
SELECT s.lang,j.value,v.term
FROM sets s CROSS JOIN term_freq v ON v.lang=s.lang
CROSS JOIN json_each(nera_terms_matching(s.expressions,v.term)) j
WHERE s.ready=0
UNION ALL
SELECT r.lang,r.expression,v.term
FROM ranged r CROSS JOIN term_freq v ON v.lang=r.lang AND v.term>=r.lo AND v.term<r.hi
WHERE nera_term_matches(r.expression,v.term,0)
UNION ALL
SELECT s.lang,j.value,v.term
FROM sets s CROSS JOIN vocabulary_pending v ON v.lang=s.lang
CROSS JOIN json_each(nera_terms_matching(s.expressions,v.term)) j
WHERE s.ready=1
UNION ALL
SELECT l.lang,l.expression,l.term
FROM vocabulary_predicate p CROSS JOIN vocabulary_literal l ON l.lang=p.lang AND l.expression=p.expression
CROSS JOIN term_freq v ON v.lang=l.lang AND v.term=l.term
WHERE p.ready=0
UNION ALL
SELECT l.lang,l.expression,l.term
FROM vocabulary_pending v CROSS JOIN vocabulary_literal l ON l.lang=v.lang AND l.term=v.term
CROSS JOIN vocabulary_predicate p ON p.lang=l.lang AND p.expression=l.expression
WHERE p.ready=1
