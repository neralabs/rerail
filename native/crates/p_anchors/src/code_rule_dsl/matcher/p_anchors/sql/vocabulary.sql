CREATE TABLE IF NOT EXISTS vocabulary_predicate(
    lang INTEGER NOT NULL, expression TEXT NOT NULL, ready INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(lang,expression)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS vocabulary_term(
    lang INTEGER NOT NULL, expression TEXT NOT NULL, term TEXT NOT NULL,
    PRIMARY KEY(lang,expression,term),
    FOREIGN KEY(lang,expression) REFERENCES vocabulary_predicate(lang,expression) ON DELETE CASCADE,
    FOREIGN KEY(lang,term) REFERENCES vlex(lang,term) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS vocabulary_term_deleted ON vocabulary_term(lang,term);
-- An exact literal set (`^(?:a|b)$`) matches exactly its literals: a join, not a regex per term.
CREATE TABLE IF NOT EXISTS vocabulary_literal(
    lang INTEGER NOT NULL, expression TEXT NOT NULL, term TEXT NOT NULL,
    PRIMARY KEY(lang,term,expression),
    FOREIGN KEY(lang,expression) REFERENCES vocabulary_predicate(lang,expression) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS vocabulary_literal_expression ON vocabulary_literal(lang,expression);
CREATE TABLE IF NOT EXISTS vocabulary_pending(
    lang INTEGER NOT NULL, term TEXT NOT NULL, PRIMARY KEY(lang,term),
    FOREIGN KEY(lang,term) REFERENCES vlex(lang,term) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS vocabulary_added AFTER INSERT ON vlex BEGIN
    INSERT OR IGNORE INTO vocabulary_pending VALUES(NEW.lang,NEW.term);
END;
