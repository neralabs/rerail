//! A key column of a launch: values the host holds, or values the host writes after the launch was begun.
//!
//! **Why staged.** A device lane builds its submission from the columns' sizes, not their values: it leases the
//! buffers, encodes the kernels and commits, and the command buffer waits at its head on a Metal shared event. The
//! host meanwhile writes the values straight into the buffers the device reads, and opens the [`Gate`]: the event is
//! signalled, and the device starts. So the launch's delay (the commit, the system's scheduling) runs while the host
//! does the setup, and nothing is copied twice (decision 0100 §9).
//!
//! **The gate.** A [`Gate`] opens once: when the caller calls [`Opener::open`], or when the [`Opener`] drops (an
//! error on the way, a panic), so a device waiting on it never waits for ever. A reader on the host ([`Col`]'s
//! `Deref`) waits until it is open. A column's values are only read after the gate opens: a staged column's writer
//! is the only one that touches it before.

use std::sync::{Arc, Condvar, Mutex};

/// A launch's key column.
#[derive(Clone)]
pub struct Col(Arc<Data>);

enum Data {
    Host(Arc<[i64]>),
    Staged(Staged),
}

/// A staged column's storage: a shared Metal buffer when the machine has one (the device reads it in place), else
/// host memory.
pub(crate) struct Staged {
    ptr: *mut i64,
    len: usize,
    gate: Arc<Gate>,
    #[allow(dead_code)]
    store: Store,
}

enum Store {
    #[cfg(all(feature = "gpu", target_os = "macos"))]
    Shared(crate::gpu::shared::Pooled),
    Host(#[allow(dead_code)] Box<[i64]>),
}

// SAFETY: the pointer is the store's own memory, kept as long as the column; it is written only by the column's one
// writer before the gate opens, and only read after.
unsafe impl Send for Staged {}
unsafe impl Sync for Staged {}

impl Col {
    /// The column's length (known before its values are).
    pub fn len(&self) -> usize {
        match &*self.0 {
            Data::Host(v) => v.len(),
            Data::Staged(s) => s.len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The gate of a staged column (`None`: the host holds its values).
    pub fn gate(&self) -> Option<&Arc<Gate>> {
        match &*self.0 {
            Data::Host(_) => None,
            Data::Staged(s) => Some(&s.gate),
        }
    }

    /// A staged column's shared buffer, which the device reads in place.
    #[cfg(all(feature = "gpu", target_os = "macos"))]
    pub(crate) fn shared(&self) -> Option<&crate::gpu::shared::Pooled> {
        match &*self.0 {
            Data::Staged(Staged { store: Store::Shared(p), .. }) => Some(p),
            _ => None,
        }
    }
}

impl std::ops::Deref for Col {
    type Target = [i64];
    /// The values: at once for the host's, after the gate opens for a staged column.
    fn deref(&self) -> &[i64] {
        match &*self.0 {
            Data::Host(v) => v,
            Data::Staged(s) => {
                s.gate.wait();
                // SAFETY: `len` values of the store, written before the gate opened and not written after.
                unsafe { std::slice::from_raw_parts(s.ptr, s.len) }
            }
        }
    }
}

impl From<Vec<i64>> for Col {
    fn from(v: Vec<i64>) -> Col {
        Col(Arc::new(Data::Host(v.into())))
    }
}

impl From<Arc<[i64]>> for Col {
    fn from(v: Arc<[i64]>) -> Col {
        Col(Arc::new(Data::Host(v)))
    }
}

impl PartialEq for Col {
    fn eq(&self, other: &Col) -> bool {
        **self == **other
    }
}

impl std::fmt::Debug for Col {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &*self.0 {
            Data::Host(v) => write!(f, "Col({} values)", v.len()),
            Data::Staged(s) => write!(f, "Col({} values, staged)", s.len),
        }
    }
}

/// When a batch's staged columns may be read: once, for good.
pub struct Gate {
    state: Mutex<GateState>,
    opened: Condvar,
}

#[derive(Default)]
struct GateState {
    open: bool,
    /// When it opened.
    at: Option<std::time::Instant>,
    /// The Metal event a device's command buffer waits on, made when a lane first needs it.
    #[cfg(all(feature = "gpu", target_os = "macos"))]
    event: Option<metal::SharedEvent>,
}

impl Gate {
    /// A closed gate, and what opens it.
    pub fn new() -> (Arc<Gate>, Opener) {
        let g = Arc::new(Gate { state: Mutex::new(GateState::default()), opened: Condvar::new() });
        (Arc::clone(&g), Opener(Some(g)))
    }

