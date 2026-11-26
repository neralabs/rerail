//! The directory: block ids, references between generations, and a launch's pins and descriptor table (decision
//! 0686; *0686: Operations* §2, E1, E2).
//!
//! Host Rust, handles only: the directory hands out a block's id, keeps a handle of each block, and keeps a `Frozen`
//! handle for each reference one block makes to another, so that a referenced generation lives while anything refers
//! to it. It never reads a row. What a block refers to is in the block itself (its reference table, `gens`), written
//! here through a write pin, and only to a frozen block: the one-writer rule holds across references.
//!
//! A launch (`Directory::launch`) pins every slot by its mode, then read-pins every block the slots refer to,
//! transitively, and writes the CPU's descriptor table of all of them, its slots too (O§2: "per block id it can reach
//! (its slots, and what they refer to, transitively)"): their host addresses, lengths, kinds and versions.
//! Any conflict refuses the start, naming the operation, and nothing stays pinned.
//!
//! A launch's table on a device (`gens::device_table`, over `Launch::pins`) is the same rows, each block's address the
//! device's: its memory wrapped in place, nothing uploaded; on a discrete GPU's memory domain, each block's replica's
//! there (`Directory::replicate`, `gens::device_rows_in`). The caller makes it on the device it launches on: this
//! crate names no device (B§7: blocks and rr_syntax alone). A kernel finds a row by id through `gens::DESC_READS`.
//!
//! On a device (E1), the launch's pins are handed to the device's fence (`Launch::to_fence`): each a GPU use until the
//! fence completes, with the same checks, nothing open between the CPU's pin and the GPU's.

use rerail_blocks::block::kind;
use rerail_blocks::gens::{self, BODY_AT, Desc, Table};
use rerail_blocks::ledger::Fence;
use rerail_blocks::region::{BlockPin, pin_block};
use rerail_blocks::{Block, Frozen};
use std::sync::{Arc, Weak};

/// A block's id: the directory's handle, the same on every device.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

enum Own {
    Open(Arc<Block>),
    Frozen(Frozen),
    /// The directory's own handle let go: the block lives while a referrer holds it.
    Released,
}

struct Entry {
    own: Own,
    /// Its whole-block region's handle, kept while it has a replica (`replicate`): the region holds the replica.
    placed: Option<rerail_blocks::region::Shared<u8>>,
    /// The blocks it refers to, each held while it does.
    held: Vec<(BlockId, Frozen)>,
    weak: Weak<Block>,
    /// Its body's bytes, as made: what a freeze trims to the body's used length.
    body: usize,
}

/// Every block the executor's operations use, by id.
#[derive(Default)]
pub struct Directory {
    entries: Vec<Entry>,
}

impl Directory {
    /// A new open block of `kind` with a body of `body` bytes, and an empty reference table.
    pub fn make(&mut self, kind: u32, body: usize) -> BlockId {
        self.make_room(kind, body, 0)
    }

    /// `make`, with `room` bytes past the body for the structures its writer makes in the block's chunks (a layer's
    /// relations, `Relation::try_continue_far`).
    pub fn make_room(&mut self, kind: u32, body: usize, room: usize) -> BlockId {
        self.sweep();
        // Free-listed: its freeze gives back what lies past the body's used length (an operation's work, past the
        // facts it wrote), whole pages to the OS, when nothing was made past the body.
        let b = Block::anon(kind, BODY_AT + body + room + rerail_blocks::block::PAGE).with_free_list();
        assert_eq!(b.alloc(BODY_AT - gens::REFS_AT, 64), gens::REFS_AT, "a new block's first allocation is its reference table");
        assert_eq!(b.alloc(body.max(8), 64), BODY_AT, "and its body follows");
        let b = Arc::new(b);
        {
            let mut w = pin_block(&b, "make", true).expect("a new block is unused");
            gens::init_refs(w.bytes_mut().expect("a write pin"));
        }
        let weak = Arc::downgrade(&b);
        self.entries.push(Entry { own: Own::Open(b), placed: None, held: Vec::new(), weak, body: body.max(8) });
        BlockId(self.entries.len() as u32 - 1)
    }

