//! A measurement aid (`RERAIL_STAGE_PROF=<file>`): the join program's cycles and instructions retired by its items'
//! kinds, each kind's stage by its read (its mode, its view or relation, whether it is keyed), and the stratum whose
//! launch ran it (`set_tag`, the launching thread's, carried to the threads that run its groups). Read by each thread
//! from its own counters (macOS's `thread_selfcounts`; elsewhere none), summed in that thread's table, and written to
//! the file as the process exits. Off, an item costs one load.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// (stratum tag, item kind, stage read) -> [items, cycles, instructions, threads' rows (t1 - t0 summed)], then the
/// item's tallies (`tally`).
type Table = HashMap<(u32, u32, u32), [u64; 4 + TALLIES]>;

/// What an item tallies (`tally`), by index: its bindings; each binding's walk by its kind (`Walk::how` 0 to 6: rows,
/// an index group, children, ancestors, a subtree, by kind, field rows); the candidates walked, those that matched,
/// its rows out; a scatter's binding by its path (its one row kept, its finds kept, walked again); the bindings run
/// at lanes; and a witness's probes.
pub const TALLIES: usize = 16;
pub const TALLY_NAMES: [&str; TALLIES] = [
    "bindings", "walk_rows", "walk_group", "walk_children", "walk_climb", "walk_subtree", "walk_kind", "walk_field",
    "cands", "matched", "out", "sc_one", "sc_finds", "sc_walk", "lanes", "probes",
];

struct Global {
    file: String,
    tags: Mutex<Vec<String>>,
    tables: Mutex<Vec<Arc<Mutex<Table>>>>,
}

static GLOBAL: OnceLock<Option<Global>> = OnceLock::new();

fn global() -> Option<&'static Global> {
    GLOBAL
        .get_or_init(|| {
            let file = std::env::var("RERAIL_STAGE_PROF").ok().filter(|f| !f.is_empty())?;
            extern "C" fn at_exit() {
                write();
            }
            // SAFETY: registers a function of no arguments that only reads this module's tables.
            unsafe { libc_atexit(at_exit) };
            Some(Global { file, tags: Mutex::new(vec!["-".into()]), tables: Mutex::new(Vec::new()) })
        })
        .as_ref()
}

unsafe extern "C" {
    #[link_name = "atexit"]
    fn libc_atexit(f: extern "C" fn()) -> i32;
}

thread_local! {
    static TAG: Cell<u32> = const { Cell::new(0) };
    static MINE: RefCell<Option<Arc<Mutex<Table>>>> = const { RefCell::new(None) };
    static TALLY: Cell<[u64; TALLIES]> = const { Cell::new([0; TALLIES]) };
}

/// Whether items tally what they walk (`RERAIL_STAGE_TALLY=1`, with `RERAIL_STAGE_PROF`): apart from the cycles,
/// which the tallies' own work adds to (some 15 G instructions of `stage_count` on all).
#[inline]
pub fn tallies_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| on() && std::env::var("RERAIL_STAGE_TALLY").is_ok_and(|v| v == "1"))
}

/// Adds `n` to this thread's tally `i` (`TALLY_NAMES`), counted into the item that runs (`add`). Off, one load.
#[inline]
pub fn tally(i: usize, n: u64) {
    if tallies_on() {
        TALLY.with(|t| {
            let mut a = t.get();
            a[i] += n;
            t.set(a);
        });
    }
}

/// Whether items are counted.
#[inline]
pub fn on() -> bool {
    global().is_some()
}

/// The tag of `name` (a stratum's), made once.
pub fn intern(name: &str) -> u32 {
    let Some(g) = global() else { return 0 };
    let mut t = g.tags.lock().unwrap();
    if let Some(i) = t.iter().position(|x| x == name) {
        return i as u32;
    }
    t.push(name.to_owned());
    (t.len() - 1) as u32
}

/// This thread's tag.
pub fn tag() -> u32 {
    TAG.with(Cell::get)
}

/// Sets this thread's tag; the one it had.
pub fn set_tag(t: u32) -> u32 {
    TAG.with(|c| c.replace(t))
}

/// This thread's instructions and cycles retired so far.
#[inline]
pub fn counts() -> [u64; 2] {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn thread_selfcounts(kind: i32, buf: *mut u64, nbytes: usize) -> i32;
        }
        let mut b = [0u64; 2];
        // SAFETY: kind 1 fills two u64s, this thread's instructions and cycles.
        unsafe { thread_selfcounts(1, b.as_mut_ptr(), 16) };
        b
    }
    #[cfg(not(target_os = "macos"))]
    {
        [0, 0]
    }
}

/// Adds an item's counts (`since`, its start's `counts`) to this thread's table.
pub fn add(kind: u32, read: u32, since: [u64; 2], rows: u64) {
    let Some(g) = global() else { return };
    let now = counts();
    let tag = tag();
    MINE.with(|m| {
        let mut m = m.borrow_mut();
        let t = m.get_or_insert_with(|| {
            let t = Arc::new(Mutex::new(Table::new()));
            g.tables.lock().unwrap().push(Arc::clone(&t));
            t
        });
        let mut t = t.lock().unwrap();
        let e = t.entry((tag, kind, read)).or_default();
        e[0] += 1;
        e[1] += now[1].wrapping_sub(since[1]);
        e[2] += now[0].wrapping_sub(since[0]);
        e[3] += rows;
        let a = TALLY.with(|t| t.replace([0; TALLIES]));
        for (x, v) in e[4..].iter_mut().zip(a) {
            *x += v;
        }
    });
}

/// The tables, summed, as `stratum kind read items cycles instructions rows` lines.
fn write() {
    let Some(g) = global() else { return };
    let mut all = Table::new();
    for t in g.tables.lock().unwrap().iter() {
        for (k, v) in t.lock().unwrap().iter() {
            let e = all.entry(*k).or_default();
            for (x, y) in e.iter_mut().zip(v) {
                *x += y;
            }
        }
    }
    let tags = g.tags.lock().unwrap();
    let mut rows: Vec<_> = all.into_iter().collect();
    rows.sort_by_key(|(_, v)| std::cmp::Reverse(v[1]));
    let mut s = String::from("stratum\tkind\tread\titems\tcycles\tinstructions\trows");
    for n in TALLY_NAMES {
        s.push('\t');
        s.push_str(n);
    }
    s.push('\n');
    for ((tag, kind, read), v) in rows {
        let name = tags.get(tag as usize).map_or("?", String::as_str);
        s.push_str(name);
        for x in [u64::from(kind), u64::from(read)].into_iter().chain(v) {
            s.push_str(&format!("\t{x}"));
        }
        s.push('\n');
    }
    let _ = std::fs::write(&g.file, s);
}
