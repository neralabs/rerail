WITH needed(lang,expression,term) AS NOT MATERIALIZED (
    SELECT p.lang,p.expression,v.term
    FROM vocabulary_predicate p CROSS JOIN vlex v
    WHERE p.ready=0 AND v.lang=p.lang
    UNION ALL
    SELECT p.lang,p.expression,v.term
    FROM vocabulary_predicate p CROSS JOIN vocabulary_pending v
    WHERE p.ready=1 AND p.lang=v.lang
)
INSERT OR IGNORE INTO vocabulary_term
SELECT lang,expression,term FROM needed WHERE nera_term_matches(expression,term,0)
