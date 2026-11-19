//! The device's automata in one block (decision 0450 step 2c): each text test's table (`table::Table`) and each
//! regex's replacement automata (`table::Replacer`), written once where they are made, with an index from a key's
//! hash to its slot in the same block. The kernels bind the block's no-copy wrap at an automaton's offset: no upload,
//! no thread's map of device buffers. A refusal (a plan the device has no table for) is kept too, with why.
//!
//! The index is open addressing over `SLOTS` slots of 64 bytes, made once (its pages taken as slots are written).
//! A slot is published by its state, written last (Release); a reader probes with no lock. Writers take `write`.
//! The automata come from the rules' patterns, not from relations: the block is the process's, as the maps it
//! replaces were.

use rerail_blocks::block::{Block, kind};
use std::sync::atomic::{AtomicU32, Ordering::{Acquire, Release}};
use std::sync::{LazyLock, Mutex};

/// The slots of the index: more automata than a run's rules make. Past them an automaton is refused (the test runs
/// on the CPU).
const SLOTS: usize = 1 << 14;
/// The address space the block reserves: the index and the automata (a table is at most 256 + 256 * 257 bytes).
const RESERVE: usize = 1 << 30;
/// An automaton's offset in the block is aligned to this, so a kernel binds the block at it.
pub const ALIGN: usize = 256;

const EMPTY: u32 = 0;
const MADE: u32 = 1;
const REFUSED: u32 = 2;

/// A slot: its key's hash, its key (bytes in the block), its value (an automaton's bytes, or a refusal's message),
/// four words of the automaton's numbers, and its state.
#[repr(C)]
struct Slot {
    hash: u64,
    key_at: u64,
    key_len: u64,
    val_at: u64,
    val_len: u64,
    words: [u32; 4],
    state: AtomicU32,
    pad: u32,
}
const _: () = assert!(std::mem::size_of::<Slot>() == 64);

/// An automaton in the block: its bytes' offset and length, and its numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Automaton {
    pub at: usize,
    pub len: usize,
    pub words: [u32; 4],
}

impl Automaton {
    /// Its bytes, where the block holds them.
    pub fn bytes(&self) -> &'static [u8] {
        AUTOMATA.block.bytes(self.at, self.at + self.len)
    }
}

pub struct Automata {
    block: Block,
    slots: usize,
    write: Mutex<()>,
}

// SAFETY: a slot is written under `write` and read after its state's Release store; an automaton's bytes are
// written before its slot is published, and never after.
unsafe impl Send for Automata {}
unsafe impl Sync for Automata {}

static AUTOMATA: LazyLock<Automata> = LazyLock::new(|| {
    let block = Block::anon(kind::SCRATCH, RESERVE);
    let slots = block.alloc(SLOTS * std::mem::size_of::<Slot>(), ALIGN);
    Automata { block, slots, write: Mutex::new(()) }
});

/// The block the automata are in (a kernel binds its wrap at an automaton's `at`).
pub fn block() -> &'static Block {
    &AUTOMATA.block
}

fn hash(key: &str) -> u64 {
    // FNV-1a, then a mix: keys are short, made once per pattern.
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, c| (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3));
    (h ^ (h >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9)
}

impl Automata {
    fn slot(&self, k: usize) -> &Slot {
        // SAFETY: `k < SLOTS`, a slot of the index, zero until written.
        unsafe { &*self.block.ptr::<Slot>(self.slots + k * std::mem::size_of::<Slot>()) }
    }

    /// The slot of `key`, or the first empty one on its probe; `None` when the index is full.
    fn probe(&self, key: &str, h: u64) -> Option<(usize, u32)> {
        let mask = SLOTS - 1;
        let mut k = h as usize & mask;
        for _ in 0..SLOTS {
            let s = self.slot(k);
            let state = s.state.load(Acquire);
            if state == EMPTY {
                return Some((k, EMPTY));
            }
            if s.hash == h && self.block.bytes(s.key_at as usize, (s.key_at + s.key_len) as usize) == key.as_bytes() {
                return Some((k, state));
            }
            k = (k + 1) & mask;
        }
        None
    }

