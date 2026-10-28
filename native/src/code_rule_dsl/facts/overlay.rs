//! The in-memory overlay over the store (C7, C8, C18; docs/final-shape/03-architecture.md §4,
//! §10.2), adapted from Y1's layered map (Moti-Velz/facts-engine `spikes/facts-b/src/lmap.rs`):
//! frozen layers shared by reference, top first, under one mutable top map; an entry in a layer
//! overrides every layer below it, and the top holds only entries that differ from what lies
//! below. Here the bottom layer is SQLite, the frozen layers are columnar runs (`runs.rs`), and
//! the layers are written back to SQLite oldest first, off the push path:
//!
//! - The push path writes the top. When the top holds more than `Limits::top` bytes it freezes
//!   into one layer of runs (bindings, derivations, inputs, readers), sorted by key.
//! - A writer (a background thread, or `flush`) merges the oldest layer into SQLite in one
//!   transaction and only then drops it, so a read always finds a fact in the overlay or in SQLite.
//! - Frozen layers beyond `Limits::resident` bytes spill to flat files; a read of a spilled layer
//!   loads it again.
//! - When more than `Limits::behind` layers wait, the push path writes the oldest itself: the
//!   writer cannot fall arbitrarily far behind, and memory stays bounded (C8).

use super::Graph;
use super::runs::{self, Builder, Run};
use crate::content_store::{
    ABSENT, Binding, ContentStore, Derivation, DerivationRecord, StoreError, StoreResult, StoredDerivation, Sym,
};
use rustc_hash::FxHashMap;
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;

const BINDINGS: u32 = 0;
const DERIVATIONS: u32 = 1;
const INPUTS: u32 = 2;
const READERS: u32 = 3;
/// The name cell of an input without a name (an immutable digest).
const NO_NAME: u32 = u32::MAX;

/// Rough heap bytes per top entry: a hash map slot plus its value.
const BINDING_BYTES: usize = 4 + 32 + 16;
const DERIVATION_BYTES: usize = 4 + 8 + 16 + 32 + 16 + 24;
const INPUT_BYTES: usize = 4 + 4 + 32;
const READER_BYTES: usize = 4 + 8;

#[derive(Clone, Debug)]
pub(crate) struct Limits {
    /// Bytes the top may hold before it freezes into a layer of runs.
    pub top: usize,
    /// Bytes of frozen layers kept in memory; older ones spill to files under `spill`.
    pub resident: usize,
    /// Frozen layers that may wait for the writer before the push path writes them itself.
    pub behind: usize,
    pub spill: PathBuf,
    /// Whether a background thread writes layers as they freeze.
    pub background: bool,
}

/// What the overlay has done, for tests and the profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Counters {
    pub frozen: u64,
    pub spilled: u64,
    pub loaded: u64,
    pub written: u64,
    /// Layers the push path wrote itself because the writer was behind.
    pub pressed: u64,
}

#[derive(Default)]
struct Top {
    bindings: FxHashMap<Sym, [u8; 32]>,
    derivations: FxHashMap<Sym, Arc<StoredDerivation>>,
    readers: FxHashMap<Sym, Vec<Sym>>,
    bytes: usize,
}

impl Top {
    fn is_empty(&self) -> bool {
        self.bindings.is_empty() && self.derivations.is_empty()
    }
}

/// One frozen layer: its runs, in memory or spilled.
struct Layer {
    id: u64,
    resident: Resident,
}

#[derive(Clone)]
enum Resident {
    Memory(Arc<Runs>),
    Spilled(PathBuf),
}

struct Runs {
    /// name, digest lanes
    bindings: Run,
    /// output, producer lanes, status, cost
    derivations: Run,
    /// output, ordinal, name (or NO_NAME), digest lanes
    inputs: Run,
    /// name, output
    readers: Run,
}

impl Runs {
    fn bytes(&self) -> usize {
        self.bindings.bytes() + self.derivations.bytes() + self.inputs.bytes() + self.readers.bytes()
    }

    fn binding(&self, name: Sym) -> Option<[u8; 32]> {
        self.bindings.find(&[name]).map(|row| runs::digest(self.bindings.row(row).skip(1)))
    }

