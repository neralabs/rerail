WITH general(lang,expression,ready) AS NOT MATERIALIZED (
    SELECT p.lang,p.expression,p.ready FROM vocabulary_predicate p
    WHERE NOT EXISTS(SELECT 1 FROM vocabulary_literal l WHERE l.lang=p.lang AND l.expression=p.expression)
),
-- Terms are atoms: their text is `atoms.value`. A new anchored predicate's terms lie in its prefix
-- ranges (`nera_prefix_ranges`), sought in `atoms_by_value`. A BLOB sorts above every TEXT, so x''
-- bounds an unbounded range.
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
FROM sets s CROSS JOIN term_freq v ON v.lang=s.lang CROSS JOIN atoms a ON a.atom=v.term
CROSS JOIN json_each(nera_terms_matching(s.expressions,a.value)) j
WHERE s.ready=0
UNION ALL
SELECT r.lang,r.expression,v.term
FROM ranged r CROSS JOIN atoms a ON a.value>=r.lo AND a.value<r.hi
CROSS JOIN term_freq v ON v.lang=r.lang AND v.term=a.atom
WHERE nera_term_matches(r.expression,a.value,0)
UNION ALL
SELECT s.lang,j.value,v.term
FROM sets s CROSS JOIN vocabulary_pending v ON v.lang=s.lang CROSS JOIN atoms a ON a.atom=v.term
CROSS JOIN json_each(nera_terms_matching(s.expressions,a.value)) j
WHERE s.ready=1
UNION ALL
SELECT l.lang,l.expression,v.term
FROM vocabulary_predicate p CROSS JOIN vocabulary_literal l ON l.lang=p.lang AND l.expression=p.expression
CROSS JOIN atoms a ON a.value=l.term CROSS JOIN term_freq v ON v.lang=l.lang AND v.term=a.atom
WHERE p.ready=0
UNION ALL
SELECT l.lang,l.expression,v.term
FROM vocabulary_pending v CROSS JOIN atoms a ON a.atom=v.term
CROSS JOIN vocabulary_literal l ON l.lang=v.lang AND l.term=a.value
CROSS JOIN vocabulary_predicate p ON p.lang=l.lang AND p.expression=l.expression
WHERE p.ready=1
