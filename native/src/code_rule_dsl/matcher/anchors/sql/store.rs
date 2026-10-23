//! The SQLite layout from ast-index-spike's tape_store.py: rec6 file rows and positional FTS.
use crate::code_rule_dsl::{budget, language, tape::TapeDoc};
use ast_grep_core::Doc;
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{Connection, Result, params};
use std::path::Path;

pub(super) const LANG_STRIDE: u32 = 1024;

pub(super) fn kind_token(code: u32) -> String {
    let mut digits = [0_u8; 7];
    let mut at = digits.len();
    let mut code = code;
    loop {
        at -= 1;
        digits[at] = b"0123456789abcdefghijklmnopqrstuvwxyz"[(code % 36) as usize];
        code /= 36;
        if code == 0 {
            break;
        }
    }
    format!(
        "k{}",
        std::str::from_utf8(&digits[at..]).expect("base36 ASCII")
    )
}

pub(super) struct Store {
    #[cfg(feature = "code-rule-profile")]
    pub(super) graph: parking_lot::Mutex<super::graph::Graph>,
    // Close incremental read cursors before the database destroys its SQL callbacks.
    pub(super) readers: super::reader::Readers,
    pub db: crate::code_rule_dsl::relational::database::Database,
    values: parking_lot::Mutex<super::values::Counts>,
    // Connection closes before the temporary database is unlinked, including on Windows.
    _temporary: Option<tempfile::NamedTempFile>,
}

impl Store {
    pub fn temporary() -> Result<Self> {
        let file = tempfile::Builder::new()
            .prefix("rerail-ast-index-")
            .suffix(".sqlite")
            .tempfile()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let mut store = Self::open(file.path())?;
        store._temporary = Some(file);
        Ok(store)
    }
    pub fn open(path: &Path) -> Result<Self> {
        let db = crate::code_rule_dsl::relational::database::Database::from_connection(
            Connection::open(path)?,
        )?;
        #[cfg(feature = "code-rule-profile")]
        db.context(serde_json::json!({"component": "ast-index", "stage": "schema"}));
        super::session::configure(&db)?;
        let version = db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;
        if version == 0 {
            let tx = db.unchecked_transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.pragma_update(None, "user_version", 2_i64)?;
            tx.commit()?;
        } else if version == 1 {
            super::kind_postings::upgrade(&db)?;
        }
        db.execute_batch(include_str!("vocabulary.sql"))?;
        let readers = super::functions::install(&db)?;
        let values = parking_lot::Mutex::new(super::values::Counts::load(&db)?);
        db.progress_handler(
            16_384,
            Some(|| {
                #[cfg(feature = "code-rule-profile")]
                if crate::code_rule_dsl::profile::sql::explaining() {
                    return false;
                }
                budget::matcher_budget_exhausted()
            }),
        )?;
        Ok(Self {
            #[cfg(feature = "code-rule-profile")]
            graph: parking_lot::Mutex::default(),
            readers,
            db,
            values,
            _temporary: None,
        })
    }
    #[cfg(test)]
    pub fn put(&self, path: &str, doc: &TapeDoc) -> Result<i64> {
        self.put_with(path, doc, &budget::matcher_budget_exhausted)
    }
    #[cfg(test)]
    fn put_with(&self, path: &str, doc: &TapeDoc, interrupted: &impl Fn() -> bool) -> Result<i64> {
        let lang = u32::from(language::tape_tag(*doc.get_lang()).expect("tape language"));
        if let Some(fid) = self
            .db
            .query_row(
                "SELECT fid FROM file WHERE path=?1 AND lang=?2 AND digest=?3",
                params![path, lang, doc.index_source_digest().as_slice()],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(fid);
        }
        let previous = self
            .db
            .query_row("SELECT fid FROM file WHERE path=?1", [path], |r| r.get(0))
            .optional()?;
        let known_language = self
            .db
            .query_row("SELECT 1 FROM lang WHERE lang_id=?1", [lang], |_| Ok(()))
            .optional()?
            .is_some();
        self.put_known(path, doc, previous, known_language, interrupted)
    }

