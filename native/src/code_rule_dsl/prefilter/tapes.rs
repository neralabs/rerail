//! The content store's tapes as `nera_tree` reads them (03 §17, the pure-SQL fallback): a tape's
//! v9np image decoded from its `tape_objects` frame on first read, its `size` column read in
//! place. The last few images are held (`HELD`), as SQLite holds pages. With each image its
//! parents are rebuilt by one stack pass over the sizes (as `TapeDoc` restores them), so a parent
//! or an ancestor chain is read, not found by descent from the root; this is a reader's working
//! state, never stored (decision 0026). `RERAIL_TREE_DESCENT=1` finds them by descent instead,
//! for the lab's comparison (list step 12).
use super::tree::{Shared, Structure, descend};
use crate::code_rule_dsl::language;
use crate::code_rule_dsl::tape::{Grammar, TapeImage};
use rusqlite::{Connection, Result, ffi, functions::FunctionFlags};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Images held: a relation's scope and inner sets are ordered by tape, so a few suffice.
const HELD: usize = 8;

pub(super) struct StoreTapes {
    database: *mut ffi::sqlite3,
    held: VecDeque<(i64, Arc<Held>)>,
    grammars: HashMap<i64, Arc<Grammar>>,
    /// The store's tape segments, mapped as rows name them (decision 0295 step 5): made at the first such row.
    maps: Option<crate::content_store::ObjectMaps>,
}

// SAFETY: the handle is the owning connection's; access is serialized by that connection and the
// mutex `Shared` wraps this in, and the module (with this) is dropped when the connection closes.
unsafe impl Send for StoreTapes {}

/// A decoded tape and, unless descent is asked for, each node's parent (`u32::MAX` at the root).
struct Held {
    image: TapeImage,
    parents: Option<Vec<u32>>,
}

fn parents(image: &TapeImage) -> Vec<u32> {
    let mut parents = Vec::with_capacity(image.node_count());
    let mut open: Vec<(u32, u32)> = Vec::new();
    for i in 0..image.node_count() as u32 {
        while open.last().is_some_and(|&(_, end)| end <= i) {
            open.pop();
        }
        parents.push(open.last().map_or(u32::MAX, |&(at, _)| at));
        open.push((i, i + image.subtree_size(i as usize)));
    }
    parents
}

fn invalid(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(message.into().into())
}

impl StoreTapes {
    fn tape(&mut self, fid: i64) -> Result<Arc<Held>> {
        if let Some(at) = self.held.iter().position(|(held, _)| *held == fid) {
            let entry = self.held.remove(at).expect("held tape");
            let image = Arc::clone(&entry.1);
            self.held.push_front(entry);
            return Ok(image);
        }
        // SAFETY: the owning connection is live while its modules are; the borrowed handle is not
        // closed when this `Connection` drops.
        let db = unsafe { Connection::from_handle(self.database)? };
        let (grammar_id, raw_length, seg, seg_at, seg_len) = db.query_row(
            "SELECT grammar_id,raw_length,seg,seg_at,seg_len FROM tape_objects WHERE tape_id=?1",
            [fid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?)),
        )?;
        if self.maps.is_none() {
            let path = db.path().ok_or_else(|| invalid("a tape store without a file"))?;
            self.maps = Some(crate::content_store::ObjectMaps::new(std::path::Path::new(path)));
        }
        // An archived tape (decision 0450 step 4) is placed back in a segment first, from a connection of its own:
        // this one is inside its statement. Read again, it is read in place there.
        let [seg, seg_at, seg_len] = if seg == crate::content_store::ARCHIVED {
            let path = db.path().ok_or_else(|| invalid("a tape store without a file"))?;
            crate::content_store::place_archived_tape_at(std::path::Path::new(path), fid)
                .map_err(|e| invalid(format!("an archived tape placed back: {e}")))?
                .ok_or_else(|| invalid("an archived tape with no archive row"))?
        } else {
            [seg, seg_at, seg_len]
        };
        let frame = self.maps.as_mut().expect("the maps").frame(seg, seg_at, seg_len);
        let frame = frame.ok_or_else(|| invalid("a tape whose segment is not there"))?;
        let bytes = frame.bytes(usize::try_from(raw_length).unwrap_or(usize::MAX)).map_err(|error| invalid(format!("a tape's bytes: {error}")))?;
        let grammar = match self.grammars.get(&grammar_id) {
            Some(grammar) => Arc::clone(grammar),
            None => {
                let lang = u8::try_from(grammar_id).ok().and_then(language::from_tape_tag).ok_or_else(|| invalid("unknown tape grammar"))?;
                let grammar = Grammar::new(lang).map_err(invalid)?;
                self.grammars.insert(grammar_id, Arc::clone(&grammar));
                grammar
            }
        };
        // Its records' preorder is the node ids rows hold.
        let image = TapeImage::from_bytes(&bytes, &grammar).map_err(invalid)?;
        let parents = std::env::var_os("RERAIL_TREE_DESCENT").is_none().then(|| parents(&image));
        let image = Arc::new(Held { image, parents });
        if self.held.len() == HELD {
            self.held.pop_back();
        }
        self.held.push_front((fid, Arc::clone(&image)));
        Ok(image)
    }
}

impl Structure for StoreTapes {
    fn size(&mut self, fid: i64, pre: usize) -> Result<usize> {
        let tape = self.tape(fid)?;
        if pre >= tape.image.node_count() {
            return Err(invalid("tree node outside its tape"));
        }
        Ok(tape.image.subtree_size(pre) as usize)
    }
    fn ancestors(&mut self, fid: i64, node: usize, out: &mut Vec<i64>) -> Result<()> {
        let tape = self.tape(fid)?;
        let Some(parents) = &tape.parents else {
            return descend(self, fid, node, out);
        };
        let mut at = *parents.get(node).ok_or_else(|| invalid("tree node outside its tape"))?;
        while at != u32::MAX {
            out.push(i64::from(at));
            at = parents[at as usize];
        }
        Ok(())
    }
}

/// `nera_tree` and `nera_size(tape, pre)` over the store's tapes, on a store connection.
pub fn install(db: &Connection) -> Result<()> {
    // SAFETY: the modules and functions are dropped with `db`, before its handle is freed.
    let tapes: Shared = Arc::new(parking_lot::Mutex::new(StoreTapes { database: unsafe { db.handle() }, held: VecDeque::new(), grammars: HashMap::new(), maps: None }));
    super::tree::install(db, Arc::clone(&tapes))?;
    db.create_scalar_function("nera_size", 2, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, move |context| {
        let fid = context.get::<i64>(0)?;
        let pre = usize::try_from(context.get::<i64>(1)?).map_err(|_| invalid("negative tree node"))?;
        Ok(tapes.lock().size(fid, pre)? as i64)
    })
}
