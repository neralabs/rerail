//! Incremental SQLite BLOB access, as in the spike's native tape adapter.
//! Handles share SQLite's bounded page cache; no decoded tape or full-column cache is kept here.
use foldhash::{HashMap, HashMapExt};
use rusqlite::{Connection, Result, ffi};
use std::{ffi::CStr, sync::Arc};

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum Column {
    Rec,
    Parent,
    Size,
    Flags,
    Spans,
    Checkpoints,
    Source,
}
impl Column {
    fn name(self) -> &'static CStr {
        match self {
            Self::Rec => c"rec",
            Self::Parent => c"parb",
            Self::Size => c"sizb",
            Self::Flags => c"flags",
            Self::Spans => c"spans",
            Self::Checkpoints => c"checkpoints",
            Self::Source => c"src",
        }
    }
}
struct Blob {
    pointer: *mut ffi::sqlite3_blob,
    length: usize,
}
impl Drop for Blob {
    fn drop(&mut self) {
        // SAFETY: each successful blob_open has exactly one owning Blob.
        unsafe {
            ffi::sqlite3_blob_close(self.pointer);
        }
    }
}
pub(super) struct Reader {
    database: *mut ffi::sqlite3,
    handles: HashMap<(i64, Column), Blob>,
    record: Option<RecordBlock>,
    spans: Option<SpanBlock>,
    #[cfg(feature = "code-rule-profile")]
    pub counters: Counters,
    #[cfg(test)]
    pub read_bytes: usize,
}
// Small transport buffers, shared by scalar calls inside a batched SQL statement. The SQL
// tables own decoded properties; these buffers only amortize incremental BLOB API reads.
struct RecordBlock {
    fid: i64,
    start: usize,
    rec: Vec<u8>,
    flags: Vec<u8>,
}
struct SpanBlock {
    fid: i64,
    block: usize,
    spans: Vec<u8>,
    checkpoint: [u8; 6],
}

#[cfg(feature = "code-rule-profile")]
#[derive(Clone, Copy, Default, serde::Serialize)]
pub(super) struct Counters {
    pub blob_opens: u64,
    pub handle_clears: u64,
    pub reads: [u64; 7],
    pub bytes: [u64; 7],
    pub texts: u64,
    pub child_steps: u64,
    pub decoded_nodes: u64,
    pub decoded_spans: u64,
    pub regex_compiles: u64,
    pub source_batches: u64,
    pub source_input_rows: u64,
    pub source_output_rows: u64,
}

#[cfg(feature = "code-rule-profile")]
impl Counters {
    pub fn since(self, before: Self) -> Self {
        Self {
            blob_opens: self.blob_opens - before.blob_opens,
            handle_clears: self.handle_clears - before.handle_clears,
            reads: std::array::from_fn(|i| self.reads[i] - before.reads[i]),
            bytes: std::array::from_fn(|i| self.bytes[i] - before.bytes[i]),
            texts: self.texts - before.texts,
            child_steps: self.child_steps - before.child_steps,
            decoded_nodes: self.decoded_nodes - before.decoded_nodes,
            decoded_spans: self.decoded_spans - before.decoded_spans,
            regex_compiles: self.regex_compiles - before.regex_compiles,
            source_batches: self.source_batches - before.source_batches,
            source_input_rows: self.source_input_rows - before.source_input_rows,
            source_output_rows: self.source_output_rows - before.source_output_rows,
        }
    }
}
// SAFETY: the connection and its callbacks move together between workers. Access is serialized
// by the owning connection and this mutex; handles close before that connection closes.
unsafe impl Send for Reader {}
pub(super) type Shared = Arc<parking_lot::Mutex<Reader>>;

/// The owner clears handles before writes and before SQLite destroys its callback closures.
pub(super) struct Readers(pub Shared);
impl Readers {
    pub fn new(db: &Connection) -> Self {
        // SAFETY: Store owns this guard before its connection and cannot outlive that connection.
        Self(Arc::new(parking_lot::Mutex::new(Reader {
            database: unsafe { db.handle() },
            handles: HashMap::new(),
            record: None,
            spans: None,
            #[cfg(feature = "code-rule-profile")]
            counters: Counters::default(),
            #[cfg(test)]
            read_bytes: 0,
        })))
    }
    pub fn clear(&self) {
        let mut reader = self.0.lock();
        #[cfg(feature = "code-rule-profile")]
        if !reader.handles.is_empty() {
            reader.counters.handle_clears += 1;
        }
        reader.handles.clear();
        reader.record = None;
        reader.spans = None;
    }
}
impl Drop for Readers {
    fn drop(&mut self) {
        self.clear();
    }
}