    pub fn is_open(&self) -> bool {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).open
    }

    /// When the gate opened, if it has.
    pub fn opened_at(&self) -> Option<std::time::Instant> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).at
    }

    /// Waits until the gate is open.
    pub fn wait(&self) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !s.open {
            s = self.opened.wait(s).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// A column of `len` values behind this gate, and its one writer.
    pub fn column(self: &Arc<Self>, len: usize) -> Result<(Col, Writer), String> {
        let (store, ptr) = Store::new(len)?;
        let col = Col(Arc::new(Data::Staged(Staged { ptr, len, gate: Arc::clone(self), store })));
        Ok((col.clone(), Writer { col, ptr, len }))
    }

    /// Makes `command` wait at its head until the gate opens, unless it is open already: whether it waits. Under the
    /// gate's lock, so an opening cannot fall between the check and the wait.
    #[cfg(all(feature = "gpu", target_os = "macos"))]
    pub(crate) fn hold(&self, command: &metal::CommandBufferRef, device: &metal::DeviceRef) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.open {
            return false;
        }
        let e = s.event.get_or_insert_with(|| device.new_shared_event());
        command.encode_wait_for_event(e, 1);
        true
    }

    fn open(&self) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if s.open {
            return;
        }
        s.open = true;
        s.at = Some(std::time::Instant::now());
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        if let Some(e) = &s.event {
            e.set_signaled_value(1);
        }
        self.opened.notify_all();
    }
}

/// Opens its gate when told to, or when dropped.
pub struct Opener(Option<Arc<Gate>>);

impl Opener {
    pub fn open(mut self) {
        if let Some(g) = self.0.take() {
            g.open();
        }
    }
}

impl Drop for Opener {
    fn drop(&mut self) {
        if let Some(g) = self.0.take() {
            g.open();
        }
    }
}

/// The one writer of a staged column, before its gate opens.
pub struct Writer {
    #[allow(dead_code)]
    col: Col,
    ptr: *mut i64,
    len: usize,
}

// SAFETY: the writer is the only one that touches the column's memory before the gate opens.
unsafe impl Send for Writer {}

impl Writer {
    /// The column's memory, to fill (its contents are a previous lease's until written).
    pub fn values(&mut self) -> &mut [i64] {
        // SAFETY: `len` values of the column's store; nothing reads them before the gate opens, and this writer is
        // the only one.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Store {
    fn new(len: usize) -> Result<(Store, *mut i64), String> {
        #[cfg(all(feature = "gpu", target_os = "macos"))]
        if crate::gpu::shared::get().is_some() {
            let p = crate::gpu::shared::lease((len * 8).max(8))?;
            let ptr = p.host().cast::<i64>();
            return Ok((Store::Shared(p), ptr));
        }
        let mut b = vec![0i64; len.max(1)].into_boxed_slice();
        let ptr = b.as_mut_ptr();
        Ok((Store::Host(b), ptr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A staged column is read after its writer fills it and the gate opens; a dropped opener opens it too.
    #[test]
    fn a_staged_column_is_read_once_its_gate_opens() {
        let (gate, opener) = Gate::new();
        let (col, mut w) = gate.column(5).unwrap();
        assert_eq!(col.len(), 5);
        assert!(!gate.is_open());
        let reader = std::thread::spawn({
            let col = col.clone();
            move || col.to_vec()
        });
        w.values().copy_from_slice(&[3, -1, 4, 1, -5]);
        drop(w);
        opener.open();
        assert_eq!(reader.join().unwrap(), vec![3, -1, 4, 1, -5]);
        let (gate, opener) = Gate::new();
        let (col, _w) = gate.column(0).unwrap();
        drop(opener);
        assert!(gate.is_open());
        assert!(col.is_empty());
    }
}
