//! File-scoped kind positions share the SQL-owned count rows and posting decoder.
use rusqlite::{Connection, Result, params};

pub(super) struct Builder {
    pub segments: Vec<Vec<u8>>,
    previous: [u32; 1024],
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            segments: vec![Vec::new(); 1024],
            previous: [0; 1024],
        }
    }
}

impl Builder {
    pub fn push(&mut self, kind: u16, pre: u32) {
        let code = usize::from(kind);
        let delta = pre - self.previous[code];
        self.previous[code] = pre;
        super::postings::append_delta(&mut self.segments[code], delta);
    }
    fn from_records(records: &[u8]) -> Result<Self> {
        if !records.len().is_multiple_of(6) {
            return Err(rusqlite::Error::UserFunctionError(
                "invalid packed kind records".into(),
            ));
        }
        let mut builder = Self::default();
        for (pre, rec) in records.chunks_exact(6).enumerate() {
            let pre = u32::try_from(pre)
                .map_err(|_| rusqlite::Error::UserFunctionError("kind posting capacity".into()))?;
            builder.push(u16::from_le_bytes([rec[0], rec[1]]) & 1023, pre);
        }
        Ok(builder)
    }
}

/// Version one stores already have the authoritative packed records and kind counts.
/// Backfill once without reparsing source or issuing a query for each node or file.
pub(super) fn upgrade(db: &Connection) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute_batch("ALTER TABLE file_kind_counts ADD COLUMN postings BLOB NOT NULL DEFAULT X''")?;
    {
        let mut files = tx.prepare("SELECT fid,rec FROM file ORDER BY fid")?;
        let mut rows = files.query([])?;
        let mut put =
            tx.prepare("UPDATE file_kind_counts SET postings=?3 WHERE fid=?1 AND code=?2")?;
        while let Some(row) = rows.next()? {
            let fid = row.get::<_, i64>(0)?;
            let builder = Builder::from_records(&row.get::<_, Vec<u8>>(1)?)?;
            for (kind, bytes) in builder
                .segments
                .iter()
                .enumerate()
                .filter(|(_, b)| !b.is_empty())
            {
                if put.execute(params![fid, kind as i64, bytes])? != 1 {
                    return Err(rusqlite::Error::UserFunctionError(
                        "missing kind count during index upgrade".into(),
                    ));
                }
            }
        }
    }
    tx.pragma_update(None, "user_version", 2_i64)?;
    tx.commit()
}