    fn derivation(&self, output: Sym) -> Option<StoredDerivation> {
        let row = self.derivations.find(&[output])?;
        let cells = self.derivations.row(row).collect::<Vec<_>>();
        let inputs = self
            .inputs
            .range(&[output])
            .map(|row| {
                let cells = self.inputs.row(row).collect::<Vec<_>>();
                let name = (cells[2] != NO_NAME).then_some(cells[2]);
                (name, runs::digest(cells[3..].iter().copied()))
            })
            .collect();
        Some(StoredDerivation {
            producer: runs::digest(cells[1..9].iter().copied()),
            status: i64::from(cells[9]),
            cost: i64::from(cells[10]),
            inputs,
        })
    }

    fn readers(&self, name: Sym) -> impl Iterator<Item = Sym> + '_ {
        self.readers.range(&[name]).map(|row| self.readers.cell(row, 1))
    }

    fn encode(&self, path: &std::path::Path) -> StoreResult<()> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        for run in [&self.bindings, &self.derivations, &self.inputs, &self.readers] {
            run.encode(&mut file)?;
        }
        std::io::Write::flush(&mut file)?;
        Ok(())
    }

    fn decode(path: &std::path::Path) -> StoreResult<Self> {
        let mut file = std::io::BufReader::new(std::fs::File::open(path)?);
        Ok(Runs {
            bindings: Run::decode(&mut file)?,
            derivations: Run::decode(&mut file)?,
            inputs: Run::decode(&mut file)?,
            readers: Run::decode(&mut file)?,
        })
    }

    /// The layer as the store's bulk write takes it.
    fn rows(&self) -> (Vec<(Sym, [u8; 32])>, Vec<(Sym, StoredDerivation)>) {
        let bindings = (0..self.bindings.len())
            .map(|row| (self.bindings.cell(row, 0), runs::digest(self.bindings.row(row).skip(1))))
            .collect();
        let derivations = (0..self.derivations.len())
            .map(|row| {
                let output = self.derivations.cell(row, 0);
                (output, self.derivation(output).expect("a layer's own derivation"))
            })
            .collect();
        (bindings, derivations)
    }
}

struct State {
    top: Top,
    /// Oldest first; the writer takes the front.
    frozen: VecDeque<Layer>,
    next: u64,
    /// The layer being written, which must neither spill nor be taken twice.
    writing: Option<u64>,
    stop: bool,
    failed: Option<String>,
    counters: Counters,
}

struct Shared {
    store: Arc<ContentStore>,
    limits: Limits,
    state: Mutex<State>,
    wake: Condvar,
}

/// The overlay: a `Graph` whose writes reach SQLite eventually.
pub(crate) struct Overlay {
    shared: Arc<Shared>,
    writer: Option<JoinHandle<()>>,
}

