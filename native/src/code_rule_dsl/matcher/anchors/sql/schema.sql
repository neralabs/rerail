-- ast-index-spike/bench/tape_store.py: contentless, positional FTS5 kind index.
-- One bare token per preorder node. An instance offset is the node's preorder.
CREATE VIRTUAL TABLE ftsk USING fts5(k, content='', contentless_delete=1, detail=full);
CREATE VIRTUAL TABLE ftsk_inst USING fts5vocab('ftsk', 'instance');
CREATE TABLE lang(lang_id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE kind(lang INTEGER NOT NULL, code INTEGER NOT NULL, name TEXT, named INTEGER NOT NULL,
    PRIMARY KEY(lang,code)) WITHOUT ROWID;
CREATE TABLE field(lang INTEGER NOT NULL, fcode INTEGER NOT NULL, name TEXT,
    PRIMARY KEY(lang,fcode)) WITHOUT ROWID;
CREATE TABLE file (
    fid INTEGER PRIMARY KEY, nodes INTEGER NOT NULL, path TEXT NOT NULL UNIQUE,
    digest BLOB NOT NULL, lang INTEGER NOT NULL, srcenc TEXT NOT NULL DEFAULT 'UTF8', src BLOB NOT NULL,
    rec BLOB NOT NULL, spans BLOB NOT NULL, checkpoints BLOB NOT NULL,
    parb BLOB NOT NULL, sizb BLOB NOT NULL, flags BLOB NOT NULL
);
CREATE TABLE fk(fid INTEGER PRIMARY KEY REFERENCES file(fid) ON DELETE CASCADE,
    lang INTEGER NOT NULL, kmask BLOB NOT NULL CHECK(length(kmask)=128));
CREATE TABLE file_kind_counts(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,
    code INTEGER NOT NULL, n INTEGER NOT NULL, postings BLOB NOT NULL,
    PRIMARY KEY(fid,code)) WITHOUT ROWID;
CREATE TABLE kfreq(lang INTEGER NOT NULL, code INTEGER NOT NULL, n INTEGER NOT NULL,
    PRIMARY KEY(lang,code)) WITHOUT ROWID;
CREATE TABLE parovf(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,
    pre INTEGER NOT NULL, val INTEGER NOT NULL, PRIMARY KEY(fid,pre)) WITHOUT ROWID;
CREATE TABLE sizeovf(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,
    pre INTEGER NOT NULL, val INTEGER NOT NULL, PRIMARY KEY(fid,pre)) WITHOUT ROWID;
-- One delta-varint posting segment per term, file, and kind. Full term keys are case-exact.
CREATE TABLE vidx(lang INTEGER NOT NULL,term TEXT NOT NULL,fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,
    kind INTEGER NOT NULL,postings BLOB NOT NULL,PRIMARY KEY(lang,term,fid,kind)) WITHOUT ROWID;
CREATE INDEX vidx_file ON vidx(fid);
CREATE TABLE vlex(lang INTEGER NOT NULL,term TEXT NOT NULL,n INTEGER NOT NULL,PRIMARY KEY(lang,term)) WITHOUT ROWID;
CREATE TABLE vcov(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,kind INTEGER NOT NULL,
    bad INTEGER NOT NULL,residue INTEGER NOT NULL,missing BLOB NOT NULL,PRIMARY KEY(fid,kind)) WITHOUT ROWID;
-- Sparse, shared decoded properties. Publication keeps the packed tape; demand fills these.
CREATE TABLE decoded_properties(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,pre INTEGER NOT NULL,
    parent INTEGER NOT NULL,end INTEGER NOT NULL,kind INTEGER NOT NULL,field INTEGER NOT NULL,flags INTEGER NOT NULL,
    PRIMARY KEY(fid,pre)) WITHOUT ROWID;
CREATE TABLE decoded_files(fid INTEGER PRIMARY KEY REFERENCES file(fid) ON DELETE CASCADE);
CREATE TABLE decoded_spans(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,pre INTEGER NOT NULL,
    start INTEGER NOT NULL,len INTEGER NOT NULL,PRIMARY KEY(fid,pre)) WITHOUT ROWID;
CREATE TABLE decoded_children(fid INTEGER NOT NULL REFERENCES file(fid) ON DELETE CASCADE,pre INTEGER NOT NULL,
    field INTEGER NOT NULL,child INTEGER,PRIMARY KEY(fid,pre,field)) WITHOUT ROWID;
CREATE TRIGGER value_insert AFTER INSERT ON vidx BEGIN
    INSERT INTO vlex VALUES(NEW.lang,NEW.term,1) ON CONFLICT(lang,term) DO UPDATE SET n=n+1;
END;
CREATE TRIGGER value_delete AFTER DELETE ON vidx BEGIN
    UPDATE vlex SET n=n-1 WHERE lang=OLD.lang AND term=OLD.term;
    DELETE FROM vlex WHERE lang=OLD.lang AND term=OLD.term AND n=0;
END;
CREATE TRIGGER file_delete BEFORE DELETE ON file BEGIN
    DELETE FROM ftsk WHERE rowid=OLD.fid;
END;
