//! A chunk's numbering of its nodes (plan "Chunk node numbers"), as the readers above the syntax table ask it.
//!
//! Within a chunk a node's, token's or span's id is 32 bits: its document's base plus its local id, read back
//! through the growth's page table (`view_reads::IdPages`) in one load. The syntax table's growth (`made::Ids`) holds
//! the table; a chunk's connection holds the growth as a `Numbering` (`set`, `of`), so the executor's document reads
//! (`docof`, a document test, a document's interval) and the corpus phase's ask it, never the id's bits.
//!
//! An id leaves the chunk as its document's key (`key_of`) or as a location (`located`: the key and the id's low bits
//! in the document's own numbering, `rerail_blocks::located::Located`), never as itself.

use rusqlite::Connection;
use std::sync::Arc;

/// A chunk's numbering: an id's document and local id, a document's ids, and how an id leaves the chunk.
pub trait Numbering: Send + Sync {
    /// The key of id `id`'s document (`docof`): its page's. A negative value is no id: its `>> 32`, as before (`unset`
    /// stays -1, a place stays below it).
    fn key_of(&self, id: i64) -> i64;
    /// Id `id`'s local id in its document.
    fn local_of(&self, id: i64) -> u32;
    /// The ids of the document of key `key`: the least and one past the greatest it holds (its base run, with its
    /// later spans in its last page where they fit; their hull where they took pages of their own). `None` for a
    /// document the chunk has not made.
    fn ids_of(&self, key: u32) -> Option<(u32, u32)>;
    /// Id `id` as it leaves the chunk: its document's key and its low bits in the document's own numbering (its
    /// granule's, `Projected::low`), `Located`'s value; a negative value as it is.
    fn located(&self, id: i64) -> i64;
    /// The chunk id of location `value` (`located`'s inverse), where its document is the chunk's; a negative value as
    /// it is; `None` where the chunk holds no such node.
    fn of_located(&self, value: i64) -> Option<i64>;
}

/// The connection's numbering, once its chunk's views are bound (`set`).
#[derive(Default)]
pub struct ConnNumbering(parking_lot::RwLock<Option<Arc<dyn Numbering>>>);

/// Binds `numbering` to `conn`, the numbering of the chunk whose views it reads: a kept connection reads the next
/// chunk's once its views are bound.
pub fn set(conn: &Connection, numbering: Arc<dyn Numbering>) {
    let cell = super::vtab::Db::of(conn).data().get::<ConnNumbering>();
    *cell.0.write() = Some(numbering);
}

/// `conn`'s numbering, if its chunk's views are bound.
pub fn of(conn: &Connection) -> Option<Arc<dyn Numbering>> {
    of_db(super::vtab::Db::of(conn))
}

/// `of`, through a connection's scope (a job off its thread).
pub fn of_db(db: super::vtab::Db<'_>) -> Option<Arc<dyn Numbering>> {
    db.data().get_if_any::<ConnNumbering>()?.0.read().clone()
}