impl Overlay {
    pub(crate) fn new(store: Arc<ContentStore>, limits: Limits) -> StoreResult<Self> {
        std::fs::create_dir_all(&limits.spill)?;
        let background = limits.background;
        let shared = Arc::new(Shared {
            store,
            limits,
            state: Mutex::new(State {
                top: Top::default(),
                frozen: VecDeque::new(),
                next: 0,
                writing: None,
                stop: false,
                failed: None,
                counters: Counters::default(),
            }),
            wake: Condvar::new(),
        });
        let writer = background.then(|| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || shared.write_in_background())
        });
        Ok(Self { shared, writer })
    }

    pub(crate) fn counters(&self) -> Counters {
        self.shared.lock().counters
    }

    /// Bytes the top and the resident layers hold.
    pub(crate) fn resident_bytes(&self) -> usize {
        let state = self.shared.lock();
        state.top.bytes + resident_layer_bytes(&state)
    }

    /// Layers frozen and not yet written.
    pub(crate) fn behind(&self) -> usize {
        self.shared.lock().frozen.len()
    }

    /// Freezes the top and writes every layer: afterwards SQLite holds everything.
    pub(crate) fn flush(&self) -> StoreResult<()> {
        let mut state = self.shared.lock();
        self.shared.freeze(&mut state);
        while !state.frozen.is_empty() {
            state = self.shared.write_front(state)?;
        }
        failed(&state)
    }

    fn after_write(&self) -> StoreResult<()> {
        let mut state = self.shared.lock();
        failed(&state)?;
        let limits = &self.shared.limits;
        if state.top.bytes > limits.top {
            self.shared.freeze(&mut state);
            self.shared.wake.notify_all();
        }
        self.shared.spill(&mut state)?;
        while state.frozen.len() > limits.behind {
            state.counters.pressed += 1;
            state = self.shared.write_front(state)?;
        }
        failed(&state)
    }

    fn binding_of(&self, name: Sym) -> StoreResult<Option<[u8; 32]>> {
        let layers = {
            let state = self.shared.lock();
            if let Some(digest) = state.top.bindings.get(&name) {
                return Ok(Some(*digest));
            }
            newest_first(&state)
        };
        for layer in layers {
            if let Some(digest) = self.shared.load(layer)?.and_then(|runs| runs.binding(name)) {
                return Ok(Some(digest));
            }
        }
        self.shared.store.binding_of(name)
    }

    fn derivation_of(&self, output: Sym) -> StoreResult<Option<Arc<StoredDerivation>>> {
        let layers = {
            let state = self.shared.lock();
            if let Some(derivation) = state.top.derivations.get(&output) {
                return Ok(Some(Arc::clone(derivation)));
            }
            newest_first(&state)
        };
        for layer in layers {
            if let Some(derivation) = self.shared.load(layer)?.and_then(|runs| runs.derivation(output)) {
                return Ok(Some(Arc::new(derivation)));
            }
        }
        Ok(self.shared.store.derivation_of(output)?.map(Arc::new))
    }

    /// Binds `name` in the top unless it is bound to `digest` already; whether it changed (a
    /// first binding to `ABSENT` is not a change, as in the store).
    fn bind(&self, name: Sym, digest: [u8; 32]) -> StoreResult<bool> {
        let held = self.binding_of(name)?;
        if held == Some(digest) {
            return Ok(false);
        }
        let mut state = self.shared.lock();
        if state.top.bindings.insert(name, digest).is_none() {
            state.top.bytes += BINDING_BYTES;
        }
        Ok(held.is_some() || digest != ABSENT)
    }
}