    fn entry(&self, id: BlockId) -> Result<&Entry, String> {
        self.entries.get(id.0 as usize).ok_or_else(|| format!("no block {id}"))
    }

    /// Block `id`, while anything holds it: the directory, or a block that refers to it.
    pub fn block(&self, id: BlockId) -> Option<Arc<Block>> {
        let e = self.entries.get(id.0 as usize)?;
        match &e.own {
            Own::Open(b) => Some(b.clone()),
            Own::Frozen(f) => Some(f.arc().clone()),
            Own::Released => e.weak.upgrade(),
        }
    }

    /// Whether block `id` is still held by anything.
    pub fn alive(&self, id: BlockId) -> bool {
        self.entries.get(id.0 as usize).is_some_and(|e| e.weak.strong_count() > 0)
    }

    pub fn is_frozen(&self, id: BlockId) -> bool {
        self.block(id).is_some_and(|b| !b.is_writable())
    }

    /// The blocks `id` refers to.
    pub fn refs(&self, id: BlockId) -> Vec<BlockId> {
        self.entries.get(id.0 as usize).map_or(Vec::new(), |e| e.held.iter().map(|(r, _)| *r).collect())
    }

    /// Block `from` refers to what lies at `at` in block `to` from now on (O§2: by block id and offset): written in
    /// `from`'s reference table, and `to` held while `from` is. Refused for a `to` that is not frozen, or a `from` that
    /// is.
    pub fn refer(&mut self, from: BlockId, to: BlockId, at: usize) -> Result<(), String> {
        let target = self.block(to).ok_or_else(|| format!("a reference to {to}, which nothing holds"))?;
        if target.is_writable() {
            return Err(format!("a reference from {from} to {to}, a block not frozen"));
        }
        let Own::Open(src) = &self.entry(from)?.own else {
            return Err(format!("a reference from {from}, a block frozen: its references are written while it is open"));
        };
        let mut w = pin_block(src, "refer", true).map_err(|e| format!("a reference from {from}: {e}"))?;
        gens::add_ref(w.bytes_mut().expect("a write pin"), to.0, at).map_err(|e| format!("{from}: {e}"))?;
        drop(w);
        let frozen = match &self.entry(to)?.own {
            Own::Frozen(f) => f.clone(),
            _ => self.entries.iter().flat_map(|e| &e.held).find(|(r, _)| *r == to).map(|(_, f)| f.clone()).expect("a frozen block held"),
        };
        self.entries[from.0 as usize].held.push((to, frozen));
        Ok(())
    }

    /// Freezes block `id`: its body trimmed to the length its writer left in its first word, then read-only. Refused
    /// while anything else holds it (a pin, a region).
    pub fn freeze(&mut self, id: BlockId) -> Result<(), String> {
        let body = self.entry(id)?.body;
        let e = &mut self.entries[id.0 as usize];
        // Its replicas are the open block's: they go with its region (the frozen block is placed again as it is read).
        e.placed = None;
        let Own::Open(b) = std::mem::replace(&mut e.own, Own::Released) else {
            return Err(format!("{id} is not open"));
        };
        let b = match Arc::try_unwrap(b) {
            Ok(b) => b,
            Err(b) => {
                e.own = Own::Open(b);
                return Err(format!("a freeze of {id} while it is in use"));
            }
        };
        let used = gens::body_len(b.bytes(0, b.used())).min(body);
        b.shrink_last(BODY_AT, body, used);
        let f = b.freeze();
        e.weak = Arc::downgrade(f.arc());
        e.own = Own::Frozen(f);
        Ok(())
    }