    fn read(&self, k: usize, state: u32) -> Result<Automaton, String> {
        let s = self.slot(k);
        let (at, len) = (s.val_at as usize, s.val_len as usize);
        match state {
            MADE => Ok(Automaton { at, len, words: s.words }),
            _ => Err(String::from_utf8_lossy(self.block.bytes(at, at + len)).into_owned()),
        }
    }

    fn put(&self, bytes: &[u8], align: usize) -> Option<usize> {
        let at = self.block.try_alloc(bytes.len().max(1), align)?;
        // SAFETY: `bytes.len()` bytes just given at `at`, which no reader has yet.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.block.ptr::<u8>(at), bytes.len()) };
        Some(at)
    }
}

/// The automaton kept under `key`, made by `make` (its bytes and numbers, or why it has none) the first time any
/// thread asks. An automaton past the block's room or index is refused, with why.
pub fn get_or_make(key: &str, make: impl FnOnce() -> Result<(Vec<u8>, [u32; 4]), String>) -> Result<Automaton, String> {
    let a = &*AUTOMATA;
    let h = hash(key);
    match a.probe(key, h) {
        Some((k, state)) if state != EMPTY => return a.read(k, state),
        None => return Err(format!("the automata block's index is full ({SLOTS})")),
        _ => {}
    }
    // Made outside the lock: another thread may make the same; the first to publish it wins.
    let made = make();
    let _w = a.write.lock().expect("automata");
    let (k, state) = a.probe(key, h).ok_or_else(|| format!("the automata block's index is full ({SLOTS})"))?;
    if state != EMPTY {
        return a.read(k, state);
    }
    let (val, words, state) = match &made {
        Ok((bytes, words)) => (bytes.as_slice(), *words, MADE),
        Err(why) => (why.as_bytes(), [0; 4], REFUSED),
    };
    let room = || format!("the automata block is full ({RESERVE} bytes)");
    let key_at = a.put(key.as_bytes(), 8).ok_or_else(room)?;
    let val_at = a.put(val, ALIGN).ok_or_else(room)?;
    // SAFETY: the slot is empty and this thread holds `write`; readers read its fields only after its state.
    unsafe {
        let s = a.block.ptr::<Slot>(a.slots + k * std::mem::size_of::<Slot>());
        (*s).hash = h;
        (*s).key_at = key_at as u64;
        (*s).key_len = key.len() as u64;
        (*s).val_at = val_at as u64;
        (*s).val_len = val.len() as u64;
        (*s).words = words;
        (*s).state.store(state, Release);
    }
    a.read(k, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0450 step 2c: an automaton is written once into the block, where every later ask finds it, from any
    /// thread; a refusal is kept with why.
    #[test]
    fn an_automaton_is_made_once_into_the_block_and_found_there() {
        let key = format!("test:{:p}", &SLOTS);
        let mut made = 0;
        let a = get_or_make(&key, || {
            made += 1;
            Ok((vec![7; 300], [1, 2, 3, 4]))
        })
        .unwrap();
        assert_eq!(made, 1);
        assert_eq!((a.len, a.words, a.at % ALIGN), (300, [1, 2, 3, 4], 0));
        assert!(a.bytes().iter().all(|&b| b == 7));
        let r = block().base_ptr() as usize;
        assert!((r..r + block().reserved()).contains(&(a.bytes().as_ptr() as usize)), "its bytes in the block");
        let again = std::thread::spawn(move || get_or_make(&key, || panic!("made again")).unwrap()).join().unwrap();
        assert_eq!(again, a);
        let no = format!("refused:{:p}", &SLOTS);
        assert_eq!(get_or_make(&no, || Err("a backtracking pattern".into())), Err("a backtracking pattern".into()));
        assert_eq!(get_or_make(&no, || panic!("made again")), Err("a backtracking pattern".into()));
    }
}