fn failure(code: i32) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)
}
fn invalid(message: &str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into())
}

impl Reader {
    /// Borrow this connection for a whole typed frame read. The wrapper never closes it.
    pub(super) fn connection(&self) -> Result<Connection> {
        // SAFETY: the owning connection is executing this callback on the same thread.
        unsafe { Connection::from_handle(self.database) }
    }
    fn blob(&mut self, fid: i64, column: Column) -> Result<&Blob> {
        let key = (fid, column);
        if !self.handles.contains_key(&key) {
            // Bound cursor ownership even for a query spanning an arbitrary number of files.
            if self.handles.len() >= 32 {
                #[cfg(feature = "code-rule-profile")]
                {
                    self.counters.handle_clears += 1;
                }
                self.handles.clear();
            }
            let mut pointer = std::ptr::null_mut();
            // SAFETY: live connection, static NUL-terminated identifiers, initialized out-pointer.
            let code = unsafe {
                ffi::sqlite3_blob_open(
                    self.database,
                    c"main".as_ptr(),
                    c"file".as_ptr(),
                    column.name().as_ptr(),
                    fid,
                    0,
                    &mut pointer,
                )
            };
            if code != ffi::SQLITE_OK {
                return Err(failure(code));
            }
            // SAFETY: blob_open succeeded, so this handle is live.
            let length = unsafe { ffi::sqlite3_blob_bytes(pointer) } as usize;
            self.handles.insert(key, Blob { pointer, length });
            #[cfg(feature = "code-rule-profile")]
            {
                self.counters.blob_opens += 1;
            }
        }
        Ok(&self.handles[&key])
    }
    fn read(&mut self, fid: i64, column: Column, offset: usize, bytes: &mut [u8]) -> Result<()> {
        #[cfg(feature = "code-rule-profile")]
        {
            self.counters.reads[column as usize] += 1;
            self.counters.bytes[column as usize] += bytes.len() as u64;
        }
        #[cfg(test)]
        {
            self.read_bytes += bytes.len();
        }
        let blob = self.blob(fid, column)?;
        if offset
            .checked_add(bytes.len())
            .is_none_or(|end| end > blob.length)
        {
            return Err(invalid("read outside index column"));
        }
        let offset = i32::try_from(offset).map_err(|_| invalid("index column offset overflow"))?;
        let length =
            i32::try_from(bytes.len()).map_err(|_| invalid("index column read overflow"))?;
        // SAFETY: validated offsets and writable slice; handle remains live during this call.
        let code = unsafe {
            ffi::sqlite3_blob_read(blob.pointer, bytes.as_mut_ptr().cast(), length, offset)
        };
        if code != ffi::SQLITE_OK {
            return Err(failure(code));
        }
        Ok(())
    }
    pub fn rec(&mut self, fid: i64, pre: usize) -> Result<[u16; 3]> {
        let block = self.record_block(fid, pre)?;
        let offset = (pre - block.start) * 6;
        let bytes = &block.rec[offset..offset + 6];
        Ok(std::array::from_fn(|i| {
            u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]])
        }))
    }
    pub fn flags(&mut self, fid: i64, pre: usize) -> Result<u8> {
        let block = self.record_block(fid, pre)?;
        Ok(block.flags[pre - block.start])
    }
    fn record_block(&mut self, fid: i64, pre: usize) -> Result<&RecordBlock> {
        let start = pre / 256 * 256;
        if !self
            .record
            .as_ref()
            .is_some_and(|block| block.fid == fid && block.start == start)
        {
            let nodes = self.blob(fid, Column::Rec)?.length / 6;
            if pre >= nodes {
                return Err(invalid("ordinal outside index"));
            }
            let count = (nodes - start).min(256);
            let mut rec = vec![0; count * 6];
            let mut flags = vec![0; count];
            self.read(fid, Column::Rec, start * 6, &mut rec)?;
            self.read(fid, Column::Flags, start, &mut flags)?;
            self.record = Some(RecordBlock {
                fid,
                start,
                rec,
                flags,
            });
        }
        let block = self.record.as_ref().expect("record block");
        if pre - block.start >= block.flags.len() {
            return Err(invalid("ordinal outside index"));
        }
        Ok(block)
    }
    /// The spike's child stepping: skip complete subtrees, then return the first field edge.
    pub fn child(&mut self, fid: i64, parent: usize, field: u16) -> Result<Option<i64>> {
        let rec = self.rec(fid, parent)?;
        let end = parent + self.value(fid, Column::Size, parent, rec[2])? as usize;
        let mut child = parent + 1;
        while child < end {
            #[cfg(feature = "code-rule-profile")]
            {
                self.counters.child_steps += 1;
            }
            if crate::code_rule_dsl::budget::step_budget_exhausted() {
                return Err(failure(ffi::SQLITE_INTERRUPT));
            }
            let rec = self.rec(fid, child)?;
            if rec[0] >> 10 == field {
                return Ok(Some(child as i64));
            }
            let size = self.value(fid, Column::Size, child, rec[2])? as usize;
            if size == 0 {
                return Err(invalid("empty indexed subtree"));
            }
            child += size;
        }
        Ok(None)
    }
    pub fn value(&mut self, fid: i64, column: Column, pre: usize, raw: u16) -> Result<u32> {
        if raw != u16::MAX {
            return Ok(u32::from(raw));
        }
        let length = self.blob(fid, column)?.length;
        if !length.is_multiple_of(8) {
            return Err(invalid("invalid overflow column"));
        }
        let (mut lo, mut hi) = (0, length / 8);
        let mut pair = [0; 8];
        while lo < hi {
            let mid = (lo + hi) / 2;
            self.read(fid, column, mid * 8, &mut pair)?;
            if (u32::from_le_bytes(pair[..4].try_into().expect("key width")) as usize) < pre {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        self.read(fid, column, lo * 8, &mut pair)?;
        if u32::from_le_bytes(pair[..4].try_into().expect("key width")) as usize != pre {
            return Err(invalid("missing overflow entry"));
        }
        Ok(u32::from_le_bytes(
            pair[4..].try_into().expect("value width"),
        ))
    }
    pub fn span(&mut self, fid: i64, pre: usize) -> Result<std::ops::Range<usize>> {
        #[cfg(feature = "code-rule-profile")]
        {
            self.counters.decoded_spans += 1;
        }
        let block = pre / 16;
        if let Some(cached) = &self.spans
            && cached.fid == fid
            && cached.block == block
        {
            return crate::code_rule_dsl::tape::index_span(
                &cached.spans,
                &cached.checkpoint,
                pre % 16,
            )
            .map_err(|error| invalid(&error));
        }
        let mut checkpoint = [0; 6];
        self.read(
            fid,
            Column::Checkpoints,
            (pre / 16)
                .checked_mul(6)
                .ok_or_else(|| invalid("checkpoint overflow"))?,
            &mut checkpoint,
        )?;
        let offset = u32::from_le_bytes([checkpoint[0], checkpoint[1], checkpoint[2], 0]) as usize;
        let length = self
            .blob(fid, Column::Spans)?
            .length
            .checked_sub(offset)
            .ok_or_else(|| invalid("span offset outside column"))?
            .min(160);
        let mut spans = [0; 160];
        self.read(fid, Column::Spans, offset, &mut spans[..length])?;
        checkpoint[..3].fill(0);
        let range = crate::code_rule_dsl::tape::index_span(&spans[..length], &checkpoint, pre % 16)
            .map_err(|error| invalid(&error))?;
        self.spans = Some(SpanBlock {
            fid,
            block,
            spans: spans[..length].to_vec(),
            checkpoint,
        });
        Ok(range)
    }
    pub fn slice(&mut self, fid: i64, start: usize, len: usize) -> Result<String> {
        #[cfg(feature = "code-rule-profile")]
        {
            self.counters.texts += 1;
        }
        // Validate the range before allocating an indexed source span.
        let source_len = self.blob(fid, Column::Source)?.length;
        if start.checked_add(len).is_none_or(|end| end > source_len) {
            return Err(invalid("span outside source"));
        }
        let mut text = vec![0; len];
        self.read(fid, Column::Source, start, &mut text)?;
        String::from_utf8(text).map_err(|error| invalid(&error.to_string()))
    }
    pub fn text(&mut self, fid: i64, pre: usize) -> Result<String> {
        let range = self.span(fid, pre)?;
        self.slice(fid, range.start, range.len())
    }
}