    /// Places block `id` in memory domain `domain` (a discrete GPU's, 1 and up; decision 0440 §7.2: "replicas by the
    /// directory"): `mirror` holds its copy there, brought to the latest version before a kernel there reads it, and
    /// back to the host after one writes it (`Region::sync_host`). The directory keeps the block's region, and so
    /// the replica, until the block is frozen or let go. Refused when the block has one there already.
    pub fn replicate(&mut self, id: BlockId, domain: u32, mirror: Box<dyn rerail_blocks::region::Mirror>) -> Result<(), String> {
        let b = self.block(id).ok_or_else(|| format!("a replica of {id}, which nothing holds"))?;
        let w = rerail_blocks::region::whole(&b);
        if domain == 0 || w.region().has_copy_in(domain) {
            return Err(format!("a replica of {id} in domain {domain}: the host's, or one it has"));
        }
        w.region().mirror_in(domain, mirror);
        self.entries[id.0 as usize].placed = Some(w);
        Ok(())
    }

    /// Lets the directory's own handle of `id` go: it lives on while a block refers to it, or a launch reads it.
    pub fn release(&mut self, id: BlockId) {
        if let Some(e) = self.entries.get_mut(id.0 as usize) {
            e.own = Own::Released;
            e.placed = None;
        }
        self.sweep();
    }

    /// Lets go of the references of every block that is gone: a block's references live as long as it does, so a
    /// generation another refers to lives until its referrer is gone (rolled up, and no reader holds it). Repeated
    /// until nothing more goes: a chain goes at once.
    pub fn sweep(&mut self) {
        loop {
            let mut gone = false;
            for e in &mut self.entries {
                if matches!(e.own, Own::Released) && !e.held.is_empty() && e.weak.strong_count() == 0 {
                    e.held.clear();
                    gone = true;
                }
            }
            if !gone {
                return;
            }
        }
    }

    /// `ids` and every block they refer to, transitively, each once, ascending.
    pub fn reach(&self, ids: &[BlockId]) -> Vec<BlockId> {
        let mut seen: Vec<BlockId> = Vec::new();
        let mut todo: Vec<BlockId> = ids.to_vec();
        while let Some(id) = todo.pop() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            todo.extend(self.refs(id));
        }
        seen.sort();
        seen
    }

    /// Pins a launch of `op` on `slots`, each by `(id, write)`: every slot by its mode, every block they refer to
    /// (transitively) to read, and the CPU's descriptor table of those. Refused, with nothing left pinned, on any
    /// conflict or a block nothing holds.
    pub fn launch(&self, op: &'static str, slots: &[(BlockId, bool)]) -> Result<Launch, String> {
        self.pin(op, slots, &[])
    }

    /// A launch that reads `ids` and every block they refer to, all in its descriptor table (`Launch::block`,
    /// `Launch::table`): how the host's tests and the executor's few control reads read a block, pinned like any.
    pub fn read(&self, op: &'static str, ids: &[BlockId]) -> Result<Launch, String> {
        self.pin(op, &[], ids)
    }

    fn pin(&self, op: &'static str, slots: &[(BlockId, bool)], read: &[BlockId]) -> Result<Launch, String> {
        let mut pins = Vec::with_capacity(slots.len());
        for (i, &(id, write)) in slots.iter().enumerate() {
            let b = self.block(id).ok_or_else(|| format!("{op} could not start: slot {i}: {id} is held by nothing"))?;
            pins.push(pin_block(&b, op, write).map_err(|e| format!("{op} could not start: slot {i} ({id}): {e}"))?);
        }
        // Every block the launch can reach, its slots among them (O§2), each a row. A slot pinned to write is its
        // pin's row alone; every other block is pinned to read here (a slot read is pinned to read twice: a reader
        // beside a reader).
        let from: Vec<BlockId> = slots.iter().map(|s| s.0).chain(read.iter().copied()).collect();
        let all = self.reach(&from);
        let mut reach = Vec::with_capacity(all.len());
        let mut rows = Vec::with_capacity(all.len());
        let mut written = Vec::new();
        for id in all {
            let b = self.block(id).ok_or_else(|| format!("{op} could not start: {id}, referred to, is held by nothing"))?;
            if let Some(k) = slots.iter().position(|s| s.0 == id && s.1) {
                let p = &pins[k];
                rows.push(Desc { id: id.0, kind: b.header().kind, version: p.version(), addr: p.addr() as u64, len: p.len() as u64 });
                written.push(p.addr() as u64);
                continue;
            }
            let p = pin_block(&b, op, false).map_err(|e| format!("{op} could not start: {id}, reached: {e}"))?;
            rows.push(Desc { id: id.0, kind: b.header().kind, version: p.version(), addr: p.addr() as u64, len: p.len() as u64 });
            reach.push(p);
        }
        // The table: a block of its own, written, then read by the launch.
        let table = Arc::new(Block::anon(kind::DESC, BODY_AT + gens::table_bytes(&rows) + rerail_blocks::block::PAGE));
        let at = table.alloc(gens::table_bytes(&rows), 64);
        {
            let mut w = pin_block(&table, "descriptor table", true).expect("a new block is unused");
            gens::write_table(&mut w.bytes_mut().expect("a write pin")[at..], &rows);
        }
        let table_pin = pin_block(&table, op, false).expect("the table is the launch's");
        Ok(Launch { slots: pins, reach, written, table: table_pin, at, read: read.to_vec() })
    }
}