    // Corpus metadata supplies identities; publication performs no per-file SELECT queries.
    pub fn put_known(
        &self,
        path: &str,
        doc: &TapeDoc,
        previous: Option<i64>,
        known_language: bool,
        interrupted: &impl Fn() -> bool,
    ) -> Result<i64> {
        #[cfg(feature = "code-rule-profile")]
        let _timer = crate::code_rule_dsl::profile::PhaseTimer::start(
            crate::code_rule_dsl::profile::Phase::Index,
        );
        self.readers.clear();
        let image = doc
            .index_image(interrupted)
            .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?;
        let lang =
            u32::from(language::tape_tag(*doc.get_lang()).expect("registered tape language"));
        let mut values = self.values.lock();
        let value_image = super::values::Image::build(doc, lang, previous, &values, interrupted)?;
        let mut tokens = String::with_capacity(doc.transducer_len() * 6);
        let mut counts = [0_i64; 1024];
        let mut positions = super::kind_postings::Builder::default();
        let mut mask = [0_u8; 128];
        for (pre, rec) in image.rec.chunks_exact(6).enumerate() {
            if pre.is_multiple_of(256) && interrupted() {
                return Err(rusqlite::Error::UserFunctionError(
                    "index construction interrupted".into(),
                ));
            }
            let kind = u16::from_le_bytes([rec[0], rec[1]]) & 1023;
            positions.push(kind, pre as u32);
            counts[usize::from(kind)] += 1;
            mask[usize::from(kind) / 8] |= 1 << (kind % 8);
            if pre != 0 {
                tokens.push(' ');
            }
            tokens.push_str(&kind_token(lang * LANG_STRIDE + u32::from(kind)));
        }
        #[cfg(feature = "code-rule-profile")]
        self.db.stage("ast-index-put");
        let tx = self.db.unchecked_transaction()?;
        if !known_language {
            tx.execute(
                "INSERT INTO lang VALUES(?1,?2)",
                params![lang, language::label(*doc.get_lang()).unwrap_or("test")],
            )?;
            let catalog = doc.get_lang().kinds();
            let mut kinds = tx.prepare_cached("INSERT INTO kind VALUES(?1,?2,?3,?4)")?;
            for code in 0..catalog.kind_count() as u16 {
                kinds.execute(params![
                    lang,
                    code,
                    catalog.kind_name(code),
                    catalog.kind_is_named(code)
                ])?;
            }
            let mut fields = tx.prepare_cached("INSERT INTO field VALUES(?1,?2,?3)")?;
            for code in 1..=catalog.field_count() as u16 {
                fields.execute(params![lang, code, catalog.field_name(code)])?;
            }
        }
        if let Some(fid) = previous {
            remove(&tx, fid)?;
        }
        tx.execute(
            "INSERT INTO file(nodes,path,lang,src,rec,spans,checkpoints,parb,sizb,flags,digest)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                doc.transducer_len() as i64,
                path,
                lang,
                doc.get_source().as_bytes(),
                image.rec,
                image.spans,
                image.checkpoints,
                image.parb,
                image.sizb,
                image.flags,
                doc.index_source_digest().as_slice()
            ],
        )?;
        let fid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO ftsk(rowid,k) VALUES(?1,?2)",
            params![fid, tokens],
        )?;
        tx.execute(
            "INSERT INTO fk VALUES(?1,?2,?3)",
            params![fid, lang, mask.as_slice()],
        )?;
        let mut freq = tx.prepare_cached(
            "INSERT INTO kfreq VALUES(?1,?2,?3)
            ON CONFLICT(lang,code) DO UPDATE SET n=n+excluded.n",
        )?;
        for (code, &n) in counts.iter().enumerate().filter(|(_, n)| **n != 0) {
            freq.execute(params![lang, code as i64, n])?;
            tx.execute(
                "INSERT INTO file_kind_counts(fid,code,n,postings) VALUES(?1,?2,?3,?4)",
                params![fid, code as i64, n, positions.segments[code]],
            )?;
        }
        drop(freq);
        for (table, bytes) in [("parovf", image.parb), ("sizeovf", image.sizb)] {
            let mut put = tx.prepare_cached(&format!("INSERT INTO {table} VALUES(?1,?2,?3)"))?;
            for pair in bytes.chunks_exact(8) {
                put.execute(params![
                    fid,
                    u32::from_le_bytes(pair[..4].try_into().expect("key width")),
                    u32::from_le_bytes(pair[4..].try_into().expect("value width"))
                ])?;
            }
        }
        value_image.put(&tx, fid, lang)?;
        tx.commit()?;
        if let Some(previous) = previous {
            values.remove(previous);
        }
        values.publish(fid, lang, &value_image);
        #[cfg(feature = "code-rule-profile")]
        crate::code_rule_dsl::profile::record_sql_index_write(doc.transducer_len());
        Ok(fid)
    }
    #[cfg(test)]
    pub fn remove(&self, path: &str) -> Result<()> {
        if let Some(fid) = self
            .db
            .query_row("SELECT fid FROM file WHERE path=?1", [path], |r| r.get(0))
            .optional()?
        {
            self.remove_known(fid)?;
        }
        Ok(())
    }

    pub fn remove_known(&self, fid: i64) -> Result<()> {
        self.readers.clear();
        let tx = self.db.unchecked_transaction()?;
        remove(&tx, fid)?;
        tx.commit()?;
        self.values.lock().remove(fid);
        Ok(())
    }
    #[cfg(test)]
    pub fn resident_bytes(&self) -> Result<usize> {
        self.db.resident_bytes()
    }
}

fn remove(db: &Connection, fid: i64) -> Result<()> {
    db.execute(
        "UPDATE kfreq SET n=n-(SELECT c.n FROM file_kind_counts c WHERE c.fid=?1 AND c.code=kfreq.code)
         WHERE lang=(SELECT lang FROM file WHERE fid=?1)
         AND code IN (SELECT code FROM file_kind_counts WHERE fid=?1)", [fid],
    )?;
    db.execute("DELETE FROM kfreq WHERE n=0", [])?;
    db.execute("DELETE FROM file WHERE fid=?1", [fid])?;
    Ok(())
}