#[cfg(test)]
impl Overlay {
    /// A crash: the writer stops where it is and nothing still in memory or spilled is written.
    /// Returns what the overlay did, as it stopped.
    pub(crate) fn crash(mut self) -> Counters {
        self.shared.lock().failed = Some("crashed".to_owned());
        self.shared.wake.notify_all();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        let counters = self.counters();
        std::mem::forget(self);
        counters
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        let _ = self.flush();
        self.shared.lock().stop = true;
        self.shared.wake.notify_all();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

impl Graph for Overlay {
    fn store(&self) -> &ContentStore {
        &self.shared.store
    }

    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        let names = self.shared.store.intern_atoms(&bindings.iter().map(|b| b.name.as_str()).collect::<Vec<_>>())?;
        let mut changed = Vec::new();
        for (binding, name) in bindings.iter().zip(names) {
            if self.bind(name, binding.digest)? {
                changed.push(binding.name.clone());
            }
        }
        self.after_write()?;
        Ok(changed)
    }

    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>> {
        let mut values = Vec::new();
        for record in records {
            values.push(record.output.as_str());
            values.extend(record.inputs.iter().filter_map(|input| input.name.as_deref()));
        }
        let atoms = self.shared.store.intern_atoms(&values)?;
        let mut atoms = atoms.into_iter();
        let mut changed = Vec::new();
        for record in records {
            let output = atoms.next().expect("one atom per output");
            let inputs = record
                .inputs
                .iter()
                .map(|input| (input.name.as_ref().map(|_| atoms.next().expect("one atom per named input")), input.digest))
                .collect::<Vec<_>>();
            for (status, what) in [(record.status, "status"), (record.cost, "cost")] {
                if u32::try_from(status).is_err() {
                    return Err(StoreError::Invalid(format!("derivation {what} is outside the run cells")));
                }
            }
            {
                let mut state = self.shared.lock();
                let top = &mut state.top;
                let mut bytes = INPUT_BYTES * inputs.len();
                for name in inputs.iter().filter_map(|(name, _)| *name) {
                    let readers = top.readers.entry(name).or_default();
                    if let Err(at) = readers.binary_search(&output) {
                        readers.insert(at, output);
                        bytes += READER_BYTES;
                    }
                }
                let derivation = StoredDerivation { producer: record.producer, status: record.status, cost: record.cost, inputs };
                if top.derivations.insert(output, Arc::new(derivation)).is_none() {
                    bytes += DERIVATION_BYTES;
                }
                top.bytes += bytes;
            }
            if self.bind(output, record.digest)? {
                changed.push(record.output.clone());
            }
        }
        self.after_write()?;
        Ok(changed)
    }

    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        match self.shared.store.atom(name)? {
            Some(name) => self.binding_of(name),
            None => Ok(None),
        }
    }

    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        let mut frontier = BTreeSet::new();
        for name in names {
            let Some(name) = self.shared.store.atom(name)? else {
                continue;
            };
            // Readers are append-only in every layer: a derivation re-recorded without this
            // input is a candidate still, and is dropped by checking what it reads now.
            let mut candidates = BTreeSet::new();
            let layers = {
                let state = self.shared.lock();
                candidates.extend(state.top.readers.get(&name).into_iter().flatten().copied());
                newest_first(&state)
            };
            for layer in layers {
                if let Some(runs) = self.shared.load(layer)? {
                    candidates.extend(runs.readers(name));
                }
            }
            candidates.extend(self.shared.store.readers_of(name)?);
            for output in candidates {
                let Some(derivation) = self.derivation_of(output)? else {
                    continue;
                };
                if !derivation.inputs.iter().any(|(read, _)| *read == Some(name)) {
                    continue;
                }
                let output = self
                    .shared
                    .store
                    .atom_value(output)?
                    .ok_or_else(|| StoreError::Corrupt("a derivation's output is not an atom".to_owned()))?;
                frontier.insert(Derivation {
                    output,
                    producer: derivation.producer,
                    status: derivation.status,
                    cost: derivation.cost,
                });
            }
        }
        Ok(frontier.into_iter().collect())
    }

    fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        let Some(output) = self.shared.store.atom(output)? else {
            return Ok(false);
        };
        let Some(derivation) = self.derivation_of(output)? else {
            return Ok(false);
        };
        for (name, digest) in &derivation.inputs {
            if let Some(name) = name
                && self.binding_of(*name)?.unwrap_or(ABSENT) != *digest
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn begin_push(&self) -> StoreResult<()> {
        // The previous push's end flushed, so no layer of an earlier push is still unwritten.
        self.shared.store.begin_push()
    }

    fn end_push(&self) -> StoreResult<()> {
        self.flush()?;
        self.shared.store.end_push()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("overlay mutex poisoned")
    }

    /// A layer's runs, loading a spilled layer from its file (called without the lock). None: the
    /// layer was written and its file removed since the caller saw it, so SQLite holds its rows.
    fn load(&self, layer: Resident) -> StoreResult<Option<Arc<Runs>>> {
        match layer {
            Resident::Memory(runs) => Ok(Some(runs)),
            Resident::Spilled(path) => {
                let runs = match Runs::decode(&path) {
                    Ok(runs) => runs,
                    Err(StoreError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error),
                };
                self.lock().counters.loaded += 1;
                Ok(Some(Arc::new(runs)))
            }
        }
    }

    /// The top becomes the newest frozen layer.
    fn freeze(&self, state: &mut State) {
        if state.top.is_empty() {
            return;
        }
        let top = std::mem::take(&mut state.top);
        let mut bindings = Builder::new(BINDINGS, 9, 1);
        for (name, digest) in &top.bindings {
            let mut row = vec![*name];
            row.extend(runs::lanes(digest));
            bindings.push(&row);
        }
        let mut derivations = Builder::new(DERIVATIONS, 11, 1);
        let mut inputs = Builder::new(INPUTS, 11, 2);
        for (output, derivation) in &top.derivations {
            let mut row = vec![*output];
            row.extend(runs::lanes(&derivation.producer));
            // Checked when recorded.
            row.extend([derivation.status as u32, derivation.cost as u32]);
            derivations.push(&row);
            for (ordinal, (name, digest)) in derivation.inputs.iter().enumerate() {
                let mut row = vec![*output, ordinal as u32, name.unwrap_or(NO_NAME)];
                row.extend(runs::lanes(digest));
                inputs.push(&row);
            }
        }
        let mut readers = Builder::new(READERS, 2, 2);
        for (name, outputs) in &top.readers {
            for output in outputs {
                readers.push(&[*name, *output]);
            }
        }
        let runs = Runs {
            bindings: bindings.seal(),
            derivations: derivations.seal(),
            inputs: inputs.seal(),
            readers: readers.seal(),
        };
        let id = state.next;
        state.next += 1;
        state.frozen.push_back(Layer { id, resident: Resident::Memory(Arc::new(runs)) });
        state.counters.frozen += 1;
    }

    /// Spills the oldest resident layers (never the one being written) until the rest fit.
    fn spill(&self, state: &mut State) -> StoreResult<()> {
        while resident_layer_bytes(state) > self.limits.resident {
            let writing = state.writing;
            let Some(layer) = state
                .frozen
                .iter_mut()
                .find(|layer| Some(layer.id) != writing && matches!(layer.resident, Resident::Memory(_)))
            else {
                return Ok(());
            };
            let Resident::Memory(runs) = &layer.resident else { unreachable!() };
            let path = self.limits.spill.join(format!("layer-{}.run", layer.id));
            runs.encode(&path)?;
            layer.resident = Resident::Spilled(path);
            state.counters.spilled += 1;
        }
        Ok(())
    }

    /// Writes the oldest layer to SQLite and then drops it; waits instead while another thread
    /// writes it. The lock is released during the write.
    fn write_front<'a>(&'a self, mut state: MutexGuard<'a, State>) -> StoreResult<MutexGuard<'a, State>> {
        if state.writing.is_some() {
            return Ok(self.wake.wait(state).expect("overlay mutex poisoned"));
        }
        let Some(front) = state.frozen.front() else {
            return Ok(state);
        };
        let (id, resident) = (front.id, front.resident.clone());
        state.writing = Some(id);
        drop(state);
        // The layer being written is never spilled or removed, so it loads.
        let written = self.load(resident).and_then(|runs| {
            let (bindings, derivations) = runs.expect("the layer being written").rows();
            self.store.write_derivation_layer(&bindings, &derivations)
        });
        let mut state = self.lock();
        state.writing = None;
        self.wake.notify_all();
        if let Err(error) = written {
            state.failed = Some(error.to_string());
            return Err(error);
        }
        let layer = state.frozen.pop_front().expect("the written layer");
        debug_assert_eq!(layer.id, id);
        if let Resident::Spilled(path) = layer.resident {
            let _ = std::fs::remove_file(path);
        }
        state.counters.written += 1;
        Ok(state)
    }

    fn write_in_background(&self) {
        let mut state = self.lock();
        loop {
            if state.failed.is_some() || (state.stop && state.frozen.is_empty()) {
                return;
            }
            if state.frozen.is_empty() || state.writing.is_some() {
                state = self.wake.wait(state).expect("overlay mutex poisoned");
                continue;
            }
            state = match self.write_front(state) {
                Ok(state) => state,
                Err(_) => return,
            };
        }
    }
}

/// The frozen layers, newest first, as they are now.
fn newest_first(state: &State) -> Vec<Resident> {
    state.frozen.iter().rev().map(|layer| layer.resident.clone()).collect()
}

fn resident_layer_bytes(state: &State) -> usize {
    state
        .frozen
        .iter()
        .map(|layer| match &layer.resident {
            Resident::Memory(runs) => runs.bytes(),
            Resident::Spilled(_) => 0,
        })
        .sum()
}

fn failed(state: &State) -> StoreResult<()> {
    match &state.failed {
        Some(error) => Err(StoreError::Invalid(format!("the overlay writer failed: {error}"))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