impl Launch {
    /// Every block the launch pins, its slots and the blocks they reach: what its descriptor table's rows name, for
    /// its table on a device (`gens::device_table`, E2), which the caller makes on the device it launches on.
    pub fn pins(&self) -> Vec<&BlockPin> {
        self.slots.iter().chain(&self.reach).collect()
    }

    /// Hands the launch to the device (E1, Operations §2: "every slot pinned from the operation's signature when it
    /// starts"; a GPU use pinned until its fence completes): each slot by its mode and each block it reaches to read
    /// become GPU uses by `op` on `fence`, in memory domain `domain` (0: the host's, read in place; 1 and up, a
    /// discrete GPU's, each block's replica there, `Directory::replicate`). Until `fence` completes no CPU use and no
    /// conflicting GPU use of them can begin, and they are not freed. Refused, the launch given back as it was, when
    /// a block has no copy in `domain`. The CPU's descriptor table goes: the device reads its own (`DeviceTable`,
    /// handed to the fence first).
    pub fn to_fence(self, fence: &Arc<Fence>, op: &'static str, domain: u32) -> Result<(), (Launch, String)> {
        if let Some(p) = self.slots.iter().chain(&self.reach).find(|p| !p.has_copy_in(domain)) {
            let e = format!("{op} on domain {domain}: a block at {:#x} it reaches has no replica there (Directory::replicate)", p.addr());
            return Err((self, e));
        }
        // The launch's pins exclude every conflicting use since it started (a CPU write, a GPU write of another
        // domain), and each has its copy in `domain`: no handoff can be refused now.
        for p in self.slots.into_iter().chain(self.reach) {
            p.to_fence(fence, op, domain).unwrap_or_else(|(_, e)| panic!("a launch's handoff refused after its checks: {e}"));
        }
        Ok(())
    }
}

/// A launch's pins: its slots', the blocks they refer to, and its descriptor table's. Dropping it lets every one go.
pub struct Launch {
    pub slots: Vec<BlockPin>,
    reach: Vec<BlockPin>,
    /// The addresses of its slots pinned to write: rows of its table that a host read through it does not resolve.
    written: Vec<u64>,
    table: BlockPin,
    at: usize,
    read: Vec<BlockId>,
}

impl Launch {
    /// The launch's slots, and its descriptor table over the blocks they reach.
    pub fn parts(&mut self) -> (&mut [BlockPin], Table<'_>) {
        (&mut self.slots, Table::new(&self.table, self.at, &self.reach, &self.written))
    }

    /// Its descriptor table.
    pub fn table(&self) -> Table<'_> {
        Table::new(&self.table, self.at, &self.reach, &self.written)
    }

    /// The bytes of the `i`th block a read launch (`Directory::read`) was asked for.
    pub fn block(&self, i: usize) -> &[u8] {
        let id = self.read[i];
        let t = Table::new(&self.table, self.at, &self.reach, &self.written);
        let d = *t.row(id.0).expect("a read block is in its table");
        self.reach.iter().find(|p| p.addr() as u64 == d.addr).expect("pinned").bytes()
    }
}
