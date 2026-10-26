WITH general(lang,expression,ready) AS NOT MATERIALIZED (
    SELECT p.lang,p.expression,p.ready FROM vocabulary_predicate p
    WHERE NOT EXISTS(SELECT 1 FROM vocabulary_literal l WHERE l.lang=p.lang AND l.expression=p.expression)
),
needed(lang,expression,term) AS NOT MATERIALIZED (
    SELECT p.lang,p.expression,v.term
    FROM general p CROSS JOIN vlex v
    WHERE p.ready=0 AND v.lang=p.lang
    UNION ALL
    SELECT p.lang,p.expression,v.term
    FROM general p CROSS JOIN vocabulary_pending v
    WHERE p.ready=1 AND p.lang=v.lang
)
INSERT OR IGNORE INTO vocabulary_term
SELECT lang,expression,term FROM needed WHERE nera_term_matches(expression,term,0)
UNION ALL
SELECT l.lang,l.expression,l.term
FROM vocabulary_predicate p CROSS JOIN vocabulary_literal l ON l.lang=p.lang AND l.expression=p.expression
CROSS JOIN vlex v ON v.lang=l.lang AND v.term=l.term
WHERE p.ready=0
UNION ALL
SELECT l.lang,l.expression,l.term
FROM vocabulary_pending v CROSS JOIN vocabulary_literal l ON l.lang=v.lang AND l.term=v.term
CROSS JOIN vocabulary_predicate p ON p.lang=l.lang AND p.expression=l.expression
WHERE p.ready=1
